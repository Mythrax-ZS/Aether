<script lang="ts">
  import { RotateCcw, Search, X } from '@lucide/svelte';
  import { COMMANDS, DEFAULT_SHORTCUTS, keymap, type Category } from '$lib/commands.svelte';
  import { tooltip } from '$lib/dom/tooltip';
  import { settings } from '$lib/stores/settings.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import { acceleratorFromEvent, formatAccelerator, matchesAccelerator, parseAccelerator } from '$lib/util/keys';
  import Button from '$lib/components/ui/Button.svelte';
  import Kbd from '$lib/components/ui/Kbd.svelte';

  let query = $state('');
  let recording = $state<string | null>(null);

  const ORDER: Category[] = ['Navigation', 'Tabs', 'Panes', 'Terminal', 'Session', 'Hosts', 'View', 'App'];

  const groups = $derived.by(() => {
    const q = query.trim().toLowerCase();
    // "Go to tab N" ×9 would drown the list; they're documented in one line below the groups.
    const list = COMMANDS.filter((c) => !c.id.startsWith('tab.go'));
    const hit = q ? list.filter((c) => c.title.toLowerCase().includes(q) || c.category.toLowerCase().includes(q) || (keymap.display(c.id) ?? '').toLowerCase().includes(q)) : list;
    return ORDER.map((cat) => ({ cat, items: hit.filter((c) => c.category === cat) })).filter((g) => g.items.length);
  });

  const customised = (id: string) => settings.value.keybindings[id] !== undefined;

  function write(next: Record<string, string>) {
    settings.patch({ keybindings: next });
  }

  function setBinding(id: string, accel: string) {
    write({ ...settings.value.keybindings, [id]: accel });
  }

  function reset(id: string) {
    const next = { ...settings.value.keybindings };
    delete next[id];
    write(next);
  }

  function clear(id: string) {
    setBinding(id, '');
  }

  function resetAll() {
    write({});
  }

  /** Another command that already uses this accelerator. */
  function conflictFor(id: string, accelText: string): string | null {
    const acc = parseAccelerator(accelText);
    if (!acc) return null;
    for (const [other, list] of keymap.bindings) {
      if (other === id) continue;
      if (list.some((a) => a.key === acc.key && a.ctrl === acc.ctrl && a.shift === acc.shift && a.alt === acc.alt && a.meta === acc.meta)) return other;
    }
    return null;
  }

  $effect(() => {
    if (!recording) return;
    const id = recording;
    const onKey = async (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === 'Escape') {
        recording = null;
        return;
      }
      const acc = acceleratorFromEvent(e);
      if (!acc) return; // still holding only modifiers
      const hasModifier = acc.ctrl || acc.alt || acc.meta;
      if (!hasModifier && !/^f\d{1,2}$/.test(acc.key)) {
        ui.toast({ kind: 'info', title: 'Add Ctrl or Alt', message: 'Shortcuts need a modifier so they don’t get in the way of typing.', timeout: 3200 });
        return;
      }
      const text = formatAccelerator(acc);
      recording = null;
      const other = conflictFor(id, text);
      if (other) {
        const otherTitle = COMMANDS.find((c) => c.id === other)?.title ?? other;
        const ok = await ui.confirm({
          title: 'Use this shortcut anyway?',
          message: `${text} is currently used by “${otherTitle}”. It will be unassigned there.`,
          confirmLabel: 'Reassign',
        });
        if (!ok) return;
        write({ ...settings.value.keybindings, [other]: '', [id]: text });
        return;
      }
      setBinding(id, text);
    };
    // Capture phase so the app's own shortcuts don't fire while recording.
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  });

  void matchesAccelerator;
</script>

