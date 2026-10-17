<script lang="ts">
  import { Check, Ban } from '@lucide/svelte';

  type Props = { value: string | null; label?: string };
  let { value = $bindable(null), label }: Props = $props();

  // Tuned to look good on dark and light surfaces alike.
  const COLORS = ['#ff6f8a', '#ff9f6b', '#f5c067', '#7fdc9b', '#4fd1c5', '#5cc8ff', '#7c9cff', '#9d8cff', '#d18cff', '#ff8ccf'];
</script>

<div class="field">
  {#if label}<span class="lbl">{label}</span>{/if}
  <div class="row" role="radiogroup" aria-label={label ?? 'Colour'}>
    <button type="button" role="radio" aria-checked={value === null} aria-label="No colour" class="sw none" class:on={value === null} onclick={() => (value = null)}>
      <Ban size={13} />
    </button>
    {#each COLORS as c (c)}
      <button type="button" role="radio" aria-checked={value === c} aria-label={c} class="sw" class:on={value === c} style:--c={c} onclick={() => (value = c)}>
        {#if value === c}<Check size={13} strokeWidth={3} />{/if}
      </button>
    {/each}
  </div>
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .lbl {
    font-size: var(--fs-sm);
    font-weight: 550;
    color: var(--text-dim);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 7px;
  }
  .sw {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 8px;
    background: var(--c);
    color: #0b0d13;
    box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.18);
    transition: transform var(--dur-fast) var(--ease-spring);
  }
  .sw:hover {
    transform: scale(1.14);
  }
  .sw.on {
    box-shadow:
      0 0 0 2px var(--bg-surface),
      0 0 0 4px var(--c, var(--accent));
  }
  .sw.none {
    background: var(--bg-input);
    border: 1px dashed var(--border-strong);
    color: var(--text-faint);
    box-shadow: none;
  }
  .sw.none.on {
    box-shadow:
      0 0 0 2px var(--bg-surface),
      0 0 0 4px var(--text-faint);
  }
</style>
