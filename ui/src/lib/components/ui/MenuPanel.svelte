<script lang="ts">
  import type { Component } from 'svelte';
  import { Check, ChevronRight } from '@lucide/svelte';
  import type { MenuItem } from '$lib/stores/ui.svelte';
  import Kbd from './Kbd.svelte';
  import Self from './MenuPanel.svelte';

  type Props = {
    items: MenuItem[];
    x: number;
    y: number;
    /** Set for submenus: the parent item's rectangle, so the panel opens beside it. */
    anchor?: DOMRect | null;
    onrun: (item: MenuItem) => void;
    /** Escape (root) or ArrowLeft (submenu). */
    onclose: () => void;
  };

  let { items, x, y, anchor = null, onrun, onclose }: Props = $props();

  let el = $state<HTMLDivElement>();
  let pos = $state({ left: 0, top: 0, ready: false });
  let openSub = $state<{ index: number; rect: DOMRect } | null>(null);
  let subTimer: ReturnType<typeof setTimeout> | undefined;

  // Place the panel once we know its size: stay inside the window, flip submenus to the left.
  $effect(() => {
    if (!el) return;
    const r = el.getBoundingClientRect();
    const pad = 8;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    let left = x;
    let top = y;
    if (anchor) {
      left = anchor.right - 4;
      if (left + r.width > vw - pad) left = anchor.left - r.width + 4;
      top = anchor.top - 6;
    }
    left = Math.max(pad, Math.min(left, vw - r.width - pad));
    top = Math.max(pad, Math.min(top, vh - r.height - pad));
    pos = { left, top, ready: true };
    // Root menus take focus so arrow keys work immediately; submenus are focused by their parent.
    if (!anchor) el.focus({ preventScroll: true });
  });

  const rows = (): HTMLButtonElement[] => (el ? [...el.querySelectorAll<HTMLButtonElement>(':scope > button.item:not(:disabled)')] : []);
  const indexOf = (btn: Element | null): number => (btn instanceof HTMLElement && btn.dataset.i !== undefined ? Number(btn.dataset.i) : -1);

  function move(to: 1 | -1 | 'first' | 'last') {
    const list = rows();
    if (list.length === 0) return;
    const i = list.indexOf(document.activeElement as HTMLButtonElement);
    const next = to === 'first' ? 0 : to === 'last' ? list.length - 1 : i < 0 ? (to === 1 ? 0 : list.length - 1) : (i + to + list.length) % list.length;
    list[next].focus();
  }

  function onKeydown(e: KeyboardEvent) {
    switch (e.key) {
      case 'ArrowDown':
        move(1);
        break;
      case 'ArrowUp':
        move(-1);
        break;
      case 'Home':
        move('first');
        break;
      case 'End':
        move('last');
        break;
      case 'ArrowRight': {
        const btn = document.activeElement;
        const i = indexOf(btn);
        if (i >= 0 && items[i].submenu && !items[i].disabled) open(i, btn as HTMLElement, true);
        break;
      }
      case 'ArrowLeft':
        if (anchor) onclose();
        break;
      case 'Escape':
      case 'Tab':
        onclose();
        break;
      default:
        return;
    }
    e.preventDefault();
    e.stopPropagation();
  }

  function open(index: number, btn: HTMLElement, focusFirst = false) {
    clearTimeout(subTimer);
    openSub = { index, rect: btn.getBoundingClientRect() };
    if (focusFirst) {
      // The child positions itself in an effect; focus its first row right after.
      setTimeout(() => el?.querySelector<HTMLButtonElement>('.panel button.item:not(:disabled)')?.focus(), 30);
    }
  }

  function hover(i: number, btn: HTMLButtonElement) {
    btn.focus({ preventScroll: true });
    clearTimeout(subTimer);
    if (items[i].submenu && !items[i].disabled) subTimer = setTimeout(() => open(i, btn), 110);
    else if (openSub) subTimer = setTimeout(() => (openSub = null), 160);
  }

  function activate(i: number, btn: HTMLButtonElement) {
    const item = items[i];
    if (item.disabled) return;
    if (item.submenu) open(i, btn);
    else onrun(item);
  }

  function closeSub(index: number) {
    openSub = null;
    el?.querySelector<HTMLButtonElement>(`:scope > button.item[data-i="${index}"]`)?.focus();
  }

  $effect(() => () => clearTimeout(subTimer));
