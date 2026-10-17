import { describe, expect, it } from 'vitest';
import { commandLine, isSshCommand } from './cmdline';

describe('commandLine', () => {
  it('joins plain words with spaces', () => {
    expect(commandLine(['ssh', '-J', 'bastion', 'me@db'])).toBe('ssh -J bastion me@db');
  });

  it('quotes what has spaces in it, and leaves backslashes alone', () => {
    expect(commandLine(['ssh', '-i', String.raw`C:\Users\Jo Smith\.ssh\work`, 'me@db'])).toBe(
      String.raw`ssh -i "C:\Users\Jo Smith\.ssh\work" me@db`,
    );
    expect(commandLine(['ssh', '-i', String.raw`C:\keys\id_ed25519`, 'me@db'])).toBe(String.raw`ssh -i C:\keys\id_ed25519 me@db`);
  });

  it('keeps an empty argument and picks the quote that fits', () => {
    expect(commandLine(['a', '', 'b'])).toBe('a "" b');
    expect(commandLine(['say "hi" there'])).toBe(`'say "hi" there'`);
    expect(commandLine(["it's"])).toBe(`"it's"`);
  });
});

describe('isSshCommand', () => {
  it('tells a pasted command from destinations', () => {
    expect(isSshCommand(['ssh', '-J', 'bastion', 'me@db'])).toBe(true);
    expect(isSshCommand(['SSH.EXE', 'me@db'])).toBe(true);
    expect(isSshCommand(['ssh'])).toBe(false);
    expect(isSshCommand(['me@db', 'other@host'])).toBe(false);
    expect(isSshCommand([])).toBe(false);
  });
});
