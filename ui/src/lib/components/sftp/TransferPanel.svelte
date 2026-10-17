<script lang="ts">
  import { slide } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { ArrowDownToLine, ArrowUpFromLine, Check, ChevronUp, FolderOpen, Pause, Play, Trash2, TriangleAlert, X } from '@lucide/svelte';
  import { api } from '$lib/api';
  import type { TransferInfo } from '$lib/bindings';
  import type { SftpController } from '$lib/sftp/controller.svelte';
  import { tooltip } from '$lib/dom/tooltip';
  import { transfers } from '$lib/stores/transfers.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import { formatBytes, formatDuration, formatSpeed, plural } from '$lib/util/format';

  let { ctl }: { ctl: SftpController } = $props();

  const list = $derived(transfers.list);
  const active = $derived(transfers.active);
  const failed = $derived(list.filter((t) => t.state === 'failed').length);
  const percent = (t: TransferInfo) => (t.bytesTotal > 0 ? Math.min(100, (t.bytesDone / t.bytesTotal) * 100) : t.filesTotal > 0 ? (t.filesDone / t.filesTotal) * 100 : 0);

  const summary = $derived.by(() => {
    if (active.length) return `${plural(active.length, 'transfer')} · ${Math.round(transfers.progress * 100)}% · ${formatSpeed(transfers.speed)}`;
    if (failed) return `${plural(failed, 'transfer')} failed`;
    if (transfers.finished.length) return `${plural(transfers.finished.length, 'transfer')} finished`;
    return 'No transfers yet';
  });

  function eta(t: TransferInfo): string {
    if (t.state !== 'running' || t.speedBps <= 0) return '';
    return `${formatDuration((t.bytesTotal - t.bytesDone) / t.speedBps)} left`;
  }

  function line(t: TransferInfo): string {
    switch (t.state) {
      case 'queued':
        return 'Waiting for a free slot…';
      case 'running':
        return t.current ? t.current : 'Starting…';
      case 'paused':
        return `Paused · ${formatBytes(t.bytesDone)} of ${formatBytes(t.bytesTotal)}`;
      case 'completed':
        return `${plural(t.filesDone, 'file')} · ${formatBytes(t.bytesTotal)}${t.skipped ? ` · ${t.skipped} skipped` : ''}`;
      case 'cancelled':
        return 'Cancelled';
      case 'failed':
        return t.error?.message ?? 'Failed';
    }
  }

  const act = (p: Promise<unknown>) => void p.catch((e) => ui.error(e));
</script>

