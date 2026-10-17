// State and operations for one SFTP tab: two panes (this PC and the server), selection,
// navigation history, and starting transfers.

import { SvelteSet } from 'svelte/reactivity';
import { api, toErrorInfo, type ErrorInfo } from '$lib/api';
import type { ConflictPolicy, ConnectionId, DiskUsage, PlanSummary } from '$lib/bindings';
import { settings } from '$lib/stores/settings.svelte';
import { ui } from '$lib/stores/ui.svelte';
import { plural } from '$lib/util/format';
import { findByPrefix, invalidName, range, sortEntries, uniqueName, type SortKey } from './paths';
import { localProvider, remoteProvider, type FileItem, type FileProvider } from './provider';

export type Side = 'local' | 'remote';

/** One side of the browser. */
export class PaneState {
  cwd = $state('');
  items = $state<FileItem[]>([]);
  loading = $state(false);
  error = $state<ErrorInfo | null>(null);
  disk = $state<DiskUsage | null>(null);

  /** Selected paths. */
  selection = new SvelteSet<string>();
  /** The row keyboard commands act on. */
  cursor = $state<string | null>(null);
  sortKey = $state<SortKey>('name');
  ascending = $state(true);
  filter = $state('');
  showHidden = $state(false);

  /** A name being typed in place: renaming an item, or naming a new folder. */
  editing = $state<{ path: string | null; value: string; error: string | null } | null>(null);

  #anchor: string | null = null;
  #back: string[] = [];
  #forward: string[] = [];
  #token = 0;
  #typed = '';
  #typedTimer: ReturnType<typeof setTimeout> | undefined;

  canGoBack = $state(false);
  canGoForward = $state(false);

  /** What is shown: filtered, hidden files removed unless asked for, sorted. */
  rows = $derived.by(() => {
    const q = this.filter.trim().toLowerCase();
    const list = this.items.filter((i) => (this.showHidden || !i.hidden) && (!q || i.name.toLowerCase().includes(q)));
    return sortEntries(list, this.sortKey, this.ascending);
  });

  selected = $derived(this.rows.filter((r) => this.selection.has(r.path)));
  selectedBytes = $derived(this.selected.reduce((n, i) => (i.isDir ? n : n + i.size), 0));

  constructor(readonly provider: FileProvider) {}

  // ── navigation ─────────────────────────────────────────────────────────────

