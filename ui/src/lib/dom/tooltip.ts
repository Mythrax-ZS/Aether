// A small tooltip action: appears after a short delay, hides on any interaction.

import { formatAccelerator, parseAccelerator } from '$lib/util/keys';

export interface TooltipOptions {
  text: string | undefined | null;
  shortcut?: string | null;
  placement?: 'top' | 'bottom' | 'right' | 'left';
  delay?: number;
}

type Param = string | TooltipOptions | undefined | null;

let layer: HTMLDivElement | null = null;

function ensureLayer(): HTMLDivElement {
  if (!layer) {
    layer = document.createElement('div');
    layer.className = 'aether-tooltip';
    layer.setAttribute('role', 'tooltip');
    Object.assign(layer.style, {
      position: 'fixed',
      zIndex: 'var(--z-tooltip)',
      pointerEvents: 'none',
      opacity: '0',
      transition: 'opacity 120ms ease-out, transform 120ms ease-out',
      transform: 'translateY(2px)',
      background: 'var(--bg-overlay)',
      color: 'var(--text)',
      border: '1px solid var(--border-strong)',
      borderRadius: '7px',
      padding: '5px 9px',
      font: '500 12px/1.3 var(--font-ui)',
      boxShadow: 'var(--shadow-md)',
      maxWidth: '280px',
      whiteSpace: 'nowrap',
      display: 'flex',
      gap: '8px',
      alignItems: 'center',
    } satisfies Partial<CSSStyleDeclaration>);
    document.body.append(layer);
  }
  return layer;
}

function normalise(p: Param): TooltipOptions {
  if (typeof p === 'string' || p == null) return { text: p };
  return p;
}

export function tooltip(node: HTMLElement, param: Param) {
  let opts = normalise(param);
  let timer: ReturnType<typeof setTimeout> | undefined;

  const hide = () => {
    clearTimeout(timer);
    if (layer) {
      layer.style.opacity = '0';
      layer.style.transform = 'translateY(2px)';
    }
  };

  const show = () => {
    if (!opts.text) return;
    const el = ensureLayer();
    el.textContent = opts.text;
    if (opts.shortcut) {
      const acc = parseAccelerator(opts.shortcut);
      const kbd = document.createElement('span');
      kbd.textContent = acc ? formatAccelerator(acc) : opts.shortcut;
      Object.assign(kbd.style, { color: 'var(--text-faint)', fontFamily: 'var(--font-mono)', fontSize: '11px' });
      el.append(kbd);
    }
    el.style.opacity = '0';
    const r = node.getBoundingClientRect();
    const t = el.getBoundingClientRect();
    const place = opts.placement ?? (r.bottom + t.height + 12 > window.innerHeight ? 'top' : 'bottom');
    let x = r.left + r.width / 2 - t.width / 2;
    let y = place === 'top' ? r.top - t.height - 8 : r.bottom + 8;
    if (place === 'right') {
      x = r.right + 8;
      y = r.top + r.height / 2 - t.height / 2;
    } else if (place === 'left') {
      x = r.left - t.width - 8;
      y = r.top + r.height / 2 - t.height / 2;
    }
    x = Math.min(Math.max(8, x), window.innerWidth - t.width - 8);
    y = Math.min(Math.max(8, y), window.innerHeight - t.height - 8);
    el.style.left = `${x}px`;
    el.style.top = `${y}px`;
    requestAnimationFrame(() => {
      el.style.opacity = '1';
      el.style.transform = 'translateY(0)';
    });
  };

  const enter = () => {
    clearTimeout(timer);
    timer = setTimeout(show, opts.delay ?? 450);
  };

  // Keyboard focus shows the tip; focus that code moved there (dialogs opening) must not.
  const focusIn = () => {
    if (node.matches(':focus-visible')) enter();
  };

  node.addEventListener('pointerenter', enter);
  node.addEventListener('pointerleave', hide);
  node.addEventListener('pointerdown', hide);
  node.addEventListener('focus', focusIn);
  node.addEventListener('blur', hide);

  return {
    update(next: Param) {
      opts = normalise(next);
    },
    destroy() {
      hide();
      node.removeEventListener('pointerenter', enter);
      node.removeEventListener('pointerleave', hide);
      node.removeEventListener('pointerdown', hide);
      node.removeEventListener('focus', focusIn);
      node.removeEventListener('blur', hide);
    },
  };
}
