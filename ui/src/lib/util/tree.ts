// Pure helpers for the host tree and sidebar search.

import type { Group, GroupId, Host, HostsData } from '$lib/bindings';
import { fuzzyMatch, rankBy, type Ranked } from './fuzzy';

export interface GroupNode {
  group: Group;
  children: GroupNode[];
  hosts: Host[];
  /** Hosts in this group and everything below it. */
  total: number;
}

export interface HostTree {
  roots: GroupNode[];
  /** Hosts that aren't in any group. */
  loose: Host[];
}

const byName = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: 'base', numeric: true });

export function sortHosts(hosts: Host[]): Host[] {
  return [...hosts].sort((a, b) => a.sort - b.sort || byName(a.label || a.hostname, b.label || b.hostname));
}

export function buildTree(data: HostsData): HostTree {
  const nodes = new Map<GroupId, GroupNode>();
  for (const g of data.groups) nodes.set(g.id, { group: g, children: [], hosts: [], total: 0 });

  const loose: Host[] = [];
  for (const h of data.hosts) {
    const node = h.groupId ? nodes.get(h.groupId) : undefined;
    if (node) node.hosts.push(h);
    else loose.push(h);
  }

  const roots: GroupNode[] = [];
  for (const node of nodes.values()) {
    const parent = node.group.parentId ? nodes.get(node.group.parentId) : undefined;
    // A group whose parent vanished is shown at the top level rather than lost.
    (parent ?? { children: roots }).children.push(node);
  }

  const finish = (node: GroupNode): number => {
    node.hosts = sortHosts(node.hosts);
    node.children.sort((a, b) => a.group.sort - b.group.sort || byName(a.group.name, b.group.name));
    node.total = node.hosts.length + node.children.reduce((n, c) => n + finish(c), 0);
    return node.total;
  };
  roots.sort((a, b) => a.group.sort - b.group.sort || byName(a.group.name, b.group.name));
  roots.forEach(finish);
  return { roots, loose: sortHosts(loose) };
}

/** `Production / EU` for a host, or null when it has no group. */
export function groupPath(data: HostsData, groupId: GroupId | null): string | null {
  const names: string[] = [];
  let cursor = groupId;
  for (let guard = 0; cursor && guard < 32; guard++) {
    const g = data.groups.find((x) => x.id === cursor);
    if (!g) break;
    names.unshift(g.name);
    cursor = g.parentId;
  }
  return names.length ? names.join(' / ') : null;
}

export interface HostHit extends Ranked<Host> {
  path: string | null;
}

/** Sidebar search. `#tag` filters by tag; everything else fuzzy-matches name, address, tags and group. */
export function searchHosts(data: HostsData, query: string): HostHit[] {
  const q = query.trim();
  let hosts = data.hosts;
  let text = q;
  const tagMatch = /(^|\s)#(\S+)/g;
  const tags: string[] = [];
  for (const m of q.matchAll(tagMatch)) tags.push(m[2].toLowerCase());
  if (tags.length) {
    text = q.replace(tagMatch, ' ').trim();
    hosts = hosts.filter((h) => tags.every((t) => h.tags.some((ht) => ht.toLowerCase().includes(t))));
  }
  const paths = new Map(data.groups.map((g) => [g.id, groupPath(data, g.id)]));
  const ranked = rankBy(hosts, text, [
    { text: (h) => h.label, weight: 1, primary: true },
    { text: (h) => h.hostname, weight: 0.75 },
    { text: (h) => `${h.username}@${h.hostname}`, weight: 0.6 },
    { text: (h) => h.tags.join(' '), weight: 0.55 },
    { text: (h) => (h.groupId ? paths.get(h.groupId) : null), weight: 0.4 },
  ]);
  return ranked.map((r) => ({ ...r, path: r.item.groupId ? (paths.get(r.item.groupId) ?? null) : null }));
}

/** Every tag in use with its host count, most used first. */
export function tagCounts(hosts: Host[]): { tag: string; count: number }[] {
  const counts = new Map<string, number>();
  for (const h of hosts) for (const t of h.tags) counts.set(t, (counts.get(t) ?? 0) + 1);
  return [...counts.entries()].map(([tag, count]) => ({ tag, count })).sort((a, b) => b.count - a.count || byName(a.tag, b.tag));
}

/** Is `descendant` inside `ancestor` (or the same group)? Guards drag-and-drop against loops. */
export function isWithin(data: HostsData, descendant: GroupId | null, ancestor: GroupId): boolean {
  let cursor = descendant;
  for (let guard = 0; cursor && guard < 32; guard++) {
    if (cursor === ancestor) return true;
    cursor = data.groups.find((g) => g.id === cursor)?.parentId ?? null;
  }
  return false;
}

export { fuzzyMatch };
