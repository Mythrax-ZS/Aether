<script lang="ts">
  import { ArrowDown, ArrowUp, Network, Play, Plus, Square, TriangleAlert } from '@lucide/svelte';
  import { api } from '$lib/api';
  import type { ConnectionId, ForwardRule } from '$lib/bindings';
  import { sessions } from '$lib/stores/sessions.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import { workspace } from '$lib/stores/workspace.svelte';
  import { formatBytes } from '$lib/util/format';
  import { describeForward, newForwardRule, validateForward } from '$lib/util/forwards';
  import Badge from '$lib/components/ui/Badge.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import ForwardRuleEditor from './ForwardRuleEditor.svelte';

  let { connectionId }: { connectionId: ConnectionId } = $props();

  const view = $derived(sessions.view(connectionId));
  const live = $derived(view?.forwards ?? []);
  const connected = $derived(view?.state.type === 'connected');
  const hostName = $derived(
    workspace.allLeaves().find((l) => l.connectionId === connectionId)?.host?.label ??
      (view?.state.type === 'connected' ? view.state.info.target : 'this connection'),
  );

  // The form edits a throwaway list; "Start" launches its rules on the live connection.
  let draft = $state<ForwardRule[]>([newForwardRule()]);
  let starting = $state(false);
  let adding = $state(false);

  async function start() {
    starting = true;
    let started = 0;
    try {
      for (const rule of draft) {
        const problem = validateForward(rule);
        if (problem) {
          ui.toast({ kind: 'warning', title: 'Check this forward', message: problem });
          return;
        }
        await api.sessions.startForward(connectionId, rule);
        started++;
      }
      draft = [newForwardRule()];
      adding = false;
      ui.success(started === 1 ? 'Forward started' : `${started} forwards started`);
    } catch (e) {
      ui.error(e, 'Couldn’t start the forward');
    } finally {
      starting = false;
    }
  }

  async function stop(id: string) {
    try {
      await api.sessions.stopForward(connectionId, id);
    } catch (e) {
      ui.error(e);
    }
  }
</script>

<Modal title="Port forwarding" subtitle={hostName} width={720} onclose={() => ui.closeDialog()}>
  {#snippet icon()}<Network size={18} />{/snippet}

  {#if !connected}
    <p class="warn">This connection isn’t active. Forwards can only be started while it’s connected.</p>
  {/if}

  <section class="fsec">
    <h3>Running</h3>
    {#if live.length === 0}
      <div class="flist-empty empty">Nothing is forwarded right now.</div>
    {:else}
      <div class="flist">
        {#each live as f (f.id)}
          <div class="live">
            <span class="pulse" aria-hidden="true"></span>
            <div class="main">
              <div class="top">
                <span class="name">{f.rule.label || describeForward(f.rule)}</span>
                <Badge size="sm" tone={f.rule.kind === 'local' ? 'accent' : f.rule.kind === 'remote' ? 'violet' : 'success'}>
                  {f.rule.kind === 'dynamic' ? 'SOCKS' : f.rule.kind}
                </Badge>
              </div>
              <div class="sub">
                {#if f.rule.label}{describeForward(f.rule)} ·{/if}
                {#if f.boundPort && f.boundPort !== f.rule.bindPort}bound to port {f.boundPort} ·{/if}
                {f.activeConnections} open, {f.totalConnections} total ·
                <ArrowUp size={10} /> {formatBytes(f.bytesUp)} <ArrowDown size={10} /> {formatBytes(f.bytesDown)}
              </div>
              {#if f.lastError}
                <p class="trouble"><TriangleAlert size={12} /> {f.lastError}{#if f.refused > 1} <span>({f.refused} connections refused)</span>{/if}</p>
              {/if}
            </div>
            <Button size="sm" variant="danger" onclick={() => stop(f.id)}>{#snippet icon()}<Square size={11} fill="currentColor" />{/snippet}Stop</Button>
          </div>
        {/each}
      </div>
    {/if}
  </section>

  <section class="fsec">
    <h3>New forward</h3>
    {#if adding}
      <ForwardRuleEditor bind:rules={draft} live />
      <div class="frow">
        <Button variant="primary" loading={starting} disabled={!connected || draft.length === 0} onclick={start}>{#snippet icon()}<Play size={13} fill="currentColor" />{/snippet}Start</Button>
        <Button variant="ghost" onclick={() => { adding = false; draft = [newForwardRule()]; }}>Cancel</Button>
      </div>
    {:else}
      <div><Button onclick={() => (adding = true)} disabled={!connected}>{#snippet icon()}<Plus size={14} />{/snippet}Add a forward</Button></div>
      <p class="fdesc">Forwards started here last until the connection ends. To keep one permanently, add it to the host under <b>Port forwarding</b>.</p>
    {/if}
  </section>
</Modal>

<style>
  .warn {
    margin-bottom: 14px;
    padding: 10px 12px;
    border-radius: var(--r-md);
    background: color-mix(in srgb, var(--warning) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--warning) 35%, transparent);
    color: var(--warning);
    font-size: var(--fs-sm);
  }
  .empty {
    border: 1px dashed var(--border-strong);
    border-radius: var(--r-md);
  }
  .live {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 14px;
  }
  .pulse {
    flex-shrink: 0;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--success);
    box-shadow: 0 0 10px var(--success);
    animation: pulse-dot 1.8s var(--ease-in-out) infinite;
  }
  .main {
    flex: 1;
    min-width: 0;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .name {
    font-weight: 600;
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
  }
  .sub {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    margin-top: 3px;
    font-size: var(--fs-xs);
    color: var(--text-faint);
  }
  .trouble {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin-top: 6px;
    font-size: var(--fs-sm);
    line-height: 1.45;
    color: var(--warning);
  }
  .trouble :global(svg) {
    margin-top: 3px;
  }
  .trouble span {
    color: var(--text-faint);
  }
</style>
