<script lang="ts">
  import { FileUp } from '@lucide/svelte';
  import { fade, scale } from 'svelte/transition';
  import { filedrop } from '$lib/stores/filedrop.svelte';
  import { workspace } from '$lib/stores/workspace.svelte';
  import { plural } from '$lib/util/format';

  // SFTP tabs draw their own, more precise drop zones.
  const show = $derived(filedrop.over && workspace.active?.kind !== 'sftp' && !!workspace.activeTerminalTab);
</script>

{#if show}
  <div class="drop" transition:fade={{ duration: 120 }} aria-hidden="true">
    <div class="card" in:scale={{ start: 0.94, duration: 200 }}>
      <FileUp size={28} />
      <strong>Drop to paste {filedrop.count === 1 ? 'the path' : 'the paths'}</strong>
      <span>{plural(filedrop.count, 'file')} will be typed at the cursor.</span>
    </div>
  </div>
{/if}

<style>
  .drop {
    position: fixed;
    inset: 0;
    z-index: calc(var(--z-modal) - 10);
    display: grid;
    place-items: center;
    background: color-mix(in srgb, var(--bg-app) 70%, transparent);
    backdrop-filter: blur(4px);
    pointer-events: none;
  }
  .drop::before {
    content: '';
    position: absolute;
    inset: 14px;
    border: 2px dashed var(--accent);
    border-radius: var(--r-xl);
    opacity: 0.7;
  }
  .card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 26px 36px;
    background: var(--bg-overlay);
    border: 1px solid var(--accent-ring);
    border-radius: var(--r-xl);
    box-shadow: var(--glow), var(--shadow-lg);
    color: var(--accent);
  }
  strong {
    color: var(--text);
    font-size: var(--fs-lg);
  }
  span {
    color: var(--text-dim);
    font-size: var(--fs-sm);
  }
</style>
