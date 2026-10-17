<script lang="ts">
  import { Copy, Lock, Search, ShieldCheck, Trash2 } from '@lucide/svelte';
  import { onMount } from 'svelte';
  import { api } from '$lib/api';
  import { copyText } from '$lib/actions';
  import type { KnownHostEntry } from '$lib/bindings';
  import { tooltip } from '$lib/dom/tooltip';
  import { ui } from '$lib/stores/ui.svelte';
  import { plural } from '$lib/util/format';
  import Badge from '$lib/components/ui/Badge.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import IconButton from '$lib/components/ui/IconButton.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import Spinner from '$lib/components/ui/Spinner.svelte';

  let entries = $state<KnownHostEntry[] | null>(null);
  let query = $state('');

  async function load() {
    try {
      entries = await api.keys.knownHosts();
    } catch (e) {
      entries = [];
      ui.error(e);
    }
  }
  onMount(load);

  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const all = entries ?? [];
    return q ? all.filter((e) => e.hosts.toLowerCase().includes(q) || e.fingerprint.toLowerCase().includes(q) || e.algorithm.toLowerCase().includes(q)) : all;
  });

  async function remove(e: KnownHostEntry) {
    const ok = await ui.confirm({
      title: 'Forget this host key?',
      message: `Next time you connect to ${e.hashed ? 'this server' : e.hosts}, Aether will ask you to verify it again.`,
      confirmLabel: 'Forget',
      danger: true,
    });
    if (!ok) return;
    try {
      const removed = await api.keys.removeKnownHost(e.line, e.fingerprint);
      // Nothing matched: the file changed behind our back (another window, an editor). Show what is really there.
      if (removed === 0) ui.info('That entry had already changed', 'The list has been refreshed.');
      await load();
    } catch (err) {
      ui.error(err);
    }
  }
</script>

<Modal title="Known hosts" subtitle="Servers whose identity you’ve verified" width={760} padded={false} onclose={() => ui.closeDialog()}>
  {#snippet icon()}<ShieldCheck size={18} />{/snippet}

  <div class="bar">
    <label class="search">
      <Search size={14} />
      <input bind:value={query} placeholder="Filter by host or fingerprint" spellcheck="false" autocomplete="off" aria-label="Filter known hosts" />
    </label>
    <span class="count">{entries ? plural(shown.length, 'entry', 'entries') : ''}</span>
  </div>

  {#if entries === null}
    <div class="center"><Spinner size={22} /></div>
  {:else if shown.length === 0}
    <div class="center">
      <ShieldCheck size={26} />
      <strong>{query ? 'No matches' : 'Nothing here yet'}</strong>
      <span>{query ? 'Try a different search.' : 'Servers you trust will appear here after your first connection.'}</span>
    </div>
  {:else}
    <ul>
      {#each shown as e (`${e.source}:${e.line}`)}
        <li>
          <div class="main">
            <div class="top">
              <span class="host">{e.hashed ? 'Hashed entry' : e.hosts}</span>
              <Badge size="sm" tone={e.source === 'aether' ? 'accent' : 'neutral'}>{e.source === 'aether' ? 'Aether' : '~/.ssh'}</Badge>
              {#if e.hashed}<span class="lock" use:tooltip={'Host name is hashed, so it can’t be shown'}><Lock size={12} /></span>{/if}
            </div>
            <div class="fp"><span class="alg">{e.algorithm}</span><code class="selectable">{e.fingerprint}</code></div>
          </div>
          <IconButton label="Copy fingerprint" size={28} onclick={() => copyText(e.fingerprint, 'Fingerprint copied')}><Copy size={14} /></IconButton>
          <IconButton label={e.source === 'aether' ? 'Forget this key' : 'Read-only — managed by ~/.ssh/known_hosts'} size={28} danger disabled={e.source !== 'aether'} onclick={() => remove(e)}><Trash2 size={14} /></IconButton>
        </li>
      {/each}
    </ul>
  {/if}

  {#snippet footer()}
    <Button variant="primary" onclick={() => ui.closeDialog()}>Done</Button>
  {/snippet}
</Modal>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 12px 20px;
    border-bottom: 1px solid var(--border);
  }
  .search {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 32px;
    padding: 0 10px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    color: var(--text-faint);
  }
  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .search input {
    flex: 1;
    background: none;
    border: 0;
    outline: none;
    color: var(--text);
    cursor: text;
  }
  .count {
    font-size: var(--fs-sm);
    color: var(--text-faint);
  }
  ul {
    margin: 0;
    padding: 6px 10px;
    list-style: none;
    max-height: 420px;
    overflow-y: auto;
  }
  li {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 10px;
    border-radius: var(--r-md);
  }
  li:hover {
    background: var(--bg-hover);
  }
  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .host {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .lock {
    display: inline-flex;
    color: var(--text-faint);
  }
  .fp {
    display: flex;
    align-items: baseline;
    gap: 10px;
    min-width: 0;
  }
  .alg {
    flex-shrink: 0;
    font-size: var(--fs-2xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--accent-2);
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .center {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 56px 40px;
    text-align: center;
    color: var(--text-dim);
  }
  .center strong {
    color: var(--text);
    font-size: var(--fs-lg);
  }
</style>
