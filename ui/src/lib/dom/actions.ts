// Small DOM helpers used across components (Svelte actions).

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function focusable(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.offsetParent !== null || el === document.activeElement);
}

/** Keeps Tab/Shift+Tab inside `node`, focuses the first field, and restores focus on close. */
export function trapFocus(node: HTMLElement, opts: { initial?: string } = {}) {
  const previous = document.activeElement as HTMLElement | null;
  // Prefer an explicit autofocus field, then the first form control, and only then a button
  // (landing on the header's close button would be a poor first impression).
  const pick = (): HTMLElement =>
    (opts.initial ? node.querySelector<HTMLElement>(opts.initial) : null) ??
    node.querySelector<HTMLElement>('[data-autofocus]') ??
    node.querySelector<HTMLElement>('input:not([type="hidden"]):not([type="checkbox"]):not([disabled]), textarea:not([disabled]), select:not([disabled])') ??
    focusable(node).find((el) => !el.closest('header')) ??
    focusable(node)[0] ??
    node;
  const take = () => {
    const target = pick();
    target.focus({ preventScroll: true });
    if (target instanceof HTMLInputElement && target.dataset.select !== undefined) target.select();
  };
  queueMicrotask(take);

  // A dialog that was fading out when it was needed again is *resumed*, not re-created. Svelte made it
  // inert for the fade, which dropped focus to <body>, and nothing else would put it back.
  const onIntro = () => {
    if (!node.contains(document.activeElement)) queueMicrotask(take);
  };
  node.addEventListener('introstart', onIntro);

  const onKey = (e: KeyboardEvent) => {
    if (e.key !== 'Tab') return;
    const items = focusable(node);
    if (items.length === 0) {
      e.preventDefault();
      return;
    }
    const first = items[0];
    const last = items[items.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };
  node.addEventListener('keydown', onKey);
  return {
    destroy() {
      node.removeEventListener('keydown', onKey);
      node.removeEventListener('introstart', onIntro);
      // Give focus back to whatever opened the dialog — but only if nobody has taken it since
      // (a terminal that grabbed focus while this dialog faded out must keep it).
      const active = document.activeElement;
      const lost = !active || active === document.body || node.contains(active);
      if (lost && previous && document.contains(previous)) previous.focus({ preventScroll: true });
    },
  };
}

/** Calls `handler` when a pointer goes down outside `node`. */
export function clickOutside(node: HTMLElement, handler: (e: PointerEvent) => void) {
  let current = handler;
  const onDown = (e: PointerEvent) => {
    if (e.target instanceof Node && !node.contains(e.target)) current(e);
  };
  // Capture so stopPropagation inside the page can't swallow it.
  document.addEventListener('pointerdown', onDown, true);
  return {
    update(next: (e: PointerEvent) => void) {
      current = next;
    },
    destroy() {
      document.removeEventListener('pointerdown', onDown, true);
    },
  };
}

/** Moves `node` to <body> so it can't be clipped by overflow or stacking contexts. */
export function portal(node: HTMLElement) {
  document.body.append(node);
  return {
    destroy() {
      node.remove();
    },
  };
}

/** Focuses (and selects the text of) an input when it mounts. */
export function autofocus(node: HTMLElement, select = false) {
  queueMicrotask(() => {
    node.focus({ preventScroll: true });
    if (select && node instanceof HTMLInputElement) node.select();
  });
}

/** Scrolls the element into view when `active` becomes true. */
export function scrollIntoViewWhen(node: HTMLElement, active: boolean) {
  const run = (on: boolean) => on && node.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  run(active);
  return { update: run };
}
