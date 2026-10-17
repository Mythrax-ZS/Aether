import { describe, expect, it, vi } from 'vitest';
import { whenPainted, type Scheduler } from './reveal';

/** A page whose frames and timers are driven by hand. */
function manualPage() {
  const frames: Array<() => void> = [];
  const timers: Array<{ ms: number; run: () => void; cancelled: boolean }> = [];
  const schedule: Scheduler = {
    frame: (callback) => void frames.push(callback),
    after: (ms, run) => {
      const timer = { ms, run, cancelled: false };
      timers.push(timer);
      return () => void (timer.cancelled = true);
    },
  };
  return {
    schedule,
    timers,
    /** Draws one frame: runs whatever was waiting for it (and what that asks for runs on the next one). */
    drawFrame: () => frames.splice(0).forEach((callback) => callback()),
    /** Lets time pass for every timer that has not been cancelled. */
    letTimePass: () => timers.filter((t) => !t.cancelled).forEach((t) => t.run()),
  };
}

describe('whenPainted', () => {
  it('waits for two frames, and no fewer', () => {
    const page = manualPage();
    const show = vi.fn();
    whenPainted(show, page.schedule);
    expect(show).not.toHaveBeenCalled();
    page.drawFrame();
    expect(show).not.toHaveBeenCalled();
    page.drawFrame();
    expect(show).toHaveBeenCalledTimes(1);
  });

  it('does not wait for frames that never come', () => {
    // WebKitGTK paints nothing for a hidden window, so the animation frames never arrive.
    const page = manualPage();
    const show = vi.fn();
    whenPainted(show, page.schedule);
    page.letTimePass();
    expect(show).toHaveBeenCalledTimes(1);
  });

  it('shows once even when the frames come after the fallback, or the fallback after the frames', () => {
    const late = manualPage();
    const lateShow = vi.fn();
    whenPainted(lateShow, late.schedule);
    late.letTimePass();
    late.drawFrame();
    late.drawFrame();
    expect(lateShow).toHaveBeenCalledTimes(1);

    const early = manualPage();
    const earlyShow = vi.fn();
    whenPainted(earlyShow, early.schedule);
    early.drawFrame();
    early.drawFrame();
    early.letTimePass();
    expect(earlyShow).toHaveBeenCalledTimes(1);
  });

  it('puts the fallback away once the frames have come', () => {
    const page = manualPage();
    whenPainted(vi.fn(), page.schedule);
    page.drawFrame();
    page.drawFrame();
    expect(page.timers.every((t) => t.cancelled)).toBe(true);
  });

  it('gives the frames 300 ms by default, and takes another time if asked', () => {
    const page = manualPage();
    whenPainted(vi.fn(), page.schedule);
    expect(page.timers.map((t) => t.ms)).toEqual([300]);

    const other = manualPage();
    whenPainted(vi.fn(), other.schedule, 1200);
    expect(other.timers.map((t) => t.ms)).toEqual([1200]);
  });
});
