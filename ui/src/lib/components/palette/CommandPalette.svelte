<script lang="ts">
  import { Command as CommandIcon, CornerDownLeft, FolderTree, Search, SquareTerminal, TerminalSquare, Zap } from '@lucide/svelte';
  import type { Component } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { connectHost, connectTarget } from '$lib/actions';
  import { keymap, paletteCommands } from '$lib/commands.svelte';
  import { scrollIntoViewWhen, trapFocus } from '$lib/dom/actions';
  import type { Host } from '$lib/bindings';
  import { terminalRegistry } from '$lib/terminal/registry';
  import { hosts } from '$lib/stores/hosts.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import { workspace } from '$lib/stores/workspace.svelte';
  import { excerptAround, highlightRuns, rankBy } from '$lib/util/fuzzy';
  import { searchHosts } from '$lib/util/tree';
  import Avatar from '$lib/components/ui/Avatar.svelte';
  import Kbd from '$lib/components/ui/Kbd.svelte';

  type Mode = 'all' | 'connect' | 'commands' | 'hosts' | 'output';

  interface Item {
    key: string;
    section: string;
    title: string;
    subtitle?: string;
    hint?: string;
    shortcut?: string | null;
    icon?: Component<{ size?: number }>;
    host?: Host;
    indices?: number[];
    score: number;
    run: () => void | Promise<void>;
  }

  let query = $state('');
  let active = $state(0);
  let input = $state<HTMLInputElement>();
  let outputHits = $state<Item[]>([]);

  // Start fresh each time it opens.
  $effect(() => {
    if (ui.palette.open) {
      query = ui.palette.query;
      active = 0;
    }
  });

  const parsed = $derived.by(() => {
    const raw = query;
    const prefix = raw[0];
    if (prefix === '>') return { mode: 'commands' as Mode, text: raw.slice(1).trim() };
    if (prefix === '@') return { mode: 'hosts' as Mode, text: raw.slice(1).trim() };
    if (prefix === '/') return { mode: 'output' as Mode, text: raw.slice(1).trim() };
    return { mode: (ui.palette.mode === 'connect' ? 'connect' : 'all') as Mode, text: raw.trim() };
  });

  const looksLikeTarget = (q: string) =>
    /^[^\s@]+@[^\s@]+$/.test(q) || /^[\w-]+(\.[\w-]+)+(:\d{1,5})?$/.test(q) || /^\d{1,3}(\.\d{1,3}){3}(:\d{1,5})?$/.test(q) || /^\[[0-9a-f:]+\](:\d+)?$/i.test(q);

  function hostItem(host: Host, score: number, indices: number[] = [], path?: string | null): Item {
    return {
      key: `host:${host.id}`,
      section: 'Hosts',
      title: host.label || host.hostname,
      subtitle: `${path ? path + ' · ' : ''}${host.username ? host.username + '@' : ''}${host.hostname}${host.port !== 22 ? ':' + host.port : ''}`,
      hint: workspace.hostStatus.get(host.id) === 'connected' ? 'Open' : 'Connect',
      host,
      indices,
      score,
      run: () => connectHost(host),
    };
  }

  const items = $derived.by<Item[]>(() => {
    const { mode, text } = parsed;
    if (mode === 'output') return outputHits;
    const out: Item[] = [];

    // ── hosts ──
    if (mode === 'all' || mode === 'connect' || mode === 'hosts') {
      if (text) {
        for (const h of searchHosts(hosts.data, text).slice(0, 8)) out.push(hostItem(h.item, h.score + 0.3, h.indices, h.path));
      } else {
        const seen = new Set<string>();
        for (const h of [...hosts.recent, ...hosts.favorites, ...hosts.data.hosts]) {
          if (seen.has(h.id) || seen.size >= (mode === 'all' ? 5 : 12)) continue;
          seen.add(h.id);
          out.push(hostItem(h, 1000 - seen.size));
        }
      }
    }

    // ── quick connect ──
    if ((mode === 'all' || mode === 'connect') && text && looksLikeTarget(text)) {
      out.push({ key: 'quick', section: 'Quick connect', title: `Connect to ${text}`, subtitle: 'Not saved — opens a one-off session', hint: 'Connect', icon: Zap, score: 999, run: () => connectTarget(text) });
    }
    if (mode === 'connect' && text && !looksLikeTarget(text) && out.length === 0) {
      out.push({ key: 'quick', section: 'Quick connect', title: `Connect to ${text}`, subtitle: 'Use an alias from ~/.ssh/config or a hostname', hint: 'Connect', icon: Zap, score: 1, run: () => connectTarget(text) });
    }

    // ── open tabs ──
    if (mode === 'all' && workspace.tabs.length > 0) {
      const tabs = workspace.tabs.map((t) => ({ t, title: workspace.tabTitle(t) }));
      const ranked = text ? rankBy(tabs, text, [{ text: (x) => x.title, weight: 1, primary: true }]) : tabs.slice(0, 4).map((item, i) => ({ item, score: 500 - i, indices: [] as number[] }));
      for (const r of ranked.slice(0, 4)) {
        const { t, title } = r.item;
        out.push({
          key: `tab:${t.id}`,
          section: 'Open tabs',
          title,
          subtitle: t.id === workspace.activeId ? 'Current tab' : undefined,
          hint: 'Switch',
          icon: t.kind === 'sftp' ? FolderTree : SquareTerminal,
          indices: r.indices,
          score: r.score + 0.2,
          run: () => workspace.activate(t.id),
        });
      }
    }

    // ── commands ──
    if (mode === 'all' || mode === 'commands') {
      const cmds = paletteCommands();
      if (text) {
        const ranked = rankBy(cmds, text, [
          { text: (c) => c.title, weight: 1, primary: true },
          { text: (c) => c.keywords, weight: 0.7 },
          { text: (c) => c.category, weight: 0.5 },
        ]);
        for (const r of ranked.slice(0, mode === 'commands' ? 30 : 6)) {
          out.push({ key: `cmd:${r.item.id}`, section: 'Commands', title: r.item.title, subtitle: r.item.category, shortcut: keymap.display(r.item.id), icon: r.item.icon, indices: r.indices, score: r.score, run: () => r.item.run() });
        }
      } else {
        const suggested = mode === 'commands' ? cmds : cmds.filter((c) => ['host.new', 'app.settings', 'pane.splitRight', 'app.keys', 'app.theme', 'host.import'].includes(c.id));
        suggested.forEach((c, i) => out.push({ key: `cmd:${c.id}`, section: mode === 'commands' ? c.category : 'Suggested', title: c.title, shortcut: keymap.display(c.id), icon: c.icon, score: 100 - i, run: () => c.run() }));
      }
    }

    // Sections ordered by their best hit; items keep their order inside a section.
    const best = new Map<string, number>();
    out.forEach((i) => best.set(i.section, Math.max(best.get(i.section) ?? -Infinity, i.score)));
    const order = text ? [...best.entries()].sort((a, b) => b[1] - a[1]).map(([s]) => s) : [...best.keys()];
    const rank = new Map(order.map((s, i) => [s, i]));
    return out
      .map((it, idx) => ({ it, idx }))
      .sort((a, b) => (rank.get(a.it.section)! - rank.get(b.it.section)!) || a.idx - b.idx)
      .map((x) => x.it);
  });

  // Searching other terminals' output is the one slow mode: debounce and cap the work.
  $effect(() => {
    const { mode, text } = parsed;
    if (mode !== 'output') {
      outputHits = [];
      return;
    }
    if (text.length < 2) {
      outputHits = [];
      return;
    }
    const timer = setTimeout(() => {
      const found: Item[] = [];
      for (const [leafId, controller] of terminalRegistry.entries()) {
        const tab = workspace.tabOfLeaf(leafId);
        if (!tab) continue;
        for (const hit of controller.searchBuffer(text, 15)) {
          const shown = excerptAround(hit.text, hit.start, text.length);
          found.push({
            key: `out:${leafId}:${hit.line}`,
            section: workspace.tabTitle(tab),
            title: shown.text,
            subtitle: `line ${hit.line + 1}`,
            hint: 'Jump',
            icon: TerminalSquare,
            indices: shown.indices,
            score: 0,
            run: () => {
              workspace.activate(tab.id);
              workspace.focusPane(leafId);
              requestAnimationFrame(() => controller.revealLine(hit.line, hit.start, text.length));
            },
          });
        }
      }
      outputHits = found.slice(0, 60);
    }, 140);
    return () => clearTimeout(timer);
  });

  $effect(() => {
    void items.length;
    if (active >= items.length) active = Math.max(0, items.length - 1);
  });

  // Reset the highlight when the query changes.
  let lastText = '';
  $effect(() => {
    if (parsed.text !== lastText) {
      lastText = parsed.text;
      active = 0;
    }
  });

  function close() {
    ui.closePalette();
  }

  async function run(item: Item | undefined) {
    if (!item) return;
    close();
    // Let the palette unmount and give focus back before the action (it may open a dialog).
    await Promise.resolve();
    try {
      await item.run();
    } catch (e) {
      ui.error(e);
    }
  }

  function onKey(e: KeyboardEvent) {
    const n = items.length;
    switch (e.key) {
      case 'ArrowDown':
      case 'Tab':
        if (e.key === 'Tab' && e.shiftKey) active = (active - 1 + n) % Math.max(n, 1);
        else active = (active + 1) % Math.max(n, 1);
        break;
      case 'ArrowUp':
        active = (active - 1 + n) % Math.max(n, 1);
        break;
      case 'PageDown':
        active = Math.min(n - 1, active + 7);
        break;
      case 'PageUp':
        active = Math.max(0, active - 7);
        break;
      case 'Home':
        if (!e.shiftKey) active = 0;
        else return;
        break;
      case 'End':
        if (!e.shiftKey) active = Math.max(0, n - 1);
        else return;
        break;
      case 'Enter':
        void run(items[active]);
        break;
      case 'Escape':
        close();
        break;
      default:
        return;
    }
    e.preventDefault();
    e.stopPropagation();
  }

  const MODES: { label: string; prefix: string; mode: Mode }[] = [
    { label: 'Everything', prefix: '', mode: 'all' },
    { label: 'Hosts', prefix: '@', mode: 'hosts' },
    { label: 'Commands', prefix: '>', mode: 'commands' },
    { label: 'Terminal output', prefix: '/', mode: 'output' },
  ];

  function setMode(prefix: string) {
    query = prefix;
    input?.focus();
  }

  const placeholder = $derived(
    parsed.mode === 'commands' ? 'Run a command…' : parsed.mode === 'hosts' ? 'Find a host…' : parsed.mode === 'output' ? 'Search the output of all open terminals…' : parsed.mode === 'connect' ? 'Connect to a host, or type user@host' : 'Search hosts, commands and tabs, or type user@host',
  );

  // Section headers appear where the section changes.
  const showHeader = (i: number) => i === 0 || items[i - 1].section !== items[i].section;
