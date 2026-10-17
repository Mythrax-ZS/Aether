// When the page may ask for its window to be shown. The window starts hidden and is revealed once there is something
// painted to see — two animation frames after the page finished starting — so it never opens on an empty background.
//
// A hidden window is only painted in some web views. WebView2 (Windows) keeps producing animation frames for it;
// WebKitGTK (Linux and the BSDs) produces none, so waiting for frames alone would leave the window hidden for good.
// If no frames come soon enough, the window is shown anyway.

export interface Scheduler {
  /** Runs `callback` before the next frame is drawn. */
  frame(callback: () => void): void;
  /** Runs `callback` after `ms`; the returned function cancels it. */
  after(ms: number, callback: () => void): () => void;
}

/** Long enough for two frames to arrive on any machine that is going to produce them, short enough not to be felt. */
const FALLBACK_MS = 300;

const inThePage: Scheduler = {
  frame: (callback) => {
    requestAnimationFrame(() => callback());
  },
  after: (ms, callback) => {
    const timer = setTimeout(callback, ms);
    return () => clearTimeout(timer);
  },
};

/** Calls `show` — once — after two animation frames, or after `fallbackMs` if the frames don't come. */
export function whenPainted(show: () => void, schedule: Scheduler = inThePage, fallbackMs = FALLBACK_MS): void {
  let shown = false;
  let cancelFallback = () => {};
  const once = () => {
    if (shown) return;
    shown = true;
    cancelFallback();
    show();
  };
  cancelFallback = schedule.after(fallbackMs, once);
  schedule.frame(() => schedule.frame(once));
}
