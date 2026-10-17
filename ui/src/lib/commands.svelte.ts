// Every user-invokable action, its default shortcut, and the rebindable keymap.

import {
  ArrowLeftRight, Columns2, Command as CommandIcon, Copy, CornerDownLeft, FolderTree, FolderOpen, Info, Keyboard, Maximize2,
  Moon, PanelLeft, Plug, Plus, Rows2, Search, Settings, ShieldCheck, SquareTerminal, Trash2, X, ZoomIn, ZoomOut, KeyRound, Download,
  AppWindow, RefreshCw, ClipboardPaste, Eraser, ArrowDownToLine, ListChecks, SquareSplitHorizontal, FileText,
} from '@lucide/svelte';
import type { Component } from 'svelte';
import { api } from '$lib/api';
import {
  closeActivePane, closeTab, closeTabs, newWindow, openSftpForActive, reconnectActive, tearOffActiveTab, toggleFullscreen,
  toggleSidebar, toggleTheme, zoomTerminal, focusedController,
} from '$lib/actions';
import { hosts } from '$lib/stores/hosts.svelte';
import { prompts } from '$lib/stores/prompts.svelte';
import { settings } from '$lib/stores/settings.svelte';
import { ui } from '$lib/stores/ui.svelte';
import { workspace } from '$lib/stores/workspace.svelte';
import { formatAccelerator, matchesAccelerator, parseAccelerator, type Accelerator } from '$lib/util/keys';

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type IconComponent = Component<any>;

export type Category = 'Navigation' | 'Tabs' | 'Panes' | 'Terminal' | 'Session' | 'Hosts' | 'View' | 'App';

export interface Command {
  id: string;
  title: string;
  category: Category;
  icon: IconComponent;
  keywords?: string;
  /** Shortcuts that work even while typing in a dialog or form field. */
  global?: boolean;
  /** Hide from the palette (shortcut-only, e.g. "go to tab 3"). */
  hidden?: boolean;
  when?: () => boolean;
  run: () => void | Promise<void>;
}

const hasTab = () => workspace.tabs.length > 0;
const hasTerminal = () => !!workspace.activeLeaf;
const hasConnection = () => !!workspace.activeConnection;

function pane(dir: 'left' | 'right' | 'up' | 'down'): void {
  const tab = workspace.activeTerminalTab;
  if (!tab) return;
  const cur = document.querySelector<HTMLElement>(`[data-pane-id="${tab.activePane}"]`)?.getBoundingClientRect();
  if (!cur) return;
  const cx = cur.left + cur.width / 2;
  const cy = cur.top + cur.height / 2;
  let best: { id: string; dist: number } | null = null;
  for (const el of document.querySelectorAll<HTMLElement>('[data-pane-id]')) {
    const id = el.dataset.paneId!;
    if (id === tab.activePane || !workspace.tabOfLeaf(id) || workspace.tabOfLeaf(id)!.id !== tab.id) continue;
    const r = el.getBoundingClientRect();
    const x = r.left + r.width / 2;
    const y = r.top + r.height / 2;
    const ok = dir === 'left' ? x < cx - 4 : dir === 'right' ? x > cx + 4 : dir === 'up' ? y < cy - 4 : y > cy + 4;
    if (!ok) continue;
    // Prefer panes that overlap on the other axis, then the closest.
    const overlap = dir === 'left' || dir === 'right' ? r.top < cur.bottom && r.bottom > cur.top : r.left < cur.right && r.right > cur.left;
    const dist = Math.hypot(x - cx, y - cy) + (overlap ? 0 : 10_000);
    if (!best || dist < best.dist) best = { id, dist };
  }
  if (best) workspace.focusPane(best.id);
}

