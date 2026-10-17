<script lang="ts">
  import {
    ChevronDown, Hourglass, KeyRound, Pencil, RefreshCw, Save, ShieldAlert, TriangleAlert, WifiOff, X,
  } from '@lucide/svelte';
  import type { Component } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { api } from '$lib/api';
  import type { ConnectStage, ErrorCode, ErrorInfo } from '$lib/bindings';
  import { reconnectPane } from '$lib/actions';
  import { sessions } from '$lib/stores/sessions.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import { workspace } from '$lib/stores/workspace.svelte';
  import type { Leaf } from '$lib/util/panes';
  import Button from '$lib/components/ui/Button.svelte';
  import Logo from '$lib/components/ui/Logo.svelte';

  let { leaf, visible }: { leaf: Leaf; visible: boolean } = $props();

  const info = $derived(workspace.info[leaf.id]);
  const view = $derived(sessions.view(leaf.connectionId));
  const st = $derived(view?.state);
  const name = $derived(leaf.host?.label || leaf.host?.hostname || leaf.target || 'server');
  const address = $derived(leaf.host ? `${leaf.host.username ? leaf.host.username + '@' : ''}${leaf.host.hostname}${leaf.host.port !== 22 ? ':' + leaf.host.port : ''}` : (leaf.target ?? ''));

  // A failed *first* attempt gets a full-pane card; a link that dropped later keeps the scrollback visible.
  const failure = $derived.by<ErrorInfo | null>(() => {
    if (info?.phase === 'error' && info.error) return info.error;
    if (st?.type === 'disconnected' && !view?.everConnected) {
      return st.error ?? { code: 'disconnected', title: 'Disconnected', message: st.reason || 'The connection ended before it was ready.', hint: null, detail: null, retryable: true };
    }
    return null;
  });
  const connecting = $derived(!failure && !view?.everConnected && (!view || st?.type === 'connecting' || st?.type === 'reconnecting') && info?.phase !== 'error');
  const lost = $derived(!!view?.everConnected && st?.type === 'reconnecting');
  const dropped = $derived(!!view?.everConnected && st?.type === 'disconnected');
  const closed = $derived(st?.type === 'closed');
  const exited = $derived(!failure && !connecting && !lost && !dropped && info?.terminal?.type === 'exited' ? info.terminal : null);

  const STEPS = ['Resolve', 'Connect', 'Secure', 'Sign in', 'Shell'] as const;
  function stepOf(stage: ConnectStage | null | undefined): number {
    switch (stage?.type) {
      case 'resolving': return 0;
      case 'connecting': return 1;
      case 'handshake': return 2;
      case 'authenticating': return 3;
      case 'establishing': return 4;
      case 'ready': return 5;
      default: return 0;
    }
  }
  const stage = $derived(st?.type === 'connecting' ? st.stage : null);
  const step = $derived(stepOf(stage));
  const hop = $derived(st?.type === 'connecting' ? st.hop : null);

  const stageText = $derived.by(() => {
    if (st?.type === 'reconnecting') return `Trying again (attempt ${st.attempt})…`;
    switch (stage?.type) {
      case 'resolving': return 'Looking up the address…';
      case 'connecting': return 'Opening a connection…';
      case 'handshake': return 'Negotiating encryption…';
      case 'authenticating': return `Signing in (${stage.method})…`;
      case 'establishing': return 'Opening a secure channel…';
      case 'ready': return 'Starting your shell…';
      default: return 'Getting ready…';
    }
  });

  // Countdown for the reconnect banner.
  let now = $state(Date.now());
  let since = $state(Date.now());
  $effect(() => {
    void st;
    since = Date.now();
  });
  $effect(() => {
    if (st?.type !== 'reconnecting') return;
    const t = setInterval(() => (now = Date.now()), 400);
    return () => clearInterval(t);
  });
  const retryIn = $derived(st?.type === 'reconnecting' ? Math.max(0, Math.ceil((since + Number(st.retryInMs) - now) / 1000)) : 0);

  let showDetail = $state(false);

  /** "connection lost: the server closed it." → "The server closed it" (the banner adds its own lead-in). */
  const sentence = (reason: string) => {
    const t = reason.replace(/^connection lost:\s*/i, '').replace(/[.\s]+$/, '');
    return t.charAt(0).toUpperCase() + t.slice(1);
  };

  function iconFor(code: ErrorCode): Component<{ size?: number }> {
    switch (code) {
      case 'dns-failure': case 'network-unreachable': case 'connection-refused': case 'connection-reset': return WifiOff;
      case 'connection-timeout': case 'timeout': return Hourglass;
      case 'host-key-changed': case 'host-key-rejected': return ShieldAlert;
      case 'auth-failed': case 'auth-cancelled': case 'no-auth-methods': case 'key-passphrase-required': case 'key-passphrase-wrong': case 'key-unreadable': case 'agent-unavailable': return KeyRound;
      default: return TriangleAlert;
    }
  }
  const Icon = $derived(failure ? iconFor(failure.code) : TriangleAlert);
  const credentials = $derived(failure ? ['auth-failed', 'no-auth-methods', 'key-passphrase-wrong', 'key-passphrase-required', 'key-unreadable', 'agent-unavailable', 'auth-cancelled'].includes(failure.code) : false);
  const hostKey = $derived(failure ? ['host-key-changed', 'host-key-rejected'].includes(failure.code) : false);

  function edit() {
    if (leaf.hostId) ui.editHost(leaf.hostId);
    else if (leaf.host) ui.newHost(null, { ...leaf.host });
  }

  function restartShell() {
    if (leaf.connectionId && leaf.terminalId) void api.sessions.restartTerminal(leaf.connectionId, leaf.terminalId).catch((e) => ui.error(e));
  }

  const closePane = () => workspace.closePane(leaf.id);
