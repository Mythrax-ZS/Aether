import { describe, expect, it } from 'vitest';
import {
  balance, deserialize, hostTarget, layoutPanes, leaves, neighbourAfterRemoval, newLeaf, removeLeaf, serialize, setRatio, splitLeaf,
} from './panes';

const L = (id: string) => newLeaf({ id });

describe('layout', () => {
  it('gives a lone pane the whole area', () => {
    const { rects, dividers } = layoutPanes(L('a'));
    expect(rects.get('a')).toEqual({ x: 0, y: 0, w: 1, h: 1 });
    expect(dividers).toEqual([]);
  });

  it('divides space by ratio and reports the divider position', () => {
    const tree = splitLeaf(L('a'), 'a', 'row', L('b'), 0.25);
    const { rects, dividers } = layoutPanes(tree);
    expect(rects.get('a')).toEqual({ x: 0, y: 0, w: 0.25, h: 1 });
    expect(rects.get('b')).toEqual({ x: 0.25, y: 0, w: 0.75, h: 1 });
    expect(dividers).toHaveLength(1);
    expect(dividers[0]).toMatchObject({ dir: 'row', at: 0.25 });
  });

  it('nests splits inside their parent rectangle', () => {
    let tree = splitLeaf(L('a'), 'a', 'row', L('b'));
    tree = splitLeaf(tree, 'b', 'col', L('c'), 0.5);
    const { rects, dividers } = layoutPanes(tree);
    expect(rects.get('b')).toEqual({ x: 0.5, y: 0, w: 0.5, h: 0.5 });
    expect(rects.get('c')).toEqual({ x: 0.5, y: 0.5, w: 0.5, h: 0.5 });
    const inner = dividers.find((d) => d.dir === 'col')!;
    expect(inner.parent).toEqual({ x: 0.5, y: 0, w: 0.5, h: 1 });
    expect(inner.at).toBe(0.5);
  });

  it('tiles the area exactly (no gaps or overlaps)', () => {
    let tree = splitLeaf(L('a'), 'a', 'row', L('b'), 0.3);
    tree = splitLeaf(tree, 'a', 'col', L('c'), 0.6);
    tree = splitLeaf(tree, 'b', 'col', L('d'), 0.4);
    const { rects } = layoutPanes(tree);
    const area = [...rects.values()].reduce((sum, r) => sum + r.w * r.h, 0);
    expect(area).toBeCloseTo(1, 10);
  });
});

describe('pane tree', () => {
  it('splits a leaf and keeps order', () => {
    const tree = splitLeaf(L('a'), 'a', 'row', L('b'));
    expect(tree.kind).toBe('split');
    expect(leaves(tree).map((l) => l.id)).toEqual(['a', 'b']);
  });

  it('splits nested leaves', () => {
    let tree = splitLeaf(L('a'), 'a', 'row', L('b'));
    tree = splitLeaf(tree, 'b', 'col', L('c'));
    expect(leaves(tree).map((l) => l.id)).toEqual(['a', 'b', 'c']);
    // c lives in a vertical split that sits in the right half.
    if (tree.kind !== 'split' || tree.b.kind !== 'split') throw new Error('shape');
    expect(tree.b.dir).toBe('col');
  });

  it('removes a leaf and hands its space to the sibling', () => {
    let tree = splitLeaf(L('a'), 'a', 'row', L('b'));
    tree = splitLeaf(tree, 'b', 'col', L('c'));
    const after = removeLeaf(tree, 'b')!;
    expect(leaves(after).map((l) => l.id)).toEqual(['a', 'c']);
    expect(removeLeaf(L('x'), 'x')).toBeNull();
    expect(removeLeaf(L('x'), 'other')).not.toBeNull();
  });

  it('chooses sensible focus after removal', () => {
    let tree = splitLeaf(L('a'), 'a', 'row', L('b'));
    tree = splitLeaf(tree, 'b', 'row', L('c'));
    expect(neighbourAfterRemoval(tree, 'b')).toBe('c');
    expect(neighbourAfterRemoval(tree, 'c')).toBe('b');
    expect(neighbourAfterRemoval(L('only'), 'only')).toBeNull();
  });

  it('clamps and balances ratios', () => {
    const tree = splitLeaf(L('a'), 'a', 'row', L('b'));
    if (tree.kind !== 'split') throw new Error('shape');
    expect(setRatio(tree, tree.id, 0.01)).toMatchObject({ ratio: 0.1 });
    expect(setRatio(tree, tree.id, 5)).toMatchObject({ ratio: 0.9 });
    expect(balance(setRatio(tree, tree.id, 0.8))).toMatchObject({ ratio: 0.5 });
  });
});

describe('persistence', () => {
  it('round-trips layout, hosts and the active pane', () => {
    let tree = splitLeaf(newLeaf({ id: 'a', hostId: 'h1' }), 'a', 'col', newLeaf({ id: 'b', target: 'me@box:2200' }), 0.3);
    const saved = serialize(tree, 'b');
    const { node, active } = deserialize(saved);
    expect(node.kind).toBe('split');
    const ls = leaves(node);
    expect(ls.map((l) => l.hostId)).toEqual(['h1', null]);
    expect(ls[1].target).toBe('me@box:2200');
    expect(active).toBe(ls[1].id);
    expect(ls.every((l) => l.open === 'new' && l.terminalId === null)).toBe(true);
    if (node.kind === 'split') expect(node.ratio).toBe(0.3);
    tree = node;
  });

  it('formats targets for quick connect', () => {
    expect(hostTarget({ username: 'me', hostname: 'box', port: 22 })).toBe('me@box');
    expect(hostTarget({ username: '', hostname: 'box', port: 2222 })).toBe('box:2222');
    expect(hostTarget({ username: 'me', hostname: 'fe80::1', port: 22 })).toBe('me@[fe80::1]');
  });
});
