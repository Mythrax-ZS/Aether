// Aether is an app, not a web page: switch off the browser behaviours that would break that
// illusion (reload, print, find-in-page, the "Back / Inspect" context menu, pinch/wheel zoom).

const BLOCKED_CTRL = new Set(['r', 'p', 'u', 's', 'g', 'f', 'j', 'h', 'o', 'l', 'n', 't', 'w']);

function editable(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ['INPUT', 'TEXTAREA'].includes(target.tagName) || !!target.closest('.selectable');
}

export function installLockdown(): () => void {
  const onKey = (e: KeyboardEvent) => {
    if (e.defaultPrevented) return; // an app shortcut already took it
    const k = e.key.toLowerCase();
    // Reload / hard reload / caret browsing / print / view-source / save / find / history …
    // (the app's own shortcuts, with or without Shift, are matched earlier and never reach here)
    if (k === 'f5' || k === 'f7' || (e.ctrlKey && !e.altKey && BLOCKED_CTRL.has(k))) e.preventDefault();
    // Browser zoom: the terminal and UI have their own scale settings.
    if (e.ctrlKey && (k === '=' || k === '+' || k === '-' || k === '0')) e.preventDefault();
  };

  const onContext = (e: MouseEvent) => {
    if (!editable(e.target)) e.preventDefault();
  };

  const onWheel = (e: WheelEvent) => {
    if (e.ctrlKey) e.preventDefault();
  };

  // A file dropped on the window must never navigate the webview to it.
  const onDrop = (e: DragEvent) => e.preventDefault();

  window.addEventListener('keydown', onKey);
  window.addEventListener('contextmenu', onContext);
  window.addEventListener('wheel', onWheel, { passive: false });
  window.addEventListener('dragover', onDrop);
  window.addEventListener('drop', onDrop);
  return () => {
    window.removeEventListener('keydown', onKey);
    window.removeEventListener('contextmenu', onContext);
    window.removeEventListener('wheel', onWheel);
    window.removeEventListener('dragover', onDrop);
    window.removeEventListener('drop', onDrop);
  };
}
