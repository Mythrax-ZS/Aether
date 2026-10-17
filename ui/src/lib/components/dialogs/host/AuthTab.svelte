<script lang="ts">
  import { ArrowDown, ArrowUp, FolderOpen, KeyRound, Trash2 } from '@lucide/svelte';
  import { onMount } from 'svelte';
  import { api } from '$lib/api';
  import type { AuthMethodKind, Host, KeyInfo } from '$lib/bindings';
  import { ui } from '$lib/stores/ui.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import IconButton from '$lib/components/ui/IconButton.svelte';
  import Input from '$lib/components/ui/Input.svelte';
  import Segmented from '$lib/components/ui/Segmented.svelte';
  import Select from '$lib/components/ui/Select.svelte';
  import Switch from '$lib/components/ui/Switch.svelte';

  type Props = {
    host: Host;
    password: string;
    hasSavedPassword: boolean;
    removePassword: boolean;
    errors: { key?: string };
  };

  let { host = $bindable(), password = $bindable(), hasSavedPassword, removePassword = $bindable(), errors }: Props = $props();

  const METHODS: { id: AuthMethodKind; label: string; desc: string }[] = [
    { id: 'agent', label: 'SSH agent', desc: 'Keys held by Windows OpenSSH Agent or Pageant.' },
    { id: 'public-key', label: 'Private key', desc: 'A key file or one saved in Aether.' },
    { id: 'keyboard-interactive', label: 'Verification code', desc: 'Two-factor prompts and one-time codes.' },
    { id: 'password', label: 'Password', desc: 'Type it each time, or save it securely.' },
  ];

  // Enabled methods keep their order; disabled ones follow.
  const ordered = $derived([
    ...host.auth.methods.map((id) => METHODS.find((m) => m.id === id)!).filter(Boolean),
    ...METHODS.filter((m) => !host.auth.methods.includes(m.id)),
  ]);

  function toggle(id: AuthMethodKind, on: boolean) {
    host.auth.methods = on ? [...host.auth.methods, id] : host.auth.methods.filter((m) => m !== id);
  }

  function move(id: AuthMethodKind, delta: -1 | 1) {
    const list = [...host.auth.methods];
    const i = list.indexOf(id);
    const j = i + delta;
    if (i < 0 || j < 0 || j >= list.length) return;
    [list[i], list[j]] = [list[j], list[i]];
    host.auth.methods = list;
  }

  let keys = $state<KeyInfo[]>([]);
  onMount(() => {
    void api.keys.list().then((k) => (keys = k)).catch(() => {});
  });

  const keyMode = $derived(host.auth.key?.type ?? 'default');

  function setKeyMode(mode: string) {
    if (mode === 'default') host.auth.key = null;
    else if (mode === 'file') host.auth.key = { type: 'file', path: host.auth.key?.type === 'file' ? host.auth.key.path : '' };
    else host.auth.key = { type: 'vault', keyId: host.auth.key?.type === 'vault' ? host.auth.key.keyId : (keys[0]?.id ?? '') };
  }

  async function browse() {
    const [path] = await api.platform.dialog.pickFiles({ title: 'Choose a private key', multiple: false }).catch(() => []);
    if (path && host.auth.key?.type === 'file') host.auth.key.path = path;
  }
</script>

<div class="fsec">
  <h3>How to sign in</h3>
  <p class="fdesc">Aether tries these in order and stops at the first one the server accepts. Turn off what you don’t use.</p>
  <div class="flist">
    {#each ordered as m (m.id)}
      {@const on = host.auth.methods.includes(m.id)}
      {@const idx = host.auth.methods.indexOf(m.id)}
      <div class="method" class:off={!on}>
        <Switch checked={on} onchange={(v) => toggle(m.id, v)} label={m.label} description={m.desc} />
        {#if on}
          <div class="mv">
            <IconButton label="Try earlier" size={24} disabled={idx === 0} onclick={() => move(m.id, -1)}><ArrowUp size={14} /></IconButton>
            <IconButton label="Try later" size={24} disabled={idx === host.auth.methods.length - 1} onclick={() => move(m.id, 1)}><ArrowDown size={14} /></IconButton>
          </div>
        {/if}
      </div>
    {/each}
  </div>
  {#if host.auth.methods.length === 0}<p class="warn">Pick at least one method, or the server can’t be reached.</p>{/if}
</div>

{#if host.auth.methods.includes('public-key')}
  <div class="fsec">
    <h3>Private key</h3>
    <Segmented
      value={keyMode}
      label="Key source"
      options={[
        { value: 'default', label: 'Default keys' },
        { value: 'file', label: 'Key file' },
        { value: 'vault', label: 'Saved key' },
      ]}
      onchange={setKeyMode}
    />
    {#if keyMode === 'default'}
      <p class="fdesc">Uses the identity files listed in Settings (like <code>~/.ssh/id_ed25519</code>).</p>
    {:else if keyMode === 'file' && host.auth.key?.type === 'file'}
      <div class="path">
        <Input bind:value={host.auth.key.path} label="Key file" mono placeholder="C:\Users\you\.ssh\id_ed25519" error={errors.key} />
        <Button onclick={browse}>{#snippet icon()}<FolderOpen size={14} />{/snippet}Browse</Button>
      </div>
    {:else if keyMode === 'vault' && host.auth.key?.type === 'vault'}
      {#if keys.length}
        <Select
          value={host.auth.key.keyId}
          options={keys.map((k) => ({ value: k.id as string, label: `${k.label} · ${k.algorithm.replace('ssh-', '')}` }))}
          label="Saved key"
          onchange={(v) => host.auth.key?.type === 'vault' && (host.auth.key.keyId = v)}
        />
        {#if errors.key}<p class="warn">{errors.key}</p>{/if}
      {:else}
        <div class="nokeys">
          <KeyRound size={18} />
          <span>No saved keys yet.</span>
          <Button size="sm" onclick={() => ui.openDialog({ type: 'keys' })}>Open key manager</Button>
        </div>
      {/if}
    {/if}
    <Switch bind:checked={host.auth.identitiesOnly} label="Only use this key" description="Skip keys offered by the agent. Helps with servers that lock you out after a few wrong keys." />
  </div>
{/if}

{#if host.auth.methods.includes('password')}
  <div class="fsec">
    <h3>Password</h3>
    <Input
      bind:value={password}
      type="password"
      label={hasSavedPassword && !removePassword ? 'Replace saved password' : 'Save a password (optional)'}
      placeholder={hasSavedPassword && !removePassword ? '•••••••• saved' : 'Ask me each time'}
      autocomplete="new-password"
      hint="Kept in Windows Credential Manager — never in a file next to your hosts."
    />
    {#if hasSavedPassword && !removePassword}
      <div>
        <Button size="sm" variant="danger" onclick={() => (removePassword = true)}>{#snippet icon()}<Trash2 size={13} />{/snippet}Forget saved password</Button>
      </div>
    {:else if removePassword}
      <p class="fdesc">The saved password will be removed when you save.</p>
    {/if}
  </div>
{/if}

<style>
  .method {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 11px 12px;
    transition: opacity var(--dur);
  }
  .method.off {
    opacity: 0.6;
  }
  .mv {
    display: flex;
    gap: 2px;
  }
  .path {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 8px;
    align-items: end;
  }
  .warn {
    color: var(--warning);
    font-size: var(--fs-sm);
  }
  .nokeys {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    border: 1px dashed var(--border-strong);
    border-radius: var(--r-md);
    color: var(--text-dim);
  }
  .nokeys span {
    flex: 1;
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text);
  }
</style>
