<script lang="ts">
  import { Keyboard } from '@lucide/svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import KeybindingsEditor from './KeybindingsEditor.svelte';
</script>

<Modal title="Keyboard shortcuts" subtitle="Click a shortcut to change it" width={640} onclose={() => ui.closeDialog()}>
  {#snippet icon()}<Keyboard size={18} />{/snippet}
  <div class="body"><KeybindingsEditor /></div>
  {#snippet footer()}
    <Button variant="primary" onclick={() => ui.closeDialog()}>Done</Button>
  {/snippet}
</Modal>

<style>
  .body {
    height: min(500px, 60vh);
  }
</style>