</script>

{#if ui.palette.open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="scrim" transition:fade={{ duration: 120 }} onpointerdown={(e) => e.target === e.currentTarget && close()}>
    <div class="panel" role="dialog" aria-modal="true" aria-label="Command palette" tabindex="-1" use:trapFocus={{ initial: 'input' }} in:fly={{ y: -14, duration: 190, easing: cubicOut }} out:fly={{ y: -8, duration: 110 }}>
      <div class="bar">
        <span class="mi">{#if parsed.mode === 'commands'}<CommandIcon size={18} />{:else}<Search size={18} />{/if}</span>
        <input
          bind:this={input}
          bind:value={query}
          type="text"
          {placeholder}
          spellcheck="false"
          autocomplete="off"
          role="combobox"
          aria-expanded="true"
          aria-controls="palette-list"
          aria-activedescendant={items[active] ? `pal-${items[active].key}` : undefined}
          aria-label="Command palette"
          onkeydown={onKey}
        />
        <kbd class="esc">esc</kbd>
      </div>

      <div class="modes" role="tablist" aria-label="Search scope">
        {#each MODES as m (m.mode)}
          <button type="button" role="tab" aria-selected={parsed.mode === m.mode || (m.mode === 'all' && parsed.mode === 'connect')} class:on={parsed.mode === m.mode || (m.mode === 'all' && parsed.mode === 'connect')} onclick={() => setMode(m.prefix)}>
            {m.label}{#if m.prefix}<span class="px">{m.prefix}</span>{/if}
          </button>
        {/each}
      </div>

      <div class="results" id="palette-list" role="listbox">
        {#each items as item, i (item.key)}
          {#if showHeader(i)}<div class="head" role="presentation">{item.section}</div>{/if}
          {@const Icon = item.icon}
          {@const runs = highlightRuns(item.title, item.indices ?? [])}
          <button
            type="button"
            id="pal-{item.key}"
            class="item"
            class:on={i === active}
            role="option"
            aria-selected={i === active}
            onpointermove={() => (active = i)}
            onclick={() => void run(item)}
            use:scrollIntoViewWhen={i === active}
          >
            <span class="lead">
              {#if item.host}<Avatar name={item.title} color={item.host.color} size={28} status={workspace.hostStatus.get(item.host.id) ?? null} />
              {:else if Icon}<span class="ico"><Icon size={16} /></span>{/if}
            </span>
            <span class="txt">
              <span class="t">{#each runs as r, k (k)}{#if r.hit}<mark>{r.text}</mark>{:else}{r.text}{/if}{/each}</span>
              {#if item.subtitle}<span class="s">{item.subtitle}</span>{/if}
            </span>
            {#if item.shortcut}<Kbd keys={item.shortcut} subtle />{:else if item.hint}<span class="hint">{item.hint}</span>{/if}
          </button>
        {:else}
          <div class="none">
            {#if parsed.mode === 'output'}
              {parsed.text.length < 2 ? 'Type to search everything printed in your open terminals.' : 'Nothing found in the open terminals.'}
            {:else}
              Nothing matches “{parsed.text}”.
              {#if looksLikeTarget(parsed.text)}{:else}<br /><span>Type <b>user@host</b> to connect to a new server.</span>{/if}
            {/if}
          </div>
        {/each}
      </div>

      <div class="foot">
        <span><kbd>↑</kbd><kbd>↓</kbd> navigate</span>
        <span><kbd><CornerDownLeft size={11} /></kbd> select</span>
        <span class="sp"></span>
        <span><kbd>@</kbd> hosts</span>
        <span><kbd>&gt;</kbd> commands</span>
        <span><kbd>/</kbd> output</span>
      </div>
    </div>
  </div>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: calc(var(--z-modal) + 20);
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding: 12vh 20px 20px;
    background: var(--scrim);
    backdrop-filter: blur(4px);
  }
  .panel {
    display: flex;
    flex-direction: column;
    width: min(660px, 100%);
    max-height: min(560px, 76vh);
    background: color-mix(in srgb, var(--bg-surface) 96%, transparent);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-xl);
    box-shadow: var(--shadow-lg);
    overflow: hidden;
    outline: none;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 56px;
    padding: 0 16px 0 18px;
    border-bottom: 1px solid var(--border);
  }
  .mi {
    display: inline-flex;
    color: var(--accent);
  }
  input {
    flex: 1;
    min-width: 0;
    height: 100%;
    background: none;
    border: 0;
    outline: none;
    color: var(--text);
    font-size: var(--fs-xl);
    cursor: text;
  }
  input::placeholder {
    color: var(--text-faint);
  }
  .esc {
    padding: 2px 6px;
    font: 600 10px var(--font-ui);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-faint);
    border: 1px solid var(--border-strong);
    border-radius: 5px;
  }
  .modes {
    display: flex;
    gap: 4px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
  }
  .modes button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    padding: 0 10px;
    border-radius: var(--r-pill);
    color: var(--text-dim);
    font-size: var(--fs-sm);
    font-weight: 550;
    transition:
      background var(--dur-fast),
      color var(--dur-fast);
  }
  .modes button:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .modes button.on {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .px {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    opacity: 0.7;
  }
  .results {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 6px 8px 10px;
  }
  .head {
    padding: 10px 10px 4px;
    font-size: var(--fs-2xs);
    font-weight: 700;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
  .item {
    position: relative;
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    min-height: 44px;
    padding: 6px 10px;
    border-radius: var(--r-md);
    text-align: left;
  }
  .item.on {
    background: var(--bg-active);
  }
  .item.on::before {
    content: '';
    position: absolute;
    left: 0;
    top: 10px;
    bottom: 10px;
    width: 3px;
    border-radius: 0 3px 3px 0;
    background: var(--grad-accent);
  }
  .lead {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 28px;
  }
  .ico {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 8px;
    background: var(--bg-hover);
    color: var(--text-dim);
  }
  .item.on .ico {
    background: var(--accent-soft);
    color: var(--accent);
  }
  .txt {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .t {
    font-weight: 560;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .t mark {
    background: none;
    color: var(--accent);
    font-weight: 750;
  }
  .s {
    font-size: var(--fs-xs);
    color: var(--text-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint {
    font-size: var(--fs-xs);
    color: var(--text-faint);
    opacity: 0;
    transition: opacity var(--dur-fast);
  }
  .item.on .hint {
    opacity: 1;
  }
  .none {
    padding: 34px 20px;
    text-align: center;
    color: var(--text-dim);
    line-height: 1.7;
  }
  .none span {
    color: var(--text-faint);
    font-size: var(--fs-sm);
  }
  .none b {
    color: var(--text-dim);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 14px;
    height: 34px;
    padding: 0 16px;
    border-top: 1px solid var(--border);
    background: color-mix(in srgb, var(--bg-surface) 70%, var(--bg-app));
    font-size: var(--fs-xs);
    color: var(--text-faint);
  }
  .foot span {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .foot .sp {
    flex: 1;
  }
  .foot kbd {
    display: inline-grid;
    place-items: center;
    min-width: 17px;
    height: 17px;
    padding: 0 4px;
    font: 600 10px var(--font-ui);
    color: var(--text-dim);
    border: 1px solid var(--border-strong);
    border-radius: 4px;
  }
</style>
