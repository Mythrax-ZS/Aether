<script lang="ts">
  import { Folder } from '@lucide/svelte';
  import { untrack } from 'svelte';
  import type { Group, GroupId } from '$lib/bindings';
  import { hosts } from '$lib/stores/hosts.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import { groupPath, isWithin } from '$lib/util/tree';
  import Button from '$lib/components/ui/Button.svelte';
  import ColorSwatches from '$lib/components/ui/ColorSwatches.svelte';
  import Input from '$lib/components/ui/Input.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import Select from '$lib/components/ui/Select.svelte';

  let { groupId, parentId = null }: { groupId?: GroupId; parentId?: GroupId | null } = $props();

  // The dialog is opened for one group (or one new group); what it was opened with never changes.
  const opened = untrack(() => ({ groupId, parentId }));
  const existing = opened.groupId ? hosts.data.groups.find((g) => g.id === opened.groupId) : undefined;

  let name = $state(existing?.name ?? '');
  let parent = $state<string>(existing?.parentId ?? opened.parentId ?? '');
  let color = $state<string | null>(existing?.color ?? null);
  let saving = $state(false);
  let tried = $state(false);

  // A group can't move into itself or anything inside it.
  const parentOptions = $derived([
    { value: '', label: 'Top level' },
    ...hosts.data.groups
      .filter((g) => !existing || !isWithin(hosts.data, g.id, existing.id))
      .map((g) => ({ value: g.id as string, label: groupPath(hosts.data, g.id) ?? g.name }))
      .sort((a, b) => a.label.localeCompare(b.label, undefined, { sensitivity: 'base' })),
  ]);

  const error = $derived(!name.trim() ? 'Give the group a name.' : null);

  async function save(e?: Event) {
    e?.preventDefault();
    tried = true;
    if (error) return;
    saving = true;
    try {
      const group: Group = {
        id: existing?.id ?? crypto.randomUUID(),
        name: name.trim(),
        parentId: parent || null,
        color,
        sort: existing?.sort ?? 0,
        collapsed: existing?.collapsed ?? false,
      };
      await hosts.saveGroup(group);
      ui.closeDialog();
    } catch (err) {
      ui.error(err, 'Couldn’t save the group');
    } finally {
      saving = false;
    }
  }
</script>

<Modal title={existing ? 'Edit group' : 'New group'} width={440} onclose={() => ui.closeDialog()}>
  {#snippet icon()}<Folder size={18} />{/snippet}
  <form class="fstack" onsubmit={save}>
    <Input bind:value={name} label="Name" placeholder="Production" autofocus select error={tried ? error : null} />
    <Select bind:value={parent} options={parentOptions} label="Inside" />
    <ColorSwatches bind:value={color} label="Colour" />
  </form>
  {#snippet footer()}
    <Button variant="ghost" onclick={() => ui.closeDialog()}>Cancel</Button>
    <Button variant="primary" loading={saving} onclick={save}>{existing ? 'Save' : 'Create group'}</Button>
  {/snippet}
</Modal>
