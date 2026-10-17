// Human-friendly formatting for sizes, speeds, times and permissions.

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];

export function formatBytes(bytes: number, digits = 1): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '—';
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toFixed(value >= 100 ? 0 : digits)} ${UNITS[unit]}`;
}

export function formatSpeed(bytesPerSecond: number): string {
  return bytesPerSecond > 0 ? `${formatBytes(bytesPerSecond)}/s` : '—';
}

export function formatDuration(totalSeconds: number): string {
  const s = Math.max(0, Math.round(totalSeconds));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, '0')}s`;
  const h = Math.floor(m / 60);
  return `${h}h ${String(m % 60).padStart(2, '0')}m`;
}

/** "just now", "5 min ago", "yesterday", or a short date. */
export function formatRelative(ms: number | null | undefined, now = Date.now()): string {
  if (!ms) return 'never';
  const diff = Math.max(0, now - ms);
  const min = 60_000;
  if (diff < min) return 'just now';
  if (diff < 60 * min) return `${Math.floor(diff / min)} min ago`;
  if (diff < 24 * 60 * min) {
    const h = Math.floor(diff / (60 * min));
    return `${h} hour${h === 1 ? '' : 's'} ago`;
  }
  const days = Math.floor(diff / (24 * 60 * min));
  if (days === 1) return 'yesterday';
  if (days < 7) return `${days} days ago`;
  return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: days > 300 ? 'numeric' : undefined });
}

export function formatDateTime(ms: number | null | undefined): string {
  if (!ms) return '—';
  const d = new Date(ms);
  const now = new Date();
  const sameYear = d.getFullYear() === now.getFullYear();
  return d.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    year: sameYear ? undefined : 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  });
}

/** `0o755` → `rwxr-xr-x` */
export function formatMode(mode: number | null | undefined): string {
  if (mode == null) return '—';
  const bit = (shift: number, ch: string) => ((mode >> shift) & 1 ? ch : '-');
  const exec = (shift: number, special: number, on: string, off: string) => {
    const x = (mode >> shift) & 1;
    const s = (mode & special) !== 0;
    return x ? (s ? on : 'x') : s ? off : '-';
  };
  return (
    bit(8, 'r') + bit(7, 'w') + exec(6, 0o4000, 's', 'S') +
    bit(5, 'r') + bit(4, 'w') + exec(3, 0o2000, 's', 'S') +
    bit(2, 'r') + bit(1, 'w') + exec(0, 0o1000, 't', 'T')
  );
}

export function modeToOctal(mode: number): string {
  return (mode & 0o7777).toString(8).padStart(3, '0');
}

export function parseOctal(text: string): number | null {
  const t = text.trim();
  if (!/^[0-7]{3,4}$/.test(t)) return null;
  return parseInt(t, 8);
}

/** Last path segment for POSIX and Windows paths. */
export function baseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, '');
  const i = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  return i >= 0 ? trimmed.slice(i + 1) : trimmed;
}

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** Clamp helper used by sliders and panes. */
export function clamp(n: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, n));
}

/** Two letters for an avatar: "Test server" → TS, "web-01" → WE, "db-01" → DB. Digits only count when there is nothing else. */
export function initials(text: string): string {
  const words = text.split(/[\s._:@/-]+/).filter(Boolean);
  if (words.length === 0) return '?';
  // Words that start with a letter are the ones people recognise a name by ("web" in "web-01").
  const named = words.filter((w) => /^\p{L}/u.test(w));
  if (named.length >= 2) return (named[0][0] + named[1][0]).toUpperCase();
  const first = (named[0] ?? words[0]).replace(/[^\p{L}\p{N}]/gu, '');
  return (first.slice(0, 2) || '?').toUpperCase();
}
