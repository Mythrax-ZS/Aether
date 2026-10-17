<script lang="ts">
  import { ui } from '$lib/stores/ui.svelte';
  import AboutDialog from './AboutDialog.svelte';
  import ForwardsDialog from './ForwardsDialog.svelte';
  import GroupEditor from './GroupEditor.svelte';
  import HostEditor from './HostEditor.svelte';
  import ImportDialog from './ImportDialog.svelte';
  import KeysDialog from './KeysDialog.svelte';
  import KnownHostsDialog from './KnownHostsDialog.svelte';
  import Onboarding from './Onboarding.svelte';
  import SettingsDialog from './SettingsDialog.svelte';
  import ShortcutsDialog from './ShortcutsDialog.svelte';
</script>

<!-- Dialogs stack: each one opened from another sits on top of it (later in the DOM wins). -->
{#each ui.dialogs as dialog, i (dialog)}
  <div class="layer" inert={i < ui.dialogs.length - 1}>
    {#if dialog.type === 'host-editor'}
      <HostEditor hostId={dialog.hostId} draft={dialog.draft} groupId={dialog.groupId ?? null} />
    {:else if dialog.type === 'group-editor'}
      <GroupEditor groupId={dialog.groupId} parentId={dialog.parentId ?? null} />
    {:else if dialog.type === 'settings'}
      <SettingsDialog section={dialog.section} />
    {:else if dialog.type === 'keys'}
      <KeysDialog />
    {:else if dialog.type === 'known-hosts'}
      <KnownHostsDialog />
    {:else if dialog.type === 'import-ssh'}
      <ImportDialog />
    {:else if dialog.type === 'forwards'}
      <ForwardsDialog connectionId={dialog.connectionId} />
    {:else if dialog.type === 'shortcuts'}
      <ShortcutsDialog />
    {:else if dialog.type === 'about'}
      <AboutDialog />
    {:else if dialog.type === 'onboarding'}
      <Onboarding />
    {/if}
  </div>
{/each}

<style>
  .layer {
    display: contents;
  }
</style>
