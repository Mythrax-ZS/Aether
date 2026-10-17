<script lang="ts">
  import { api } from '$lib/api';
  import { ui } from '$lib/stores/ui.svelte';
</script>

<div class="controls" role="group" aria-label="Window controls">
  <button type="button" class="ctl" aria-label="Minimize" onclick={() => api.platform.window.minimize()}>
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0 5h10" stroke="currentColor" stroke-width="1" /></svg>
  </button>
  <button type="button" class="ctl" aria-label={ui.maximized ? 'Restore' : 'Maximize'} onclick={() => api.platform.window.toggleMaximize()}>
    {#if ui.maximized}
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
        <path d="M2.5 2.5V.5h7v7h-2" fill="none" stroke="currentColor" />
        <rect x=".5" y="2.5" width="7" height="7" fill="none" stroke="currentColor" />
      </svg>
    {:else}
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><rect x=".5" y=".5" width="9" height="9" fill="none" stroke="currentColor" /></svg>
    {/if}
  </button>
  <button type="button" class="ctl close" aria-label="Close" onclick={() => api.platform.window.close()}>
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0 0l10 10M10 0L0 10" stroke="currentColor" stroke-width="1.1" /></svg>
  </button>
</div>

<style>
  .controls {
    display: flex;
    align-self: stretch;
    margin-left: 6px;
  }
  .ctl {
    display: grid;
    place-items: center;
    width: 46px;
    color: var(--text-dim);
    border-radius: 0;
    transition:
      background var(--dur-fast),
      color var(--dur-fast);
  }
  .ctl:hover {
    background: var(--bg-active);
    color: var(--text);
  }
  .ctl.close:hover {
    background: #e5334f;
    color: #fff;
  }
  .ctl:focus-visible {
    outline-offset: -3px;
    border-radius: 0;
  }
</style>