<div class="editor">
  <div class="bar">
    <label class="search">
      <Search size={14} />
      <input bind:value={query} placeholder="Search commands or shortcuts" spellcheck="false" autocomplete="off" aria-label="Search shortcuts" />
    </label>
    <Button size="sm" variant="ghost" disabled={Object.keys(settings.value.keybindings).length === 0} onclick={resetAll}>
      {#snippet icon()}<RotateCcw size={13} />{/snippet}Reset all
    </Button>
  </div>

  <div class="groups">
    {#each groups as g (g.cat)}
      <section>
        <h4>{g.cat}</h4>
        {#each g.items as c (c.id)}
          {@const Icon = c.icon}
          {@const mine = customised(c.id)}
          {@const shown = keymap.display(c.id)}
          <div class="row" class:rec={recording === c.id}>
            <span class="ico"><Icon size={15} /></span>
            <span class="title">{c.title}</span>
            <span class="keys">
              <button
                type="button"
                class="chip"
                class:empty={!shown}
                class:mine
                aria-label="Change shortcut for {c.title}"
                onclick={() => (recording = recording === c.id ? null : c.id)}
              >
                {#if recording === c.id}
                  <span class="hint">Press keys…</span>
                {:else if shown}
                  <Kbd keys={shown} />
                {:else}
                  <span class="none">Not set</span>
                {/if}
              </button>
              {#if mine}
                <button type="button" class="mini" aria-label="Reset to default" onclick={() => reset(c.id)} use:tooltip={DEFAULT_SHORTCUTS[c.id] ? `Reset to ${DEFAULT_SHORTCUTS[c.id][0]}` : 'Reset to default (none)'}><RotateCcw size={12} /></button>
              {/if}
              {#if shown && !mine}
                <button type="button" class="mini" aria-label="Remove shortcut" onclick={() => clear(c.id)} use:tooltip={'Remove shortcut'}><X size={12} /></button>
              {/if}
            </span>
          </div>
        {/each}
      </section>
    {:else}
      <p class="none-found">No commands match “{query}”.</p>
    {/each}
    {#if !query}
      <p class="foot">Jump straight to a tab with <Kbd keys="Ctrl+Alt+1" subtle /> … <Kbd keys="Ctrl+Alt+9" subtle /> (the last tab is 9).</p>
    {/if}
  </div>
</div>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 0 12px;
  }
  .search {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 32px;
    padding: 0 10px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    color: var(--text-faint);
  }
  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .search input {
    flex: 1;
    background: none;
    border: 0;
    outline: none;
    color: var(--text);
    cursor: text;
  }
  .groups {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-right: 4px;
  }
  section + section {
    margin-top: 18px;
  }
  h4 {
    padding: 0 8px 6px;
    font-size: var(--fs-2xs);
    font-weight: 700;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 38px;
    padding: 0 8px;
    border-radius: var(--r-md);
  }
  .row:hover,
  .row.rec {
    background: var(--bg-hover);
  }
  .ico {
    display: inline-flex;
    color: var(--text-faint);
  }
  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .keys {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    min-width: 96px;
    height: 28px;
    padding: 0 8px;
    justify-content: flex-end;
    border-radius: var(--r-sm);
    border: 1px solid transparent;
    transition:
      border-color var(--dur-fast),
      background var(--dur-fast);
  }
  .chip:hover {
    border-color: var(--border-strong);
    background: var(--bg-input);
  }
  .row.rec .chip {
    border-color: var(--accent);
    background: var(--accent-soft);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .hint {
    color: var(--accent);
    font-size: var(--fs-sm);
    font-weight: 600;
    animation: pulse-dot 1.2s var(--ease-in-out) infinite;
  }
  .none {
    color: var(--text-faint);
    font-size: var(--fs-sm);
  }
  .mine :global(kbd) {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border-strong));
    color: var(--accent);
  }
  .mini {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 5px;
    color: var(--text-faint);
    opacity: 0;
    transition: opacity var(--dur-fast);
  }
  .row:hover .mini {
    opacity: 1;
  }
  .mini:hover {
    background: var(--bg-active);
    color: var(--text);
  }
  .none-found {
    padding: 40px;
    text-align: center;
    color: var(--text-faint);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 18px 8px 4px;
    font-size: var(--fs-sm);
    color: var(--text-faint);
  }
</style>
