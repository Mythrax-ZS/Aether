<script lang="ts">
  import type { Snippet } from 'svelte';
  import { X } from '@lucide/svelte';
  import { fade, scale } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { trapFocus } from '$lib/dom/actions';
  import IconButton from './IconButton.svelte';

  type Props = {
    title: string;
    subtitle?: string;
    /** Max width in px. */
    width?: number;
    /** Bodies that manage their own padding (lists, tabs) turn this off. */
    padded?: boolean;
    dismissable?: boolean;
    onclose?: () => void;
    children: Snippet;
    footer?: Snippet;
    icon?: Snippet;
    headerExtra?: Snippet;
  };

  let { title, subtitle, width = 520, padded = true, dismissable = true, onclose, children, footer, icon, headerExtra }: Props = $props();

  const titleId = `modal-title-${Math.random().toString(36).slice(2, 8)}`;

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && dismissable) {
      e.stopPropagation();
      onclose?.();
    }
  }

</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="scrim"
  transition:fade|global={{ duration: 140 }}
  onpointerdown={(e) => e.target === e.currentTarget && dismissable && onclose?.()}
>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby={titleId}
    tabindex="-1"
    style:--w="{width}px"
    use:trapFocus
    onkeydown={onKeydown}
    in:scale|global={{ start: 0.96, duration: 200, easing: cubicOut }}
    out:scale|global={{ start: 0.97, duration: 120 }}
  >
    <header>
      {#if icon}<div class="icon">{@render icon()}</div>{/if}
      <div class="titles">
        <h2 id={titleId}>{title}</h2>
        {#if subtitle}<p>{subtitle}</p>{/if}
      </div>
      {#if headerExtra}{@render headerExtra()}{/if}
      {#if dismissable}
        <IconButton label="Close" onclick={() => onclose?.()}><X size={16} /></IconButton>
      {/if}
    </header>
    <div class="body" class:padded>
      {@render children()}
    </div>
    {#if footer}
      <footer>{@render footer()}</footer>
    {/if}
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: var(--z-modal);
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--scrim);
    backdrop-filter: blur(6px) saturate(1.1);
  }
  .dialog {
    width: min(var(--w), 100%);
    max-height: min(86vh, 900px);
    display: flex;
    flex-direction: column;
    background: var(--bg-surface);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-xl);
    box-shadow: var(--shadow-lg);
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px 16px 14px 20px;
    border-bottom: 1px solid var(--border);
  }
  .icon {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border-radius: 10px;
    background: var(--grad-accent-soft);
    color: var(--accent);
  }
  .titles {
    flex: 1;
    min-width: 0;
  }
  h2 {
    font-size: var(--fs-xl);
    font-weight: 650;
    letter-spacing: -0.01em;
  }
  p {
    margin-top: 2px;
    color: var(--text-dim);
    font-size: var(--fs-sm);
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .body.padded {
    padding: 18px 20px;
  }
  footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 16px;
    border-top: 1px solid var(--border);
    background: color-mix(in srgb, var(--bg-surface) 70%, var(--bg-app));
  }
</style>
