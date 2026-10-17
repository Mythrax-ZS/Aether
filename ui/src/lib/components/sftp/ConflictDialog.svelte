<script lang="ts">
  import { CopyPlus, FileWarning, Replace, SkipForward } from '@lucide/svelte';
  import type { Component } from 'svelte';
  import type { ConflictPolicy } from '$lib/bindings';
  import type { SftpController } from '$lib/sftp/controller.svelte';
  import { settings } from '$lib/stores/settings.svelte';
  import { plural } from '$lib/util/format';
  import Button from '$lib/components/ui/Button.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';

  let { ctl }: { ctl: SftpController } = $props();

  let choice = $state<ConflictPolicy>('keep-both');
  let remember = $state(false);

  // Each question starts with the safe answer.
  $effect(() => {
    if (ctl.conflict) {
      choice = 'keep-both';
      remember = false;
    }
  });

  const OPTIONS: { value: ConflictPolicy; title: string; desc: string; icon: Component<{ size?: number }> }[] = [
    { value: 'keep-both', title: 'Keep both', desc: 'The new files get a number added to their name.', icon: CopyPlus },
    { value: 'overwrite', title: 'Replace', desc: 'The existing files are overwritten.', icon: Replace },
    { value: 'skip', title: 'Skip', desc: 'Leave the existing files alone and transfer the rest.', icon: SkipForward },
  ];

  function answer(p: ConflictPolicy | null) {
    if (p && remember) settings.patch({ sftp: { overwrite: p === 'keep-both' ? 'rename' : p } });
    ctl.conflict?.resolve(p);
  }
</script>

{#if ctl.conflict}
  {@const c = ctl.conflict}
  <Modal title="Some files already exist" subtitle="{plural(c.names.length, 'item')} in the {c.direction === 'upload' ? 'server folder' : 'folder on this PC'}" width={480} onclose={() => answer(null)}>
    {#snippet icon()}<FileWarning size={18} />{/snippet}
    <ul class="names">
      {#each c.names.slice(0, 5) as n (n)}<li>{n}</li>{/each}
      {#if c.names.length > 5}<li class="more">…and {c.names.length - 5} more</li>{/if}
    </ul>
    <div class="options" role="radiogroup" aria-label="What to do">
      {#each OPTIONS as o (o.value)}
        {@const Icon = o.icon}
        <button type="button" role="radio" aria-checked={choice === o.value} class:on={choice === o.value} class:danger={o.value === 'overwrite'} onclick={() => (choice = o.value)}>
          <span class="oi"><Icon size={17} /></span>
          <span class="ot"><b>{o.title}</b><small>{o.desc}</small></span>
          <span class="dot"></span>
        </button>
      {/each}
    </div>
    <label class="check"><input type="checkbox" bind:checked={remember} /> <span>Always do this, without asking</span></label>
    {#snippet footer()}
      <Button variant="ghost" onclick={() => answer(null)}>Cancel</Button>
      <Button variant="primary" autofocus onclick={() => answer(choice)}>Continue</Button>
    {/snippet}
  </Modal>
{/if}

<style>
  .names {
    margin: 0 0 16px;
    padding: 10px 14px;
    list-style: none;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    font: 12px/1.7 var(--font-mono);
    color: var(--text-dim);
  }
  .names li {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .names .more {
    color: var(--text-faint);
    font-family: var(--font-ui);
  }
  .options {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .options button {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 11px 13px;
    text-align: left;
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    transition:
      border-color var(--dur-fast),
      background var(--dur-fast);
  }
  .options button:hover {
    background: var(--bg-hover);
  }
  .options button.on {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .options button.on.danger {
    border-color: var(--danger);
    background: color-mix(in srgb, var(--danger) 10%, transparent);
  }
  .oi {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: 10px;
    background: var(--bg-hover);
    color: var(--text-dim);
  }
  .on .oi {
    color: var(--accent);
  }
  .on.danger .oi {
    color: var(--danger);
  }
  .ot {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .ot small {
    color: var(--text-faint);
    font-size: var(--fs-sm);
  }
  .dot {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    border: 2px solid var(--border-strong);
  }
  .on .dot {
    border-color: var(--accent);
    background: radial-gradient(circle, var(--accent) 38%, transparent 42%);
  }
  .check {
    display: flex;
    align-items: center;
    gap: 9px;
    margin-top: 16px;
    color: var(--text-dim);
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .check input {
    accent-color: var(--accent);
    width: 15px;
    height: 15px;
  }
</style>
