<script lang="ts">
  import { ui } from '$lib/stores/ui.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Input from '$lib/components/ui/Input.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';

  let value = $state('');
  let touched = $state(false);

  $effect(() => {
    value = ui.textState?.options.value ?? '';
    touched = false;
  });

  const error = $derived.by(() => {
    const s = ui.textState;
    if (!s) return null;
    return s.options.validate?.(value) ?? null;
  });

  function submit(e?: Event) {
    e?.preventDefault();
    touched = true;
    if (error) return;
    ui.answerText(value);
  }
</script>

{#if ui.textState}
  {@const o = ui.textState.options}
  <Modal title={o.title} width={420} onclose={() => ui.answerText(null)}>
    <form id="text-prompt" onsubmit={submit}>
      <Input bind:value label={o.label} placeholder={o.placeholder} error={touched ? error : null} autofocus select oninput={() => (touched = true)} />
    </form>
    {#snippet footer()}
      <Button variant="ghost" onclick={() => ui.answerText(null)}>Cancel</Button>
      <Button variant="primary" type="submit" disabled={!!error} onclick={submit}>{o.confirmLabel ?? 'OK'}</Button>
    {/snippet}
  </Modal>
{/if}
