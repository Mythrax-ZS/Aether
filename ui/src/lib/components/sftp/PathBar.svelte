<script lang="ts">
  import { ChevronRight } from '@lucide/svelte';
  import type { Crumb } from '$lib/sftp/paths';

  type Props = {
    path: string;
    crumbs: Crumb[];
    onnavigate: (path: string) => void;
    /** Where dropped items go when released on a crumb. */
    ondrophint?: (path: string | null) => void;
  };

  let { path, crumbs, onnavigate }: Props = $props();

  let editing = $state(false);
  let draft = $state('');
  let input = $state<HTMLInputElement>();
  let scroller = $state<HTMLDivElement>();
  /** The start of a long path is scrolled out of view; fade it rather than cutting a crumb in half. */
  let clipped = $state(false);

  function measure() {
    clipped = !!scroller && scroller.scrollLeft > 1;
  }

  // Long paths show their tail: keep the last crumb in view.
  $effect(() => {
    void crumbs;
    if (scroller && !editing) {
      requestAnimationFrame(() => {
        if (!scroller) return;
        scroller.scrollLeft = scroller.scrollWidth;
        measure();
      });
    }
  });

  export function edit() {
    draft = path;
    editing = true;
    queueMicrotask(() => {
      input?.focus();
      input?.select();
    });
  }

  function commit() {
    const value = draft.trim();
    editing = false;
    if (value && value !== path) onnavigate(value);
  }
</script>

{#if editing}
  <input
    class="edit"
    bind:this={input}
    bind:value={draft}
    spellcheck="false"
    autocomplete="off"
    aria-label="Path"
    onkeydown={(e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        commit();
      } else if (e.key === 'Escape') {
        e.stopPropagation();
        editing = false;
      }
    }}
    onblur={() => (editing = false)}
  />
{:else}
  <!-- Clicking the empty part of the bar switches to typing a path; the keyboard has Ctrl+L for that. -->
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions, a11y_no_noninteractive_element_interactions -->
  <div class="bar" onclick={(e) => (e.target === e.currentTarget || e.target === scroller) && edit()} role="navigation" aria-label="Path">
    <div class="track" class:clipped bind:this={scroller} onscroll={measure}>
      {#each crumbs as c, i (c.path + i)}
        {#if i > 0}<span class="sep"><ChevronRight size={12} /></span>{/if}
        <button type="button" class="crumb" class:last={i === crumbs.length - 1} onclick={() => onnavigate(c.path)}>{c.name}</button>
      {/each}
    </div>
  </div>
{/if}

<style>
  .bar {
    flex: 1;
    min-width: 0;
    display: flex;
    height: 28px;
    padding: 0 4px;
    overflow: hidden;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    cursor: text;
  }
  .track {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    overflow-x: auto;
    overflow-y: hidden;
    white-space: nowrap;
    scrollbar-width: none;
  }
  .track::-webkit-scrollbar {
    display: none;
  }
  .track.clipped {
    mask-image: linear-gradient(to right, transparent 0, #000 22px);
  }
  .crumb {
    flex-shrink: 0;
    height: 22px;
    padding: 0 6px;
    border-radius: 5px;
    color: var(--text-dim);
    font-size: var(--fs-sm);
    font-weight: 550;
    cursor: pointer;
  }
  .crumb:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .crumb.last {
    color: var(--text);
  }
  .sep {
    display: inline-flex;
    color: var(--text-faint);
  }
  .edit {
    flex: 1;
    min-width: 0;
    height: 28px;
    padding: 0 10px;
    background: var(--bg-input);
    border: 1px solid var(--accent);
    border-radius: var(--r-md);
    box-shadow: 0 0 0 3px var(--accent-soft);
    outline: none;
    color: var(--text);
    font: 12px var(--font-mono);
    cursor: text;
  }
</style>
