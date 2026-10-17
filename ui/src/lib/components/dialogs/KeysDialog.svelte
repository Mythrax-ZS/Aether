<script lang="ts">
  import { ArrowLeft, Copy, Download, FileDown, KeyRound, Lock, Pencil, Plus, Server, Sparkles, Trash2, Upload } from '@lucide/svelte';
  import { onMount } from 'svelte';
  import { api, toErrorInfo } from '$lib/api';
  import { copyText } from '$lib/actions';
  import type { KeyInfo, KeyKind, KeyMeta } from '$lib/bindings';
  import { sessions } from '$lib/stores/sessions.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import { workspace } from '$lib/stores/workspace.svelte';
  import { formatRelative } from '$lib/util/format';
  import Badge from '$lib/components/ui/Badge.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import IconButton from '$lib/components/ui/IconButton.svelte';
  import Input from '$lib/components/ui/Input.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import Segmented from '$lib/components/ui/Segmented.svelte';
  import Select from '$lib/components/ui/Select.svelte';
  import Spinner from '$lib/components/ui/Spinner.svelte';
  import Switch from '$lib/components/ui/Switch.svelte';
  import TextArea from '$lib/components/ui/TextArea.svelte';

  type View = 'list' | 'generate' | 'import';
  let view = $state<View>('list');
  let keys = $state<KeyInfo[] | null>(null);

  async function load() {
    try {
      keys = await api.keys.list();
    } catch (e) {
      keys = [];
      ui.error(e);
    }
  }
  onMount(load);

  const connectionId = $derived(workspace.activeConnection);
  const canInstall = $derived(!!connectionId && sessions.isConnected(connectionId));

  // ── list actions ───────────────────────────────────────────────────────────

  async function copyPublic(k: KeyInfo) {
    await copyText(k.publicKey, 'Public key copied');
  }

  async function exportPublic(k: KeyInfo) {
    const path = await api.platform.dialog.saveFile({ title: 'Save public key', defaultPath: `${k.label.replace(/[^\w.-]+/g, '_')}.pub` }).catch(() => null);
    if (!path) return;
    try {
      await api.keys.exportPublic(k.id, path);
      ui.success('Public key saved');
    } catch (e) {
      ui.error(e);
    }
  }

  async function rename(k: KeyInfo) {
    const label = await ui.askText({ title: 'Rename key', label: 'Name', value: k.label, confirmLabel: 'Rename', validate: (v) => (v.trim() ? null : 'Enter a name.') });
    if (label === null) return;
    try {
      await api.keys.rename(k.id, label.trim());
      await load();
    } catch (e) {
      ui.error(e);
    }
  }

  async function install(k: KeyInfo) {
    if (!connectionId) return;
    try {
      await api.keys.install(connectionId, k.id);
      ui.success('Key installed', 'You can now sign in to that server with this key.');
    } catch (e) {
      ui.error(e, 'Couldn’t install the key');
    }
  }

  async function remove(k: KeyInfo) {
    const ok = await ui.confirm({
      title: `Delete “${k.label}”?`,
      message: 'The private key and its saved passphrase are removed from Aether. Hosts using it fall back to your default keys. Servers that trust this key keep trusting it.',
      confirmLabel: 'Delete key',
      danger: true,
    });
    if (!ok) return;
    try {
      await api.keys.delete(k.id);
      await load();
    } catch (e) {
      ui.error(e);
    }
  }

  // ── generate ───────────────────────────────────────────────────────────────

  const KINDS: { value: KeyKind; label: string; group: string }[] = [
    { value: 'ed25519', label: 'Ed25519 — fast, modern (recommended)', group: 'Recommended' },
    { value: 'ecdsa-p256', label: 'ECDSA P-256', group: 'Other' },
    { value: 'ecdsa-p384', label: 'ECDSA P-384', group: 'Other' },
    { value: 'ecdsa-p521', label: 'ECDSA P-521', group: 'Other' },
    { value: 'rsa4096', label: 'RSA 4096 — widest compatibility', group: 'Compatibility' },
    { value: 'rsa3072', label: 'RSA 3072', group: 'Compatibility' },
    { value: 'rsa2048', label: 'RSA 2048', group: 'Compatibility' },
  ];

  let gen = $state({ label: '', kind: 'ed25519' as string, comment: '', passphrase: '', confirm: '', save: false });
  let working = $state(false);
  const genError = $derived(gen.passphrase && gen.passphrase !== gen.confirm ? 'The passphrases don’t match.' : null);

  async function generate() {
    if (genError) return;
    working = true;
    try {
      const k = await api.keys.generate({
        label: gen.label.trim() || 'New key',
        kind: gen.kind as KeyKind,
        comment: gen.comment.trim(),
        passphrase: gen.passphrase || null,
        savePassphrase: !!gen.passphrase && gen.save,
      });
      await load();
      view = 'list';
      gen = { label: '', kind: 'ed25519', comment: '', passphrase: '', confirm: '', save: false };
      ui.success('Key created', k.fingerprint);
    } catch (e) {
      ui.error(e, 'Couldn’t create the key');
    } finally {
      working = false;
    }
  }

  // ── import ─────────────────────────────────────────────────────────────────

  let imp = $state({ mode: 'file' as 'file' | 'text', path: '', pem: '', label: '', passphrase: '', save: false });
  let inspected = $state<KeyMeta | null>(null);
  let needsPass = $state(false);
  let inspectError = $state<string | null>(null);
  let passError = $state<string | null>(null);

  const source = $derived(imp.mode === 'file' ? ({ type: 'file', path: imp.path } as const) : ({ type: 'text', pem: imp.pem } as const));
  const hasSource = $derived(imp.mode === 'file' ? !!imp.path.trim() : !!imp.pem.trim());

  async function browse() {
    const [p] = await api.platform.dialog.pickFiles({ title: 'Choose a private key', multiple: false }).catch(() => []);
    if (p) {
      imp.path = p;
      if (!imp.label) imp.label = p.split(/[\\/]/).pop() ?? '';
      void inspect();
    }
  }

  async function inspect() {
    inspected = null;
    inspectError = null;
    passError = null;
    if (!hasSource) {
      needsPass = false;
      return;
    }
    working = true;
    try {
      const r = await api.keys.inspect(source, imp.passphrase || undefined);
      needsPass = r.type !== 'ready';
      if (r.type === 'ready') {
        inspected = r.meta;
        // An encrypted OpenSSH key keeps its comment inside the encrypted part, so fall back to the file name.
        if (!imp.label) imp.label = r.meta.comment || (imp.mode === 'file' ? imp.path.split(/[\\/]/).pop() : '') || 'Imported key';
      }
    } catch (e) {
      const info = toErrorInfo(e);
      // A wrong passphrase keeps the passphrase step open so it can simply be typed again.
      needsPass = info.code === 'key-passphrase-wrong';
      if (needsPass) imp.passphrase = '';
      inspectError = info.message || 'That doesn’t look like a private key.';
    } finally {
      working = false;
    }
  }

  /** OpenSSH keys show their public half without a passphrase; one that is typed anyway gets checked here. */
  async function checkPassphrase() {
    passError = null;
    if (!imp.passphrase) return;
    try {
      await api.keys.inspect(source, imp.passphrase);
    } catch (e) {
      const info = toErrorInfo(e);
      passError = info.code === 'key-passphrase-wrong' ? 'That passphrase doesn’t unlock this key.' : info.message;
    }
  }

  async function doImport() {
    await checkPassphrase();
    if (passError) return;
    working = true;
    try {
      await api.keys.import({ label: imp.label.trim() || 'Imported key', source, passphrase: imp.passphrase || null, savePassphrase: !!imp.passphrase && imp.save });
      await load();
      view = 'list';
      imp = { mode: 'file', path: '', pem: '', label: '', passphrase: '', save: false };
      inspected = null;
      ui.success('Key imported');
    } catch (e) {
      ui.error(e, 'Couldn’t import the key');
    } finally {
      working = false;
    }
  }

  const title = $derived(view === 'generate' ? 'Create a key' : view === 'import' ? 'Import a key' : 'SSH keys');
