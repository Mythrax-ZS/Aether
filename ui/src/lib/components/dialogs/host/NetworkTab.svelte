<script lang="ts">
  import { ArrowDown, ArrowUp, Plus, Server, Trash2 } from '@lucide/svelte';
  import type { Host, JumpRef } from '$lib/bindings';
  import { hosts } from '$lib/stores/hosts.svelte';
  import { ui, type MenuItem } from '$lib/stores/ui.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import IconButton from '$lib/components/ui/IconButton.svelte';
  import Input from '$lib/components/ui/Input.svelte';
  import Switch from '$lib/components/ui/Switch.svelte';
  import Avatar from '$lib/components/ui/Avatar.svelte';

  let { host = $bindable() }: { host: Host } = $props();

  const num = (v: string | number, lo: number, hi: number) => Math.min(hi, Math.max(lo, Math.floor(Number(v) || 0)));

  function move(i: number, d: -1 | 1) {
    const j = i + d;
    if (j < 0 || j >= host.jump.length) return;
    const list = [...host.jump];
    [list[i], list[j]] = [list[j], list[i]];
    host.jump = list;
  }

  function addSaved(e: MouseEvent) {
    const used = new Set(host.jump.flatMap((j) => (j.type === 'host' ? [j.hostId] : [])));
    const candidates = hosts.data.hosts.filter((h) => h.id !== host.id && !used.has(h.id));
    const items: MenuItem[] = candidates.length
      ? candidates.map((h) => ({ label: h.label || h.hostname, icon: Server, run: () => (host.jump = [...host.jump, { type: 'host', hostId: h.id }]) }))
      : [{ label: 'No other saved hosts', disabled: true }];
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    ui.showMenu({ clientX: r.left, clientY: r.bottom + 6 }, items);
  }

  const nameOf = (j: JumpRef) => (j.type === 'host' ? (hosts.get(j.hostId)?.label ?? 'Missing host') : j.spec);
</script>

<div class="fsec">
  <h3>Jump through other servers</h3>
  <p class="fdesc">Reach a server that’s only visible from a bastion host. Aether connects through each one in order (ProxyJump).</p>
  {#if host.jump.length}
    <div class="flist">
      {#each host.jump as j, i (i)}
        <div class="hop">
          <span class="n">{i + 1}</span>
          {#if j.type === 'host'}
            {@const h = hosts.get(j.hostId)}
            <Avatar name={nameOf(j)} color={h?.color ?? null} size={24} />
            <span class="name">
              {nameOf(j)}
              {#if h}<small>{h.username ? h.username + '@' : ''}{h.hostname}</small>{:else}<small class="bad">This host was deleted</small>{/if}
            </span>
          {:else}
            <div class="grow"><Input size="sm" mono bind:value={j.spec} placeholder="user@bastion.example.com:22" /></div>
          {/if}
          <IconButton label="Move up" size={24} disabled={i === 0} onclick={() => move(i, -1)}><ArrowUp size={14} /></IconButton>
          <IconButton label="Move down" size={24} disabled={i === host.jump.length - 1} onclick={() => move(i, 1)}><ArrowDown size={14} /></IconButton>
          <IconButton label="Remove" size={24} danger onclick={() => (host.jump = host.jump.filter((_, k) => k !== i))}><Trash2 size={14} /></IconButton>
        </div>
      {/each}
    </div>
  {/if}
  <div class="frow">
    <Button size="sm" onclick={addSaved}>{#snippet icon()}<Plus size={14} />{/snippet}Add a saved host</Button>
    <Button size="sm" onclick={() => (host.jump = [...host.jump, { type: 'adhoc', spec: '' }])}>{#snippet icon()}<Plus size={14} />{/snippet}Add an address</Button>
  </div>
  <Input
    bind:value={host.options.proxyCommand}
    label="Proxy command"
    mono
    placeholder="ssh -W %h:%p bastion"
    hint="Advanced. Runs this command on this PC instead of connecting directly; %h and %p become the host and port. Windows asks you to confirm it whenever it is added or changed."
  />
</div>

<div class="fsec">
  <h3>Connection</h3>
  <div class="fgrid three">
    <Input value={host.options.connectTimeoutSecs} label="Timeout (s)" type="number" min={1} max={300} mono oninput={(e) => (host.options.connectTimeoutSecs = num((e.currentTarget as HTMLInputElement).value, 1, 300))} />
    <Input value={host.options.keepaliveIntervalSecs} label="Keepalive (s)" type="number" min={0} max={3600} mono hint="0 turns it off." oninput={(e) => (host.options.keepaliveIntervalSecs = num((e.currentTarget as HTMLInputElement).value, 0, 3600))} />
    <Input value={host.options.keepaliveMax} label="Missed keepalives" type="number" min={1} max={20} mono oninput={(e) => (host.options.keepaliveMax = num((e.currentTarget as HTMLInputElement).value, 1, 20))} />
  </div>
  <div class="fstack tight">
    <Switch bind:checked={host.options.autoReconnect} label="Reconnect automatically" description="If the connection drops, try again and restore your tunnels." />
    <Switch bind:checked={host.options.agentForwarding} label="Forward my SSH agent" description="Lets this server use your agent’s keys to reach other servers. Only enable it for servers you trust." />
    <Switch
      bind:checked={host.options.x11Forwarding}
      label="Show the server’s graphical programs here (X11)"
      description="Needs an X server on this PC — VcXsrv, X410 or Xming — listening on the address set in Settings → SSH. Programs on the server get full access to that X server, so only use it with servers you trust."
    />
    <Switch bind:checked={host.options.compression} label="Compress traffic" description="Helps on slow links; costs a little CPU." />
    <Switch bind:checked={host.options.legacyAlgorithms} label="Allow legacy algorithms" description="For old routers and embedded devices that only speak ssh-rsa, CBC ciphers or SHA-1. Weaker — leave off otherwise." />
  </div>
</div>

<style>
  .hop {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
  }
  .n {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--bg-active);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 700;
  }
  .name {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    line-height: 1.25;
    font-weight: 560;
  }
  .name small {
    color: var(--text-faint);
    font-weight: 400;
    font-size: var(--fs-xs);
  }
  .name small.bad {
    color: var(--danger);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .tight {
    gap: 16px;
  }
</style>
