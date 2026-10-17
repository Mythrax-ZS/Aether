// User-level operations that combine stores, the backend and confirmations.
// Commands, menus, buttons and shortcuts all call these so behaviour is identical everywhere.

import { api } from '$lib/api';
import type { Group, GroupId, Host, HostId } from '$lib/bindings';
import { terminalRegistry } from '$lib/terminal/registry';
import { hostTarget } from '$lib/util/panes';
import { plural } from '$lib/util/format';
import { isWithin } from '$lib/util/tree';
import { hosts } from '$lib/stores/hosts.svelte';
import { sessions } from '$lib/stores/sessions.svelte';
import { settings } from '$lib/stores/settings.svelte';
import { ui } from '$lib/stores/ui.svelte';
import { workspace, type Tab } from '$lib/stores/workspace.svelte';

export function connectHost(host: Host): void {
  ui.closePalette();
  workspace.openHost(host);
}

export function connectHostId(id: HostId): void {
  const host = hosts.get(id);
  if (host) connectHost(host);
}

/** Opens `user@host[:port]` (or an alias from ~/.ssh/config) in a new tab. */
export function connectTarget(target: string): void {
  const t = target.trim();
  if (!t) return;
  ui.closePalette();
  workspace.openTarget(t);
}

export async function closeTab(tabId: string, opts: { force?: boolean } = {}): Promise<boolean> {
  const tab = workspace.tabs.find((t) => t.id === tabId);
  if (!tab) return false;
  if (!opts.force && settings.value.general.confirmCloseActive && workspace.hasLiveSession(tab)) {
    const ok = await ui.confirm({
      title: 'Close this tab?',
      message: `${describeTab(tab)} has an active connection that will be disconnected.`,
      confirmLabel: 'Close tab',
      danger: true,
    });
    if (!ok) return false;
  }
  workspace.closeTab(tabId);
  return true;
}

/** Asks before a whole window goes away with live connections in it. Resolves to "may it close?". */
export async function confirmWindowClose(): Promise<boolean> {
  if (!settings.value.general.confirmCloseActive) return true;
  const live = workspace.tabs.filter((t) => workspace.hasLiveSession(t));
  if (live.length === 0) return true;
  return ui.confirm({
    title: 'Close this window?',
    message: `${plural(live.length, 'tab')} with an active connection will be disconnected.`,
    confirmLabel: 'Close window',
    danger: true,
  });
}

function describeTab(tab: Tab): string {
  return `“${workspace.tabTitle(tab)}”`;
}

export async function closeTabs(ids: string[]): Promise<void> {
  const live = workspace.tabs.filter((t) => ids.includes(t.id) && workspace.hasLiveSession(t));
  if (live.length > 0 && settings.value.general.confirmCloseActive) {
    const ok = await ui.confirm({
      title: `Close ${plural(ids.length, 'tab')}?`,
      message: `${plural(live.length, 'active connection')} will be disconnected.`,
      confirmLabel: 'Close',
      danger: true,
    });
    if (!ok) return;
  }
  ids.forEach((id) => workspace.closeTab(id));
}

/** Closes the focused pane (and the tab when it was the last one). */
export async function closeActivePane(): Promise<void> {
  const tab = workspace.active;
  if (!tab) return;
  if (tab.kind === 'sftp') {
    await closeTab(tab.id);
    return;
  }
  const leaf = workspace.activeLeaf;
  if (!leaf) return;
  const single = tab.root.kind === 'leaf';
  if (single) {
    await closeTab(tab.id);
    return;
  }
  if (settings.value.general.confirmCloseActive && sessions.isConnected(leaf.connectionId)) {
    const ok = await ui.confirm({ title: 'Close this pane?', message: 'Its shell will be closed.', confirmLabel: 'Close pane', danger: true });
    if (!ok) return;
  }
  workspace.closePane(leaf.id);
}

export function reconnectActive(): void {
  const id = workspace.activeConnection;
  if (id) void api.sessions.reconnect(id).catch((e) => ui.error(e));
}

/** Brings a pane back: a fresh login if it never connected, otherwise re-dials the existing session. */
export async function reconnectPane(leafId: string): Promise<void> {
  const leaf = workspace.leaf(leafId);
  if (!leaf) return;
  const id = leaf.connectionId;
  const everConnected = !!sessions.view(id)?.everConnected;
  if (!id || (!everConnected && workspace.connectionUsers(id) <= 1)) {
    await workspace.retryFresh(leafId);
    return;
  }
  await api.sessions.reconnect(id).catch((e) => ui.error(e));
}

export function openSftpForActive(): void {
  const tab = workspace.active;
  if (!tab) return;
  if (tab.kind === 'sftp') return;
  const leaf = workspace.activeLeaf;
  if (!leaf?.connectionId || !leaf.host) {
    ui.info('Connect first', 'Open a session, then browse its files.');
    return;
  }
  workspace.openSftp(leaf.connectionId, leaf.host);
}

