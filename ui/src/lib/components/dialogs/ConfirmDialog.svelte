<script lang="ts">
  import { AlertTriangle, HelpCircle } from '@lucide/svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';

  let checked = $state(false);

  // A new question starts unchecked.
  $effect(() => {
    void ui.confirmState;
    checked = false;
  });
</script>

{#if ui.confirmState}
  {@const o = ui.confirmState.options}
  <Modal title={o.title} width={440} onclose={() => ui.answerConfirm(false)}>
    {#snippet icon()}
      {#if o.danger}<span class="warn"><AlertTriangle size={18} /></span>{:else}<HelpCircle size={18} />{/if}
    {/snippet}

    {#if o.message}<p class="message">{o.message}</p>{/if}
    {#if o.checkbox}
      <label class="check">
        <input type="checkbox" bind:checked />
        <span>{o.checkbox}</span>
      </label>
    {/if}

    {#snippet footer()}
      <Button variant="ghost" autofocus={o.danger} onclick={() => ui.answerConfirm(false, checked)}>{o.cancelLabel ?? 'Cancel'}</Button>
      <Button variant={o.danger ? 'danger' : 'primary'} autofocus={!o.danger} onclick={() => ui.answerConfirm(true, checked)}>{o.confirmLabel ?? 'OK'}</Button>
    {/snippet}
  </Modal>
{/if}

<style>
  .message {
    color: var(--text-dim);
    line-height: 1.55;
  }
  .warn {
    display: inline-flex;
    color: var(--danger);
  }
  .check {
    display: flex;
    align-items: center;
    gap: 9px;
    margin-top: 14px;
    color: var(--text-dim);
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .check input {
    accent-color: var(--accent);
    width: 15px;
    height: 15px;
  }
</style>
