<script lang="ts">
  import type { Snippet } from 'svelte';
  import { tooltip } from '$lib/dom/tooltip';

  type Props = {
    label: string;
    shortcut?: string | null;
    size?: number;
    active?: boolean;
    danger?: boolean;
    disabled?: boolean;
    class?: string;
    children: Snippet;
    onclick?: (e: MouseEvent) => void;
    oncontextmenu?: (e: MouseEvent) => void;
  };

  let { label, shortcut = null, size = 28, active = false, danger = false, disabled = false, class: extra = '', children, onclick, oncontextmenu }: Props = $props();
</script>

<button
  class="icon-btn {extra}"
  class:active
  class:danger
  style:--size="{size}px"
  aria-label={label}
  {disabled}
  {onclick}
  {oncontextmenu}
  use:tooltip={{ text: label, shortcut }}
>
  {@render children()}
</button>

<style>
  .icon-btn {
    display: inline-grid;
    place-items: center;
    width: var(--size);
    height: var(--size);
    border-radius: var(--r-sm);
    color: var(--text-dim);
    transition:
      background var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out),
      transform var(--dur-fast) var(--ease-out);
  }
  .icon-btn:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--text);
  }
  .icon-btn:active:not(:disabled) {
    transform: scale(0.92);
  }
  .icon-btn.active {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .icon-btn.danger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--danger) 18%, transparent);
    color: var(--danger);
  }
  .icon-btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }
</style>
