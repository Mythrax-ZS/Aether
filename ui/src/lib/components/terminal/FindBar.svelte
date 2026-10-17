<script lang="ts">
  import { ArrowDown, ArrowUp, CaseSensitive, Regex, WholeWord, X } from '@lucide/svelte';
  import { fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { tooltip } from '$lib/dom/tooltip';
  import type { TerminalController } from '$lib/terminal/controller';
  import { settings } from '$lib/stores/settings.svelte';

  type Props = {
    controller: TerminalController | undefined;
    index: number;
    count: number;
    onclose: () => void;
  };

  let { controller, index, count, onclose }: Props = $props();

  let query = $state('');
  let caseSensitive = $state(false);
  let regex = $state(false);
  let wholeWord = $state(false);
  let input = $state<HTMLInputElement>();

  const options = $derived({ caseSensitive, regex, wholeWord });
  const invalid = $derived.by(() => {
    if (!regex || !query) return false;
    try {
      new RegExp(query);
      return false;
    } catch {
      return true;
    }
  });

  // Start from the current selection, like most editors.
  $effect(() => {
    const sel = controller?.getSelection();
    if (sel && !sel.includes('\n') && sel.length < 120) query = sel;
    input?.focus();
    input?.select();
  });

  function run(forward: boolean) {
    if (!controller || invalid) return;
    controller.find(query, forward, options);
  }

  // Search as you type.
  $effect(() => {
    void options;
    if (!controller) return;
    if (!query) controller.clearSearch();
    else if (!invalid) controller.find(query, true, options);
  });

  function close() {
    controller?.clearSearch();
    onclose();
    controller?.focus();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      run(!e.shiftKey);
    }
  }

  const status = $derived(!query ? '' : invalid ? 'Bad pattern' : count === 0 ? 'No results' : index >= 0 ? `${index + 1} of ${count}${count >= settings.value.terminal.scrollback ? '+' : ''}` : `${count} found`);
</script>

<div class="find" role="search" in:fly={{ y: -10, duration: 170, easing: cubicOut }} out:fly={{ y: -8, duration: 110 }}>
  <input
    bind:this={input}
    bind:value={query}
    class:invalid
    type="text"
    placeholder="Find in terminal"
    aria-label="Find in terminal"
    spellcheck="false"
    autocomplete="off"
    onkeydown={onKey}
  />
  <span class="status" class:none={!!query && count === 0}>{status}</span>
  <div class="opts">
    <button type="button" class:on={caseSensitive} aria-pressed={caseSensitive} aria-label="Match case" onclick={() => (caseSensitive = !caseSensitive)} use:tooltip={'Match case'}><CaseSensitive size={16} /></button>
    <button type="button" class:on={wholeWord} aria-pressed={wholeWord} aria-label="Whole word" onclick={() => (wholeWord = !wholeWord)} use:tooltip={'Whole word'}><WholeWord size={16} /></button>
    <button type="button" class:on={regex} aria-pressed={regex} aria-label="Regular expression" onclick={() => (regex = !regex)} use:tooltip={'Regular expression'}><Regex size={16} /></button>
  </div>
  <span class="sep"></span>
  <button type="button" aria-label="Previous match" onclick={() => run(false)} use:tooltip={{ text: 'Previous', shortcut: 'Shift+Enter' }}><ArrowUp size={15} /></button>
  <button type="button" aria-label="Next match" onclick={() => run(true)} use:tooltip={{ text: 'Next', shortcut: 'Enter' }}><ArrowDown size={15} /></button>
  <button type="button" aria-label="Close search" onclick={close} use:tooltip={{ text: 'Close', shortcut: 'Esc' }}><X size={15} /></button>
</div>

<style>
  .find {
    position: absolute;
    top: 10px;
    right: 22px;
    z-index: 6;
    display: flex;
    align-items: center;
    gap: 2px;
    height: 38px;
    padding: 0 5px 0 4px;
    background: color-mix(in srgb, var(--bg-overlay) 96%, transparent);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-lg);
    box-shadow: var(--shadow-lg);
  }
  input {
    width: 190px;
    height: 28px;
    padding: 0 8px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
    outline: none;
    color: var(--text);
    font-size: var(--fs-md);
    user-select: text;
    cursor: text;
  }
  input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  input.invalid {
    border-color: var(--danger);
  }
  .status {
    min-width: 62px;
    padding: 0 6px;
    text-align: right;
    font-size: var(--fs-xs);
    color: var(--text-faint);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .status.none {
    color: var(--danger);
  }
  .opts {
    display: flex;
    gap: 1px;
  }
  button {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: var(--r-sm);
    color: var(--text-dim);
  }
  button:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  button.on {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .sep {
    width: 1px;
    height: 18px;
    margin: 0 3px;
    background: var(--border);
  }
</style>
