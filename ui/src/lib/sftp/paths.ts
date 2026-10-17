// Path handling for the two sides of the file browser: POSIX paths on the server, Windows paths here.

export interface Crumb {
  name: string;
  path: string;
}

export interface PathOps {
  /** `dir` + `name` with exactly one separator. */
  join(dir: string, name: string): string;
  /** The folder above, or `null` at the top. */
  parent(path: string): string | null;
  /** Clickable pieces for a breadcrumb bar. */
  crumbs(path: string): Crumb[];
  /** Last segment. */
  base(path: string): string;
  /** Is `path` inside `dir` (or equal to it)? Guards against dropping a folder into itself. */
  within(path: string, dir: string): boolean;
}

export const posix: PathOps = {
  join(dir, name) {
    return dir.endsWith('/') ? dir + name : `${dir}/${name}`;
  },
  parent(path) {
    const trimmed = path.replace(/\/+$/, '');
    if (trimmed === '') return null;
    const i = trimmed.lastIndexOf('/');
    return i <= 0 ? '/' : trimmed.slice(0, i);
  },
  crumbs(path) {
    const out: Crumb[] = [{ name: '/', path: '/' }];
    let acc = '';
    for (const part of path.split('/').filter(Boolean)) {
      acc += `/${part}`;
      out.push({ name: part, path: acc });
    }
    return out;
  },
  base(path) {
    const trimmed = path.replace(/\/+$/, '');
    return trimmed.slice(trimmed.lastIndexOf('/') + 1) || '/';
  },
  within(path, dir) {
    const d = dir.replace(/\/+$/, '');
    return path === dir || path === d || path.startsWith(`${d}/`);
  },
};

/** The synthetic top of the local tree: lists drives and shortcuts. */
export const THIS_PC = '';

export const win: PathOps = {
  join(dir, name) {
    return dir.endsWith('\\') || dir.endsWith('/') ? dir + name : `${dir}\\${name}`;
  },
  parent(path) {
    if (path === THIS_PC) return null;
    const trimmed = path.replace(/[\\/]+$/, '');
    const i = Math.max(trimmed.lastIndexOf('\\'), trimmed.lastIndexOf('/'));
    // `C:` or `C:\` → the list of drives.
    if (i < 0 || /^[a-z]:$/i.test(trimmed)) return THIS_PC;
    const up = trimmed.slice(0, i);
    return /^[a-z]:$/i.test(up) ? `${up}\\` : up || THIS_PC;
  },
  crumbs(path) {
    const out: Crumb[] = [{ name: 'This PC', path: THIS_PC }];
    if (path === THIS_PC) return out;
    const parts = path.split(/[\\/]+/).filter(Boolean);
    let acc = '';
    for (let i = 0; i < parts.length; i++) {
      acc = i === 0 ? (/^[a-z]:$/i.test(parts[0]) ? `${parts[0]}\\` : parts[0]) : `${acc.replace(/\\$/, '')}\\${parts[i]}`;
      out.push({ name: parts[i], path: acc });
    }
    return out;
  },
  base(path) {
    const trimmed = path.replace(/[\\/]+$/, '');
    return trimmed.slice(Math.max(trimmed.lastIndexOf('\\'), trimmed.lastIndexOf('/')) + 1) || path;
  },
  within(path, dir) {
    const norm = (s: string) => s.replace(/\//g, '\\').replace(/\\+$/, '').toLowerCase();
    const p = norm(path);
    const d = norm(dir);
    return p === d || p.startsWith(`${d}\\`);
  },
};

// ── sorting ──────────────────────────────────────────────────────────────────

export type SortKey = 'name' | 'size' | 'modified' | 'kind' | 'mode';

export interface Sortable {
  name: string;
  isDir: boolean;
  size: number;
  modified: number | null;
  mode?: number | null;
}

const collator = new Intl.Collator(undefined, { sensitivity: 'base', numeric: true });

export function extension(name: string): string {
  const i = name.lastIndexOf('.');
  return i > 0 ? name.slice(i + 1).toLowerCase() : '';
}

/** Folders first, then by the chosen column; ties fall back to the name. */
export function sortEntries<T extends Sortable>(entries: readonly T[], key: SortKey, ascending: boolean): T[] {
  const dir = ascending ? 1 : -1;
  const byName = (a: T, b: T) => collator.compare(a.name, b.name);
  return [...entries].sort((a, b) => {
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
    let c = 0;
    switch (key) {
      case 'size':
        c = a.size - b.size;
        break;
      case 'modified':
        c = (a.modified ?? 0) - (b.modified ?? 0);
        break;
      case 'kind':
        c = collator.compare(extension(a.name), extension(b.name));
        break;
      case 'mode':
        c = (a.mode ?? 0) - (b.mode ?? 0);
        break;
      default:
        c = byName(a, b);
    }
    return c !== 0 ? c * dir : byName(a, b);
  });
}

/** Indices between two positions, inclusive and order-independent (shift-click ranges). */
export function range(a: number, b: number): number[] {
  const [lo, hi] = a <= b ? [a, b] : [b, a];
  return Array.from({ length: hi - lo + 1 }, (_, i) => lo + i);
}

/** First entry at or after `from` whose name starts with `prefix` (type-ahead find). */
export function findByPrefix(names: readonly string[], prefix: string, from: number): number {
  const p = prefix.toLowerCase();
  if (!p) return -1;
  for (let i = 0; i < names.length; i++) {
    const idx = (from + i) % names.length;
    if (names[idx].toLowerCase().startsWith(p)) return idx;
  }
  return -1;
}

/** A name that doesn't collide: `report.pdf` → `report (2).pdf`. */
export function uniqueName(name: string, taken: ReadonlySet<string>): string {
  if (!taken.has(name)) return name;
  const dot = name.lastIndexOf('.');
  const [stem, ext] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ''];
  for (let n = 2; n < 10_000; n++) {
    const candidate = `${stem} (${n})${ext}`;
    if (!taken.has(candidate)) return candidate;
  }
  return `${stem} (${Date.now()})${ext}`;
}

/** Characters servers and Windows disagree about; used to reject a name before sending it. */
export function invalidName(name: string, side: 'local' | 'remote'): string | null {
  const n = name.trim();
  if (!n) return 'Enter a name.';
  if (n === '.' || n === '..') return 'That name is reserved.';
  if (n.includes('/') || (side === 'local' && /[\\:*?"<>|]/.test(n))) return side === 'local' ? 'Names can’t contain \\ / : * ? " < > |' : 'Names can’t contain “/”.';
  if (side === 'local' && /[. ]$/.test(n)) return 'Windows names can’t end with a dot or space.';
  return null;
}
