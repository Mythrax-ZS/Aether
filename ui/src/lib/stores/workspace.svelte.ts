// The workspace: tabs, split panes, and the connections they use.

import { api, toErrorInfo, type ErrorInfo } from '$lib/api';
import type { ConnectionId, Host, HostId, SessionSummary, TermSize, TerminalId, TerminalState } from '$lib/bindings';
import {
  balance, deserialize, findLeaf, hostTarget, leaves, neighbourAfterRemoval, newLeaf, removeLeaf, serialize, setRatio,
  splitLeaf, uid, type Leaf, type PaneNode, type SavedNode, type SplitDir,
} from '$lib/util/panes';
import { hosts } from './hosts.svelte';
import { prompts } from './prompts.svelte';
import { sessions } from './sessions.svelte';
import { settings } from './settings.svelte';

export interface TerminalTab {
  kind: 'terminal';
  id: string;
  title: string;
  /** The user gave this tab a name; it wins over the remote program's window title. */
  renamed?: boolean;
  root: PaneNode;
  activePane: string;
}

export interface SftpTab {
  kind: 'sftp';
  id: string;
  title: string;
  connectionId: ConnectionId;
  host: Host;
}

export type Tab = TerminalTab | SftpTab;

/** How a pane's (or tab's, or host's) connection is doing, for status dots. */
export type LinkStatus = 'connected' | 'connecting' | 'warning' | 'error';

/** What the UI knows about a pane beyond its layout position. */
export interface PaneInfo {
  /** `new` panes start as `idle`, become `opening` while the session is requested. */
  phase: 'idle' | 'opening' | 'ready' | 'error';
  error: ErrorInfo | null;
  /** Window title set by the remote program (OSC 0/2). */
  title: string | null;
  /** Working directory reported by the shell (OSC 7). */
  cwd: string | null;
  terminal: TerminalState | null;
  /** Output arrived while the pane wasn't visible. */
  activity: boolean;
  /** Current size of the terminal grid. */
  size: { cols: number; rows: number } | null;
}

interface SavedWorkspace {
  version: 1;
  tabs: { root: SavedNode }[];
  active: number;
}

class Workspace {
  tabs = $state<Tab[]>([]);
  activeId = $state<string | null>(null);
  info = $state<Record<string, PaneInfo>>({});

  #mru: string[] = [];

  active = $derived<Tab | null>(this.tabs.find((t) => t.id === this.activeId) ?? null);

  activeTerminalTab = $derived<TerminalTab | null>(this.active?.kind === 'terminal' ? this.active : null);

  /** The focused pane of the active tab, if it's a terminal tab. */
  activeLeaf = $derived.by<Leaf | null>(() => {
    const t = this.activeTerminalTab;
    return t ? (findLeaf(t.root, t.activePane) ?? null) : null;
  });

  /** Connection backing whatever is in front (terminal pane or SFTP tab). */
  activeConnection = $derived.by<ConnectionId | null>(() => {
    const a = this.active;
    if (!a) return null;
    return a.kind === 'sftp' ? a.connectionId : (this.activeLeaf?.connectionId ?? null);
  });

  /** Live connection state per saved host, for the dots in the sidebar (best state wins). */
  hostStatus = $derived.by(() => {
    const order: LinkStatus[] = ['connected', 'warning', 'connecting', 'error'];
    const out = new Map<string, LinkStatus>();
    for (const leaf of this.allLeaves()) {
      if (!leaf.hostId) continue;
      const s = this.leafStatus(leaf);
      if (!s) continue;
      const prev = out.get(leaf.hostId);
      if (!prev || order.indexOf(s) < order.indexOf(prev)) out.set(leaf.hostId, s);
    }
    return out;
  });

  // ── lookup ─────────────────────────────────────────────────────────────────

  /** Status of one pane. Reads only (safe inside `$derived`). */
  leafStatus(leaf: Leaf): LinkStatus | null {
    const info = this.info[leaf.id];
    if (info?.phase === 'error') return 'error';
    if (!leaf.connectionId) return 'connecting';
    const state = sessions.view(leaf.connectionId)?.state;
    switch (state?.type) {
      case 'connected':
        return 'connected';
      case 'reconnecting':
        return 'warning';
      case 'disconnected':
        return 'error';
      case 'closed':
        return null;
      default:
        return 'connecting';
    }
  }

  /** Worst state across a tab's panes (so a broken split is still visible on the tab). */
  tabStatus(tab: Tab): LinkStatus | null {
    if (tab.kind === 'sftp') {
      const s = sessions.view(tab.connectionId)?.state.type;
      return s === 'connected' ? 'connected' : s === 'reconnecting' ? 'warning' : s === 'disconnected' ? 'error' : 'connecting';
    }
    const order: LinkStatus[] = ['error', 'warning', 'connecting', 'connected'];
    let worst: LinkStatus | null = null;
    for (const leaf of leaves(tab.root)) {
      const s = this.leafStatus(leaf);
      if (s && (worst === null || order.indexOf(s) < order.indexOf(worst))) worst = s;
    }
    return worst;
  }