</script>

{#if connecting}
  <div class="screen" role="status" aria-live="polite" transition:fade={{ duration: 220 }}>
    <div class="aurora ambient" aria-hidden="true"><i class="a"></i><i class="b"></i></div>
    <div class="card" in:fly={{ y: 14, duration: 320, easing: cubicOut }}>
      <div class="mark"><Logo size={64} animated={visible} /></div>
      <h2>{view?.everConnected ? 'Reconnecting' : 'Connecting'} to {name}</h2>
      <p class="addr">{address}</p>
      <ol class="steps" aria-label="Progress">
        {#each STEPS as s, i (s)}
          <li class:done={i < step} class:now={i === step}>
            <i></i><span>{s}</span>
          </li>
        {/each}
      </ol>
      <p class="stage">{stageText}</p>
      {#if hop && hop.total > 1}<p class="hop">via {hop.label} · hop {hop.index + 1} of {hop.total}</p>{/if}
      {#if view?.banner}<pre class="banner-text selectable">{view.banner.trim()}</pre>{/if}
      <div class="row"><Button variant="ghost" size="sm" onclick={closePane}>Cancel</Button></div>
    </div>
  </div>
{:else if failure}
  <div class="screen" role="alert" transition:fade={{ duration: 200 }}>
    <div class="aurora fail ambient" aria-hidden="true"><i class="a"></i><i class="b"></i></div>
    <div class="card err" in:fly={{ y: 14, duration: 320, easing: cubicOut }}>
      <div class="glyph"><Icon size={28} /></div>
      <h2>{failure.title}</h2>
      <p class="addr">{name} · {address}</p>
      <p class="msg">{failure.message}</p>
      {#if failure.hint}<p class="hint">{failure.hint}</p>{/if}
      {#if failure.detail}
        <button type="button" class="more" aria-expanded={showDetail} onclick={() => (showDetail = !showDetail)}>
          <span class="chev" class:open={showDetail}><ChevronDown size={13} /></span> Technical details
        </button>
        {#if showDetail}<pre class="detail selectable">{failure.detail}</pre>{/if}
      {/if}
      <div class="row">
        {#if credentials}
          <Button variant="primary" onclick={edit}>{#snippet icon()}<Pencil size={14} />{/snippet}{leaf.hostId ? 'Edit host' : 'Save & edit'}</Button>
          <Button onclick={() => void reconnectPane(leaf.id)}>{#snippet icon()}<RefreshCw size={14} />{/snippet}Try again</Button>
        {:else if hostKey}
          <Button variant="primary" onclick={() => ui.openDialog({ type: 'known-hosts' })}>{#snippet icon()}<ShieldAlert size={14} />{/snippet}Review known hosts</Button>
          <Button onclick={() => void reconnectPane(leaf.id)}>{#snippet icon()}<RefreshCw size={14} />{/snippet}Try again</Button>
        {:else}
          <Button variant="primary" onclick={() => void reconnectPane(leaf.id)}>{#snippet icon()}<RefreshCw size={14} />{/snippet}Try again</Button>
          {#if leaf.hostId}<Button onclick={edit}>{#snippet icon()}<Pencil size={14} />{/snippet}Edit host</Button>{:else}<Button onclick={edit}>{#snippet icon()}<Save size={14} />{/snippet}Save host…</Button>{/if}
        {/if}
        <Button variant="ghost" onclick={closePane}>Close</Button>
      </div>
    </div>
  </div>
{/if}

{#if lost && st?.type === 'reconnecting'}
  <div class="banner warn" role="status" transition:fly={{ y: -16, duration: 220, easing: cubicOut }}>
    <i class="pulse"></i>
    <span class="t">
      <b>Connection lost{st.reason ? ':' : '.'}</b>{#if st.reason} {sentence(st.reason)}.{/if}
      <span class="eta">{retryIn > 0 ? `Reconnecting in ${retryIn}s` : 'Reconnecting now'} · attempt {st.attempt}{st.maxAttempts ? ` of ${st.maxAttempts}` : ''}</span>
    </span>
    <Button size="sm" onclick={() => void reconnectPane(leaf.id)}>Reconnect now</Button>
    <Button size="sm" variant="ghost" onclick={closePane}>Close</Button>
  </div>
{:else if dropped && st?.type === 'disconnected'}
  <div class="banner bad" role="alert" transition:fly={{ y: -16, duration: 220, easing: cubicOut }}>
    <i class="solid"></i>
    <span class="t"><b>Disconnected.</b> {st.error?.message ?? st.reason}</span>
    <Button size="sm" variant="primary" onclick={() => void reconnectPane(leaf.id)}>{#snippet icon()}<RefreshCw size={13} />{/snippet}Reconnect</Button>
    <Button size="sm" variant="ghost" onclick={closePane}>Close</Button>
  </div>
{:else if closed}
  <div class="banner" role="status" transition:fly={{ y: -16, duration: 220, easing: cubicOut }}>
    <i class="solid mute"></i>
    <span class="t"><b>Session closed.</b></span>
    <Button size="sm" variant="ghost" onclick={closePane}>Close pane</Button>
  </div>
{/if}

{#if exited}
  <div class="exitbar" role="status" transition:fly={{ y: 16, duration: 220, easing: cubicOut }}>
    <span class="t">
      The shell {exited.type === 'exited' && exited.signal ? `was ended by ${exited.signal}` : `exited${exited.type === 'exited' && exited.status !== null ? ` with status ${exited.status}` : ''}`}.
    </span>
    <Button size="sm" variant="primary" onclick={restartShell}>New shell <kbd>↵</kbd></Button>
    <Button size="sm" variant="ghost" onclick={closePane}>{#snippet icon()}<X size={13} />{/snippet}Close</Button>
  </div>
{/if}

<style>
  .screen {
    position: absolute;
    inset: 0;
    z-index: 5;
    display: grid;
    place-items: center;
    padding: 20px;
    overflow: hidden;
    background: color-mix(in srgb, var(--bg-terminal) 94%, transparent);
  }
  .aurora {
    position: absolute;
    inset: 0;
    pointer-events: none;
    opacity: 0.55;
  }
  /* Transform-only motion (see Welcome.svelte): no blur filter, no backdrop filter over moving layers. */
  .aurora i {
    position: absolute;
    border-radius: 50%;
    will-change: transform;
  }
  .aurora .a {
    left: 12%;
    top: 4%;
    width: 46%;
    aspect-ratio: 1;
    background: radial-gradient(circle, rgb(70 216 255 / 0.55), transparent 65%);
    animation: aurora-a 17s ease-in-out infinite;
  }
  .aurora .b {
    right: 6%;
    bottom: 0;
    width: 42%;
    aspect-ratio: 1;
    background: radial-gradient(circle, rgb(155 120 255 / 0.5), transparent 65%);
    animation: aurora-b 21s ease-in-out infinite;
  }
  .aurora.fail .a {
    background: radial-gradient(circle, rgb(255 111 138 / 0.4), transparent 65%);
  }
  .aurora.fail .b {
    background: radial-gradient(circle, rgb(245 192 103 / 0.28), transparent 65%);
  }

  .card {
    position: relative;
    z-index: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    width: min(480px, 100%);
    padding: 30px 30px 24px;
    text-align: center;
    background: color-mix(in srgb, var(--bg-surface) 93%, transparent);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-xl);
    box-shadow: var(--shadow-lg);
  }
  .mark {
    margin-bottom: 14px;
    filter: drop-shadow(0 10px 28px rgb(111 140 255 / 0.4));
  }
  h2 {
    font-size: var(--fs-xl);
    font-weight: 650;
    letter-spacing: -0.01em;
  }
  .addr {
    margin-top: 3px;
    font-size: var(--fs-sm);
    color: var(--text-faint);
    font-family: var(--font-mono);
  }
  .steps {
    display: flex;
    gap: 6px;
    margin: 22px 0 0;
    padding: 0;
    list-style: none;
  }
  .steps li {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 7px;
    width: 62px;
    font-size: var(--fs-2xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-faint);
    transition: color var(--dur);
  }
  .steps i {
    position: relative;
    display: block;
    width: 100%;
    height: 3px;
    border-radius: 3px;
    background: var(--bg-active);
    overflow: hidden;
  }
  .steps li.done {
    color: var(--text-dim);
  }
  .steps li.done i {
    background: var(--grad-accent);
  }
  .steps li.now {
    color: var(--accent);
  }
  .steps li.now i::after {
    content: '';
    position: absolute;
    inset: 0;
    background: linear-gradient(90deg, transparent, var(--accent), transparent);
    background-size: 200% 100%;
    animation: shimmer 1.2s linear infinite;
  }
  .stage {
    margin-top: 16px;
    color: var(--text-dim);
    font-size: var(--fs-md);
  }
  .hop {
    margin-top: 4px;
    font-size: var(--fs-sm);
    color: var(--accent-2);
  }
  .banner-text {
    width: 100%;
    max-height: 120px;
    margin: 14px 0 0;
    padding: 10px 12px;
    overflow: auto;
    text-align: left;
    white-space: pre-wrap;
    font: 11.5px/1.5 var(--font-mono);
    color: var(--text-dim);
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
    margin-top: 20px;
  }

  .err .glyph {
    display: grid;
    place-items: center;
    width: 54px;
    height: 54px;
    margin-bottom: 14px;
    border-radius: 16px;
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 14%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--danger) 30%, transparent);
  }
  .msg {
    margin-top: 12px;
    color: var(--text);
    line-height: 1.55;
  }
  .hint {
    margin-top: 12px;
    padding: 9px 12px;
    width: 100%;
    text-align: left;
    font-size: var(--fs-sm);
    line-height: 1.5;
    color: var(--text);
    background: var(--accent-soft);
    border-radius: var(--r-md);
    border-left: 3px solid var(--accent);
  }
  .more {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    margin-top: 12px;
    font-size: var(--fs-xs);
    color: var(--text-faint);
  }
  .more:hover {
    color: var(--text-dim);
  }
  .chev {
    display: inline-flex;
    transition: transform var(--dur) var(--ease-out);
  }
  .chev.open {
    transform: rotate(180deg);
  }
  .detail {
    width: 100%;
    max-height: 130px;
    margin: 8px 0 0;
    padding: 9px 11px;
    overflow: auto;
    text-align: left;
    white-space: pre-wrap;
    word-break: break-word;
    font: 11px/1.5 var(--font-mono);
    color: var(--text-dim);
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
  }

  .banner {
    position: absolute;
    top: 10px;
    left: 50%;
    z-index: 6;
    translate: -50% 0;
    display: flex;
    align-items: center;
    gap: 10px;
    max-width: calc(100% - 28px);
    padding: 8px 8px 8px 14px;
    background: color-mix(in srgb, var(--bg-overlay) 96%, transparent);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-lg);
    box-shadow: var(--shadow-lg);
    font-size: var(--fs-sm);
  }
  .banner .t {
    min-width: 0;
    color: var(--text-dim);
    line-height: 1.4;
  }
  .banner .t b {
    color: var(--text);
  }
  .banner .eta {
    display: block;
    color: var(--text-faint);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .banner.warn {
    border-color: color-mix(in srgb, var(--warning) 45%, transparent);
  }
  .banner.bad {
    border-color: color-mix(in srgb, var(--danger) 45%, transparent);
  }
  .banner i {
    flex-shrink: 0;
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .pulse {
    background: var(--warning);
    animation: pulse-dot 1.1s var(--ease-in-out) infinite;
  }
  .solid {
    background: var(--danger);
  }
  .solid.mute {
    background: var(--text-faint);
  }

  .exitbar {
    position: absolute;
    bottom: 12px;
    left: 50%;
    z-index: 6;
    translate: -50% 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 8px 8px 14px;
    background: color-mix(in srgb, var(--bg-overlay) 96%, transparent);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-lg);
    box-shadow: var(--shadow-lg);
    font-size: var(--fs-sm);
    color: var(--text-dim);
  }
  kbd {
    margin-left: 4px;
    font: 600 11px var(--font-ui);
    opacity: 0.7;
  }
</style>