</script>

<div
  bind:this={el}
  class="panel"
  class:ready={pos.ready}
  role="menu"
  tabindex="-1"
  style:left="{pos.left}px"
  style:top="{pos.top}px"
  onkeydown={onKeydown}
>
  {#each items as item, i (i)}
    {#if item.separator}
      <div class="sep" role="separator"></div>
    {:else if item.heading && !item.label}
      <div class="heading" role="presentation">{item.heading}</div>
    {:else}
      {@const Icon = item.icon as Component<{ size?: number }> | undefined}
      <button
        type="button"
        class="item"
        class:danger={item.danger}
        class:open={openSub?.index === i}
        data-i={i}
        role={item.checked !== undefined ? 'menuitemcheckbox' : 'menuitem'}
        aria-checked={item.checked}
        aria-haspopup={item.submenu ? 'menu' : undefined}
        aria-expanded={item.submenu ? openSub?.index === i : undefined}
        disabled={item.disabled}
        onpointerenter={(e) => hover(i, e.currentTarget)}
        onclick={(e) => activate(i, e.currentTarget)}
      >
        <span class="ico">
          {#if item.checked}<Check size={14} />{:else if Icon}<Icon size={15} />{/if}
        </span>
        <span class="label">{item.label}</span>
        {#if item.shortcut}<Kbd keys={item.shortcut} subtle />{/if}
        {#if item.submenu}<span class="chev"><ChevronRight size={14} /></span>{/if}
      </button>
      {#if openSub?.index === i && item.submenu}
        <Self items={item.submenu} x={0} y={0} anchor={openSub.rect} {onrun} onclose={() => closeSub(i)} />
      {/if}
    {/if}
  {/each}
</div>

<style>
  .panel {
    position: fixed;
    z-index: calc(var(--z-modal) + 50);
    min-width: 208px;
    max-width: 340px;
    max-height: calc(100vh - 16px);
    overflow-y: auto;
    padding: 5px;
    background: color-mix(in srgb, var(--bg-overlay) 97%, transparent);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-lg);
    box-shadow: var(--shadow-lg);
    outline: none;
    /* Hidden until measured and placed. (Not `visibility: hidden`: hidden elements can't take focus.) */
    opacity: 0;
    pointer-events: none;
    transform-origin: top left;
  }
  .panel.ready {
    opacity: 1;
    pointer-events: auto;
    animation: scale-in 130ms var(--ease-out) backwards;
  }
  .item {
    position: relative;
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    height: 30px;
    padding: 0 8px 0 6px;
    border-radius: var(--r-sm);
    color: var(--text);
    font-size: var(--fs-md);
    text-align: left;
    transition: background var(--dur-fast);
  }
  .item:hover:not(:disabled),
  .item:focus-visible,
  .item.open {
    background: var(--bg-active);
    outline: none;
  }
  .item:focus-visible {
    box-shadow: inset 0 0 0 1px var(--accent-ring);
  }
  .item:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .item.danger {
    color: var(--danger);
  }
  .item.danger:hover:not(:disabled),
  .item.danger:focus-visible {
    background: color-mix(in srgb, var(--danger) 16%, transparent);
  }
  .ico {
    display: inline-grid;
    place-items: center;
    width: 18px;
    color: var(--text-dim);
  }
  .danger .ico {
    color: inherit;
  }
  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chev {
    display: inline-flex;
    color: var(--text-faint);
  }
  .sep {
    height: 1px;
    margin: 5px 6px;
    background: var(--border);
  }
  .heading {
    padding: 8px 8px 3px;
    font-size: var(--fs-2xs);
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
</style>