  paneInfo(leafId: string): PaneInfo {
    if (!this.info[leafId]) {
      this.info[leafId] = { phase: 'idle', error: null, title: null, cwd: null, terminal: null, activity: false, size: null };
    }
    return this.info[leafId];
  }

  leaf(leafId: string): Leaf | undefined {
    for (const t of this.tabs) if (t.kind === 'terminal') {
      const l = findLeaf(t.root, leafId);
      if (l) return l;
    }
    return undefined;
  }

  tabOfLeaf(leafId: string): TerminalTab | undefined {
    return this.tabs.find((t): t is TerminalTab => t.kind === 'terminal' && !!findLeaf(t.root, leafId));
  }

  allLeaves(): Leaf[] {
    return this.tabs.flatMap((t) => (t.kind === 'terminal' ? leaves(t.root) : []));
  }

  /** How many panes/tabs currently use a connection. */
  connectionUsers(id: ConnectionId): number {
    return (
      this.allLeaves().filter((l) => l.connectionId === id).length +
      this.tabs.filter((t) => t.kind === 'sftp' && t.connectionId === id).length
    );
  }

  tabTitle(tab: Tab): string {
    if (tab.kind === 'sftp' || tab.renamed) return tab.title;
    const leaf = findLeaf(tab.root, tab.activePane);
    const info = leaf ? this.info[leaf.id] : undefined;
    return info?.title || leaf?.host?.label || tab.title || leaf?.target || '';
  }

  // ── opening ────────────────────────────────────────────────────────────────

