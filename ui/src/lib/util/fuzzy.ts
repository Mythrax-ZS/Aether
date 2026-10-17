// Fuzzy subsequence matching in the style of fzy: consecutive runs and word starts
// score higher, gaps cost a little. Powers the command palette and sidebar search.

export interface Match {
  score: number;
  /** Positions in the text that matched, for highlighting. */
  indices: number[];
}

const SCORE_GAP_LEADING = -0.005;
const SCORE_GAP_TRAILING = -0.005;
const SCORE_GAP_INNER = -0.01;
const SCORE_CONSECUTIVE = 1.0;
const BONUS_START = 0.9;
const BONUS_WORD = 0.8;
const BONUS_CAMEL = 0.7;
const BONUS_DOT = 0.6;

function bonusAt(text: string, i: number): number {
  if (i === 0) return BONUS_START;
  const prev = text[i - 1];
  const cur = text[i];
  if (prev === '/' || prev === '\\') return BONUS_START;
  if (prev === ' ' || prev === '-' || prev === '_' || prev === '@' || prev === ':') return BONUS_WORD;
  if (prev === '.') return BONUS_DOT;
  if (prev >= 'a' && prev <= 'z' && cur >= 'A' && cur <= 'Z') return BONUS_CAMEL;
  return 0;
}

/** Returns a match, or `null` when `query` isn't a subsequence of `text`. */
export function fuzzyMatch(query: string, text: string): Match | null {
  const n = query.length;
  const m = text.length;
  if (n === 0) return { score: 0, indices: [] };
  if (n > m) return null;

  const q = query.toLowerCase();
  const t = text.toLowerCase();

  // Cheap rejection: is it a subsequence at all?
  let pos = 0;
  for (let i = 0; i < m && pos < n; i++) if (t[i] === q[pos]) pos++;
  if (pos < n) return null;

  // D[i][j]: best score matching q[0..i] with q[i] at t[j]; M[i][j]: best for q[0..i] within t[0..j].
  const NEG = -Infinity;
  const D: number[][] = Array.from({ length: n }, () => new Array<number>(m).fill(NEG));
  const M: number[][] = Array.from({ length: n }, () => new Array<number>(m).fill(NEG));
  for (let i = 0; i < n; i++) {
    const gap = i === n - 1 ? SCORE_GAP_TRAILING : SCORE_GAP_INNER;
    for (let j = 0; j < m; j++) {
      if (q[i] === t[j]) {
        let score = NEG;
        if (i === 0) {
          score = j * SCORE_GAP_LEADING + bonusAt(text, j);
        } else if (j > 0) {
          score = Math.max(M[i - 1][j - 1] + bonusAt(text, j), D[i - 1][j - 1] + SCORE_CONSECUTIVE);
        }
        D[i][j] = score;
        M[i][j] = Math.max(score, j > 0 ? M[i][j - 1] + gap : NEG);
      } else {
        M[i][j] = j > 0 ? M[i][j - 1] + gap : NEG;
      }
    }
  }
  const score = M[n - 1][m - 1];
  if (score === NEG) return null;

  // Backtrack to recover which characters matched.
  const indices = new Array<number>(n);
  let matchRequired = false;
  for (let i = n - 1, j = m - 1; i >= 0; i--) {
    for (; j >= 0; j--) {
      if (D[i][j] !== NEG && (matchRequired || D[i][j] === M[i][j])) {
        matchRequired = i > 0 && j > 0 && M[i][j] === D[i - 1][j - 1] + SCORE_CONSECUTIVE;
        indices[i] = j--;
        break;
      }
    }
  }
  return { score, indices };
}

export interface Ranked<T> {
  item: T;
  score: number;
  /** Highlight positions within the primary field. */
  indices: number[];
}

export interface Field<T> {
  text: (item: T) => string | undefined | null;
  /** Multiplier so a hit in the name outranks a hit in the notes. */
  weight: number;
  /** Whether this field's indices are the ones to highlight. */
  primary?: boolean;
}

/** Ranks items by their best-matching field. An empty query keeps the input order. */
export function rankBy<T>(items: readonly T[], query: string, fields: Field<T>[]): Ranked<T>[] {
  const q = query.trim();
  if (!q) return items.map((item) => ({ item, score: 0, indices: [] }));
  const out: Ranked<T>[] = [];
  for (const item of items) {
    let best = -Infinity;
    let indices: number[] = [];
    for (const f of fields) {
      const text = f.text(item);
      if (!text) continue;
      const m = fuzzyMatch(q, text);
      if (!m) continue;
      // Scores are small and can be negative; shift so weights act as a bonus.
      const weighted = m.score * f.weight + f.weight * 2;
      if (weighted > best) {
        best = weighted;
        indices = f.primary ? m.indices : [];
      }
    }
    if (best > -Infinity) out.push({ item, score: best, indices });
  }
  return out.sort((a, b) => b.score - a.score);
}

/**
 * Fits one line of terminal output into a result row without losing the match: trims it, and when the match would
 * fall off the end, starts a little before it. Returns the text to show and which of its characters to highlight.
 */
export function excerptAround(line: string, start: number, length: number, max = 160): { text: string; indices: number[] } {
  const trimmed = line.trim();
  const at = Math.min(Math.max(start - (line.length - line.trimStart().length), 0), trimmed.length);
  const end = Math.min(at + length, trimmed.length);
  const from = end > max ? Math.max(0, at - 30) : 0;
  const prefix = from > 0 ? '…' : '';
  const body = trimmed.slice(from, from + max);
  const suffix = from + max < trimmed.length ? '…' : '';
  const indices: number[] = [];
  for (let i = Math.max(at, from); i < Math.min(end, from + max); i++) indices.push(i - from + prefix.length);
  return { text: prefix + body + suffix, indices };
}

/** Splits text into runs for rendering `<mark>`s at the matched indices. */
export function highlightRuns(text: string, indices: number[]): { text: string; hit: boolean }[] {
  if (indices.length === 0) return [{ text, hit: false }];
  const hits = new Set(indices);
  const runs: { text: string; hit: boolean }[] = [];
  for (let i = 0; i < text.length; i++) {
    const hit = hits.has(i);
    const last = runs[runs.length - 1];
    if (last && last.hit === hit) last.text += text[i];
    else runs.push({ text: text[i], hit });
  }
  return runs;
}