</script>

{#snippet actions()}
  {#if view === 'list'}
    <Button onclick={() => (view = 'import')}>{#snippet icon()}<Upload size={14} />{/snippet}Import</Button>
    <Button variant="primary" onclick={() => (view = 'generate')}>{#snippet icon()}<Plus size={14} />{/snippet}Create key</Button>
  {:else if view === 'generate'}
    <Button variant="ghost" onclick={() => (view = 'list')}>Cancel</Button>
    <Button variant="primary" loading={working} disabled={!!genError} onclick={generate}>Create key</Button>
  {:else}
    <Button variant="ghost" onclick={() => (view = 'list')}>Cancel</Button>
    <Button variant="primary" loading={working} disabled={!inspected} onclick={doImport}>Import key</Button>
  {/if}
{/snippet}

<Modal
  {title}
  subtitle={view === 'list' ? 'Keys saved in Aether’s encrypted vault' : undefined}
  width={680}
  padded={view !== 'list'}
  footer={view === 'list' && !keys?.length ? undefined : actions}
  onclose={() => (view === 'list' ? ui.closeDialog() : (view = 'list'))}
>
  {#snippet icon()}<KeyRound size={18} />{/snippet}
  {#snippet headerExtra()}
    {#if view !== 'list'}<Button size="sm" variant="ghost" onclick={() => (view = 'list')}>{#snippet icon()}<ArrowLeft size={14} />{/snippet}Back</Button>{/if}
  {/snippet}

  {#if view === 'list'}
    {#if keys === null}
      <div class="center"><Spinner size={22} /></div>
    {:else if keys.length === 0}
      <div class="center">
        <div class="art"><KeyRound size={26} /></div>
        <strong>No keys yet</strong>
        <span>Create a modern key pair, or import one you already use. Private keys are stored encrypted with your Windows account.</span>
        <div class="cta">
          <Button variant="primary" onclick={() => (view = 'generate')}>{#snippet icon()}<Sparkles size={14} />{/snippet}Create a key</Button>
          <Button onclick={() => (view = 'import')}>{#snippet icon()}<Upload size={14} />{/snippet}Import</Button>
        </div>
      </div>
    {:else}
      <ul>
        {#each keys as k (k.id)}
          <li>
            <div class="ico"><KeyRound size={17} /></div>
            <div class="main">
              <div class="top">
                <span class="name">{k.label}</span>
                <Badge size="sm" tone="violet">{k.algorithm.replace('ssh-', '').replace('ecdsa-sha2-', 'ecdsa-')}{k.bits ? ` · ${k.bits}` : ''}</Badge>
                {#if k.encrypted}<Badge size="sm">{#snippet children()}<Lock size={10} /> passphrase{/snippet}</Badge>{/if}
              </div>
              <code class="selectable">{k.fingerprint}</code>
              <span class="when">{k.origin === 'generated' ? 'Created' : 'Imported'} {formatRelative(k.createdAt)}</span>
            </div>
            <div class="acts">
              <IconButton label="Copy public key" size={28} onclick={() => copyPublic(k)}><Copy size={14} /></IconButton>
              <IconButton label="Save public key…" size={28} onclick={() => exportPublic(k)}><FileDown size={14} /></IconButton>
              <IconButton label={canInstall ? 'Install on the current server' : 'Connect to a server first to install this key'} size={28} disabled={!canInstall} onclick={() => install(k)}><Server size={14} /></IconButton>
              <IconButton label="Rename" size={28} onclick={() => rename(k)}><Pencil size={14} /></IconButton>
              <IconButton label="Delete" size={28} danger onclick={() => remove(k)}><Trash2 size={14} /></IconButton>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  {:else if view === 'generate'}
    <div class="fstack">
      <Input bind:value={gen.label} label="Name" placeholder="Work laptop" autofocus />
      <Select bind:value={gen.kind} options={KINDS} label="Type" />
      <Input bind:value={gen.comment} label="Comment" placeholder="you@your-pc" hint="Shown at the end of the public key so you can recognise it." />
      <div class="fgrid">
        <Input bind:value={gen.passphrase} label="Passphrase (recommended)" type="password" autocomplete="new-password" />
        <Input bind:value={gen.confirm} label="Repeat passphrase" type="password" autocomplete="new-password" error={genError} />
      </div>
      {#if gen.passphrase}
        <Switch bind:checked={gen.save} label="Remember the passphrase" description="Saved in Windows Credential Manager so you aren’t asked every time." />
      {/if}
    </div>
  {:else}
    <div class="fstack">
      <Segmented
        value={imp.mode}
        label="Key source"
        options={[
          { value: 'file', label: 'From a file' },
          { value: 'text', label: 'Paste text' },
        ]}
        onchange={(v) => {
          imp.mode = v as 'file' | 'text';
          inspected = null;
          needsPass = false;
          inspectError = null;
        }}
      />
      {#if imp.mode === 'file'}
        <div class="pick">
          <Input bind:value={imp.path} label="Private key file" mono placeholder="C:\Users\you\.ssh\id_ed25519" autofocus onblur={inspect} onkeydown={(e) => e.key === 'Enter' && inspect()} />
          <Button onclick={browse}>{#snippet icon()}<Download size={14} />{/snippet}Browse</Button>
        </div>
      {:else}
        <TextArea bind:value={imp.pem} label="Private key" mono rows={7} placeholder="-----BEGIN OPENSSH PRIVATE KEY-----" autofocus oninput={() => { inspected = null; needsPass = false; }} />
        <div><Button size="sm" onclick={inspect}>Check key</Button></div>
      {/if}

      {#if inspectError}<p class="bad">{inspectError}</p>{/if}
      {#if inspected}
        <div class="meta">
          <Badge tone="success" size="sm">Looks good</Badge>
          <span>{inspected.algorithm}{inspected.bits ? ` · ${inspected.bits}-bit` : ''}</span>
          <code class="selectable">{inspected.fingerprint}</code>
        </div>
        <Input bind:value={imp.label} label="Name" placeholder="Imported key" autofocus select onkeydown={(e) => e.key === 'Enter' && !working && doImport()} />
      {/if}
      <!-- One passphrase field serves both cases: formats that can't be read without it (asked for up front)
           and OpenSSH keys whose public half is readable (optional, but checked and rememberable). -->
      {#if needsPass || inspected?.encrypted}
        <Input
          bind:value={imp.passphrase}
          label={needsPass ? 'This key is protected — passphrase' : 'Passphrase (optional)'}
          hint={needsPass ? undefined : 'Enter it to check it now, and to remember it so you aren’t asked on every connection.'}
          type="password"
          autocomplete="current-password"
          autofocus={needsPass}
          error={passError}
          oninput={() => (passError = null)}
          onblur={() => inspected && checkPassphrase()}
          onkeydown={(e) => e.key === 'Enter' && (needsPass ? inspect() : !working && doImport())}
        />
        {#if needsPass}<div><Button size="sm" onclick={inspect}>Unlock</Button></div>{/if}
      {/if}
      {#if inspected && imp.passphrase}
        <Switch bind:checked={imp.save} label="Remember the passphrase" description="Saved in Windows Credential Manager." />
      {/if}
    </div>
  {/if}

</Modal>

<style>
  .center {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 50px 44px;
    text-align: center;
    color: var(--text-dim);
    line-height: 1.55;
  }
  .center strong {
    color: var(--text);
    font-size: var(--fs-lg);
  }
  .art {
    display: grid;
    place-items: center;
    width: 56px;
    height: 56px;
    margin-bottom: 4px;
    border-radius: 17px;
    background: var(--grad-accent-soft);
    color: var(--accent);
  }
  .cta {
    display: flex;
    gap: 8px;
    margin-top: 12px;
  }
  ul {
    margin: 0;
    padding: 8px 10px;
    list-style: none;
    max-height: 430px;
    overflow-y: auto;
  }
  li {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 12px;
    border-radius: var(--r-md);
  }
  li:hover {
    background: var(--bg-hover);
  }
  .ico {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 36px;
    height: 36px;
    border-radius: 11px;
    background: var(--grad-accent-soft);
    color: var(--accent);
  }
  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .name {
    font-weight: 620;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .when {
    font-size: var(--fs-xs);
    color: var(--text-faint);
  }
  .acts {
    display: flex;
    gap: 1px;
  }
  .pick {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 8px;
    align-items: end;
  }
  .bad {
    color: var(--danger);
    font-size: var(--fs-sm);
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    font-size: var(--fs-sm);
    color: var(--text-dim);
  }
</style>
