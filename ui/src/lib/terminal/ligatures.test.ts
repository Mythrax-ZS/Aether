import { describe, expect, it } from 'vitest';
import { ligatureRanges } from './ligatures';

describe('ligatureRanges', () => {
  it('finds common operators', () => {
    expect(ligatureRanges('a => b')).toEqual([[2, 4]]);
    expect(ligatureRanges('x != y')).toEqual([[2, 4]]);
    expect(ligatureRanges('if (a === b)')).toEqual([[6, 9]]);
  });

  it('prefers the longest sequence', () => {
    expect(ligatureRanges('<--->')).toEqual([[0, 5]]);
    expect(ligatureRanges('a ==> b')).toEqual([[2, 5]]);
    expect(ligatureRanges('<=>')).toEqual([[0, 3]]);
  });

  it('handles several in one line without overlap', () => {
    expect(ligatureRanges('a -> b => c')).toEqual([[2, 4], [7, 9]]);
  });

  it('leaves ordinary text alone', () => {
    expect(ligatureRanges('hello world')).toEqual([]);
    expect(ligatureRanges('')).toEqual([]);
    expect(ligatureRanges('a=b')).toEqual([]);
  });
});
