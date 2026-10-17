// Transient UI state: dialogs, palette, menus, toasts and window state.

import { toErrorInfo } from '$lib/api';
import type { ErrorInfo, GroupId, Host, HostId } from '$lib/bindings';
import type { ConnectionId } from '$lib/bindings';

export interface ToastAction {
  label: string;
  run: () => void;
}

export interface Toast {
  id: number;
  kind: 'info' | 'success' | 'warning' | 'error';
  title: string;
  message?: string;
  hint?: string | null;
  detail?: string | null;
  actions?: ToastAction[];
  /** Milliseconds until it disappears; 0 keeps it until dismissed. */
  timeout: number;
}

export interface MenuItem {
  label?: string;
  icon?: unknown;
  shortcut?: string;
  danger?: boolean;
  disabled?: boolean;
  checked?: boolean;
  separator?: boolean;
  /** Small heading above a group of items. */
  heading?: string;
  run?: () => void;
  submenu?: MenuItem[];
}

export type DialogState =
  | { type: 'host-editor'; hostId?: HostId; draft?: Partial<Host>; groupId?: GroupId | null }
  | { type: 'group-editor'; groupId?: GroupId; parentId?: GroupId | null }
  | { type: 'settings'; section?: string }
  | { type: 'keys' }
  | { type: 'known-hosts' }
  | { type: 'import-ssh' }
  | { type: 'forwards'; connectionId: ConnectionId }
  | { type: 'shortcuts' }
  | { type: 'about' }
  | { type: 'onboarding' };

export interface ConfirmOptions {
  title: string;
  message?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
  /** An extra "don't ask again"-style checkbox. */
  checkbox?: string;
}

export interface TextPromptOptions {
  title: string;
  label?: string;
  value?: string;
  placeholder?: string;
  confirmLabel?: string;
  /** Return an error message to block submitting. */
  validate?: (value: string) => string | null;
}

type PaletteMode = 'all' | 'connect' | 'commands';

class UiStore {
  sidebarOpen = $state(true);
  windowFocused = $state(true);
  maximized = $state(false);
  fullscreen = $state(false);

  palette = $state<{ open: boolean; mode: PaletteMode; query: string }>({ open: false, mode: 'all', query: '' });
  dialogs = $state<DialogState[]>([]);
  menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  toasts = $state<Toast[]>([]);

  confirmState = $state<{ options: ConfirmOptions; resolve: (v: { ok: boolean; checked: boolean }) => void } | null>(null);
  textState = $state<{ options: TextPromptOptions; resolve: (v: string | null) => void } | null>(null);

  #toastId = 1;

  // ── dialogs ────────────────────────────────────────────────────────────────

  openDialog(d: DialogState): void {
    this.dialogs.push(d);
  }

  closeDialog(): void {
    this.dialogs.pop();
  }

  closeAllDialogs(): void {
    this.dialogs = [];
  }

  get topDialog(): DialogState | null {
    return this.dialogs[this.dialogs.length - 1] ?? null;
  }

  editHost(hostId: HostId): void {
    this.openDialog({ type: 'host-editor', hostId });
  }

  newHost(groupId: GroupId | null = null, draft?: Partial<Host>): void {
    this.openDialog({ type: 'host-editor', groupId, draft });
  }

  openSettings(section?: string): void {
    const existing = this.dialogs.find((d) => d.type === 'settings');
    if (existing && existing.type === 'settings') existing.section = section ?? existing.section;
    else this.openDialog({ type: 'settings', section });
  }

  // ── palette ────────────────────────────────────────────────────────────────

  openPalette(mode: PaletteMode = 'all', query = ''): void {
    this.palette = { open: true, mode, query };
  }

  closePalette(): void {
    this.palette.open = false;
  }

  togglePalette(): void {
    if (this.palette.open) this.closePalette();
    else this.openPalette();
  }

  // ── context menu ───────────────────────────────────────────────────────────

  showMenu(e: MouseEvent | { clientX: number; clientY: number }, items: MenuItem[]): void {
    if ('preventDefault' in e) e.preventDefault();
    this.menu = { x: e.clientX, y: e.clientY, items };
  }

  hideMenu(): void {
    this.menu = null;
  }

  // ── toasts ─────────────────────────────────────────────────────────────────

  toast(t: Partial<Toast> & { title: string }): number {
    const id = this.#toastId++;
    const kind = t.kind ?? 'info';
    const toast: Toast = { timeout: kind === 'error' ? 9000 : kind === 'warning' ? 7000 : 4200, ...t, id, kind };
    this.toasts = [...this.toasts.slice(-3), toast];
    if (toast.timeout > 0) setTimeout(() => this.dismissToast(id), toast.timeout);
    return id;
  }

  success(title: string, message?: string): number {
    return this.toast({ kind: 'success', title, message });
  }

  info(title: string, message?: string): number {
    return this.toast({ kind: 'info', title, message });
  }

  dismissToast(id: number): void {
    this.toasts = this.toasts.filter((t) => t.id !== id);
  }

  /** Shows a backend error in plain language with its hint and optional details. */
  error(e: unknown, fallbackTitle?: string): number {
    const info: ErrorInfo = toErrorInfo(e);
    // Saying no to a question the backend asked (a native confirmation, a cancelled login) is an answer, not a failure.
    if (info.code === 'cancelled') return -1;
    return this.toast({
      kind: 'error',
      title: fallbackTitle && info.code === 'internal' ? fallbackTitle : info.title,
      message: info.message,
      hint: info.hint,
      detail: info.detail,
    });
  }

  // ── modal questions ────────────────────────────────────────────────────────

  confirm(options: ConfirmOptions): Promise<boolean> {
    return this.confirmWithCheckbox(options).then((r) => r.ok);
  }

  confirmWithCheckbox(options: ConfirmOptions): Promise<{ ok: boolean; checked: boolean }> {
    return new Promise((resolve) => {
      // A newer question replaces an unanswered one (counts as "no").
      this.confirmState?.resolve({ ok: false, checked: false });
      this.confirmState = { options, resolve };
    });
  }

  answerConfirm(ok: boolean, checked = false): void {
    const s = this.confirmState;
    this.confirmState = null;
    s?.resolve({ ok, checked });
  }

  askText(options: TextPromptOptions): Promise<string | null> {
    return new Promise((resolve) => {
      this.textState?.resolve(null);
      this.textState = { options, resolve };
    });
  }

  answerText(value: string | null): void {
    const s = this.textState;
    this.textState = null;
    s?.resolve(value);
  }
}

export const ui = new UiStore();
