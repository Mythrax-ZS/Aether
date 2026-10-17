import { describe, expect, it } from 'vitest';
import { acceleratorFromEvent, formatAccelerator, matchesAccelerator, normalizeAccelerator, parseAccelerator } from './keys';

function ev(init: Partial<KeyboardEvent> & { key: string; code: string }): KeyboardEvent {
  return { ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, ...init } as KeyboardEvent;
}

describe('accelerators', () => {
  it('parses modifiers and keys', () => {
    expect(parseAccelerator('Ctrl+Shift+T')).toEqual({ ctrl: true, shift: true, alt: false, meta: false, key: 't' });
    expect(parseAccelerator('ctrl+,')).toEqual({ ctrl: true, shift: false, alt: false, meta: false, key: ',' });
    expect(parseAccelerator('Alt+Left')?.key).toBe('arrowleft');
    expect(parseAccelerator('F11')).toEqual({ ctrl: false, shift: false, alt: false, meta: false, key: 'f11' });
    expect(parseAccelerator('Ctrl++')?.key).toBe('+');
    expect(parseAccelerator('Bogus+X')).toBeNull();
    expect(parseAccelerator('')).toBeNull();
  });

  it('matches events regardless of layout shifting', () => {
    const acc = parseAccelerator('Ctrl+Shift+T')!;
    expect(matchesAccelerator(ev({ key: 'T', code: 'KeyT', ctrlKey: true, shiftKey: true }), acc)).toBe(true);
    expect(matchesAccelerator(ev({ key: 't', code: 'KeyT', ctrlKey: true }), acc)).toBe(false);
    expect(matchesAccelerator(ev({ key: 'T', code: 'KeyT', ctrlKey: true, shiftKey: true, altKey: true }), acc)).toBe(false);

    // Digits are matched by physical key so Shift+1 ("!") still counts as 1.
    const digit = parseAccelerator('Ctrl+Alt+1')!;
    expect(matchesAccelerator(ev({ key: '1', code: 'Digit1', ctrlKey: true, altKey: true }), digit)).toBe(true);
  });

  it('does not insist on Shift for symbol keys', () => {
    const plus = parseAccelerator('Ctrl++')!;
    expect(matchesAccelerator(ev({ key: '+', code: 'Equal', ctrlKey: true, shiftKey: true }), plus)).toBe(true);
    expect(matchesAccelerator(ev({ key: '+', code: 'NumpadAdd', ctrlKey: true }), plus)).toBe(true);
  });

  it('formats for display and normalises', () => {
    expect(formatAccelerator(parseAccelerator('ctrl+shift+p')!)).toBe('Ctrl+Shift+P');
    expect(formatAccelerator(parseAccelerator('alt+right')!)).toBe('Alt+→');
    expect(normalizeAccelerator('ctrl+k')).toBe('Ctrl+K');
    expect(normalizeAccelerator('nonsense+??')).toBe('nonsense+??');
  });

  it('records shortcuts from events but ignores bare modifiers', () => {
    expect(acceleratorFromEvent(ev({ key: 'Control', code: 'ControlLeft', ctrlKey: true }))).toBeNull();
    expect(acceleratorFromEvent(ev({ key: 'k', code: 'KeyK', ctrlKey: true }))).toEqual({
      ctrl: true, shift: false, alt: false, meta: false, key: 'k',
    });
  });
});
