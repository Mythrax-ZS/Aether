<script lang="ts">
  import { ArrowRight, Plus, Trash2 } from '@lucide/svelte';
  import type { ForwardKind, ForwardRule } from '$lib/bindings';
  import { newForwardRule } from '$lib/util/forwards';
  import Button from '$lib/components/ui/Button.svelte';
  import Input from '$lib/components/ui/Input.svelte';
  import IconButton from '$lib/components/ui/IconButton.svelte';
  import Segmented from '$lib/components/ui/Segmented.svelte';
  import Switch from '$lib/components/ui/Switch.svelte';

  type Props = {
    rules: ForwardRule[];
    /** Hide the "start automatically" switch (live forwards are started by hand). */
    live?: boolean;
    onadd?: (rule: ForwardRule) => void;
  };

  let { rules = $bindable([]), live = false, onadd }: Props = $props();

  const KINDS = [
    { value: 'local', label: 'Local' },
    { value: 'remote', label: 'Remote' },
    { value: 'dynamic', label: 'SOCKS' },
  ];

  function explain(r: ForwardRule): string {
    const bind = `${r.bindHost || '127.0.0.1'}:${r.bindPort}`;
    const dest = `${r.destHost}:${r.destPort}`;
    if (r.kind === 'local') return `Connections to ${bind} on this PC travel through the server to ${dest}.`;
    if (r.kind === 'remote') return `Connections to ${bind} on the server come back through to ${dest} from this PC.`;
    return `A SOCKS5 proxy on ${bind}: apps pointed at it browse as if they were on the server.`;
  }

  function setKind(r: ForwardRule, kind: string) {
    r.kind = kind as ForwardKind;
    if (kind === 'dynamic' && r.bindPort === 8080) r.bindPort = 1080;
    if (kind === 'remote' && r.bindHost === '127.0.0.1') r.bindHost = 'localhost';
  }

  const port = (v: string | number) => Math.max(0, Math.min(65535, Math.floor(Number(v) || 0)));

  function add() {
    const rule = newForwardRule();
    rules = [...rules, rule];
    onadd?.(rule);
  }
</script>

<div class="fstack">
  {#if rules.length === 0}
    <div class="flist-empty bordered">No port forwards yet.</div>
  {/if}
  {#each rules as rule, i (rule.id)}
    <div class="rule">
      <div class="top">
        <Segmented size="sm" value={rule.kind} options={KINDS} label="Forward type" onchange={(v) => setKind(rule, v)} />
        <div class="grow">
          <Input size="sm" bind:value={rule.label} placeholder="Label (optional) — e.g. Postgres" />
        </div>
        {#if !live}<Switch bind:checked={rule.autoStart} label="Auto" />{/if}
        <IconButton label="Remove forward" size={28} danger onclick={() => (rules = rules.filter((_, k) => k !== i))}><Trash2 size={15} /></IconButton>
      </div>
      <div class="ends">
        <div class="end">
          <span class="cap">{rule.kind === 'remote' ? 'Server listens on' : 'Listen on'}</span>
          <div class="pair">
            <Input size="sm" mono bind:value={rule.bindHost} placeholder="127.0.0.1" />
            <Input size="sm" mono type="number" min={0} max={65535} value={rule.bindPort} oninput={(e) => (rule.bindPort = port((e.currentTarget as HTMLInputElement).value))} />
          </div>
        </div>
        {#if rule.kind !== 'dynamic'}
          <span class="arrow"><ArrowRight size={16} /></span>
          <div class="end">
            <span class="cap">{rule.kind === 'remote' ? 'Forward to (from this PC)' : 'Forward to (from the server)'}</span>
            <div class="pair">
              <Input size="sm" mono bind:value={rule.destHost} placeholder="localhost" />
              <Input size="sm" mono type="number" min={1} max={65535} value={rule.destPort} oninput={(e) => (rule.destPort = port((e.currentTarget as HTMLInputElement).value))} />
            </div>
          </div>
        {/if}
      </div>
      <p class="why">{explain(rule)}</p>
    </div>
  {/each}
  <div>
    <Button size="sm" onclick={add}>{#snippet icon()}<Plus size={14} />{/snippet}Add forward</Button>
  </div>
</div>

<style>
  .rule {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
  }
  .top {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .ends {
    display: flex;
    align-items: flex-end;
    gap: 10px;
  }
  .end {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .cap {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-faint);
  }
  .pair {
    display: grid;
    grid-template-columns: 1fr 86px;
    gap: 6px;
  }
  .arrow {
    display: grid;
    place-items: center;
    height: 28px;
    color: var(--accent);
  }
  .why {
    font-size: var(--fs-xs);
    color: var(--text-faint);
    line-height: 1.5;
  }
  .bordered {
    border: 1px dashed var(--border-strong);
    border-radius: var(--r-md);
  }
</style>
