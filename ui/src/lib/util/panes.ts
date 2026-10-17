// The split-pane tree: leaves are terminals, splits divide space. All operations return
// new trees so they're easy to reason about and test.

import type { ConnectionId, Host, HostId, TerminalId } from '$lib/bindings';

export type SplitDir = 'row' | 'col';

export interface Leaf {
  kind: 'leaf';
  id: string;
  hostId: HostId | null;
  /** The host this pane connects to (also covers unsaved quick-connect targets). */
  host: Host | null;
  /** `user@host` typed into quick connect, until it resolves to a host. */
  target: string | null;
  connectionId: ConnectionId | null;
  terminalId: TerminalId | null;
  /** How the pane obtains its terminal. */
  open: 'new' | 'split' | 'attach';
  /** For `open: 'split'`: the connection to start another shell on. */
  splitOf: ConnectionId | null;
}

export interface Split {
  kind: 'split';
  id: string;
  dir: SplitDir;
  /** Share of the space given to `a`, 0..1. */
  ratio: number;
  a: PaneNode;
  b: PaneNode;
}

export type PaneNode = Leaf | Split;

let counter = 0;
export function uid(prefix = 'p'): string {
  counter += 1;
  return `${prefix}${Date.now().toString(36)}${counter.toString(36)}`;
}

export function newLeaf(init: Partial<Leaf> = {}): Leaf {
  return {
    kind: 'leaf',
    id: uid('pane-'),
    hostId: null,
    host: null,
    target: null,
    connectionId: null,
    terminalId: null,
    open: 'new',
    splitOf: null,
    ...init,
  };
}

export function leaves(node: PaneNode): Leaf[] {
  return node.kind === 'leaf' ? [node] : [...leaves(node.a), ...leaves(node.b)];
}

export function findLeaf(node: PaneNode, id: string): Leaf | undefined {
  return leaves(node).find((l) => l.id === id);
}

/** Splits `leafId` in two, placing `fresh` after it. */
export function splitLeaf(node: PaneNode, leafId: string, dir: SplitDir, fresh: Leaf, ratio = 0.5): PaneNode {
  if (node.kind === 'leaf') {
    return node.id === leafId ? { kind: 'split', id: uid('split-'), dir, ratio, a: node, b: fresh } : node;
  }
  return { ...node, a: splitLeaf(node.a, leafId, dir, fresh, ratio), b: splitLeaf(node.b, leafId, dir, fresh, ratio) };
}

/** Removes a leaf; its sibling takes the freed space. Returns `null` when the tree empties. */
export function removeLeaf(node: PaneNode, leafId: string): PaneNode | null {
  if (node.kind === 'leaf') return node.id === leafId ? null : node;
  const a = removeLeaf(node.a, leafId);
  const b = removeLeaf(node.b, leafId);
  if (a && b) return { ...node, a, b };
  return a ?? b;
}

export function setRatio(node: PaneNode, splitId: string, ratio: number): PaneNode {
  if (node.kind === 'leaf') return node;
  const clamped = Math.min(0.9, Math.max(0.1, ratio));
  if (node.id === splitId) return { ...node, ratio: clamped };
  return { ...node, a: setRatio(node.a, splitId, ratio), b: setRatio(node.b, splitId, ratio) };
}

/** Returns the leaf that should receive focus after `leafId` is removed. */
export function neighbourAfterRemoval(node: PaneNode, leafId: string): string | null {
  const all = leaves(node);
  const i = all.findIndex((l) => l.id === leafId);
  if (i < 0) return all[0]?.id ?? null;
  return (all[i + 1] ?? all[i - 1])?.id ?? null;
}

/** Resets every split to equal shares (the "balance panes" command). */
export function balance(node: PaneNode): PaneNode {
  return node.kind === 'leaf' ? node : { ...node, ratio: 0.5, a: balance(node.a), b: balance(node.b) };
}

// ── layout ───────────────────────────────────────────────────────────────────

/** A rectangle as fractions (0..1) of the tab's pane area. */
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** The draggable line between the two halves of a split. */
export interface Divider {
  splitId: string;
  dir: SplitDir;
  /** The area the split occupies, so a drag can turn pointer position back into a ratio. */
  parent: Rect;
  /** Where the line sits along the split's axis, as a fraction of the whole area. */
  at: number;
}

export interface PaneLayout {
  rects: Map<string, Rect>;
  dividers: Divider[];
}

/** Turns the split tree into flat rectangles. Panes are rendered from this (not recursively) so a
 *  terminal's component never remounts when its neighbours are split, closed or resized. */
export function layoutPanes(node: PaneNode, rect: Rect = { x: 0, y: 0, w: 1, h: 1 }, out: PaneLayout = { rects: new Map(), dividers: [] }): PaneLayout {
  if (node.kind === 'leaf') {
    out.rects.set(node.id, rect);
    return out;
  }
  if (node.dir === 'row') {
    const aw = rect.w * node.ratio;
    layoutPanes(node.a, { x: rect.x, y: rect.y, w: aw, h: rect.h }, out);
    layoutPanes(node.b, { x: rect.x + aw, y: rect.y, w: rect.w - aw, h: rect.h }, out);
    out.dividers.push({ splitId: node.id, dir: 'row', parent: rect, at: rect.x + aw });
  } else {
    const ah = rect.h * node.ratio;
    layoutPanes(node.a, { x: rect.x, y: rect.y, w: rect.w, h: ah }, out);
    layoutPanes(node.b, { x: rect.x, y: rect.y + ah, w: rect.w, h: rect.h - ah }, out);
    out.dividers.push({ splitId: node.id, dir: 'col', parent: rect, at: rect.y + ah });
  }
  return out;
}

/** `user@host[:port]` for a host (the shape quick-connect accepts). */
export function hostTarget(host: Pick<Host, 'username' | 'hostname' | 'port'>): string {
  const h = host.hostname.includes(':') ? `[${host.hostname}]` : host.hostname;
  return `${host.username ? host.username + '@' : ''}${h}${host.port !== 22 ? ':' + host.port : ''}`;
}

// ── persistence ──────────────────────────────────────────────────────────────

export type SavedNode =
  | { kind: 'leaf'; hostId: HostId | null; target: string | null; active?: boolean }
  | { kind: 'split'; dir: SplitDir; ratio: number; a: SavedNode; b: SavedNode };

export function serialize(node: PaneNode, activeId: string): SavedNode {
  if (node.kind === 'leaf') {
    return {
      kind: 'leaf',
      hostId: node.hostId,
      target: node.hostId ? null : node.host ? hostTarget(node.host) : node.target,
      active: node.id === activeId || undefined,
    };
  }
  return { kind: 'split', dir: node.dir, ratio: node.ratio, a: serialize(node.a, activeId), b: serialize(node.b, activeId) };
}

export function deserialize(saved: SavedNode): { node: PaneNode; active: string | null } {
  let active: string | null = null;
  const walk = (s: SavedNode): PaneNode => {
    if (s.kind === 'leaf') {
      const leaf = newLeaf({ hostId: s.hostId, target: s.target });
      if (s.active) active = leaf.id;
      return leaf;
    }
    return { kind: 'split', id: uid('split-'), dir: s.dir, ratio: s.ratio, a: walk(s.a), b: walk(s.b) };
  };
  const node = walk(saved);
  return { node, active: active ?? leaves(node)[0]?.id ?? null };
}
