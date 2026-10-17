<script lang="ts">
  type Props = {
    checked: boolean;
    /** Visible text beside the toggle. */
    label?: string;
    description?: string;
    /** Accessible name when the text is shown elsewhere (e.g. a settings row's title). */
    aria?: string;
    disabled?: boolean;
    onchange?: (checked: boolean) => void;
  };

  let { checked = $bindable(false), label, description, aria, disabled = false, onchange }: Props = $props();

  function toggle() {
    if (disabled) return;
    checked = !checked;
    onchange?.(checked);
  }
</script>

<div class="row" class:disabled>
  <button type="button" role="switch" aria-checked={checked} aria-label={aria ?? label} class="switch" class:on={checked} {disabled} onclick={toggle}>
    <span class="thumb"></span>
  </button>
  {#if label || description}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="text" onclick={toggle}>
      {#if label}<span class="label">{label}</span>{/if}
      {#if description}<span class="desc">{description}</span>{/if}
    </div>
  {/if}
</div>

<style>
  .row {
    display: flex;
    align-items: flex-start;
    gap: 12px;
  }
  .row.disabled {
    opacity: 0.5;
  }
  .switch {
    position: relative;
    flex-shrink: 0;
    width: 36px;
    height: 20px;
    margin-top: 1px;
    border-radius: var(--r-pill);
    background: var(--bg-active);
    border: 1px solid var(--border-strong);
    transition:
      background var(--dur) var(--ease-out),
      border-color var(--dur);
  }
  .switch.on {
    background: var(--grad-accent);
    border-color: transparent;
  }
  .thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--text-dim);
    transition:
      transform var(--dur) var(--ease-spring),
      background var(--dur);
  }
  .on .thumb {
    transform: translateX(16px);
    background: #fff;
    box-shadow: 0 1px 4px rgb(0 0 0 / 0.35);
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    cursor: pointer;
  }
  .label {
    font-size: var(--fs-md);
    font-weight: 500;
  }
  .desc {
    font-size: var(--fs-sm);
    color: var(--text-dim);
    line-height: 1.4;
  }
</style>