export async function copyText(text: string, what = 'Copied'): Promise<void> {
  try {
    await api.platform.clipboard.writeText(text);
    ui.success(what);
  } catch (e) {
    ui.error(e);
  }
}

export function copyAddress(host: Host): Promise<void> {
  return copyText(hostTarget(host), 'Address copied');
}

export async function deleteHosts(list: Host[]): Promise<void> {
  if (list.length === 0) return;
  const ok = await ui.confirm({
    title: list.length === 1 ? `Delete “${list[0].label || list[0].hostname}”?` : `Delete ${list.length} hosts?`,
    message: 'The saved settings and password are removed. Open sessions stay connected.',
    confirmLabel: 'Delete',
    danger: true,
  });
  if (!ok) return;
  try {
    await hosts.remove(list.map((h) => h.id));
    ui.success(list.length === 1 ? 'Host deleted' : `${list.length} hosts deleted`);
  } catch (e) {
    ui.error(e);
  }
}

/** Opens several hosts at once (asks first when it's a lot of tabs). */
export async function connectHosts(list: Host[]): Promise<void> {
  if (list.length === 0) return;
  if (list.length > 4) {
    const ok = await ui.confirm({ title: `Open ${list.length} connections?`, message: 'Each host opens in its own tab.', confirmLabel: 'Connect to all' });
    if (!ok) return;
  }
  ui.closePalette();
  for (const h of list) workspace.openHost(h);
}

export async function moveHosts(ids: HostId[], groupId: GroupId | null): Promise<void> {
  if (ids.length === 0) return;
  try {
    await hosts.move(ids, groupId);
    const where = groupId ? (hosts.data.groups.find((g) => g.id === groupId)?.name ?? 'the group') : 'the top level';
    ui.success(ids.length === 1 ? `Moved to ${where}` : `Moved ${ids.length} hosts to ${where}`);
  } catch (e) {
    ui.error(e);
  }
}

export async function deleteGroup(group: Group): Promise<void> {
  const count = hosts.data.hosts.filter((h) => isWithin(hosts.data, h.groupId, group.id)).length;
  const { ok, checked } = await ui.confirmWithCheckbox({
    title: `Delete “${group.name}”?`,
    message: count
      ? `It holds ${plural(count, 'host')}. They move out of the group unless you choose otherwise.`
      : 'The group is empty.',
    confirmLabel: 'Delete group',
    danger: true,
    checkbox: count ? `Also delete the ${plural(count, 'host')} inside` : undefined,
  });
  if (!ok) return;
  try {
    await hosts.removeGroup(group.id, checked ? 'delete-contents' : 'move-contents');
    ui.success('Group deleted');
  } catch (e) {
    ui.error(e);
  }
}

export function newWindow(): void {
  void api.app.newWindow({ adoptConnection: null, adoptTerminal: null, adoptHost: null, title: null }).catch((e) => ui.error(e));
}

/** Moves a terminal tab into its own window; the session keeps running. */
export async function tearOffActiveTab(): Promise<void> {
  const tab = workspace.activeTerminalTab;
  const leaf = workspace.activeLeaf;
  if (!tab || !leaf?.connectionId || !leaf.terminalId) return;
  // Only simple, single-pane tabs can move for now.
  if (tab.root.kind !== 'leaf') {
    ui.info('Tear-off works on single-pane tabs', 'Close the splits first, or open the pane in a new tab.');
    return;
  }
  try {
    await api.app.newWindow({ adoptConnection: leaf.connectionId, adoptTerminal: leaf.terminalId, adoptHost: leaf.hostId, title: workspace.tabTitle(tab) });
    // The new window owns the session now; drop it here without closing the connection.
    const i = workspace.tabs.findIndex((t) => t.id === tab.id);
    if (i >= 0) workspace.tabs.splice(i, 1);
    workspace.activeId = workspace.tabs[Math.min(i, workspace.tabs.length - 1)]?.id ?? null;
  } catch (e) {
    ui.error(e);
  }
}

export function toggleTheme(): void {
  const dark = settings.activeTheme.kind === 'dark';
  settings.patch({ appearance: { theme: dark ? settings.value.appearance.lightTheme : settings.value.appearance.darkTheme } });
}

export function zoomTerminal(delta: number): void {
  const t = settings.value.terminal;
  const size = delta === 0 ? 13.5 : Math.min(40, Math.max(7, Math.round((t.fontSize + delta) * 2) / 2));
  settings.patch({ terminal: { fontSize: size } });
}

export function focusedController() {
  return terminalRegistry.get(workspace.activeLeaf?.id);
}

export function toggleSidebar(): void {
  ui.sidebarOpen = !ui.sidebarOpen;
  settings.patch({ appearance: { sidebarCollapsed: !ui.sidebarOpen } });
}

export async function toggleFullscreen(): Promise<void> {
  const on = !ui.fullscreen;
  await api.platform.window.setFullscreen(on).catch(() => {});
  ui.fullscreen = on;
}
