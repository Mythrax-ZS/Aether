// Pointer-based drag and drop for things *inside* the app (reordering tabs, moving hosts into
// groups, dragging files between SFTP panes).
//
// HTML5 drag-and-drop can't be used: with Tauri's native file-drop handler on (needed to receive
// files dragged in from Explorer with their real paths) WebView2 never delivers HTML5 drag events.
// Pointer events are also easier to make look good: custom ghost, drop highlights, auto-scroll.

export interface DragPayload {
  /** What is being dragged: `host`, `tab`, `file`, ... Targets use it to decide whether to accept. */
  type: string;
  /** All ids being dragged (the grabbed item plus the rest of a multi-selection). */
  ids: string[];
  label: string;
  sublabel?: string;
  /** Anything else the drop handler needs. */
  data?: unknown;
}

export interface DropPoint {
  x: number;
  y: number;
  /** The drop target's bounds. */
  rect: DOMRect;
}

class DndState {
  active = $state<DragPayload | null>(null);
  x = $state(0);
  y = $state(0);
  /** The element currently under the pointer that accepts this drag. */
  over = $state<HTMLElement | null>(null);
}

export const dnd = new DndState();

// ── drop targets ─────────────────────────────────────────────────────────────

export interface DropOptions {
  accepts: (payload: DragPayload) => boolean;
  ondrop: (payload: DragPayload, point: DropPoint) => void;
  /** Called continuously while a valid drag hovers (to position an insertion marker). */
  onmove?: (payload: DragPayload, point: DropPoint) => void;
  onenter?: (payload: DragPayload) => void;
  onleave?: () => void;
  /** Hovering this long (ms) calls `onhold` (e.g. expand a collapsed group). */
  holdMs?: number;
  onhold?: () => void;
}

const targets = new Map<HTMLElement, () => DropOptions>();
let holdTimer: ReturnType<typeof setTimeout> | undefined;

export function dropTarget(node: HTMLElement, options: DropOptions) {
  let current = options;
  targets.set(node, () => current);
  return {
    update(next: DropOptions) {
      current = next;
    },
    destroy() {
      targets.delete(node);
      if (dnd.over === node) setOver(null, null);
    },
  };
}

function setOver(el: HTMLElement | null, payload: DragPayload | null): void {
  if (dnd.over === el) return;
  clearTimeout(holdTimer);
  const prev = dnd.over;
  if (prev) {
    delete prev.dataset.dropOver;
    targets.get(prev)?.().onleave?.();
  }
  dnd.over = el;
  if (el && payload) {
    el.dataset.dropOver = 'true';
    const opts = targets.get(el)?.();
    opts?.onenter?.(payload);
    if (opts?.onhold) holdTimer = setTimeout(() => opts.onhold?.(), opts.holdMs ?? 650);
  }
}

function hitTest(x: number, y: number, payload: DragPayload): HTMLElement | null {
  for (const el of document.elementsFromPoint(x, y)) {
    const opts = targets.get(el as HTMLElement)?.();
    if (opts && opts.accepts(payload)) return el as HTMLElement;
  }
  return null;
}

// ── auto-scroll ──────────────────────────────────────────────────────────────

const EDGE = 40;

function scrollable(el: Element | null, axis: 'x' | 'y'): HTMLElement | null {
  for (let cur = el; cur && cur !== document.body; cur = cur.parentElement) {
    const s = getComputedStyle(cur);
    const mode = axis === 'y' ? s.overflowY : s.overflowX;
    const can = axis === 'y' ? cur.scrollHeight > cur.clientHeight + 1 : cur.scrollWidth > cur.clientWidth + 1;
    if (can && (mode === 'auto' || mode === 'scroll')) return cur as HTMLElement;
  }
  return null;
}

function autoScroll(x: number, y: number): void {
  const under = document.elementFromPoint(x, y);
  const vert = scrollable(under, 'y');
  if (vert) {
    const r = vert.getBoundingClientRect();
    if (y < r.top + EDGE) vert.scrollTop -= Math.ceil(((r.top + EDGE - y) / EDGE) * 14);
    else if (y > r.bottom - EDGE) vert.scrollTop += Math.ceil(((y - (r.bottom - EDGE)) / EDGE) * 14);
  }
  const horiz = scrollable(under, 'x');
  if (horiz) {
    const r = horiz.getBoundingClientRect();
    if (x < r.left + EDGE) horiz.scrollLeft -= Math.ceil(((r.left + EDGE - x) / EDGE) * 14);
    else if (x > r.right - EDGE) horiz.scrollLeft += Math.ceil(((x - (r.right - EDGE)) / EDGE) * 14);
  }
}

