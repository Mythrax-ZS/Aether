<script lang="ts">
  import { GripVertical } from '@lucide/svelte';
  import { dnd } from '$lib/dom/dnd.svelte';
</script>

{#if dnd.active}
  {@const d = dnd.active}
  <div class="ghost" class:valid={!!dnd.over} style:transform="translate({dnd.x + 14}px, {dnd.y + 12}px)" aria-hidden="true">
    <span class="grip"><GripVertical size={13} /></span>
    <span class="text">
      <span class="label">{d.label}</span>
      {#if d.sublabel}<span class="sub">{d.sublabel}</span>{/if}
    </span>
    {#if d.ids.length > 1}<span class="count">{d.ids.length}</span>{/if}
  </div>
{/if}

<style>
  .ghost {
    position: fixed;
    left: 0;
    top: 0;
    z-index: calc(var(--z-tooltip) + 10);
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 260px;
    padding: 6px 10px 6px 6px;
    background: color-mix(in srgb, var(--bg-overlay) 94%, transparent);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-md);
    box-shadow: var(--shadow-lg);
    pointer-events: none;
    will-change: transform;
    opacity: 0.94;
    transition:
      border-color var(--dur-fast),
      box-shadow var(--dur-fast);
  }
  .ghost.valid {
    border-color: var(--accent-ring);
    box-shadow: var(--shadow-lg), 0 0 0 1px var(--accent-ring);
  }
  .grip {
    display: inline-flex;
    color: var(--text-faint);
  }
  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
    line-height: 1.25;
  }
  .label {
    font-size: var(--fs-sm);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    font-size: var(--fs-xs);
    color: var(--text-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    display: grid;
    place-items: center;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 9px;
    background: var(--grad-accent);
    color: var(--on-accent);
    font-size: var(--fs-2xs);
    font-weight: 700;
  }
</style>