<section class="panel" class:open={ctl.panelOpen} aria-label="Transfers">
  <button type="button" class="head" onclick={() => (ctl.panelOpen = !ctl.panelOpen)} aria-expanded={ctl.panelOpen}>
    <span class="ico" class:busy={active.length > 0}><ArrowUpFromLine size={14} /><ArrowDownToLine size={14} /></span>
    <span class="title">Transfers</span>
    <span class="sum" class:bad={failed > 0 && active.length === 0}>{summary}</span>
    {#if active.length}<span class="mini" aria-hidden="true"><i style:width="{transfers.progress * 100}%"></i></span>{/if}
    <span class="chev" class:open={ctl.panelOpen}><ChevronUp size={15} /></span>
  </button>

  {#if ctl.panelOpen}
    <div class="body" transition:slide={{ duration: 200, easing: cubicOut }}>
      {#if list.length === 0}
        <p class="none">Files you upload or download show up here, with speed and time left. Drag files between the two sides, or drop them in from Explorer.</p>
      {:else}
        <div class="bar">
          <span>{plural(list.length, 'item')}</span>
          {#if transfers.finished.length}
            <button type="button" class="clear" onclick={() => act(transfers.clearFinished())}><Trash2 size={12} /> Clear finished</button>
          {/if}
        </div>
        <ul>
          {#each list as t (t.id)}
            <li class={t.state}>
              <span class="dir" class:up={t.direction === 'upload'}>
                {#if t.state === 'completed'}<Check size={14} strokeWidth={3} />
                {:else if t.state === 'failed'}<TriangleAlert size={14} />
                {:else if t.direction === 'upload'}<ArrowUpFromLine size={14} />
                {:else}<ArrowDownToLine size={14} />{/if}
              </span>
              <div class="main">
                <div class="top">
                  <span class="t">{t.title}</span>
                  {#if t.state === 'running' || t.state === 'paused'}<span class="pct">{Math.round(percent(t))}%</span>{/if}
                </div>
                {#if t.state === 'running' || t.state === 'paused' || t.state === 'queued'}
                  <div class="track" class:paused={t.state === 'paused'}><i style:width="{percent(t)}%"></i></div>
                {/if}
                <div class="sub">
                  <span class="cur" title={line(t)}>{line(t)}</span>
                  {#if t.state === 'running'}<span class="spd">{formatSpeed(t.speedBps)} {eta(t) ? '· ' + eta(t) : ''}</span>{/if}
                </div>
              </div>
              <div class="acts">
                {#if t.state === 'running' || t.state === 'queued'}
                  <button type="button" aria-label="Pause" use:tooltip={'Pause'} onclick={() => act(transfers.pause(t.id))}><Pause size={14} /></button>
                {:else if t.state === 'paused'}
                  <button type="button" aria-label="Resume" use:tooltip={'Resume'} onclick={() => act(transfers.resume(t.id))}><Play size={14} /></button>
                {/if}
                {#if t.state === 'running' || t.state === 'queued' || t.state === 'paused'}
                  <button type="button" aria-label="Cancel" use:tooltip={'Cancel'} onclick={() => act(transfers.cancel(t.id))}><X size={14} /></button>
                {:else}
                  {#if t.state === 'completed' && t.reveal}
                    <button type="button" aria-label="Show in folder" use:tooltip={'Show in folder'} onclick={() => t.reveal && act(api.app.openPath(t.reveal))}><FolderOpen size={14} /></button>
                  {/if}
                  <button type="button" aria-label="Remove from list" use:tooltip={'Remove from list'} onclick={() => act(transfers.dismiss(t.id))}><X size={14} /></button>
                {/if}
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}
</section>

<style>
  .panel {
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    overflow: hidden;
  }
  .head {
    position: relative;
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    height: 38px;
    padding: 0 14px;
    text-align: left;
  }
  .head:hover {
    background: var(--bg-hover);
  }
  .ico {
    display: inline-flex;
    gap: 1px;
    color: var(--text-faint);
  }
  .ico.busy {
    color: var(--accent);
  }
  .title {
    font-weight: 650;
  }
  .sum {
    flex: 1;
    color: var(--text-dim);
    font-size: var(--fs-sm);
    font-variant-numeric: tabular-nums;
  }
  .sum.bad {
    color: var(--danger);
  }
  .mini {
    width: 120px;
    height: 4px;
    border-radius: 2px;
    background: var(--bg-active);
    overflow: hidden;
  }
  .mini i,
  .track i {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--grad-accent);
    transition: width 250ms linear;
  }
  .chev {
    display: inline-flex;
    color: var(--text-faint);
    transform: rotate(180deg);
    transition: transform var(--dur) var(--ease-out);
  }
  .chev.open {
    transform: rotate(0deg);
  }
  .body {
    max-height: 240px;
    overflow-y: auto;
    border-top: 1px solid var(--border);
  }
  .none {
    padding: 22px 24px;
    color: var(--text-faint);
    font-size: var(--fs-sm);
    line-height: 1.55;
    text-align: center;
  }
  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 14px;
    font-size: var(--fs-xs);
    color: var(--text-faint);
  }
  .clear {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--text-dim);
  }
  .clear:hover {
    color: var(--text);
  }
  ul {
    margin: 0;
    padding: 0 6px 8px;
    list-style: none;
  }
  li {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px;
    border-radius: var(--r-md);
  }
  li:hover {
    background: var(--bg-hover);
  }
  .dir {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 28px;
    height: 28px;
    border-radius: 9px;
    background: var(--accent-soft);
    color: var(--accent);
  }
  li.completed .dir {
    background: color-mix(in srgb, var(--success) 16%, transparent);
    color: var(--success);
  }
  li.failed .dir {
    background: color-mix(in srgb, var(--danger) 16%, transparent);
    color: var(--danger);
  }
  li.paused .dir,
  li.cancelled .dir {
    background: color-mix(in srgb, var(--warning) 16%, transparent);
    color: var(--warning);
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
    justify-content: space-between;
    gap: 10px;
  }
  .t {
    font-weight: 560;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pct {
    color: var(--text-dim);
    font-size: var(--fs-sm);
    font-variant-numeric: tabular-nums;
  }
  .track {
    height: 4px;
    border-radius: 2px;
    background: var(--bg-active);
    overflow: hidden;
  }
  .track.paused i {
    background: var(--warning);
  }
  .sub {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    font-size: var(--fs-xs);
    color: var(--text-faint);
  }
  .cur {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  li.failed .cur {
    color: var(--danger);
  }
  .spd {
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }
  .acts {
    display: flex;
    gap: 2px;
  }
  .acts button {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: var(--r-sm);
    color: var(--text-dim);
  }
  .acts button:hover {
    background: var(--bg-active);
    color: var(--text);
  }
</style>
