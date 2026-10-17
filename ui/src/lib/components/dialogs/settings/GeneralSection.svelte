<script lang="ts">
  import { FolderOpen, RotateCcw, ScrollText, Sparkles } from '@lucide/svelte';
  import { onMount } from 'svelte';
  import { api } from '$lib/api';
  import { settings } from '$lib/stores/settings.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Segmented from '$lib/components/ui/Segmented.svelte';
  import Switch from '$lib/components/ui/Switch.svelte';
  import SettingRow from './SettingRow.svelte';

  const g = $derived(settings.value.general);

  // A portable copy can be moved or removed, so it never registers itself with Windows.
  let installed = $state(true);
  onMount(() => void api.app.info().then((i) => (installed = i.storageMode === 'installed')).catch(() => {}));

  async function resetAll() {
    const ok = await ui.confirm({
      title: 'Reset all settings?',
      message: 'Appearance, terminal, SSH and shortcut preferences go back to their defaults. Your hosts, keys and passwords are not touched.',
      confirmLabel: 'Reset settings',
      danger: true,
    });
    if (!ok) return;
    try {
      await settings.reset();
      ui.success('Settings were reset');
    } catch (e) {
      ui.error(e);
    }
  }
</script>

<div class="fsec">
  <h3>Startup</h3>
  <SettingRow title="When Aether starts" description="Reopen the tabs you had, or begin with a clean slate. Restored tabs connect when you open them.">
    <Segmented
      value={g.startup}
      label="Startup behaviour"
      options={[
        { value: 'welcome', label: 'Start fresh' },
        { value: 'restore', label: 'Restore tabs' },
      ]}
      onchange={(v) => settings.patch({ general: { startup: v as 'welcome' | 'restore' } })}
    />
  </SettingRow>
  <SettingRow
    title="Launch at sign-in"
    description={installed
      ? 'Start Aether in the background when you log in to Windows. It waits in the tray; click the icon to open it.'
      : 'Not available in the portable version: it would point at wherever this folder happens to be. Install Aether to use it.'}
  >
    <Switch checked={g.launchAtLogin && installed} disabled={!installed} aria="Launch at sign-in" onchange={(v) => settings.patch({ general: { launchAtLogin: v } })} />
  </SettingRow>
  <SettingRow title="Keep running in the tray" description="Closing the last window hides Aether instead of quitting, so sessions and tunnels stay alive. Use the tray icon to reopen it.">
    <Switch checked={g.closeToTray} aria="Keep running in the tray" onchange={(v) => settings.patch({ general: { closeToTray: v } })} />
  </SettingRow>
</div>

<div class="fsec">
  <h3>Safety</h3>
  <SettingRow title="Confirm before closing live sessions" description="Ask first when a tab or window still has an active connection.">
    <Switch checked={g.confirmCloseActive} aria="Confirm before closing live sessions" onchange={(v) => settings.patch({ general: { confirmCloseActive: v } })} />
  </SettingRow>
</div>

<div class="fsec">
  <h3>Your data</h3>
  <div class="frow wrap">
    <Button size="sm" onclick={() => void api.app.openConfig().catch((e) => ui.error(e))}>{#snippet icon()}<FolderOpen size={13} />{/snippet}Open config folder</Button>
    <Button size="sm" onclick={() => void api.app.openLogs().catch((e) => ui.error(e))}>{#snippet icon()}<ScrollText size={13} />{/snippet}Open logs</Button>
    <Button size="sm" onclick={() => { settings.patch({ general: { onboardingCompleted: false } }); ui.closeAllDialogs(); ui.openDialog({ type: 'onboarding' }); }}>{#snippet icon()}<Sparkles size={13} />{/snippet}Show the welcome tour</Button>
  </div>
  <p class="fdesc">Hosts, groups and themes are plain JSON files in the config folder, so they’re easy to back up or sync. Passwords and key passphrases are never stored there — they live in Windows Credential Manager.</p>
  <div><Button size="sm" variant="danger" onclick={resetAll}>{#snippet icon()}<RotateCcw size={13} />{/snippet}Reset all settings</Button></div>
</div>

<style>
  .wrap {
    flex-wrap: wrap;
  }
</style>
