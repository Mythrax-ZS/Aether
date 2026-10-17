<script lang="ts">
  import { FolderOpen, ScrollText } from '@lucide/svelte';
  import { onMount } from 'svelte';
  import { api } from '$lib/api';
  import { copyText } from '$lib/actions';
  import type { AppInfo } from '$lib/bindings';
  import { ui } from '$lib/stores/ui.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Logo from '$lib/components/ui/Logo.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';

  let info = $state<AppInfo | null>(null);
  onMount(() => void api.app.info().then((i) => (info = i)).catch(() => {}));

  const mode = $derived(info?.storageMode === 'portable' ? 'Portable — settings live next to the app' : info?.storageMode === 'custom' ? 'Custom location' : 'Installed — settings in your user profile');

  function copyDetails() {
    if (!info) return;
    void copyText(
      [`Aether ${info.version}${info.debug ? ' (debug)' : ''}`, info.os, `Storage: ${info.storageMode}`, `Secrets: ${info.vaultBackend}`, `Config: ${info.configDir}`].join('\n'),
      'Details copied',
    );
  }
</script>

<Modal title="About Aether" width={460} onclose={() => ui.closeDialog()}>
  <div class="about">
    <div class="mark"><Logo size={88} animated="ambient" /></div>
    <h2>Aether</h2>
    <p class="ver">Version {info?.version ?? '…'}{info?.debug ? ' · debug build' : ''}</p>
    <p class="tag">A fast, beautiful SSH client for Windows.</p>

    {#if info}
      <dl>
        <dt>System</dt>
        <dd>{info.os}</dd>
        <dt>Storage</dt>
        <dd>{mode}</dd>
        <dt>Secrets</dt>
        <dd>{info.vaultBackend}</dd>
        <dt>Config folder</dt>
        <dd class="mono selectable">{info.configDir}</dd>
      </dl>
    {/if}

    <div class="row">
      <Button size="sm" onclick={() => void api.app.openConfig().catch((e) => ui.error(e))}>{#snippet icon()}<FolderOpen size={13} />{/snippet}Config folder</Button>
      <Button size="sm" onclick={() => void api.app.openLogs().catch((e) => ui.error(e))}>{#snippet icon()}<ScrollText size={13} />{/snippet}Logs</Button>
      <Button size="sm" variant="ghost" onclick={copyDetails}>Copy details</Button>
    </div>
    <p class="legal">Built with Rust, Tauri, russh and xterm.js. Your keys and passwords never leave this PC except to authenticate to the servers you choose.</p>
  </div>
</Modal>

<style>
  .about {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
  }
  .mark {
    margin: 6px 0 10px;
    filter: drop-shadow(0 14px 34px rgb(111 140 255 / 0.4));
  }
  h2 {
    font-size: var(--fs-2xl);
    font-weight: 750;
    letter-spacing: -0.02em;
    background: var(--grad-accent);
    -webkit-background-clip: text;
    background-clip: text;
    color: transparent;
  }
  .ver {
    margin-top: 2px;
    color: var(--text-dim);
    font-size: var(--fs-sm);
  }
  .tag {
    margin-top: 10px;
    color: var(--text);
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 7px 16px;
    width: 100%;
    margin: 22px 0 18px;
    padding: 14px 16px;
    text-align: left;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    font-size: var(--fs-sm);
  }
  dt {
    color: var(--text-faint);
  }
  dd {
    margin: 0;
    color: var(--text-dim);
    word-break: break-all;
  }
  dd.mono {
    font-size: var(--fs-xs);
  }
  .row {
    display: flex;
    gap: 8px;
  }
  .legal {
    margin-top: 18px;
    color: var(--text-faint);
    font-size: var(--fs-xs);
    line-height: 1.55;
  }
</style>
