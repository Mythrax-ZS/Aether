<script lang="ts">
  import { X } from '@lucide/svelte';

  type Props = {
    tags: string[];
    label?: string;
    hint?: string;
    placeholder?: string;
    /** Existing tags offered as quick picks. */
    suggestions?: string[];
  };

  let { tags = $bindable([]), label, hint, placeholder = 'Add a tag and press Enter', suggestions = [] }: Props = $props();

  let draft = $state('');
  let input = $state<HTMLInputElement>();
  const id = `tags-${Math.random().toString(36).slice(2, 8)}`;

  const offered = $derived(suggestions.filter((s) => !tags.includes(s) && (!draft || s.toLowerCase().includes(draft.toLowerCase()))).slice(0, 6));

  function add(raw: string) {
    const t = raw.trim().replace(/^#/, '').toLowerCase().replace(/\s+/g, '-');
    if (t && !tags.includes(t)) tags = [...tags, t];
    draft = '';
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Enter' || e.key === ',' || e.key === 'Tab') {
      if (draft.trim()) {
        e.preventDefault();
        add(draft);
      }
    } else if (e.key === 'Backspace' && !draft && tags.length) {
      tags = tags.slice(0, -1);
    }
  }
</script>

<div class="field">
  {#if label}<label for={id}>{label}</label>{/if}
  <!-- Clicking the padding focuses the real input; keyboard users go straight to it. -->
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="box" onclick={() => input?.focus()}>
    {#each tags as t (t)}
      <span class="chip">
        #{t}
        <button type="button" aria-label="Remove tag {t}" onclick={() => (tags = tags.filter((x) => x !== t))}><X size={11} strokeWidth={2.6} /></button>
      </span>
    {/each}
    <input {id} bind:this={input} bind:value={draft} {placeholder} onkeydown={onKey} onblur={() => add(draft)} spellcheck="false" autocomplete="off" />
  </div>
  {#if offered.length}
    <div class="sugg">
      {#each offered as s (s)}<button type="button" onclick={() => add(s)}>#{s}</button>{/each}
    </div>
  {/if}
  {#if hint}<p class="msg">{hint}</p>{/if}
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  label {
    font-size: var(--fs-sm);
    font-weight: 550;
    color: var(--text-dim);
  }
  .box {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 5px;
    min-height: 34px;
    padding: 4px 6px;
    background: var(--bg-input);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-md);
    transition:
      border-color var(--dur-fast),
      box-shadow var(--dur);
    cursor: text;
  }
  .box:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 4px 0 8px;
    border-radius: var(--r-pill);
    background: var(--accent-soft);
    color: var(--accent);
    font-size: var(--fs-sm);
    font-weight: 600;
  }
  .chip button {
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    color: inherit;
    opacity: 0.7;
  }
  .chip button:hover {
    opacity: 1;
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  input {
    flex: 1;
    min-width: 110px;
    height: 24px;
    padding: 0 5px;
    background: none;
    border: 0;
    outline: none;
    color: var(--text);
    font-size: var(--fs-md);
    cursor: text;
  }
  input::placeholder {
    color: var(--text-faint);
  }
  .sugg {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }
  .sugg button {
    height: 22px;
    padding: 0 8px;
    border-radius: var(--r-pill);
    border: 1px dashed var(--border-strong);
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .sugg button:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .msg {
    font-size: var(--fs-xs);
    color: var(--text-faint);
  }
</style>
