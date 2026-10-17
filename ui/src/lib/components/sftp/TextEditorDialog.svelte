<script lang="ts">
  import { FileEdit, Save } from '@lucide/svelte';
  import { onMount } from 'svelte';
  import { api, toErrorInfo } from '$lib/api';
  import type { ConnectionId, FileVersion, LineEnding } from '$lib/bindings';
  import type { FileItem } from '$lib/sftp/provider';
  import { ui } from '$lib/stores/ui.svelte';
  import { formatBytes } from '$lib/util/format';
  import Button from '$lib/components/ui/Button.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';
  import Spinner from '$lib/components/ui/Spinner.svelte';

  type Props = { connectionId: ConnectionId; item: FileItem; onclose: () => void; onsaved: () => void };
  let { connectionId, item, onclose, onsaved }: Props = $props();

  let text = $state('');
  let original = $state('');
  let loading = $state(true);
  let truncated = $state(false);
  let saving = $state(false);
  let failed = $state<string | null>(null);
  let area = $state<HTMLTextAreaElement>();
  let cursor = $state({ line: 1, col: 1 });
  // What the file was like when it was opened: put back on save, so editing a line doesn't rewrite every other one.
  let lineEnding = $state<LineEnding>('lf');
  let mixedEndings = $state(false);
  let bom = $state(false);
  let version = $state<FileVersion | null>(null);

  const dirty = $derived(!loading && !truncated && text !== original);

  onMount(async () => {
    try {
      const file = await api.sftp.readText(connectionId, item.path);
      text = original = file.text;
      truncated = file.truncated;
      lineEnding = file.lineEnding;
      mixedEndings = file.mixedLineEndings;
      bom = file.bom;
      version = file.version;
    } catch (e) {
      failed = toErrorInfo(e).message || 'The file couldn’t be opened.';
    } finally {
      loading = false;
    }
  });

  async function save() {
    if (!dirty || saving) return;
    saving = true;
    // The box stays editable while this goes on, so what is saved is what it held now — and only that counts as saved.
    const sent = text;
    // `expected` makes the save refuse if the file changed on the server after it was opened; the person can then
    // choose to overwrite anyway (which sends `null`).
    let expected = version;
    try {
      for (;;) {
        try {
          version = await api.sftp.writeText(connectionId, { path: item.path, text: sent, lineEnding, bom, expected });
          break;
        } catch (e) {
          const info = toErrorInfo(e);
          if (info.code !== 'conflict' || expected === null) throw e;
          const overwrite = await ui.confirm({
            title: 'The file changed on the server',
            message: `“${item.name}” was modified after you opened it. Saving now replaces that newer version with yours.`,
            confirmLabel: 'Save anyway',
            danger: true,
          });
          if (!overwrite) return;
          expected = null;
        }
      }
      original = sent;
      mixedEndings = false;
      onsaved();
      ui.success('Saved', item.name);
    } catch (e) {
      ui.error(e, 'Couldn’t save the file');
    } finally {
      saving = false;
    }
  }

  async function close() {
    if (dirty) {
      const ok = await ui.confirm({ title: 'Discard your changes?', message: `“${item.name}” has edits that haven’t been saved.`, confirmLabel: 'Discard', danger: true });
      if (!ok) return;
    }
    onclose();
  }

  function onKey(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
      e.preventDefault();
      e.stopPropagation();
      void save();
    } else if (e.key === 'Tab' && !e.shiftKey && !e.ctrlKey && area) {
      // A tab character, not a jump to the next control.
      e.preventDefault();
      const { selectionStart: s, selectionEnd: en } = area;
      text = text.slice(0, s) + '\t' + text.slice(en);
      queueMicrotask(() => area?.setSelectionRange(s + 1, s + 1));
    }
  }

  function track() {
    if (!area) return;
    const before = text.slice(0, area.selectionStart);
    const line = before.split('\n').length;
    cursor = { line, col: area.selectionStart - before.lastIndexOf('\n') };
  }
</script>

<svelte:window onkeydown={(e) => (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's' && e.preventDefault()} />

<Modal title={item.name} subtitle={item.path} width={860} padded={false} onclose={close}>
  {#snippet icon()}<FileEdit size={18} />{/snippet}
  {#snippet headerExtra()}
    {#if dirty}<span class="dirty">Unsaved changes</span>{/if}
  {/snippet}

  <div class="editor">
    {#if loading}
      <div class="center"><Spinner size={22} /><span>Opening…</span></div>
    {:else if failed}
      <div class="center bad">{failed}</div>
    {:else}
      {#if truncated}<div class="banner">This file is larger than the editor handles ({formatBytes(item.size)}), so it’s shown read-only. Use a terminal editor like <code>vim</code> or <code>nano</code> for big files.</div>{/if}
      {#if mixedEndings && !truncated}<div class="banner">This file mixes Windows and Unix line endings. Saving makes them all {lineEnding === 'crlf' ? 'Windows (CRLF)' : 'Unix (LF)'}.</div>{/if}
      <textarea
        bind:this={area}
        bind:value={text}
        readonly={truncated}
        spellcheck="false"
        wrap="off"
        aria-label="File contents"
        onkeydown={onKey}
        onkeyup={track}
        onclick={track}
      ></textarea>
    {/if}
  </div>

  {#snippet footer()}
    <span class="pos">Ln {cursor.line}, Col {cursor.col} · {text.length.toLocaleString()} characters · {lineEnding === 'crlf' ? 'CRLF' : 'LF'}{bom ? ' · BOM' : ''}</span>
    <Button variant="ghost" onclick={close}>Close</Button>
    <Button variant="primary" loading={saving} disabled={!dirty} onclick={save}>{#snippet icon()}<Save size={14} />{/snippet}Save</Button>
  {/snippet}
</Modal>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    height: min(560px, 64vh);
  }
  textarea {
    flex: 1;
    min-height: 0;
    padding: 14px 18px;
    resize: none;
    background: var(--bg-terminal);
    border: 0;
    outline: none;
    color: var(--text);
    font: 13px/1.6 var(--font-mono);
    tab-size: 4;
    user-select: text;
    cursor: text;
  }
  .center {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: var(--text-dim);
  }
  .center.bad {
    color: var(--danger);
  }
  .banner {
    padding: 9px 18px;
    background: color-mix(in srgb, var(--warning) 12%, transparent);
    border-bottom: 1px solid color-mix(in srgb, var(--warning) 30%, transparent);
    color: var(--warning);
    font-size: var(--fs-sm);
  }
  code {
    font-family: var(--font-mono);
  }
  .dirty {
    padding: 2px 9px;
    border-radius: var(--r-pill);
    background: color-mix(in srgb, var(--warning) 16%, transparent);
    color: var(--warning);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .pos {
    margin-right: auto;
    color: var(--text-faint);
    font-size: var(--fs-sm);
    font-variant-numeric: tabular-nums;
  }
</style>
