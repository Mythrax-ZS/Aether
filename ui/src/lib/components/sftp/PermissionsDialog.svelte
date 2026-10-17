<script lang="ts">
  import { Shield } from '@lucide/svelte';
  import { untrack } from 'svelte';
  import { api } from '$lib/api';
  import type { ConnectionId } from '$lib/bindings';
  import type { FileItem } from '$lib/sftp/provider';
  import { ui } from '$lib/stores/ui.svelte';
  import { formatMode, modeToOctal, parseOctal, plural } from '$lib/util/format';
  import Button from '$lib/components/ui/Button.svelte';
  import Input from '$lib/components/ui/Input.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import Switch from '$lib/components/ui/Switch.svelte';

  type Props = {
    connectionId: ConnectionId;
    items: FileItem[];
    onclose: () => void;
    ondone: () => void;
  };

  let { connectionId, items, onclose, ondone }: Props = $props();

  // The dialog is opened for one fixed selection.
  const targets = untrack(() => items);
  const single = targets.length === 1;
  // Start from the first item's mode (they usually match when you select several siblings).
  let mode = $state((targets[0]?.mode ?? 0o644) & 0o777);
  let octal = $state(modeToOctal((targets[0]?.mode ?? 0o644) & 0o777));
  let recursive = $state(false);
  let busy = $state(false);

  const hasDir = targets.some((i) => i.isDir);
  const bad = $derived(parseOctal(octal) === null);

  const WHO = [
    { label: 'Owner', shift: 6 },
    { label: 'Group', shift: 3 },
    { label: 'Everyone else', shift: 0 },
  ];
  const WHAT = [
    { label: 'Read', bit: 4 },
    { label: 'Write', bit: 2 },
    { label: 'Run', bit: 1 },
  ];

  const has = (shift: number, bit: number) => ((mode >> shift) & bit) !== 0;

  function flip(shift: number, bit: number) {
    mode = mode ^ (bit << shift);
    octal = modeToOctal(mode);
  }

  function typed() {
    const m = parseOctal(octal);
    if (m !== null) mode = m & 0o777;
  }

  async function apply() {
    busy = true;
    try {
      for (const item of items) {
        // Keep setuid/setgid/sticky as they were; the grid only edits the nine permission bits.
        const special = (item.mode ?? 0) & 0o7000;
        await api.sftp.chmod(connectionId, item.path, special | mode, recursive && item.isDir);
      }
      ondone();
      ui.success('Permissions updated');
    } catch (e) {
      ui.error(e, 'Couldn’t change permissions');
    } finally {
      busy = false;
    }
  }
</script>

<Modal title="Permissions" subtitle={single ? items[0].name : plural(items.length, 'item')} width={460} {onclose}>
  {#snippet icon()}<Shield size={18} />{/snippet}
  <div class="grid" role="group" aria-label="Permissions">
    <span></span>
    {#each WHAT as w (w.bit)}<span class="h">{w.label}</span>{/each}
    {#each WHO as who (who.shift)}
      <span class="who">{who.label}</span>
      {#each WHAT as w (w.bit)}
        <button type="button" class="cell" class:on={has(who.shift, w.bit)} role="checkbox" aria-checked={has(who.shift, w.bit)} aria-label="{who.label} can {w.label.toLowerCase()}" onclick={() => flip(who.shift, w.bit)}>
          <span class="tick"></span>
        </button>
      {/each}
    {/each}
  </div>

  <div class="line">
    <div class="oct"><Input mono size="sm" bind:value={octal} label="Number" oninput={typed} error={bad ? 'Use 3 or 4 digits, 0–7' : null} /></div>
    <div class="preview">
      <span class="lbl">Reads as</span>
      <code>{formatMode(mode)}</code>
    </div>
  </div>

  {#if hasDir}
    <div class="rec"><Switch bind:checked={recursive} label="Apply to everything inside folders" description="Changes every file and folder below the selected folders too." /></div>
  {/if}

  {#snippet footer()}
    <Button variant="ghost" onclick={onclose}>Cancel</Button>
    <Button variant="primary" loading={busy} disabled={bad} onclick={apply}>Apply</Button>
  {/snippet}
</Modal>

<style>
  .grid {
    display: grid;
    grid-template-columns: 1.3fr repeat(3, 1fr);
    gap: 8px 10px;
    align-items: center;
    margin-bottom: 20px;
  }
  .h {
    text-align: center;
    font-size: var(--fs-xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
  .who {
    font-weight: 560;
  }
  .cell {
    display: grid;
    place-items: center;
    height: 36px;
    border-radius: var(--r-md);
    background: var(--bg-input);
    border: 1px solid var(--border);
    transition:
      background var(--dur-fast),
      border-color var(--dur-fast);
  }
  .cell:hover {
    border-color: var(--border-strong);
  }
  .cell.on {
    background: var(--accent-soft);
    border-color: var(--accent);
  }
  .tick {
    width: 14px;
    height: 14px;
    border-radius: 4px;
    border: 2px solid var(--border-strong);
    transition: all var(--dur-fast);
  }
  .on .tick {
    background: var(--accent);
    border-color: var(--accent);
    box-shadow: inset 0 0 0 2px var(--bg-surface);
  }
  .line {
    display: flex;
    gap: 18px;
    align-items: flex-end;
  }
  .oct {
    width: 130px;
  }
  .preview {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .lbl {
    font-size: var(--fs-sm);
    font-weight: 550;
    color: var(--text-dim);
  }
  code {
    display: block;
    padding: 6px 10px;
    border-radius: var(--r-sm);
    background: var(--bg-input);
    border: 1px solid var(--border);
    font: 13px var(--font-mono);
    color: var(--accent);
  }
  .rec {
    margin-top: 20px;
  }
</style>
