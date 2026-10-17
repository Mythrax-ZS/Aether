// Text that crosses a trust boundary at the terminal: what the *server* tells the app to show (titles, notifications), and
// what the *app* types on the user's behalf (paste, dropped file names). A terminal is where a program's output and a
// person's input meet, so both directions need care.

/** Bidirectional overrides and marks: they make text read differently from how it is stored. */
const BIDI = /[؜‎‏‪-‮⁦-⁩]/g;
/** C0 controls except TAB / LF / CR, DEL, and the C1 range. */
const CONTROLS = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/g;
/** Zero-width characters that hide text (the joiners that emoji need are left alone). */
const INVISIBLE = /[​⁠﻿]/g;

/** Cuts to `max` characters (code points, never half of an emoji), marking the cut. */
function clip(text: string, max: number): string {
  const chars = Array.from(text);
  return chars.length > max ? chars.slice(0, max - 1).join('') + '…' : text;
}

/**
 * A line of text from the server that is about to be shown by the app itself — a tab title, a desktop notification —
 * reduced to something that can only be read: no control codes, no direction overrides, one line, bounded length.
 */
export function plainText(text: string, max: number): string {
  const flat = text.replace(CONTROLS, '').replace(BIDI, '').replace(INVISIBLE, '').replace(/\s+/g, ' ').trim();
  return clip(flat, max);
}

/**
 * Text taken from the clipboard that is about to be sent to a shell. Escape and the other control codes are dropped:
 * an ESC in the clipboard can end a bracketed paste early (`ESC [ 2 0 1 ~`) so that what follows is typed as if by hand,
 * or move the cursor and redraw the line to hide what is being run. Direction overrides are dropped for the same
 * reason. Trailing line breaks go too — a copied line usually carries one, and it would run the command before the
 * person has seen it.
 */
export function safePaste(text: string): string {
  return text.replace(CONTROLS, '').replace(BIDI, '').replace(/(?:\r\n|\r|\n)+$/, '');
}

/** True when a paste had anything in it that `safePaste` would remove (other than a trailing line break). */
export function hadHiddenControls(text: string): boolean {
  return /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f؜‎‏‪-‮⁦-⁩]/.test(text);
}

/** Characters that mean nothing to a POSIX shell and so need no quoting. */
const SHELL_SAFE = /^[A-Za-z0-9_@%+=:,./-]+$/;

/**
 * A word for a POSIX shell: plain if it is made of harmless characters, otherwise in single quotes — the only quoting in
 * which nothing is special, `$(…)`, backticks, `!` and backslashes included. (A file called `a$(touch x).txt` must stay
 * a name.)
 */
export function shellQuote(word: string): string {
  if (word === '') return "''";
  if (SHELL_SAFE.test(word)) return word;
  return `'${word.replace(/'/g, `'\\''`)}'`;
}

/** Lets an action through at most once per `minMs`. */
export class Throttle {
  #last = Number.NEGATIVE_INFINITY;
  constructor(private readonly minMs: number) {}

  /** True when enough time has passed since the last time this returned true. */
  allow(now = performance.now()): boolean {
    if (now - this.#last < this.minMs) return false;
    this.#last = now;
    return true;
  }
}
