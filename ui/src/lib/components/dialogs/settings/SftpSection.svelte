<script lang="ts">
  import { FolderOpen } from '@lucide/svelte';
  import { api } from '$lib/api';
  import type { OverwritePolicy } from '$lib/bindings';
  import { settings } from '$lib/stores/settings.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Input from '$lib/components/ui/Input.svelte';
  import Select from '$lib/components/ui/Select.svelte';
  import Slider from '$lib/components/ui/Slider.svelte';
  import Switch from '$lib/components/ui/Switch.svelte';
  import SettingRow from './SettingRow.svelte';

  const f = $derived(settings.value.sftp);

  async function chooseFolder() {
    const dir = await api.platform.dialog.pickFolder({ title: 'Choose where downloads go' }).catch(() => null);
    if (dir) settings.patch({ sftp: { downloadDir: dir } });
  }
</script>

<div class="fsec">
  <h3>Browsing</h3>
  <SettingRow title="Show hidden files" description="Include names starting with a dot, on both sides.">
    <Switch checked={f.showHidden} aria="Show hidden files" onchange={(v) => settings.patch({ sftp: { showHidden: v } })} />
  </SettingRow>
</div>

<div class="fsec">
  <h3>Transfers</h3>
  <SettingRow title="Simultaneous transfers" description="How many files move at once. More is faster on fast links with small files.">
    <Slider value={f.concurrentTransfers} min={1} max={8} step={1} label="Simultaneous transfers" width={180} format={(v) => String(v)} onchange={(v) => settings.patch({ sftp: { concurrentTransfers: v } })} />
  </SettingRow>
  <SettingRow title="When a file already exists" description="The default answer; “Ask” shows a choice every time.">
    <div class="w180">
      <Select
        value={f.overwrite}
        size="sm"
        label=""
        options={[
          { value: 'ask', label: 'Ask me' },
          { value: 'overwrite', label: 'Replace it' },
          { value: 'skip', label: 'Skip it' },
          { value: 'rename', label: 'Keep both' },
        ]}
        onchange={(v) => settings.patch({ sftp: { overwrite: v as OverwritePolicy } })}
      />
    </div>
  </SettingRow>
  <SettingRow title="Keep modification times" description="Copied files keep their original timestamps.">
    <Switch checked={f.preserveTimes} aria="Preserve modification times" onchange={(v) => settings.patch({ sftp: { preserveTimes: v } })} />
  </SettingRow>
  <SettingRow title="Download folder" stacked description="Where files land when you don’t choose a folder. Empty uses your Downloads folder.">
    <div class="path">
      <Input size="sm" mono value={f.downloadDir ?? ''} placeholder="Windows Downloads folder" oninput={(e) => settings.patch({ sftp: { downloadDir: (e.currentTarget as HTMLInputElement).value.trim() || null } })} />
      <Button size="sm" onclick={chooseFolder}>{#snippet icon()}<FolderOpen size={13} />{/snippet}Browse</Button>
      {#if f.downloadDir}<Button size="sm" variant="ghost" onclick={() => settings.patch({ sftp: { downloadDir: null } })}>Reset</Button>{/if}
    </div>
  </SettingRow>
</div>

<style>
  .w180 {
    width: 180px;
  }
  .path {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .path :global(.field) {
    flex: 1;
  }
</style>
