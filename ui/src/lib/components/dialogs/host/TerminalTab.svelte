<script lang="ts">
  import { Plus, Trash2 } from '@lucide/svelte';
  import type { Host } from '$lib/bindings';
  import { settings } from '$lib/stores/settings.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import IconButton from '$lib/components/ui/IconButton.svelte';
  import Input from '$lib/components/ui/Input.svelte';
  import Select from '$lib/components/ui/Select.svelte';

  let { host = $bindable() }: { host: Host } = $props();

  const themeOptions = $derived([
    { value: '', label: 'Same as the rest of Aether' },
    ...settings.themes.map((t) => ({ value: t.id, label: t.name, group: t.kind === 'dark' ? 'Dark' : 'Light' })),
  ]);

  const size = (v: string) => {
    const n = Number(v);
    return Number.isFinite(n) && n > 0 ? Math.min(40, Math.max(7, n)) : null;
  };
</script>

<div class="fsec">
  <h3>On login</h3>
  <Input
    bind:value={host.startup.command}
    label="Run this command"
    mono
    placeholder="tmux new -As main"
    hint="Typed into the shell every time it opens, also after an automatic reconnect — so make it safe to repeat. Handy for tmux, a directory, or a sudo shell."
  />
  <div>
    <span class="flabel">Environment variables</span>
    <p class="fhint tight">Requested from the server; it only accepts the ones its <code>AcceptEnv</code> allows.</p>
    {#if host.startup.env.length}
      <div class="flist envs">
        {#each host.startup.env as v, i (i)}
          <div class="env">
            <Input size="sm" mono bind:value={v.name} placeholder="NAME" />
            <Input size="sm" mono bind:value={v.value} placeholder="value" />
            <IconButton label="Remove" size={26} danger onclick={() => (host.startup.env = host.startup.env.filter((_, k) => k !== i))}><Trash2 size={14} /></IconButton>
          </div>
        {/each}
      </div>
    {/if}
    <div class="add">
      <Button size="sm" onclick={() => (host.startup.env = [...host.startup.env, { name: '', value: '' }])}>{#snippet icon()}<Plus size={14} />{/snippet}Add variable</Button>
    </div>
  </div>
</div>

<div class="fsec">
  <h3>Terminal</h3>
  <Input bind:value={host.options.term} label="Terminal type ($TERM)" mono placeholder="xterm-256color" hint="Older systems may need plain “xterm” or “vt100”." />
  <p class="fdesc">Give this server its own look — a red theme for production is a classic.</p>
  <Select
    value={host.terminal.themeId ?? ''}
    options={themeOptions}
    label="Colour theme"
    onchange={(v) => (host.terminal.themeId = v || null)}
  />
  <div class="fgrid wide-left">
    <Input
      value={host.terminal.fontFamily ?? ''}
      label="Font"
      mono
      placeholder={settings.value.terminal.fontFamily.split(',')[0].replace(/['"]/g, '')}
      oninput={(e) => (host.terminal.fontFamily = (e.currentTarget as HTMLInputElement).value.trim() || null)}
    />
    <Input
      value={host.terminal.fontSize ?? ''}
      label="Size"
      type="number"
      min={7}
      max={40}
      step={0.5}
      mono
      placeholder={String(settings.value.terminal.fontSize)}
      oninput={(e) => (host.terminal.fontSize = size((e.currentTarget as HTMLInputElement).value))}
    />
  </div>
</div>

<style>
  .env {
    display: grid;
    grid-template-columns: 1fr 1.4fr auto;
    gap: 8px;
    align-items: center;
    padding: 7px 8px;
  }
  .envs {
    margin-top: 8px;
  }
  .add {
    margin-top: 10px;
  }
  .tight {
    margin: -2px 0 0;
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
</style>
