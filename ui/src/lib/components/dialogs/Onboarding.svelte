<script lang="ts">
  import { ArrowRight, Check, Command, Download, FolderTree, KeyRound, Plus, Rows2 } from '@lucide/svelte';
  import { onMount } from 'svelte';
  import { fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { api } from '$lib/api';
  import type { SshConfigPreview } from '$lib/bindings';
  import { keymap } from '$lib/commands.svelte';
  import { hosts } from '$lib/stores/hosts.svelte';
  import { settings } from '$lib/stores/settings.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import { plural } from '$lib/util/format';
  import Button from '$lib/components/ui/Button.svelte';
  import Kbd from '$lib/components/ui/Kbd.svelte';
  import Logo from '$lib/components/ui/Logo.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';

  let step = $state(0);
  let dir = $state(1);
  let preview = $state<SshConfigPreview | null>(null);
  let imported = $state<number | null>(null);
  let importing = $state(false);

  const STEPS = 3;
  const THEMES = ['aether-dark', 'midnight', 'aether-light'];

  onMount(() => void api.hosts.sshConfigPreview().then((p) => (preview = p)).catch(() => {}));

  const a = $derived(settings.value.appearance);
  const followSystem = $derived(a.theme === 'system');
  const fresh = $derived(preview?.candidates.filter((c) => !c.alreadyImported) ?? []);

  function go(delta: 1 | -1) {
    dir = delta;
    step = Math.max(0, Math.min(STEPS - 1, step + delta));
  }

  function finish() {
    settings.patch({ general: { onboardingCompleted: true } });
    ui.closeAllDialogs();
  }

  async function importAll() {
    importing = true;
    try {
      const r = await api.hosts.sshConfigImport(fresh.map((c) => c.alias), null);
      await hosts.refresh();
      imported = r.added;
    } catch (e) {
      ui.error(e, 'Import failed');
    } finally {
      importing = false;
    }
  }

  function openAnd(fn: () => void) {
    // Finish the tour first so the next dialog isn't stacked on top of it.
    settings.patch({ general: { onboardingCompleted: true } });
    ui.closeAllDialogs();
    queueMicrotask(fn);
  }
</script>

<Modal title="" width={640} padded={false} dismissable onclose={finish}>
  {#snippet icon()}<Logo size={26} tile={false} />{/snippet}
  <div class="stage">
    {#key step}
      <div class="step" in:fly={{ x: dir * 28, duration: 260, easing: cubicOut }}>
        {#if step === 0}
          <h2>Welcome to Aether</h2>
          <p class="lead">A calmer way to work with servers. Let’s pick how it should look — you can change this any time in Settings.</p>
          <div class="themes">
            {#each THEMES as id (id)}
              {@const t = settings.themes.find((x) => x.id === id)}
              {#if t}
                {@const p = t.terminal}
                {@const on = !followSystem && a.theme === id}
                <button type="button" class="theme" class:on onclick={() => settings.patch({ appearance: { theme: id } })} aria-pressed={on}>
                  <span class="shot" style:background={p.background}>
                    <span class="ln"><i style:background={p.green}></i><i style:background={p.foreground}></i></span>
                    <span class="ln"><i style:background={p.cyan}></i><i style:background={p.yellow}></i></span>
                    <span class="ln"><i style:background={p.magenta}></i><i style:background={p.blue}></i><i style:background={p.foreground}></i></span>
                  </span>
                  <span class="nm">{t.name}{#if on}<span class="ck"><Check size={11} strokeWidth={3} /></span>{/if}</span>
                </button>
              {/if}
            {/each}
          </div>
          <button type="button" class="sys" class:on={followSystem} onclick={() => settings.patch({ appearance: { theme: followSystem ? a.darkTheme : 'system' } })} aria-pressed={followSystem}>
            <span class="box">{#if followSystem}<Check size={11} strokeWidth={3} />{/if}</span>
            Match Windows light / dark mode
          </button>
        {:else if step === 1}
          <h2>Bring your servers</h2>
          <p class="lead">Aether can pick up where you left off, or you can start from scratch.</p>
          <div class="options">
            {#if preview?.found && fresh.length > 0}
              <div class="option hot">
                <span class="oi"><Download size={20} /></span>
                <span class="ot">
                  <b>{plural(fresh.length, 'host')} found in your ssh config</b>
                  <small>{fresh.slice(0, 4).map((c) => c.alias).join(', ')}{fresh.length > 4 ? '…' : ''}</small>
                </span>
                {#if imported !== null}
                  <span class="done"><Check size={14} strokeWidth={3} /> Imported {imported}</span>
                {:else}
                  <Button variant="primary" size="sm" loading={importing} onclick={importAll}>Import all</Button>
                {/if}
              </div>
            {/if}
            <button type="button" class="option" onclick={() => openAnd(() => ui.newHost())}>
              <span class="oi"><Plus size={20} /></span>
              <span class="ot"><b>Add a server</b><small>Name, address and how you sign in.</small></span>
              <ArrowRight size={16} />
            </button>
            <button type="button" class="option" onclick={() => openAnd(() => ui.openDialog({ type: 'keys' }))}>
              <span class="oi"><KeyRound size={20} /></span>
              <span class="ot"><b>Create an SSH key</b><small>A modern Ed25519 key, stored encrypted on this PC.</small></span>
              <ArrowRight size={16} />
            </button>
          </div>
          <p class="hint">Prefer to just connect? Type <code>user@server</code> in the box on the home screen.</p>
        {:else}
          <h2>You’re all set</h2>
          <p class="lead">A few things worth knowing:</p>
          <ul class="tips">
            <li>
              <span class="ti"><Command size={18} /></span>
              <span><b>Search everything.</b> Hosts, commands and even the text in your terminals.</span>
              <Kbd keys={keymap.display('palette.open')} />
            </li>
            <li>
              <span class="ti"><Rows2 size={18} /></span>
              <span><b>Split the view.</b> Run several shells side by side over one connection.</span>
              <Kbd keys={keymap.display('pane.splitRight')} />
            </li>
            <li>
              <span class="ti"><FolderTree size={18} /></span>
              <span><b>Move files.</b> Browse the server and drag files in and out.</span>
              <Kbd keys={keymap.display('session.sftp')} />
            </li>
          </ul>
        {/if}
      </div>
    {/key}
  </div>

  {#snippet footer()}
    <div class="dots" aria-hidden="true">
      {#each Array(STEPS) as _, i (i)}<i class:on={i === step}></i>{/each}
    </div>
    {#if step < STEPS - 1}
      <Button variant="ghost" onclick={finish}>Skip</Button>
    {/if}
    {#if step > 0}<Button onclick={() => go(-1)}>Back</Button>{/if}
    {#if step < STEPS - 1}
      <Button variant="primary" autofocus onclick={() => go(1)}>Continue</Button>
    {:else}
      <Button variant="primary" autofocus onclick={finish}>Start using Aether</Button>
    {/if}
  {/snippet}
</Modal>

<style>
  .stage {
    min-height: 372px;
    padding: 26px 32px 18px;
    overflow: hidden;
  }
  .step {
    display: flex;
    flex-direction: column;
  }
  h2 {
    font-size: var(--fs-2xl);
    font-weight: 750;
    letter-spacing: -0.02em;
  }
  .lead {
    margin: 6px 0 22px;
    color: var(--text-dim);
    line-height: 1.55;
  }
  .themes {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 12px;
  }
  .theme {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 7px;
    text-align: left;
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    transition:
      transform var(--dur) var(--ease-out),
      border-color var(--dur-fast),
      box-shadow var(--dur);
  }
  .theme:hover {
    transform: translateY(-3px);
  }
  .theme.on {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .shot {
    display: flex;
    flex-direction: column;
    gap: 7px;
    height: 86px;
    padding: 14px 12px;
    border-radius: 9px;
    box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.07);
  }
  .ln {
    display: flex;
    gap: 5px;
  }
  .ln i {
    display: block;
    height: 5px;
    border-radius: 3px;
  }
  .ln i:nth-child(1) {
    width: 22px;
  }
  .ln i:nth-child(2) {
    width: 46px;
  }
  .ln i:nth-child(3) {
    width: 28px;
    opacity: 0.6;
  }
  .nm {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 4px 2px;
    font-weight: 600;
  }
  .ck {
    display: grid;
    place-items: center;
    width: 17px;
    height: 17px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--on-accent);
  }
  .sys {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    align-self: flex-start;
    margin-top: 18px;
    color: var(--text-dim);
  }
  .sys:hover {
    color: var(--text);
  }
  .box {
    display: grid;
    place-items: center;
    width: 17px;
    height: 17px;
    border-radius: 5px;
    border: 1px solid var(--border-strong);
    color: var(--on-accent);
  }
  .sys.on .box {
    background: var(--accent);
    border-color: var(--accent);
  }

  .options {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .option {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 14px 16px;
    text-align: left;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    color: var(--text-dim);
    transition:
      transform var(--dur) var(--ease-out),
      border-color var(--dur-fast);
  }
  button.option:hover {
    transform: translateX(3px);
    border-color: var(--accent-ring);
    color: var(--text);
  }
  .option.hot {
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
    background: var(--grad-accent-soft);
  }
  .oi {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 40px;
    height: 40px;
    border-radius: 12px;
    background: var(--accent-soft);
    color: var(--accent);
  }
  .ot {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ot b {
    color: var(--text);
    font-weight: 620;
  }
  .ot small {
    font-size: var(--fs-sm);
    color: var(--text-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .done {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--success);
    font-weight: 600;
  }
  .hint {
    margin-top: 16px;
    color: var(--text-faint);
    font-size: var(--fs-sm);
  }
  code {
    font-family: var(--font-mono);
    color: var(--text-dim);
  }

  .tips {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .tips li {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 14px 16px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    color: var(--text-dim);
    line-height: 1.45;
  }
  .tips li span:nth-child(2) {
    flex: 1;
  }
  .tips b {
    color: var(--text);
  }
  .ti {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 36px;
    height: 36px;
    border-radius: 11px;
    background: var(--grad-accent-soft);
    color: var(--accent);
  }

  .dots {
    display: flex;
    gap: 6px;
    margin-right: auto;
  }
  .dots i {
    width: 7px;
    height: 7px;
    border-radius: 4px;
    background: var(--bg-active);
    transition:
      width var(--dur) var(--ease-out),
      background var(--dur);
  }
  .dots i.on {
    width: 22px;
    background: var(--grad-accent);
  }
</style>