export const COMMANDS: Command[] = [
  // ── navigation ──
  { id: 'palette.open', title: 'Command palette', category: 'Navigation', icon: CommandIcon, global: true, hidden: true, run: () => ui.openPalette('all') },
  { id: 'palette.commands', title: 'Show all commands', category: 'Navigation', icon: CommandIcon, global: true, hidden: true, run: () => ui.openPalette('commands', '>') },
  { id: 'tab.new', title: 'New connection…', category: 'Navigation', icon: Plus, keywords: 'connect open quick', run: () => ui.openPalette('connect') },
  { id: 'app.sidebar', title: 'Toggle sidebar', category: 'View', icon: PanelLeft, global: true, run: toggleSidebar },
  { id: 'app.settings', title: 'Settings', category: 'App', icon: Settings, global: true, keywords: 'preferences options', run: () => ui.openSettings() },
  { id: 'app.shortcuts', title: 'Keyboard shortcuts', category: 'App', icon: Keyboard, global: true, run: () => ui.openDialog({ type: 'shortcuts' }) },
  { id: 'app.fullscreen', title: 'Toggle full screen', category: 'View', icon: Maximize2, global: true, run: toggleFullscreen },
  { id: 'app.theme', title: 'Switch light / dark theme', category: 'View', icon: Moon, keywords: 'appearance', run: toggleTheme },
  { id: 'app.newWindow', title: 'New window', category: 'App', icon: AppWindow, run: newWindow },

  // ── tabs ──
  { id: 'tab.close', title: 'Close tab', category: 'Tabs', icon: X, when: hasTab, run: () => (workspace.activeId ? void closeTab(workspace.activeId) : undefined) },
  { id: 'tab.closeOthers', title: 'Close other tabs', category: 'Tabs', icon: X, when: () => workspace.tabs.length > 1, run: () => void closeTabs(workspace.tabs.filter((t) => t.id !== workspace.activeId).map((t) => t.id)) },
  { id: 'tab.next', title: 'Next tab', category: 'Tabs', icon: ArrowLeftRight, when: () => workspace.tabs.length > 1, run: () => workspace.cycle(1) },
  { id: 'tab.prev', title: 'Previous tab', category: 'Tabs', icon: ArrowLeftRight, when: () => workspace.tabs.length > 1, run: () => workspace.cycle(-1) },
  { id: 'tab.duplicate', title: 'Duplicate tab', category: 'Tabs', icon: Copy, when: () => workspace.activeTerminalTab !== null, run: () => { if (workspace.activeId) workspace.duplicateTab(workspace.activeId); } },
  { id: 'tab.tearOff', title: 'Move tab to a new window', category: 'Tabs', icon: AppWindow, when: () => workspace.activeTerminalTab?.root.kind === 'leaf', run: () => void tearOffActiveTab() },
  ...Array.from({ length: 9 }, (_, i): Command => ({
    id: `tab.go${i + 1}`,
    title: i === 8 ? 'Go to last tab' : `Go to tab ${i + 1}`,
    category: 'Tabs',
    icon: ArrowLeftRight,
    hidden: true,
    run: () => workspace.activateIndex(i),
  })),

  // ── panes ──
  { id: 'pane.splitRight', title: 'Split pane right', category: 'Panes', icon: Columns2, when: hasTerminal, keywords: 'vertical', run: () => workspace.splitActive('row') },
  { id: 'pane.splitDown', title: 'Split pane down', category: 'Panes', icon: Rows2, when: hasTerminal, keywords: 'horizontal', run: () => workspace.splitActive('col') },
  { id: 'pane.close', title: 'Close pane', category: 'Panes', icon: X, when: hasTab, run: () => void closeActivePane() },
  { id: 'pane.balance', title: 'Balance pane sizes', category: 'Panes', icon: SquareSplitHorizontal, when: () => workspace.activeTerminalTab?.root.kind === 'split', run: () => workspace.balanceActive() },
  { id: 'pane.left', title: 'Focus pane to the left', category: 'Panes', icon: ArrowLeftRight, hidden: true, run: () => pane('left') },
  { id: 'pane.right', title: 'Focus pane to the right', category: 'Panes', icon: ArrowLeftRight, hidden: true, run: () => pane('right') },
  { id: 'pane.up', title: 'Focus pane above', category: 'Panes', icon: ArrowLeftRight, hidden: true, run: () => pane('up') },
  { id: 'pane.down', title: 'Focus pane below', category: 'Panes', icon: ArrowLeftRight, hidden: true, run: () => pane('down') },

  // ── terminal ──
  { id: 'term.find', title: 'Find in terminal', category: 'Terminal', icon: Search, when: hasTerminal, keywords: 'search scrollback', run: () => { window.dispatchEvent(new CustomEvent('aether:find')); } },
  { id: 'term.copy', title: 'Copy selection', category: 'Terminal', icon: Copy, when: hasTerminal, run: () => focusedController()?.copy() },
  { id: 'term.paste', title: 'Paste', category: 'Terminal', icon: ClipboardPaste, when: hasTerminal, run: () => void focusedController()?.paste() },
  { id: 'term.selectAll', title: 'Select all', category: 'Terminal', icon: ListChecks, when: hasTerminal, run: () => focusedController()?.selectAll() },
  { id: 'term.clear', title: 'Clear scrollback', category: 'Terminal', icon: Eraser, when: hasTerminal, run: () => focusedController()?.clear() },
  { id: 'term.bottom', title: 'Scroll to bottom', category: 'Terminal', icon: ArrowDownToLine, when: hasTerminal, run: () => focusedController()?.scrollToBottom() },
  { id: 'term.zoomIn', title: 'Increase font size', category: 'Terminal', icon: ZoomIn, run: () => zoomTerminal(1) },
  { id: 'term.zoomOut', title: 'Decrease font size', category: 'Terminal', icon: ZoomOut, run: () => zoomTerminal(-1) },
  { id: 'term.zoomReset', title: 'Reset font size', category: 'Terminal', icon: ZoomIn, run: () => zoomTerminal(0) },

  // ── session ──
  { id: 'session.reconnect', title: 'Reconnect', category: 'Session', icon: RefreshCw, when: hasConnection, run: reconnectActive },
  { id: 'session.sftp', title: 'Browse files (SFTP)', category: 'Session', icon: FolderTree, when: () => !!workspace.activeLeaf?.connectionId, keywords: 'upload download transfer', run: openSftpForActive },
  {
    id: 'session.forwards', title: 'Port forwarding…', category: 'Session', icon: Plug, when: hasConnection, keywords: 'tunnel socks proxy',
    run: () => { const c = workspace.activeConnection; if (c) ui.openDialog({ type: 'forwards', connectionId: c }); },
  },
  {
    id: 'session.close', title: 'Disconnect and close tab', category: 'Session', icon: Trash2, when: hasTab,
    run: () => (workspace.activeId ? void closeTab(workspace.activeId) : undefined),
  },

  // ── hosts & keys ──
  { id: 'host.new', title: 'New host…', category: 'Hosts', icon: Plus, global: true, run: () => ui.newHost() },
  {
    id: 'host.editCurrent', title: 'Edit this host…', category: 'Hosts', icon: FileText,
    when: () => !!workspace.activeLeaf?.hostId,
    run: () => { const id = workspace.activeLeaf?.hostId; if (id) ui.editHost(id); },
  },
  { id: 'host.newGroup', title: 'New group…', category: 'Hosts', icon: FolderOpen, run: () => ui.openDialog({ type: 'group-editor' }) },
  { id: 'host.import', title: 'Import from ~/.ssh/config…', category: 'Hosts', icon: Download, keywords: 'openssh', run: () => ui.openDialog({ type: 'import-ssh' }) },
  { id: 'app.keys', title: 'SSH keys…', category: 'Hosts', icon: KeyRound, keywords: 'generate import private public', run: () => ui.openDialog({ type: 'keys' }) },
  { id: 'app.knownHosts', title: 'Known hosts…', category: 'Hosts', icon: ShieldCheck, keywords: 'fingerprint trust', run: () => ui.openDialog({ type: 'known-hosts' }) },

  // ── app ──
  { id: 'app.about', title: 'About Aether', category: 'App', icon: Info, run: () => ui.openDialog({ type: 'about' }) },
  { id: 'app.logs', title: 'Open logs folder', category: 'App', icon: FolderOpen, run: () => void api.app.openLogs().catch((e) => ui.error(e)) },
  { id: 'app.config', title: 'Open config folder', category: 'App', icon: FolderOpen, run: () => void api.app.openConfig().catch((e) => ui.error(e)) },
  { id: 'app.quit', title: 'Quit Aether', category: 'App', icon: CornerDownLeft, run: () => void api.app.quit() },
];

