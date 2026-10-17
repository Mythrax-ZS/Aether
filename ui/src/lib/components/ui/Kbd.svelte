<script lang="ts">
  import { acceleratorParts, parseAccelerator } from '$lib/util/keys';

  let { keys, subtle = false }: { keys: string | null | undefined; subtle?: boolean } = $props();

  const parts = $derived.by(() => {
    if (!keys) return [];
    const acc = parseAccelerator(keys);
    return acc ? acceleratorParts(acc) : [keys];
  });
</script>

{#if parts.length}
  <span class="kbds" class:subtle>
    {#each parts as p, i (i)}<kbd>{p}</kbd>{/each}
  </span>
{/if}

<style>
  .kbds {
    display: inline-flex;
    gap: 3px;
    align-items: center;
  }
  kbd {
    min-width: 18px;
    padding: 1px 5px;
    text-align: center;
    font: 500 10.5px/1.5 var(--font-ui);
    letter-spacing: 0.01em;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--bg-active) 70%, transparent);
    border: 1px solid var(--border-strong);
    border-bottom-width: 2px;
    border-radius: 5px;
  }
  .subtle kbd {
    background: transparent;
    border-color: var(--border);
    color: var(--text-faint);
  }
</style>
