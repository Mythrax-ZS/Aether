import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TypingMark } from './typing';

describe('TypingMark', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('is down until the person does something', () => {
    expect(new TypingMark().active).toBe(false);
  });

  it('goes up for the event being handled and down again by itself', () => {
    const mark = new TypingMark();
    mark.begin();
    expect(mark.active).toBe(true); // xterm turns the key into data in this very turn
    vi.advanceTimersByTime(0);
    expect(mark.active).toBe(false);
  });

  it('never depends on anyone lowering it: it cannot get stuck up', () => {
    const mark = new TypingMark();
    for (let i = 0; i < 50; i++) {
      mark.begin();
      vi.advanceTimersByTime(1);
    }
    expect(mark.active).toBe(false);
    mark.begin();
    mark.begin();
    vi.advanceTimersByTime(1);
    expect(mark.active).toBe(false);
  });

  it('stays up a little longer for input xterm reads off a timer', () => {
    const mark = new TypingMark();
    mark.begin(30);
    vi.advanceTimersByTime(29);
    expect(mark.active).toBe(true);
    vi.advanceTimersByTime(1);
    expect(mark.active).toBe(false);
  });

  it('a newer keystroke extends the window instead of being cut short by an older timer', () => {
    const mark = new TypingMark();
    mark.begin(30);
    vi.advanceTimersByTime(20);
    mark.begin(30);
    vi.advanceTimersByTime(20); // 40 ms after the first, 20 after the second
    expect(mark.active).toBe(true);
    vi.advanceTimersByTime(10);
    expect(mark.active).toBe(false);
  });

  it('is up for exactly the length of a paste, and returns what the paste returns', () => {
    const mark = new TypingMark();
    let during = false;
    expect(
      mark.during(() => {
        during = mark.active;
        return 'pasted';
      }),
    ).toBe('pasted');
    expect(during).toBe(true);
    expect(mark.active).toBe(false);
  });

  it('comes down after a paste that throws', () => {
    const mark = new TypingMark();
    expect(() =>
      mark.during(() => {
        throw new Error('boom');
      }),
    ).toThrow('boom');
    expect(mark.active).toBe(false);
  });

  it('a paste in the middle of a keystroke leaves that keystroke’s window alone', () => {
    const mark = new TypingMark();
    mark.begin();
    mark.during(() => undefined);
    expect(mark.active).toBe(true);
    vi.advanceTimersByTime(0);
    expect(mark.active).toBe(false);
  });

  it('is down once disposed, and stays down', () => {
    const mark = new TypingMark();
    mark.begin(1000);
    mark.dispose();
    expect(mark.active).toBe(false);
    vi.advanceTimersByTime(2000);
    expect(mark.active).toBe(false);
  });
});
