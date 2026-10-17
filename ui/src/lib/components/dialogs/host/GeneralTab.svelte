<script lang="ts">
  import type { Host } from '$lib/bindings';
  import { hosts } from '$lib/stores/hosts.svelte';
  import { groupPath } from '$lib/util/tree';
  import ColorSwatches from '$lib/components/ui/ColorSwatches.svelte';
  import Input from '$lib/components/ui/Input.svelte';
  import Select from '$lib/components/ui/Select.svelte';
  import Switch from '$lib/components/ui/Switch.svelte';
  import TagInput from '$lib/components/ui/TagInput.svelte';
  import TextArea from '$lib/components/ui/TextArea.svelte';

  type Props = {
    host: Host;
    errors: { hostname?: string; port?: string };
  };

  let { host = $bindable(), errors }: Props = $props();

  const groupOptions = $derived([
    { value: '', label: 'No group' },
    ...hosts.data.groups
      .map((g) => ({ value: g.id as string, label: groupPath(hosts.data, g.id) ?? g.name }))
      .sort((a, b) => a.label.localeCompare(b.label, undefined, { sensitivity: 'base' })),
  ]);

  const port = (v: string | number) => Math.floor(Number(v) || 0);
</script>

<div class="fsec">
  <h3>Server</h3>
  <Input bind:value={host.label} label="Name" placeholder="Production web" hint="Shown in the sidebar and on the tab. Defaults to the address." />
  <div class="fgrid wide-left">
    <Input bind:value={host.hostname} label="Address" placeholder="server.example.com or 203.0.113.7" mono autofocus error={errors.hostname} />
    <Input
      value={host.port}
      label="Port"
      type="number"
      min={1}
      max={65535}
      mono
      error={errors.port}
      oninput={(e) => (host.port = port((e.currentTarget as HTMLInputElement).value))}
    />
  </div>
  <Input bind:value={host.username} label="User" placeholder="Leave empty to use the default" mono hint="Used when signing in. Empty means your default user from Settings." />
</div>

<div class="fsec">
  <h3>Organise</h3>
  <div class="fgrid">
    <Select
      value={host.groupId ?? ''}
      options={groupOptions}
      label="Group"
      onchange={(v) => (host.groupId = v || null)}
    />
    <div class="center">
      <Switch bind:checked={host.favorite} label="Favourite" description="Pinned to the top of the sidebar." />
    </div>
  </div>
  <TagInput bind:tags={host.tags} label="Tags" suggestions={hosts.tags.map((t) => t.tag)} hint="Search with #tag in the sidebar." />
  <ColorSwatches bind:value={host.color} label="Colour" />
  <TextArea bind:value={host.notes} label="Notes" rows={3} placeholder="Anything worth remembering about this server" />
</div>

<style>
  .center {
    display: flex;
    align-items: center;
    padding-top: 22px;
  }
</style>