/** Shortcuts shipped with Aether. Terminals own plain Ctrl+<letter>, so app shortcuts use
 *  Ctrl+Shift or Ctrl+Alt — except the palette, which is Ctrl+K as people expect. */
export const DEFAULT_SHORTCUTS: Record<string, string[]> = {
  'palette.open': ['Ctrl+K', 'Ctrl+Shift+P'],
  'tab.new': ['Ctrl+Shift+T'],
  'tab.next': ['Ctrl+Tab'],
  'tab.prev': ['Ctrl+Shift+Tab'],
  'pane.close': ['Ctrl+Shift+W'],
  'pane.splitRight': ['Ctrl+Shift+D'],
  'pane.splitDown': ['Ctrl+Shift+E'],
  'pane.left': ['Ctrl+Alt+Left'],
  'pane.right': ['Ctrl+Alt+Right'],
  'pane.up': ['Ctrl+Alt+Up'],
  'pane.down': ['Ctrl+Alt+Down'],
  'term.find': ['Ctrl+Shift+F'],
  'term.copy': ['Ctrl+Shift+C'],
  'term.paste': ['Ctrl+Shift+V'],
  'term.selectAll': ['Ctrl+Shift+A'],
  'term.clear': ['Ctrl+Shift+K'],
  'term.zoomIn': ['Ctrl+=', 'Ctrl++'],
  'term.zoomOut': ['Ctrl+-'],
  'term.zoomReset': ['Ctrl+0'],
  'session.reconnect': ['Ctrl+Shift+R'],
  'session.sftp': ['Ctrl+Shift+S'],
  'session.forwards': ['Ctrl+Shift+L'],
  'host.new': ['Ctrl+Alt+N'],
  'app.sidebar': ['Ctrl+B'],
  'app.settings': ['Ctrl+,'],
  'app.shortcuts': ['Ctrl+/'],
  'app.fullscreen': ['F11'],
  'app.newWindow': ['Ctrl+Shift+N'],
  ...Object.fromEntries(Array.from({ length: 9 }, (_, i) => [`tab.go${i + 1}`, [`Ctrl+Alt+${i + 1}`]])),
};

