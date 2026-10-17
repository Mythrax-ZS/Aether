<script lang="ts">
  import { Bell, FolderTree, Keyboard, Palette, Settings, SquareTerminal, Terminal } from '@lucide/svelte';
  import type { Component } from 'svelte';
  import { fade } from 'svelte/transition';
  import { ui } from '$lib/stores/ui.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import AppearanceSection from './settings/AppearanceSection.svelte';
  import GeneralSection from './settings/GeneralSection.svelte';
  import NotificationsSection from './settings/NotificationsSection.svelte';
  import SftpSection from './settings/SftpSection.svelte';
  import SshSection from './settings/SshSection.svelte';
  import TerminalSection from './settings/TerminalSection.svelte';
  import KeybindingsEditor from './KeybindingsEditor.svelte';

  let { section = 'general' }: { section?: string } = $props();

  type Id = 'general' | 'appearance' | 'terminal' | 'ssh' | 'sftp' | 'notifications' | 'keyboard';
  const NAV: { id: Id; label: string; icon: Component<{ size?: number }> }[] = [
    { id: 'general', label: 'General', icon: Settings },
    { id: 'appearance', label: 'Appearance', icon: Palette },
    { id: 'terminal', label: 'Terminal', icon: SquareTerminal },
    { id: 'ssh', label: 'SSH', icon: Terminal },
    { id: 'sftp', label: 'Files', icon: FolderTree },
    { id: 'notifications', label: 'Notifications', icon: Bell },
    { id: 'keyboard', label: 'Keyboard', icon: Keyboard },
  ];

  // `ui.openSettings('appearance')` while the dialog is already open switches the section.
  const live = $derived(ui.dialogs.find((d) => d.type === 'settings'));
  const active = $derived<Id>(((live && live.type === 'settings' ? live.section : section) as Id) || 'general');

  function go(id: Id) {
    if (live && live.type === 'settings') live.section = id;
  }
</script>

<Modal title="Settings" width={900} padded={false} onclose={() => ui.closeDialog()}>
  {#snippet icon()}<Settings size={18} />{/snippet}
  <div class="layout">
    <nav class="nav" aria-label="Settings sections">
      {#each NAV as n (n.id)}
        {@const Icon = n.icon}
        <button type="button" class:on={active === n.id} aria-current={active === n.id ? 'page' : undefined} onclick={() => go(n.id)}>
          <Icon size={16} /><span>{n.label}</span>
        </button>
      {/each}
    </nav>
    <div class="content" class:fill={active === 'keyboard'}>
      {#key active}
        <div class="page" in:fade={{ duration: 140 }}>
          {#if active === 'general'}<GeneralSection />
          {:else if active === 'appearance'}<AppearanceSection />
          {:else if active === 'terminal'}<TerminalSection />
          {:else if active === 'ssh'}<SshSection />
          {:else if active === 'sftp'}<SftpSection />
          {:else if active === 'notifications'}<NotificationsSection />
          {:else}<KeybindingsEditor />{/if}
        </div>
      {/key}
    </div>
  </div>
</Modal>

<style>
  .layout {
    display: grid;
    grid-template-columns: 196px 1fr;
    height: min(640px, 76vh);
  }
  .nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 12px 10px;
    border-right: 1px solid var(--border);
    background: color-mix(in srgb, var(--bg-surface) 70%, var(--bg-app));
  }
  .nav button {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 34px;
    padding: 0 10px;
    border-radius: var(--r-md);
    color: var(--text-dim);
    font-weight: 550;
    text-align: left;
    transition:
      background var(--dur-fast),
      color var(--dur-fast);
  }
  .nav button:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .nav button.on {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .content {
    overflow-y: auto;
    padding: 22px 28px 30px;
  }
  .content.fill {
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .content.fill .page {
    flex: 1;
    min-height: 0;
  }
</style>
