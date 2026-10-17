<script lang="ts">
  import { AlertTriangle, Check, FileSearch, Network, Waypoints } from '@lucide/svelte';
  import { onMount } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { api } from '$lib/api';
  import type { SshConfigPreview } from '$lib/bindings';
  import { hosts } from '$lib/stores/hosts.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import { groupPath } from '$lib/util/tree';
  import Badge from '$lib/components/ui/Badge.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import Select from '$lib/components/ui/Select.svelte';
  import Spinner from '$lib/components/ui/Spinner.svelte';
  import { plural } from '$lib/util/format';

  let preview = $state<SshConfigPreview | null>(null);
  let failed = $state<string | null>(null);
  let importing = $state(false);
  let group = $state('');
  const picked = new SvelteSet<string>();

  onMount(async () => {
    try {
      preview = await api.hosts.sshConfigPreview();
      for (const c of preview.candidates) if (!c.alreadyImported) picked.add(c.alias);
    } catch (e) {
      failed = e instanceof Error ? e.message : 'Couldn’t read your ssh config.';
    }
  });

  const fresh = $derived(preview?.candidates.filter((c) => !c.alreadyImported) ?? []);
  const allPicked = $derived(fresh.length > 0 && fresh.every((c) => picked.has(c.alias)));
  const groupOptions = $derived([
    { value: '', label: 'No group' },
    ...hosts.data.groups.map((g) => ({ value: g.id as string, label: groupPath(hosts.data, g.id) ?? g.name })),
  ]);

  function toggleAll() {
    if (allPicked) picked.clear();
    else fresh.forEach((c) => picked.add(c.alias));
  }

  async function run() {
    importing = true;
    try {
      const r = await api.hosts.sshConfigImport([...picked], group || null);
      await hosts.refresh();
      ui.closeDialog();
      ui.success(`Imported ${plural(r.added, 'host')}`, r.skipped ? `${plural(r.skipped, 'entry', 'entries')} skipped (already saved).` : undefined);
      if (r.unlinkedJumps) {
        ui.toast({
          kind: 'warning',
          title: `${plural(r.unlinkedJumps, 'jump host link')} couldn’t be kept`,
          message: 'Those hosts would connect directly. Open them and set their jump hosts before using them.',
        });
      }
    } catch (e) {
      ui.error(e, 'Import failed');
    } finally {
      importing = false;
    }
  }
</script>

<Modal title="Import from ssh config" subtitle={preview?.path ?? '~/.ssh/config'} width={640} padded={false} onclose={() => ui.closeDialog()}>
  {#snippet icon()}<FileSearch size={18} />{/snippet}

  {#if !preview && !failed}
    <div class="center"><Spinner size={22} /><span>Reading your ssh config…</span></div>
  {:else if failed}
    <div class="center"><AlertTriangle size={22} /><span>{failed}</span></div>
  {:else if preview && !preview.found}
    <div class="center">
      <FileSearch size={26} />
      <strong>No ssh config found</strong>
      <span>Aether looks for <code>~/.ssh/config</code>. If yours lives elsewhere, add hosts by hand with “New host”.</span>
    </div>
  {:else if preview}
    <div class="bar">
      <label class="all">
        <input type="checkbox" checked={allPicked} disabled={fresh.length === 0} onchange={toggleAll} />
        <span>{fresh.length ? `Select all (${fresh.length})` : 'Nothing new to import'}</span>
      </label>
      <div class="dest"><Select bind:value={group} options={groupOptions} size="sm" label="" /></div>
    </div>
    <ul class="list">
      {#each preview.candidates as c, i (`${i}:${c.alias}`)}
        <li class:done={c.alreadyImported}>
          <label>
            <input
              type="checkbox"
              checked={picked.has(c.alias)}
              disabled={c.alreadyImported}
              onchange={(e) => (e.currentTarget.checked ? picked.add(c.alias) : picked.delete(c.alias))}
            />
            <span class="main">
              <span class="alias">{c.alias}</span>
              <span class="addr">{c.user ? c.user + '@' : ''}{c.hostname}{c.port !== 22 ? ':' + c.port : ''}</span>
            </span>
            <span class="tags">
              {#if c.alreadyImported}<Badge tone="success" size="sm">{#snippet children()}<Check size={10} /> Saved{/snippet}</Badge>{/if}
              {#if c.via.length}<Badge tone="violet" size="sm">{#snippet children()}<Waypoints size={10} /> via {c.via.join(', ')}{/snippet}</Badge>{/if}
              {#if c.usesProxyCommand}<Badge tone="warning" size="sm">ProxyCommand</Badge>{/if}
              {#if c.forwardCount}<Badge size="sm">{#snippet children()}<Network size={10} /> {plural(c.forwardCount, 'forward')}{/snippet}</Badge>{/if}
              {#if c.identityFile}<Badge size="sm">key</Badge>{/if}
            </span>
          </label>
          {#if c.warnings.length}<p class="warn"><AlertTriangle size={12} /> {c.warnings.join(' · ')}</p>{/if}
        </li>
      {/each}
    </ul>
    {#if preview.warnings.length}<p class="note">{preview.warnings.join(' · ')}</p>{/if}
  {/if}

  {#snippet footer()}
    <span class="sum">{picked.size ? `${plural(picked.size, 'host')} selected` : ''}</span>
    <Button variant="ghost" onclick={() => ui.closeDialog()}>Cancel</Button>
    <Button variant="primary" loading={importing} disabled={picked.size === 0} onclick={run}>Import {picked.size ? plural(picked.size, 'host') : ''}</Button>
  {/snippet}
</Modal>

<style>
  .center {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 56px 40px;
    text-align: center;
    color: var(--text-dim);
    line-height: 1.55;
  }
  .center strong {
    color: var(--text);
    font-size: var(--fs-lg);
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
    color: var(--text);
  }
  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 20px;
    border-bottom: 1px solid var(--border);
  }
  .all,
  li label {
    display: flex;
    align-items: center;
    gap: 12px;
    cursor: pointer;
  }
  input[type='checkbox'] {
    accent-color: var(--accent);
    width: 15px;
    height: 15px;
    flex-shrink: 0;
  }
  .dest {
    width: 220px;
  }
  .list {
    margin: 0;
    padding: 6px 8px;
    list-style: none;
    max-height: 380px;
    overflow-y: auto;
  }
  li {
    padding: 9px 12px;
    border-radius: var(--r-md);
  }
  li:hover {
    background: var(--bg-hover);
  }
  li.done {
    opacity: 0.55;
  }
  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    line-height: 1.3;
  }
  .alias {
    font-weight: 600;
  }
  .addr {
    font-size: var(--fs-xs);
    color: var(--text-faint);
    font-family: var(--font-mono);
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 5px;
  }
  .warn {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 4px 0 0 27px;
    font-size: var(--fs-xs);
    color: var(--warning);
  }
  .note {
    padding: 0 20px 14px;
    font-size: var(--fs-xs);
    color: var(--text-faint);
  }
  .sum {
    margin-right: auto;
    color: var(--text-dim);
    font-size: var(--fs-sm);
  }
</style>
