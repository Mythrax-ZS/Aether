import { describe, expect, it } from 'vitest';
import type { Group, Host, HostsData } from '$lib/bindings';
import { sampleHost } from '$lib/api/fake';
import { buildTree, groupPath, isWithin, searchHosts, tagCounts } from './tree';

const group = (id: string, name: string, parentId: string | null = null, sort = 0): Group => ({
  id, name, parentId, color: null, sort, collapsed: false,
});
const host = (label: string, extra: Partial<Host> = {}): Host => sampleHost(label, `${label}.example.com`, extra);

function data(): HostsData {
  const prod = group('g-prod', 'Production', null, 0);
  const eu = group('g-eu', 'EU', 'g-prod', 0);
  const lab = group('g-lab', 'Lab', null, 1);
  return {
    version: 1,
    groups: [lab, eu, prod],
    hosts: [
      host('web-02', { groupId: 'g-prod', tags: ['web'] }),
      host('web-01', { groupId: 'g-prod', tags: ['web', 'nginx'] }),
      host('eu-db', { groupId: 'g-eu', tags: ['postgres'] }),
      host('pi', { groupId: 'g-lab' }),
      host('bastion'),
    ],
  };
}

describe('buildTree', () => {
  it('nests groups, sorts everything and counts hosts recursively', () => {
    const t = buildTree(data());
    expect(t.roots.map((r) => r.group.name)).toEqual(['Production', 'Lab']);
    const prod = t.roots[0];
    expect(prod.hosts.map((h) => h.label)).toEqual(['web-01', 'web-02']);
    expect(prod.children.map((c) => c.group.name)).toEqual(['EU']);
    expect(prod.total).toBe(3);
    expect(t.loose.map((h) => h.label)).toEqual(['bastion']);
  });

  it('does not lose groups whose parent disappeared', () => {
    const d = data();
    d.groups = d.groups.filter((g) => g.id !== 'g-prod');
    const t = buildTree(d);
    expect(t.roots.map((r) => r.group.name).sort()).toEqual(['EU', 'Lab']);
  });
});

describe('searchHosts', () => {
  it('fuzzy matches names first', () => {
    const hits = searchHosts(data(), 'w01');
    expect(hits[0].item.label).toBe('web-01');
    expect(hits[0].path).toBe('Production');
  });

  it('supports #tag filters, alone or combined with text', () => {
    expect(searchHosts(data(), '#nginx').map((h) => h.item.label)).toEqual(['web-01']);
    expect(searchHosts(data(), '#web').map((h) => h.item.label).sort()).toEqual(['web-01', 'web-02']);
    expect(searchHosts(data(), '#web 02').map((h) => h.item.label)).toEqual(['web-02']);
  });

  it('finds hosts by group name', () => {
    expect(searchHosts(data(), 'lab').map((h) => h.item.label)).toContain('pi');
  });

  it('returns everything for an empty query', () => {
    expect(searchHosts(data(), '')).toHaveLength(5);
  });
});

describe('groups', () => {
  it('builds breadcrumb paths', () => {
    expect(groupPath(data(), 'g-eu')).toBe('Production / EU');
    expect(groupPath(data(), null)).toBeNull();
  });

  it('detects containment to prevent drag-drop cycles', () => {
    const d = data();
    expect(isWithin(d, 'g-eu', 'g-prod')).toBe(true);
    expect(isWithin(d, 'g-prod', 'g-eu')).toBe(false);
    expect(isWithin(d, 'g-prod', 'g-prod')).toBe(true);
    expect(isWithin(d, null, 'g-prod')).toBe(false);
  });

  it('counts tags', () => {
    expect(tagCounts(data().hosts)[0]).toEqual({ tag: 'web', count: 2 });
  });
});
