// Saved hosts, groups and recent targets, kept in sync with the backend.

import { api } from '$lib/api';
import type { Group, GroupDeleteMode, GroupId, Host, HostId, HostsData, RecentTarget } from '$lib/bindings';
import { buildTree, sortHosts, tagCounts } from '$lib/util/tree';

class HostsStore {
  data = $state<HostsData>({ version: 1, hosts: [], groups: [] });
  recents = $state<RecentTarget[]>([]);
  loaded = $state(false);

  tree = $derived(buildTree(this.data));
  byId = $derived(new Map(this.data.hosts.map((h) => [h.id, h])));
  favorites = $derived(sortHosts(this.data.hosts.filter((h) => h.favorite)));
  /** Most recently connected hosts. */
  recent = $derived(
    this.data.hosts
      .filter((h) => h.lastConnectedAt)
      .sort((a, b) => (b.lastConnectedAt ?? 0) - (a.lastConnectedAt ?? 0))
      .slice(0, 6),
  );
  tags = $derived(tagCounts(this.data.hosts));
  isEmpty = $derived(this.loaded && this.data.hosts.length === 0);

  get(id: HostId | null | undefined): Host | undefined {
    return id ? this.byId.get(id) : undefined;
  }

  async init(): Promise<void> {
    await this.refresh();
    await api.on('hosts:changed', () => void this.refresh());
  }

  async refresh(): Promise<void> {
    const [data, recents] = await Promise.all([api.hosts.snapshot(), api.hosts.recents().catch(() => [])]);
    this.data = data;
    this.recents = recents;
    this.loaded = true;
  }

  async save(host: Host): Promise<Host> {
    const saved = await api.hosts.save(host);
    await this.refresh();
    return saved;
  }

  async remove(ids: HostId[]): Promise<void> {
    await api.hosts.delete(ids);
    await this.refresh();
  }

  async duplicate(id: HostId): Promise<Host> {
    const copy = await api.hosts.duplicate(id);
    await this.refresh();
    return copy;
  }

  async toggleFavorite(host: Host): Promise<void> {
    const next = !host.favorite;
    // Optimistic: the star should respond instantly.
    const local = this.data.hosts.find((h) => h.id === host.id);
    if (local) local.favorite = next;
    try {
      await api.hosts.favorite(host.id, next);
    } catch (e) {
      if (local) local.favorite = !next;
      throw e;
    }
  }

  async move(ids: HostId[], groupId: GroupId | null): Promise<void> {
    await api.hosts.move(ids, groupId);
    await this.refresh();
  }

  async saveGroup(group: Group): Promise<Group> {
    const saved = await api.hosts.saveGroup(group);
    await this.refresh();
    return saved;
  }

  async removeGroup(id: GroupId, mode: GroupDeleteMode): Promise<void> {
    await api.hosts.deleteGroup(id, mode);
    await this.refresh();
  }

  async setCollapsed(group: Group, collapsed: boolean): Promise<void> {
    const local = this.data.groups.find((g) => g.id === group.id);
    if (local) local.collapsed = collapsed;
    await api.hosts.collapseGroup(group.id, collapsed).catch(() => {});
  }

  async clearRecents(): Promise<void> {
    await api.hosts.clearRecents();
    this.recents = [];
  }
}

export const hosts = new HostsStore();
