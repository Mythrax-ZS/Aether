<script lang="ts">
  import type { Snippet } from 'svelte';

  type Props = {
    tone?: 'neutral' | 'accent' | 'success' | 'warning' | 'danger' | 'violet';
    dot?: boolean;
    pulse?: boolean;
    size?: 'sm' | 'md';
    children: Snippet;
  };

  let { tone = 'neutral', dot = false, pulse = false, size = 'md', children }: Props = $props();
</script>

<span class="badge {tone} {size}">
  {#if dot}<i class="dot" class:pulse></i>{/if}
  {@render children()}
</span>

<style>
  .badge {
    --c: var(--text-dim);
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border-radius: var(--r-pill);
    font-weight: 600;
    letter-spacing: 0.01em;
    white-space: nowrap;
    color: var(--c);
    background: color-mix(in srgb, var(--c) 13%, transparent);
    border: 1px solid color-mix(in srgb, var(--c) 22%, transparent);
  }
  .md {
    height: 20px;
    padding: 0 8px;
    font-size: var(--fs-xs);
  }
  .sm {
    height: 17px;
    padding: 0 6px;
    font-size: var(--fs-2xs);
  }
  .accent {
    --c: var(--accent);
  }
  .success {
    --c: var(--success);
  }
  .warning {
    --c: var(--warning);
  }
  .danger {
    --c: var(--danger);
  }
  .violet {
    --c: var(--accent-2);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--c);
  }
  .dot.pulse {
    animation: pulse-dot 1.4s var(--ease-in-out) infinite;
  }
</style>