  /** Panes get their UI bookkeeping up front, so templates can read `info` without writing to it. */
  #register(root: PaneNode): void {
    for (const leaf of leaves(root)) this.paneInfo(leaf.id);
  }

  #addTerminalTab(root: PaneNode, title: string): TerminalTab {
    this.#register(root);
    const tab: TerminalTab = { kind: 'terminal', id: uid('tab-'), title, root, activePane: leaves(root)[0].id };
    this.tabs.push(tab);
    this.activate(tab.id);
    return tab;
  }

  openHost(host: Host): TerminalTab {
    return this.#addTerminalTab(newLeaf({ hostId: host.id, host }), host.label || host.hostname);
  }

  /** Opens `user@host[:port]` typed into quick connect. */
  openTarget(target: string): TerminalTab {
    return this.#addTerminalTab(newLeaf({ target: target.trim() }), target.trim());
  }

  /** A new tab to the same place as `tabId`'s active pane. */
  duplicateTab(tabId: string): void {
    const tab = this.tabs.find((t) => t.id === tabId);
    if (!tab || tab.kind !== 'terminal') return;
    const leaf = findLeaf(tab.root, tab.activePane);
    if (leaf?.host) this.#addTerminalTab(newLeaf({ hostId: leaf.hostId, host: leaf.host, target: leaf.hostId ? null : hostTarget(leaf.host) }), tab.title);
  }

  /** Shows a session that another window handed over (tab tear-off). */
  adopt(summary: SessionSummary, terminalId: TerminalId, hostId: HostId | null = null): void {
    const host = hostId ? hosts.get(hostId) : undefined;
    const leaf = newLeaf({ hostId: host ? hostId : null, host, target: summary.target, connectionId: summary.id, terminalId, open: 'attach' });
    const view = sessions.ensure(summary.id);
    view.state = summary.state;
    if (summary.state.type === 'connected') view.everConnected = true;
    this.#addTerminalTab(leaf, summary.label || summary.target);
    this.paneInfo(leaf.id).phase = 'ready';
  }

  /** Called by a terminal view once it knows its size: creates the session or extra shell. */
  async openLeaf(leafId: string, size: TermSize): Promise<void> {
    const leaf = this.leaf(leafId);
    if (!leaf || leaf.open === 'attach') return;
    const info = this.paneInfo(leafId);
    if (info.phase === 'opening') return;
    info.phase = 'opening';
    info.error = null;
    try {
      if (leaf.open === 'split' && leaf.splitOf && leaf.host) {
        leaf.terminalId = await api.sessions.newTerminal(leaf.splitOf, size, leaf.host);
        leaf.connectionId = leaf.splitOf;
      } else {
        const opened = leaf.hostId
          ? await api.sessions.open({ hostId: leaf.hostId, size })
          : await api.sessions.quickConnect({ target: leaf.target ?? (leaf.host ? hostTarget(leaf.host) : ''), size });
        leaf.connectionId = opened.connectionId;
        leaf.terminalId = opened.terminalId;
        leaf.host = opened.host;
        leaf.hostId = opened.saved ? opened.host.id : null;
        sessions.ensure(opened.connectionId);
        // Refresh the tab title now that we know the real label.
        const tab = this.tabOfLeaf(leafId);
        if (tab && tab.root.kind === 'leaf') tab.title = opened.host.label || opened.host.hostname;
      }
      leaf.open = 'attach';
      info.phase = 'ready';
    } catch (e) {
      info.phase = 'error';
      info.error = toErrorInfo(e);
    }
  }

  /**
   * After a failed first connection: open a brand-new session in the same pane. Unlike
   * "reconnect" (which replays the old connection settings) this picks up edits made to the
   * host in the meantime — the usual fix for a wrong password or key.
   */
  async retryFresh(leafId: string): Promise<void> {
    const leaf = this.leaf(leafId);
    if (!leaf) return;
    const old = leaf.connectionId;
    const oldTerminal = leaf.terminalId;
    leaf.connectionId = null;
    leaf.terminalId = null;
    leaf.open = leaf.splitOf ? 'split' : 'new';
    if (leaf.hostId) leaf.host = hosts.get(leaf.hostId) ?? leaf.host;
    const info = this.paneInfo(leafId);
    info.phase = 'idle';
    info.error = null;
    info.terminal = null;
    if (old) {
      prompts.forget(old);
      sessions.remove(old);
      if (oldTerminal) await api.sessions.closeTerminal(old, oldTerminal).catch(() => {});
      if (this.connectionUsers(old) === 0) await api.sessions.close(old).catch(() => {});
    }
  }

  // ── splitting ──────────────────────────────────────────────────────────────

  /** Splits the active pane. The new pane shares the connection (no second login). */
  splitActive(dir: SplitDir): void {
    const tab = this.activeTerminalTab;
    if (!tab) return;
    const leaf = findLeaf(tab.root, tab.activePane);
    if (!leaf) return;
    const live = leaf.connectionId && sessions.isConnected(leaf.connectionId);
    const fresh = newLeaf(
      live
        ? { hostId: leaf.hostId, host: leaf.host, open: 'split', splitOf: leaf.connectionId }
        : { hostId: leaf.hostId, host: leaf.host, target: leaf.target },
    );
    this.paneInfo(fresh.id);
    tab.root = splitLeaf(tab.root, leaf.id, dir, fresh);
    tab.activePane = fresh.id;
  }

  setSplitRatio(tabId: string, splitId: string, ratio: number): void {
    const tab = this.tabs.find((t) => t.id === tabId);
    if (tab?.kind === 'terminal') tab.root = setRatio(tab.root, splitId, ratio);
  }

  balanceActive(): void {
    const tab = this.activeTerminalTab;
    if (tab) tab.root = balance(tab.root);
  }

  focusPane(leafId: string): void {
    const tab = this.tabOfLeaf(leafId);
    if (!tab) return;
    tab.activePane = leafId;
    this.paneInfo(leafId).activity = false;
  }

  // ── closing ────────────────────────────────────────────────────────────────

  async #release(leaf: Leaf): Promise<void> {
    this.info[leaf.id] && delete this.info[leaf.id];
    const { connectionId, terminalId } = leaf;
    if (!connectionId) return;
    if (terminalId) await api.sessions.closeTerminal(connectionId, terminalId).catch(() => {});
    if (this.connectionUsers(connectionId) === 0) {
      prompts.forget(connectionId);
      sessions.remove(connectionId);
      await api.sessions.close(connectionId).catch(() => {});
    }
  }

  closePane(leafId: string): void {
    const tab = this.tabOfLeaf(leafId);
    if (!tab) return;
    const leaf = findLeaf(tab.root, leafId)!;
    const next = neighbourAfterRemoval(tab.root, leafId);
    const root = removeLeaf(tab.root, leafId);
    if (!root) {
      this.closeTab(tab.id);
      return;
    }
    tab.root = root;
    if (tab.activePane === leafId && next) tab.activePane = next;
    void this.#release(leaf);
  }

  closeTab(tabId: string): void {
    const i = this.tabs.findIndex((t) => t.id === tabId);
    if (i < 0) return;
    const tab = this.tabs[i];
    this.tabs.splice(i, 1);
    this.#mru = this.#mru.filter((id) => id !== tabId);
    if (tab.kind === 'terminal') leaves(tab.root).forEach((l) => void this.#release(l));
    else if (this.connectionUsers(tab.connectionId) === 0) void api.sessions.close(tab.connectionId).catch(() => {});
    if (this.activeId === tabId) {
      const fallback = this.#mru[0] ?? this.tabs[Math.min(i, this.tabs.length - 1)]?.id ?? null;
      this.activeId = fallback;
    }
  }

  closeOthers(tabId: string): void {
    this.tabs.filter((t) => t.id !== tabId).forEach((t) => this.closeTab(t.id));
  }

  closeToRight(tabId: string): void {
    const i = this.tabs.findIndex((t) => t.id === tabId);
    this.tabs.slice(i + 1).forEach((t) => this.closeTab(t.id));
  }

  closeAll(): void {
    [...this.tabs].forEach((t) => this.closeTab(t.id));
  }

  /** Is anything in this tab connected (so closing it deserves a confirmation)? */
  hasLiveSession(tab: Tab): boolean {
    if (tab.kind === 'sftp') return sessions.isConnected(tab.connectionId);
    return leaves(tab.root).some((l) => sessions.isConnected(l.connectionId));
  }

  // ── activation & order ─────────────────────────────────────────────────────

  activate(tabId: string): void {
    if (!this.tabs.some((t) => t.id === tabId)) return;
    if (this.activeId && this.activeId !== tabId) {
      this.#mru = [this.activeId, ...this.#mru.filter((id) => id !== this.activeId && id !== tabId)].slice(0, 20);
    }
    this.activeId = tabId;
    const tab = this.tabs.find((t) => t.id === tabId);
    if (tab?.kind === 'terminal') this.paneInfo(tab.activePane).activity = false;
  }

  activateIndex(i: number): void {
    const tab = i === 8 ? this.tabs[this.tabs.length - 1] : this.tabs[i];
    if (tab) this.activate(tab.id);
  }

  cycle(delta: 1 | -1): void {
    if (this.tabs.length < 2) return;
    const i = this.tabs.findIndex((t) => t.id === this.activeId);
    this.activate(this.tabs[(i + delta + this.tabs.length) % this.tabs.length].id);
  }

  moveTab(from: number, to: number): void {
    if (from === to || from < 0 || to < 0 || from >= this.tabs.length || to >= this.tabs.length) return;
    const [tab] = this.tabs.splice(from, 1);
    this.tabs.splice(to, 0, tab);
  }

  rename(tabId: string, title: string): void {
    const tab = this.tabs.find((t) => t.id === tabId);
    if (!tab) return;
    const next = title.trim();
    if (next && tab.kind === 'terminal') {
      tab.title = next;
      tab.renamed = true;
    } else if (next) {
      tab.title = next;
    }
  }

  // ── SFTP ───────────────────────────────────────────────────────────────────

  openSftp(connectionId: ConnectionId, host: Host): void {
    const existing = this.tabs.find((t) => t.kind === 'sftp' && t.connectionId === connectionId);
    if (existing) {
      this.activate(existing.id);
      return;
    }
    const tab: SftpTab = { kind: 'sftp', id: uid('tab-'), title: `${host.label || host.hostname} · Files`, connectionId, host };
    this.tabs.push(tab);
    this.activate(tab.id);
  }

  // ── activity ───────────────────────────────────────────────────────────────

  noteOutput(leafId: string): void {
    const tab = this.tabOfLeaf(leafId);
    if (!tab) return;
    const visible = tab.id === this.activeId && tab.activePane === leafId;
    if (!visible || !settings.loaded) this.paneInfo(leafId).activity = true;
  }

  // ── persistence ────────────────────────────────────────────────────────────

  snapshot(): SavedWorkspace {
    return {
      version: 1,
      tabs: this.tabs.filter((t): t is TerminalTab => t.kind === 'terminal').map((t) => ({ root: serialize(t.root, t.activePane) })),
      active: Math.max(0, this.tabs.filter((t) => t.kind === 'terminal').findIndex((t) => t.id === this.activeId)),
    };
  }

  /** Reopens saved tabs (each connects afresh). */
  restore(saved: unknown): number {
    const s = saved as SavedWorkspace | null;
    if (!s || s.version !== 1 || !Array.isArray(s.tabs)) return 0;
    let count = 0;
    for (const entry of s.tabs) {
      const { node, active } = deserialize(entry.root);
      const first = leaves(node)[0];
      const known = first?.hostId ? hosts.get(first.hostId) : undefined;
      // Skip panes whose host was deleted since.
      if (first?.hostId && !known) continue;
      const title = known?.label ?? first?.target ?? 'Session';
      const tab: TerminalTab = { kind: 'terminal', id: uid('tab-'), title, root: node, activePane: active ?? first.id };
      for (const l of leaves(node)) if (l.hostId) l.host = hosts.get(l.hostId) ?? null;
      this.#register(node);
      this.tabs.push(tab);
      count++;
    }
    const target = this.tabs[Math.min(s.active, this.tabs.length - 1)];
    if (target) this.activeId = target.id;
    return count;
  }
}

export const workspace = new Workspace();
