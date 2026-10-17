<script lang="ts">
  import { Bell } from '@lucide/svelte';
  import { api } from '$lib/api';
  import { settings } from '$lib/stores/settings.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Slider from '$lib/components/ui/Slider.svelte';
  import Switch from '$lib/components/ui/Switch.svelte';
  import SettingRow from './SettingRow.svelte';

  const n = $derived(settings.value.notifications);

  async function test() {
    try {
      await api.platform.notify('Aether', 'Notifications are working.');
    } catch (e) {
      ui.error(e, 'Notifications are blocked');
    }
  }
</script>

<div class="fsec">
  <h3>Windows notifications</h3>
  <SettingRow title="When a connection drops" description="Get told when a session is lost, so you notice before you type into nothing.">
    <Switch checked={n.onDisconnect} aria="Notify on disconnect" onchange={(v) => settings.patch({ notifications: { onDisconnect: v } })} />
  </SettingRow>
  <SettingRow title="When a long command finishes" description="Needs shell integration: your prompt must emit the OSC 133 markers (supported by recent bash, zsh, fish and PowerShell setups).">
    <Switch checked={n.onCommandFinished} aria="Notify when a command finishes" onchange={(v) => settings.patch({ notifications: { onCommandFinished: v } })} />
  </SettingRow>
  <SettingRow title="Only for commands longer than">
    <Slider value={n.minCommandSecs} min={1} max={120} step={1} label="Minimum command duration" width={200} format={(v) => `${v} s`} onchange={(v) => settings.patch({ notifications: { minCommandSecs: v } })} />
  </SettingRow>
  <SettingRow title="Only when Aether isn’t in front" description="Skip notifications while you’re looking at the window.">
    <Switch checked={n.onlyWhenUnfocused} aria="Only when unfocused" onchange={(v) => settings.patch({ notifications: { onlyWhenUnfocused: v } })} />
  </SettingRow>
  <SettingRow title="Let programs on the server send notifications" description="Tools can ask for a desktop notification (a build that finished, a message). They are shown with the session’s name, shortened, and at most one every few seconds.">
    <Switch checked={n.fromServer} aria="Allow notifications from the server" onchange={(v) => settings.patch({ notifications: { fromServer: v } })} />
  </SettingRow>
  <div><Button size="sm" onclick={test}>{#snippet icon()}<Bell size={13} />{/snippet}Send a test notification</Button></div>
</div>
