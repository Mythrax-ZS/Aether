import { describe, expect, it } from 'vitest';
import { excerptAround, fuzzyMatch, highlightRuns, rankBy } from './fuzzy';

describe('fuzzyMatch', () => {
  it('matches subsequences case-insensitively', () => {
    expect(fuzzyMatch('wb1', 'web-01')).not.toBeNull();
    expect(fuzzyMatch('WEB', 'web-01')).not.toBeNull();
    expect(fuzzyMatch('xyz', 'web-01')).toBeNull();
    expect(fuzzyMatch('web-011', 'web-01')).toBeNull();
  });

  it('reports the matched positions', () => {
    expect(fuzzyMatch('wb', 'web-01')?.indices).toEqual([0, 2]);
    expect(fuzzyMatch('', 'anything')).toEqual({ score: 0, indices: [] });
  });

  it('prefers consecutive runs and word starts', () => {
    const consecutive = fuzzyMatch('db', 'db-primary')!;
    const scattered = fuzzyMatch('db', 'a-d-x-b')!;
    expect(consecutive.score).toBeGreaterThan(scattered.score);

    const wordStart = fuzzyMatch('p', 'db-primary')!;
    const midWord = fuzzyMatch('p', 'dbsprimary')!;
    expect(wordStart.score).toBeGreaterThan(midWord.score);
  });

  it('picks the best alignment, not just the first', () => {
    // "ab" appears early scattered and later as a clean run; the run should win.
    const m = fuzzyMatch('ab', 'axxxxb ab')!;
    expect(m.indices).toEqual([7, 8]);
  });
});

describe('rankBy', () => {
  const hosts = [
    { name: 'web-01', target: 'deploy@10.0.1.11' },
    { name: 'db-primary', target: 'postgres@10.0.2.5' },
    { name: 'nas', target: 'admin@192.168.1.20' },
  ];
  const fields = [
    { text: (h: (typeof hosts)[number]) => h.name, weight: 1, primary: true },
    { text: (h: (typeof hosts)[number]) => h.target, weight: 0.6 },
  ];

  it('ranks name hits above target hits', () => {
    const r = rankBy(hosts, 'db', fields);
    expect(r[0].item.name).toBe('db-primary');
    expect(r[0].indices).toEqual([0, 1]);
  });

  it('finds matches in secondary fields without highlighting them', () => {
    const r = rankBy(hosts, 'postgres', fields);
    expect(r.map((x) => x.item.name)).toEqual(['db-primary']);
    expect(r[0].indices).toEqual([]);
  });

  it('keeps input order for an empty query', () => {
    expect(rankBy(hosts, '  ', fields).map((x) => x.item.name)).toEqual(['web-01', 'db-primary', 'nas']);
  });
});

describe('highlightRuns', () => {
  it('splits text around hits', () => {
    expect(highlightRuns('web-01', [0, 2])).toEqual([
      { text: 'w', hit: true },
      { text: 'e', hit: false },
      { text: 'b', hit: true },
      { text: '-01', hit: false },
    ]);
    expect(highlightRuns('abc', [])).toEqual([{ text: 'abc', hit: false }]);
  });
});

describe('excerptAround', () => {
  const marked = (e: { text: string; indices: number[] }) => e.indices.map((i) => e.text[i]).join('');

  it('trims the line and keeps the highlight on the match', () => {
    const line = '    13:17:06 ERROR worker timed out';
    const e = excerptAround(line, line.indexOf('ERROR'), 5);
    expect(e.text).toBe('13:17:06 ERROR worker timed out');
    expect(marked(e)).toBe('ERROR');
  });

  it('keeps a match that sits near the end of a long line in view', () => {
    const line = `${'x'.repeat(400)} needle ${'y'.repeat(50)}`;
    const e = excerptAround(line, line.indexOf('needle'), 6);
    expect(e.text.startsWith('…')).toBe(true);
    expect(e.text.length).toBeLessThanOrEqual(162);
    expect(marked(e)).toBe('needle');
  });

  it('only marks the end of a line it had to cut short', () => {
    const line = `needle ${'y'.repeat(400)}`;
    const e = excerptAround(line, 0, 6);
    expect(e.text.startsWith('needle')).toBe(true);
    expect(e.text.endsWith('…')).toBe(true);
    expect(marked(e)).toBe('needle');
  });

  it('copes with positions outside the line', () => {
    expect(excerptAround('abc', 10, 3)).toEqual({ text: 'abc', indices: [] });
    expect(excerptAround('', 0, 2)).toEqual({ text: '', indices: [] });
  });
});
