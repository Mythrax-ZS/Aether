<script lang="ts">
  import type { Snippet } from 'svelte';
  import Spinner from './Spinner.svelte';

  type Props = {
    variant?: 'primary' | 'secondary' | 'ghost' | 'danger';
    size?: 'sm' | 'md' | 'lg';
    loading?: boolean;
    disabled?: boolean;
    full?: boolean;
    /** Receives focus when its dialog opens (see `trapFocus`). */
    autofocus?: boolean;
    type?: 'button' | 'submit';
    title?: string;
    class?: string;
    icon?: Snippet;
    children?: Snippet;
    onclick?: (e: MouseEvent) => void;
  };

  let {
    variant = 'secondary',
    size = 'md',
    loading = false,
    disabled = false,
    full = false,
    autofocus = false,
    type = 'button',
    title,
    class: extra = '',
    icon,
    children,
    onclick,
  }: Props = $props();
</script>

<button class="btn {variant} {size} {extra}" class:full {type} {title} disabled={disabled || loading} data-autofocus={autofocus || undefined} {onclick}>
  {#if loading}
    <Spinner size={14} />
  {:else if icon}
    <span class="ico">{@render icon()}</span>
  {/if}
  {#if children}<span class="label">{@render children()}</span>{/if}
</button>

<style>
  .btn {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    border-radius: var(--r-md);
    font-weight: 550;
    letter-spacing: 0.005em;
    white-space: nowrap;
    border: 1px solid transparent;
    transition:
      background var(--dur-fast) var(--ease-out),
      border-color var(--dur-fast) var(--ease-out),
      box-shadow var(--dur) var(--ease-out),
      transform var(--dur-fast) var(--ease-out),
      filter var(--dur-fast) var(--ease-out);
  }
  .btn:active:not(:disabled) {
    transform: scale(0.975);
  }
  .btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .full {
    width: 100%;
  }
  .ico {
    display: inline-flex;
  }

  .sm {
    height: 26px;
    padding: 0 10px;
    font-size: var(--fs-sm);
  }
  .md {
    height: 32px;
    padding: 0 14px;
    font-size: var(--fs-md);
  }
  .lg {
    height: 40px;
    padding: 0 20px;
    font-size: var(--fs-lg);
    border-radius: var(--r-lg);
  }

  .primary {
    background: var(--grad-accent);
    color: var(--on-accent);
    box-shadow:
      0 1px 0 rgb(255 255 255 / 0.25) inset,
      0 6px 18px -6px var(--accent-ring);
  }
  .primary:hover:not(:disabled) {
    filter: brightness(1.08) saturate(1.05);
    box-shadow:
      0 1px 0 rgb(255 255 255 / 0.3) inset,
      0 8px 24px -6px var(--accent-ring);
  }

  .secondary {
    background: var(--bg-raised);
    border-color: var(--border-strong);
    color: var(--text);
  }
  .secondary:hover:not(:disabled) {
    background: var(--bg-overlay);
    border-color: color-mix(in srgb, var(--accent) 35%, var(--border-strong));
  }

  .ghost {
    color: var(--text-dim);
  }
  .ghost:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--text);
  }

  .danger {
    background: color-mix(in srgb, var(--danger) 16%, transparent);
    border-color: color-mix(in srgb, var(--danger) 40%, transparent);
    color: var(--danger);
  }
  .danger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--danger) 26%, transparent);
  }
</style>
