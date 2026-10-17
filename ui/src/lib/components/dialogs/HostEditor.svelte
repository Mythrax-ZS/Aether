<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { KeyRound, Network, Server, SquareTerminal, Trash2, Waypoints } from '@lucide/svelte';
  import { fade } from 'svelte/transition';
  import { api } from '$lib/api';
  import { connectHost, deleteHosts } from '$lib/actions';
  import type { Host } from '$lib/bindings';
  import { hosts } from '$lib/stores/hosts.svelte';
  import { settings } from '$lib/stores/settings.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import { blankHost, hostProblems } from '$lib/util/hostdraft';
  import Button from '$lib/components/ui/Button.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import AuthTab from './host/AuthTab.svelte';
  import ForwardRuleEditor from './ForwardRuleEditor.svelte';
  import GeneralTab from './host/GeneralTab.svelte';
  import NetworkTab from './host/NetworkTab.svelte';
  import TerminalTab from './host/TerminalTab.svelte';

  type Props = { hostId?: string; draft?: Partial<Host>; groupId?: string | null };
  let { hostId, draft, groupId = null }: Props = $props();

  // The editor is opened for one host (or one new host); what it was opened with never changes.
  const opened = untrack(() => ({ hostId, draft, groupId }));
  const existing = opened.hostId ? hosts.get(opened.hostId) : undefined;
  const editing = !!existing;

  // Work on a private copy; nothing changes until Save.
  let host = $state<Host>(structuredClone($state.snapshot(existing ?? { ...blankHost(settings.value, opened.groupId), ...opened.draft })) as Host);
  let password = $state('');
  let removePassword = $state(false);
  let hasSavedPassword = $state(false);
  let saving = $state(false);
  let tried = $state(false);
  let tab = $state<'general' | 'auth' | 'network' | 'forwards' | 'terminal'>('general');

  onMount(() => {
    if (editing) void api.hosts.passwordExists(host.id).then((v) => (hasSavedPassword = v)).catch(() => {});
  });

  const problems = $derived(hostProblems(host));
  const hasProblems = $derived(Object.keys(problems).length > 0);
  const visible = $derived(tried ? problems : {});

  const NAV = [
    { id: 'general', label: 'General', icon: Server },
    { id: 'auth', label: 'Sign-in', icon: KeyRound },
    { id: 'network', label: 'Network', icon: Network },
    { id: 'forwards', label: 'Port forwarding', icon: Waypoints },
    { id: 'terminal', label: 'Terminal', icon: SquareTerminal },
  ] as const;

  // Which tab holds the problem, so Save can jump there.
  const problemTab = $derived<(typeof NAV)[number]['id'] | null>(
    problems.hostname || problems.port ? 'general' : problems.methods || problems.key ? 'auth' : null,
  );

  async function save(connect = false) {
    tried = true;
    if (hasProblems) {
      if (problemTab) tab = problemTab;
      return;
    }
    saving = true;
    try {
      const payload = $state.snapshot(host) as Host;
      payload.hostname = payload.hostname.trim();
      payload.label = payload.label.trim();
      payload.username = payload.username.trim();
      payload.jump = payload.jump.filter((j) => (j.type === 'adhoc' ? j.spec.trim() : true));
      payload.startup.env = payload.startup.env.filter((v) => v.name.trim());
      payload.updatedAt = Date.now();
      const saved = await hosts.save(payload);
      if (removePassword) await api.hosts.clearPassword(saved.id).catch(() => {});
      if (password) await api.hosts.setPassword(saved.id, password);
      ui.closeDialog();
      if (connect) connectHost(saved);
      else ui.success(editing ? 'Host updated' : 'Host saved', saved.label || saved.hostname);
    } catch (e) {
      ui.error(e, 'Couldn’t save this host');
    } finally {
      saving = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      void save();
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<Modal
  title={editing ? `Edit ${existing?.label || existing?.hostname}` : 'New host'}
  subtitle={editing ? undefined : 'Save a server so you can connect in one click.'}
  width={800}
  padded={false}
  onclose={() => ui.closeDialog()}
>
  {#snippet icon()}<Server size={18} />{/snippet}
  <div class="layout">
    <nav class="nav" aria-label="Sections">
      {#each NAV as n (n.id)}
        {@const Icon = n.icon}
        <button type="button" class:on={tab === n.id} onclick={() => (tab = n.id)}>
          <Icon size={16} />
          <span>{n.label}</span>
          {#if tried && problemTab === n.id}<i class="dot" aria-label="Needs attention"></i>{/if}
        </button>
      {/each}
    </nav>
    <div class="content">
      {#key tab}
        <div class="page" in:fade={{ duration: 140 }}>
          {#if tab === 'general'}
            <GeneralTab bind:host errors={visible} />
          {:else if tab === 'auth'}
            <AuthTab bind:host bind:password bind:removePassword {hasSavedPassword} errors={visible} />
            {#if visible.methods}<p class="problem">{visible.methods}</p>{/if}
          {:else if tab === 'network'}
            <NetworkTab bind:host />
          {:else if tab === 'forwards'}
            <div class="fsec">
              <h3>Tunnels</h3>
              <p class="fdesc">Reach services behind the server (local), expose your own (remote), or browse through it (SOCKS). Rules marked Auto start every time you connect.</p>
              <ForwardRuleEditor bind:rules={host.forwards} />
            </div>
          {:else}
            <TerminalTab bind:host />
          {/if}
        </div>
      {/key}
    </div>
  </div>

  {#snippet footer()}
    {#if editing}
      <div class="left">
        <Button variant="ghost" onclick={() => { void deleteHosts([existing!]).then(() => !hosts.get(existing!.id) && ui.closeDialog()); }}>
          {#snippet icon()}<Trash2 size={14} />{/snippet}Delete
        </Button>
      </div>
    {/if}
    <Button variant="ghost" onclick={() => ui.closeDialog()}>Cancel</Button>
    <Button loading={saving} onclick={() => save(false)}>Save</Button>
    <Button variant="primary" loading={saving} onclick={() => save(true)}>Save and connect</Button>
  {/snippet}
</Modal>

<style>
  .layout {
    display: grid;
    grid-template-columns: 188px 1fr;
    height: min(560px, 66vh);
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
    position: relative;
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
  .nav .dot {
    margin-left: auto;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--danger);
  }
  .content {
    overflow-y: auto;
    padding: 22px 26px 26px;
  }
  .problem {
    margin-top: 12px;
    color: var(--danger);
    font-size: var(--fs-sm);
  }
  .left {
    margin-right: auto;
  }
</style>