const byId = new Map(COMMANDS.map((c) => [c.id, c]));

export function commandById(id: string): Command | undefined {
  return byId.get(id);
}

function isEditable(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  if (el.classList.contains('xterm-helper-textarea')) return false;
  return el.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(el.tagName);
}

class Keymap {
  /** command id → accelerators currently bound (defaults overlaid with the user's choices). */
  bindings = $derived.by(() => {
    const map = new Map<string, Accelerator[]>();
    const user = settings.value.keybindings;
    for (const cmd of COMMANDS) {
      const custom = user[cmd.id];
      const raw = custom !== undefined ? (custom ? [custom] : []) : (DEFAULT_SHORTCUTS[cmd.id] ?? []);
      const parsed = raw.map(parseAccelerator).filter((a): a is Accelerator => a !== null);
      if (parsed.length) map.set(cmd.id, parsed);
    }
    return map;
  });

  /** The text to show next to a command ("Ctrl+Shift+T"), or null when unbound. */
  display(id: string): string | null {
    const first = this.bindings.get(id)?.[0];
    return first ? formatAccelerator(first) : null;
  }

  isDefault(id: string): boolean {
    return settings.value.keybindings[id] === undefined;
  }

  /** The command a key event triggers in the current context, if any. */
  match(e: KeyboardEvent): Command | null {
    if (e.isComposing) return null;
    const inField = isEditable(e.target);
    const modal = ui.dialogs.length > 0 || ui.confirmState !== null || ui.textState !== null || prompts.current !== null;
    const inTerminal = e.target instanceof HTMLElement && e.target.classList.contains('xterm-helper-textarea');
    for (const [id, accels] of this.bindings) {
      if (!accels.some((a) => matchesAccelerator(e, a))) continue;
      const cmd = byId.get(id)!;
      if (!cmd.global && (inField || modal)) return null;
      if (cmd.when && !cmd.when()) return null;
      // Plain Ctrl+K inside a terminal belongs to the shell unless the user opted in.
      if (id === 'palette.open' && inTerminal && !settings.value.terminal.paletteInTerminal && !e.shiftKey) return null;
      return cmd;
    }
    return null;
  }
}

export const keymap = new Keymap();

/** Available commands for the palette (respecting `when`). */
export function paletteCommands(): Command[] {
  return COMMANDS.filter((c) => !c.hidden && (!c.when || c.when()));
}

/** Window-level key handler. Install once. */
export function installKeyHandler(): () => void {
  const handler = (e: KeyboardEvent) => {
    const cmd = keymap.match(e);
    if (!cmd) return;
    e.preventDefault();
    e.stopPropagation();
    void Promise.resolve(cmd.run()).catch((err) => ui.error(err));
  };
  window.addEventListener('keydown', handler, { capture: true });
  return () => window.removeEventListener('keydown', handler, { capture: true });
}

/** Does this event trigger an app command? Terminals use it to stay out of the way. */
export function isAppShortcut(e: KeyboardEvent): boolean {
  return keymap.match(e) !== null;
}

// Referenced so the import list stays honest if a command is removed.
void SquareTerminal;
void hosts;