  /** Opens a folder. `push` keeps the back/forward history. */
  async open(path: string, opts: { push?: boolean; keepFilter?: boolean; select?: string } = {}): Promise<boolean> {
    const token = ++this.#token;
    const previous = this.cwd;
    this.loading = true;
    this.error = null;
    try {
      const listing = await this.provider.list(path);
      if (token !== this.#token) return false; // a newer navigation won
      if ((opts.push ?? true) && previous && previous !== listing.path) {
        this.#back.push(previous);
        if (this.#back.length > 100) this.#back.shift();
        this.#forward = [];
      }
      this.cwd = listing.path;
      this.items = listing.items;
      if (!opts.keepFilter) this.filter = '';
      this.selection.clear();
      this.#anchor = null;
      this.cursor = null;
      this.#syncHistory();
      if (opts.select) this.selectOnly(opts.select);
      void this.#loadDisk(listing.path, token);
      return true;
    } catch (e) {
      if (token !== this.#token) return false;
      this.error = toErrorInfo(e);
      // Keep showing the old folder's path so the bar stays meaningful.
      if (!this.cwd) this.cwd = path;
      return false;
    } finally {
      if (token === this.#token) this.loading = false;
    }
  }

  async #loadDisk(path: string, token: number) {
    if (this.provider.side !== 'remote') return;
    const usage = await this.provider.diskUsage(path);
    if (token === this.#token) this.disk = usage;
  }

  #syncHistory() {
    this.canGoBack = this.#back.length > 0;
    this.canGoForward = this.#forward.length > 0;
  }

  refresh(): Promise<boolean> {
    const keep = [...this.selection];
    return this.open(this.cwd, { push: false, keepFilter: true }).then((ok) => {
      if (ok) for (const p of keep) if (this.items.some((i) => i.path === p)) this.selection.add(p);
      return ok;
    });
  }

  async openHome(): Promise<void> {
    const home = await this.provider.home().catch(() => '/');
    await this.open(home, { push: false });
  }

  up(): Promise<boolean> | undefined {
    const parent = this.provider.ops.parent(this.cwd);
    if (parent === null) return undefined;
    const from = this.cwd;
    return this.open(parent, { select: from });
  }

  async back() {
    const target = this.#back.pop();
    if (target === undefined) return;
    const from = this.cwd;
    const ok = await this.open(target, { push: false });
    if (ok) this.#forward.push(from);
    else this.#back.push(target);
    this.#syncHistory();
  }

  async forward() {
    const target = this.#forward.pop();
    if (target === undefined) return;
    const from = this.cwd;
    const ok = await this.open(target, { push: false });
    if (ok) this.#back.push(from);
    else this.#forward.push(target);
    this.#syncHistory();
  }

  // ── selection ──────────────────────────────────────────────────────────────

  selectOnly(path: string) {
    this.selection.clear();
    this.selection.add(path);
    this.#anchor = path;
    this.cursor = path;
  }

  /** Mouse selection with the usual modifier rules. */
  click(path: string, mods: { ctrl: boolean; shift: boolean }) {
    this.cursor = path;
    if (mods.shift && this.#anchor) {
      const paths = this.rows.map((r) => r.path);
      const a = paths.indexOf(this.#anchor);
      const b = paths.indexOf(path);
      if (a >= 0 && b >= 0) {
        if (!mods.ctrl) this.selection.clear();
        for (const i of range(a, b)) this.selection.add(paths[i]);
        return;
      }
    }
    if (mods.ctrl) {
      if (this.selection.has(path)) this.selection.delete(path);
      else this.selection.add(path);
      this.#anchor = path;
    } else {
      this.selectOnly(path);
    }
  }

  clearSelection() {
    this.selection.clear();
    this.#anchor = null;
  }

  selectAll() {
    for (const r of this.rows) this.selection.add(r.path);
  }

  /** Moves the cursor by `delta` rows (or to an end), optionally extending the selection. */
  move(delta: number | 'start' | 'end', extend: boolean) {
    const rows = this.rows;
    if (rows.length === 0) return;
    const at = rows.findIndex((r) => r.path === this.cursor);
    const next = delta === 'start' ? 0 : delta === 'end' ? rows.length - 1 : Math.min(rows.length - 1, Math.max(0, (at < 0 ? (delta > 0 ? -1 : rows.length) : at) + delta));
    const path = rows[next].path;
    if (extend && this.#anchor) {
      const paths = rows.map((r) => r.path);
      const a = paths.indexOf(this.#anchor);
      this.selection.clear();
      for (const i of range(a < 0 ? next : a, next)) this.selection.add(paths[i]);
      this.cursor = path;
    } else {
      this.selectOnly(path);
    }
  }

  /** Type-ahead: jump to the next name starting with what was typed. */
  typeAhead(ch: string) {
    this.#typed += ch;
    clearTimeout(this.#typedTimer);
    this.#typedTimer = setTimeout(() => (this.#typed = ''), 900);
    const rows = this.rows;
    const from = Math.max(0, rows.findIndex((r) => r.path === this.cursor));
    // Typing the same letter repeatedly cycles through matches.
    const start = this.#typed.length === 1 ? from + 1 : from;
    const i = findByPrefix(rows.map((r) => r.name), this.#typed, start % Math.max(1, rows.length));
    if (i >= 0) this.selectOnly(rows[i].path);
  }

  sortBy(key: SortKey) {
    if (this.sortKey === key) this.ascending = !this.ascending;
    else {
      this.sortKey = key;
      this.ascending = key === 'name' || key === 'kind';
    }
  }

  // ── editing in place ───────────────────────────────────────────────────────

  beginRename(item: FileItem) {
    this.editing = { path: item.path, value: item.name, error: null };
  }

  beginNewFolder() {
    this.editing = { path: null, value: uniqueName('New folder', new Set(this.items.map((i) => i.name))), error: null };
  }

  cancelEdit() {
    this.editing = null;
  }

  async commitEdit(): Promise<void> {
    const edit = this.editing;
    if (!edit) return;
    const name = edit.value.trim();
    const problem = invalidName(name, this.provider.side);
    if (problem) {
      edit.error = problem;
      return;
    }
    const ops = this.provider.ops;
    // Windows names ignore case; the server's don't.
    const same = (a: string) => (this.provider.side === 'local' ? a.toLowerCase() === name.toLowerCase() : a === name);
    if (this.items.some((i) => i.path !== edit.path && same(i.name))) {
      edit.error = 'Something with that name is already here.';
      return;
    }
    try {
      if (edit.path === null) {
        const path = ops.join(this.cwd, name);
        await this.provider.mkdir(path);
        this.editing = null;
        await this.refresh();
        this.selectOnly(path);
      } else {
        const current = this.items.find((i) => i.path === edit.path);
        if (!current || current.name === name) {
          this.editing = null;
          return;
        }
        const target = ops.join(ops.parent(edit.path) ?? this.cwd, name);
        await this.provider.rename(edit.path, target);
        this.editing = null;
        await this.refresh();
        this.selectOnly(target);
      }
    } catch (e) {
      this.editing = null;
      ui.error(e);
    }
  }

  // ── destructive ────────────────────────────────────────────────────────────

  async remove(items: FileItem[]): Promise<void> {
    if (items.length === 0) return;
    const local = this.provider.side === 'local';
    const what = items.length === 1 ? `“${items[0].name}”` : plural(items.length, 'item');
    const folders = items.filter((i) => i.isDir).length;
    const ok = await ui.confirm({
      title: `Delete ${what}?`,
      message: local
        ? 'It goes to the Recycle Bin, so you can still get it back.'
        : `${folders ? 'Folders are deleted with everything inside them. ' : ''}This is permanent — there is no trash on the server.`,
      confirmLabel: local ? 'Move to Recycle Bin' : 'Delete',
      danger: true,
    });
    if (!ok) return;
    try {
      await this.provider.remove(items.map((i) => i.path));
    } catch (e) {
      ui.error(e, 'Couldn’t delete everything');
    }
    await this.refresh();
  }
}

/** Both panes of an SFTP tab, plus the transfers between them. */
export class SftpController {
  readonly local = new PaneState(localProvider());
  readonly remote: PaneState;
  active = $state<Side>('remote');
  /** The transfer queue is expanded. */
  panelOpen = $state(false);
  /** Set while the user is asked what to do about files that already exist. */
  conflict = $state<{ names: string[]; direction: 'upload' | 'download'; resolve: (p: ConflictPolicy | null) => void } | null>(null);

  constructor(readonly connectionId: ConnectionId) {
    this.remote = new PaneState(remoteProvider(connectionId));
  }

  pane(side: Side): PaneState {
    return side === 'local' ? this.local : this.remote;
  }

  other(side: Side): PaneState {
    return side === 'local' ? this.remote : this.local;
  }

  async init(): Promise<void> {
    await Promise.all([this.local.openHome(), this.remote.openHome()]);
  }

  // ── transfers ──────────────────────────────────────────────────────────────

  upload(paths: string[], remoteDir = this.remote.cwd): Promise<void> {
    return this.#plan(() => api.transfers.planUpload(this.connectionId, paths, remoteDir), 'upload');
  }

  download(paths: string[], localDir = this.local.cwd): Promise<void> {
    return this.#plan(() => api.transfers.planDownload(this.connectionId, paths, localDir), 'download');
  }

  /** Sends the selection of one pane to the other pane's folder (or a chosen subfolder). */
  sendSelection(from: Side, items: FileItem[] = this.pane(from).selected, intoDir?: string): Promise<void> {
    if (items.length === 0) {
      ui.info('Nothing selected', 'Pick one or more files first.');
      return Promise.resolve();
    }
    const paths = items.map((i) => i.path);
    if (from === 'local') {
      if (this.local.cwd === '') {
        ui.info('Open a folder first', 'Drives can’t be uploaded directly.');
        return Promise.resolve();
      }
      return this.upload(paths, intoDir ?? this.remote.cwd);
    }
    if (this.local.cwd === '') {
      ui.info('Choose where to put the files', 'Open a folder on this PC in the left pane first.');
      return Promise.resolve();
    }
    return this.download(paths, intoDir ?? this.local.cwd);
  }

  async #plan(make: () => Promise<PlanSummary>, direction: 'upload' | 'download'): Promise<void> {
    let plan: PlanSummary;
    try {
      plan = await make();
    } catch (e) {
      ui.error(e, direction === 'upload' ? 'Couldn’t prepare the upload' : 'Couldn’t prepare the download');
      return;
    }
    try {
      if (plan.fileCount === 0 && plan.conflicts.length === 0) {
        await api.transfers.discardPlan(plan.planId).catch(() => {});
        ui.info('Nothing to transfer', 'The selection has no files in it.');
        return;
      }
      let policy: ConflictPolicy = 'overwrite';
      if (plan.conflicts.length > 0) {
        const preferred = settings.value.sftp.overwrite;
        if (preferred === 'ask') {
          const answer = await this.#askConflict(plan.conflicts, direction);
          if (!answer) {
            await api.transfers.discardPlan(plan.planId).catch(() => {});
            return;
          }
          policy = answer;
        } else {
          policy = preferred === 'overwrite' ? 'overwrite' : preferred === 'skip' ? 'skip' : 'keep-both';
        }
      }
      for (const w of plan.warnings) ui.toast({ kind: 'warning', title: 'Heads up', message: w });
      await api.transfers.start(plan.planId, policy);
      this.panelOpen = true;
    } catch (e) {
      await api.transfers.discardPlan(plan.planId).catch(() => {});
      ui.error(e, 'The transfer couldn’t start');
    }
  }

  #askConflict(names: string[], direction: 'upload' | 'download'): Promise<ConflictPolicy | null> {
    return new Promise((resolve) => {
      this.conflict = {
        names,
        direction,
        resolve: (p) => {
          this.conflict = null;
          resolve(p);
        },
      };
    });
  }

  /** Moves items into a folder on the same side (drag onto a folder). */
  async moveInto(side: Side, items: FileItem[], destDir: string): Promise<void> {
    const pane = this.pane(side);
    const ops = pane.provider.ops;
    const moving = items.filter((i) => i.path !== destDir && ops.parent(i.path) !== destDir && !(i.isDir && ops.within(destDir, i.path)));
    if (moving.length === 0) return;
    let moved = 0;
    for (const item of moving) {
      try {
        await pane.provider.rename(item.path, ops.join(destDir, item.name));
        moved++;
      } catch (e) {
        ui.error(e, `Couldn’t move “${item.name}”`);
        break;
      }
    }
    await pane.refresh();
    if (moved) ui.success(moved === 1 ? `Moved “${moving[0].name}”` : `Moved ${moved} items`);
  }
}