// ── draggable sources ────────────────────────────────────────────────────────

export interface DraggableOptions {
  /** Builds what is dragged. Return `null` to cancel (e.g. nothing selected). */
  payload: (e: PointerEvent) => DragPayload | null;
  disabled?: boolean;
  /** Pixels the pointer must travel before a drag begins (so clicks still work). */
  threshold?: number;
  onstart?: (payload: DragPayload) => void;
  onend?: (dropped: boolean) => void;
}

export function draggable(node: HTMLElement, options: DraggableOptions) {
  let current = options;
  let origin: { x: number; y: number } | null = null;
  let frame = 0;
  let last = { x: 0, y: 0 };
  let didDrag = false;

  const tick = () => {
    if (!dnd.active) return;
    autoScroll(last.x, last.y);
    // Scrolling moves things under a stationary pointer; re-test.
    refresh();
    frame = requestAnimationFrame(tick);
  };

  function refresh(): void {
    const payload = dnd.active;
    if (!payload) return;
    const el = hitTest(last.x, last.y, payload);
    setOver(el, payload);
    if (el) {
      const opts = targets.get(el)?.();
      opts?.onmove?.(payload, { x: last.x, y: last.y, rect: el.getBoundingClientRect() });
    }
  }

  function down(e: PointerEvent): void {
    if (e.button !== 0 || current.disabled || dnd.active) return;
    if ((e.target as HTMLElement | null)?.closest('[data-no-drag], input, textarea, select')) return;
    origin = { x: e.clientX, y: e.clientY };
    didDrag = false;
    window.addEventListener('pointermove', move, true);
    window.addEventListener('pointerup', up, true);
    window.addEventListener('pointercancel', cancel, true);
    window.addEventListener('keydown', key, true);
  }

  function move(e: PointerEvent): void {
    if (!origin) return;
    last = { x: e.clientX, y: e.clientY };
    if (!dnd.active) {
      if (Math.hypot(e.clientX - origin.x, e.clientY - origin.y) < (current.threshold ?? 5)) return;
      const payload = current.payload(e);
      if (!payload) {
        stop();
        return;
      }
      dnd.active = payload;
      didDrag = true;
      document.documentElement.classList.add('dnd-active');
      getSelection()?.removeAllRanges();
      current.onstart?.(payload);
      frame = requestAnimationFrame(tick);
    }
    dnd.x = e.clientX;
    dnd.y = e.clientY;
    e.preventDefault();
    refresh();
  }

  function up(): void {
    const payload = dnd.active;
    const target = dnd.over;
    let dropped = false;
    if (payload && target) {
      const opts = targets.get(target)?.();
      if (opts) {
        dropped = true;
        const point = { x: last.x, y: last.y, rect: target.getBoundingClientRect() };
        // Let the pointerup finish before app state changes under the cursor.
        queueMicrotask(() => opts.ondrop(payload, point));
      }
    }
    finish(dropped);
  }

  function cancel(): void {
    finish(false);
  }

  function key(e: KeyboardEvent): void {
    if (e.key === 'Escape' && dnd.active) {
      e.preventDefault();
      e.stopPropagation();
      finish(false);
    }
  }

  function finish(dropped: boolean): void {
    const wasActive = !!dnd.active;
    stop();
    if (wasActive) {
      // The click that follows pointerup must not activate whatever is under the cursor.
      const swallow = (e: MouseEvent) => {
        e.stopPropagation();
        e.preventDefault();
      };
      window.addEventListener('click', swallow, { capture: true, once: true });
      setTimeout(() => window.removeEventListener('click', swallow, true), 0);
      current.onend?.(dropped);
    }
  }

  function stop(): void {
    origin = null;
    cancelAnimationFrame(frame);
    window.removeEventListener('pointermove', move, true);
    window.removeEventListener('pointerup', up, true);
    window.removeEventListener('pointercancel', cancel, true);
    window.removeEventListener('keydown', key, true);
    setOver(null, null);
    dnd.active = null;
    document.documentElement.classList.remove('dnd-active');
    void didDrag;
  }

  node.addEventListener('pointerdown', down);
  return {
    update(next: DraggableOptions) {
      current = next;
    },
    destroy() {
      node.removeEventListener('pointerdown', down);
      // A drag in progress outlives its source element (the list may navigate while you hover a
      // folder); only an unfinished press is abandoned.
      if (origin && !dnd.active) stop();
    },
  };
}
