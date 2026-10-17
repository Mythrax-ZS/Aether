// Programming ligatures for the WebGL renderer.
//
// xterm's WebGL renderer draws one cell at a time, so the font's `calt` shaping never gets
// to join `=>` or `!=`. A character joiner tells the renderer which cell runs belong together;
// the run is then drawn as one string and the font shapes it. The list below covers the
// sequences JetBrains Mono, Fira Code and Cascadia Code turn into ligatures. Fonts without
// ligatures simply draw the same glyphs they would have anyway.

const SEQUENCES = [
  '<--->', '<---', '--->', '<-->', '<--', '-->', '<->', '<-', '->', '->>', '<<-', '-<<', '>->', '<-<', '>>-', '-<', '>-',
  '<===>', '<===', '===>', '<==>', '<==', '==>', '<=>', '<=<', '>=>', '=>>', '<<=', '>>=', '=<<', '=>', '<=', '>=', '=<', '=>=',
  '!==', '===', '=!=', '==', '!=', '=/=', '=:=', ':=', '=:', '!!', '!!!', '?:', '?.', '??', '???',
  '<~>', '<~~', '~~>', '<~', '~>', '~~', '~=', '=~', '~-', '-~', '~@',
  '|||>', '<|||', '||>', '<||', '|>', '<|', '<|>', '||', '|||', '|=', '|-', '-|', '|}', '{|', '|]', '[|',
  '&&', '&&&', '&=', '++', '+++', '+=', '--', '---', '-=', '**', '***', '*=', '*>', '<*', '<*>', '/=', '//', '///', '/*', '*/', '/>', '</', '</>',
  '<$>', '<$', '$>', '<+>', '<+', '+>', '<>', '><',
  '::', ':::', '...', '..', '.=', '..<', '..=', '.-', '.?',
  '<<', '>>', '<<<', '>>>', '>>=', '<<=', '^=', '%=', '#{', '#(', '#[', '#_', '#?', '#!', '##', '###', '####', '#:', '#=',
  'www', '\\\\', '\\\\\\', ';;', '__', '0xF', ':>', '<:', ':<', '>:', '(*', '*)', '{.', '.}', '<.', '.>', '<.>',
];

const LONGEST = SEQUENCES.reduce((n, s) => Math.max(n, s.length), 0);
const SET = new Set(SEQUENCES);
const INTERESTING = /[=<>!|&:.\-+*/#~^?$%\\;_@{}()[\]w0]/;

/** Ranges `[start, end)` of `text` that should be drawn as one ligature. Longest match wins. */
export function ligatureRanges(text: string): [number, number][] {
  if (!INTERESTING.test(text)) return [];
  const ranges: [number, number][] = [];
  let i = 0;
  while (i < text.length) {
    let matched = 0;
    for (let len = Math.min(LONGEST, text.length - i); len >= 2; len--) {
      if (SET.has(text.substr(i, len))) {
        matched = len;
        break;
      }
    }
    if (matched) {
      ranges.push([i, i + matched]);
      i += matched;
    } else {
      i++;
    }
  }
  return ranges;
}
