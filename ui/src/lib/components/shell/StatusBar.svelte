<script lang="ts">
  import { ArrowDownUp, FolderTree, Lock, Network, Palette } from '@lucide/svelte';
  import { copyText, openSftpForActive } from '$lib/actions';
  import { keymap } from '$lib/commands.svelte';
  import { tooltip } from '$lib/dom/tooltip';
  import { sessions } from '$lib/stores/sessions.svelte';
  import { settings } from '$lib/stores/settings.svelte';
  import { transfers } from '$lib/stores/transfers.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import { workspace } from '$lib/stores/workspace.svelte';
  import { formatBytes, formatSpeed, plural } from '$lib/util/format';

  const leaf = $derived(workspace.activeLeaf);
  const connectionId = $derived(workspace.activeConnection);
  const view = $derived(sessions.view(connectionId));
  const status = $derived(workspace.active ? workspace.tabStatus(workspace.active) : null);
  const info = $derived(view?.state.type === 'connected' ? view.state.info : null);
  const target = $derived(info?.target ?? (leaf?.host ? `${leaf.host.username}@${leaf.host.hostname}` : null));
  const latency = $derived(view?.latencyMs ?? null);
  const latencyTone = $derived(latency === null ? '' : latency < 90 ? 'good' : latency < 220 ? 'warn' : 'bad');
  const termInfo = $derived(leaf ? workspace.info[leaf.id] : undefined);
  const forwardCount = $derived(view?.forwards.length ?? 0);
  const active = $derived(transfers.active);
  const label = $derived(
    status === 'connected' ? 'Connected' : status === 'warning' ? 'Reconnecting' : status === 'connecting' ? 'Connecting' : status === 'error' ? 'Disconnected' : '',
  );
</script>

<footer class="status" aria-label="Status">
  <div class="group">
    {#if workspace.active && target}
      <button
        type="button"
        class="chip conn {status ?? ''}"
        onclick={() => void copyText(target, 'Address copied')}
        use:tooltip={{ text: `${label} · click to copy`, placement: 'top' }}
      >
        <i class="dot"></i>
        <span class="mono">{target}</span>
      </button>
      {#if info && info.via.length}<span class="chip dim">via {info.via.join(' → ')}</span>{/if}
      {#if latency !== null}
        <span class="chip {latencyTone}" use:tooltip={{ text: 'Round-trip time to the server', placement: 'top' }}>
          <ArrowDownUp size={12} /> {Math.round(latency)} ms
        </span>
      {/if}
      {#if info?.negotiated}
        <span class="chip dim" use:tooltip={{ text: `${info.negotiated.kex} · ${info.negotiated.hostKey} · ${info.negotiated.mac}${info.negotiated.compression !== 'none' ? ' · ' + info.negotiated.compression : ''}`, placement: 'top' }}>
          <Lock size={12} /> {info.negotiated.cipher}
        </span>
      {/if}
    {:else}
      <span class="chip dim">Ready</span>
    {/if}
  </div>

  <div class="group right">
    {#if active.length}
      {@const done = active.reduce((n, t) => n + t.bytesDone, 0)}
      {@const total = active.reduce((n, t) => n + t.bytesTotal, 0)}
      {@const speed = active.reduce((n, t) => n + t.speedBps, 0)}
      <span class="chip accent" use:tooltip={{ text: `${formatBytes(done)} of ${formatBytes(total)}`, placement: 'top' }}>
        <span class="bar"><i style:width="{total ? Math.min(100, (done / total) * 100) : 0}%"></i></span>
        {plural(active.length, 'transfer')} · {formatSpeed(speed)}
      </span>
    {/if}
    {#if connectionId && status === 'connected'}
      <button type="button" class="chip" onclick={openSftpForActive} use:tooltip={{ text: 'Browse files on this server', shortcut: keymap.display('session.sftp'), placement: 'top' }}>
        <FolderTree size={12} /> Files
      </button>
    {/if}
    {#if connectionId && forwardCount > 0}
      <button type="button" class="chip" onclick={() => ui.openDialog({ type: 'forwards', connectionId })} use:tooltip={{ text: 'Port forwards', placement: 'top' }}>
        <Network size={12} /> {plural(forwardCount, 'forward')}
      </button>
    {/if}
    {#if termInfo?.size}
      <span class="chip dim mono" use:tooltip={{ text: 'Terminal size (columns × rows)', placement: 'top' }}>{termInfo.size.cols}×{termInfo.size.rows}</span>
    {/if}
    <button type="button" class="chip dim" onclick={() => ui.openSettings('appearance')} use:tooltip={{ text: 'Theme', placement: 'top' }}>
      <Palette size={12} /> {settings.terminalTheme.name}
    </button>
  </div>
</footer>

<style>
  .status {
    grid-area: status;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    height: var(--statusbar-h);
    padding: 0 8px;
    background: var(--bg-sidebar);
    border-top: 1px solid var(--border);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    user-select: none;
  }
  .group {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 20px;
    padding: 0 7px;
    border-radius: var(--r-sm);
    white-space: nowrap;
    color: var(--text-dim);
  }
  button.chip:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .chip.dim {
    color: var(--text-faint);
  }
  .chip.good {
    color: var(--success);
  }
  .chip.warn {
    color: var(--warning);
  }
  .chip.bad {
    color: var(--danger);
  }
  .chip.accent {
    color: var(--accent);
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-faint);
  }
  .conn.connected .dot {
    background: var(--success);
    box-shadow: 0 0 8px color-mix(in srgb, var(--success) 70%, transparent);
  }
  .conn.connecting .dot {
    background: var(--accent);
    animation: pulse-dot 1.1s var(--ease-in-out) infinite;
  }
  .conn.warning .dot {
    background: var(--warning);
    animation: pulse-dot 1.1s var(--ease-in-out) infinite;
  }
  .conn.error .dot {
    background: var(--danger);
  }
  .conn span {
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .bar {
    width: 54px;
    height: 4px;
    border-radius: 2px;
    background: var(--bg-active);
    overflow: hidden;
  }
  .bar i {
    display: block;
    height: 100%;
    background: var(--grad-accent);
    transition: width 300ms linear;
  }
</style>
