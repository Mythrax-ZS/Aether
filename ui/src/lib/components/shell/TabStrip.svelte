<script lang="ts">
  import { AppWindow, Columns2, Copy, FolderTree, Pencil, Plus, RefreshCw, Rows2, X } from '@lucide/svelte';
  import { flip } from 'svelte/animate';
  import { closeTab, closeTabs, openSftpForActive, reconnectPane, tearOffActiveTab } from '$lib/actions';
  import { keymap } from '$lib/commands.svelte';
  import { draggable, dropTarget } from '$lib/dom/dnd.svelte';
  import { tooltip } from '$lib/dom/tooltip';
  import { ui, type MenuItem } from '$lib/stores/ui.svelte';
  import { workspace, type Tab } from '$lib/stores/workspace.svelte';
  import { leaves } from '$lib/util/panes';

  let scroller = $state<HTMLDivElement>();
  let dragging = $state<string | null>(null);

  function revealActive() {
    const id = workspace.activeId;
    if (!scroller || !id) return;
    scroller.querySelector<HTMLElement>(`[data-tab-id="${id}"]`)?.scrollIntoView({ inline: 'nearest', block: 'nearest' });
  }

  // Keep the active tab on screen when there are more tabs than room — as tabs come and go, and
  // whenever the strip itself changes size (window resized, sidebar toggled).
  $effect(() => {
    void workspace.activeId;
    void workspace.tabs.length;
    revealActive();
  });
  $effect(() => {
    if (!scroller) return;
    const observer = new ResizeObserver(revealActive);
    observer.observe(scroller);
    return () => observer.disconnect();
  });

  function hasActivity(tab: Tab): boolean {
    return tab.kind === 'terminal' && tab.id !== workspace.activeId && leaves(tab.root).some((l) => workspace.info[l.id]?.activity);
  }

  async function rename(tab: Tab) {
    const name = await ui.askText({ title: 'Rename tab', label: 'Tab name', value: workspace.tabTitle(tab), confirmLabel: 'Rename' });
    if (name !== null) workspace.rename(tab.id, name);
  }

  function menuFor(tab: Tab): MenuItem[] {
    workspace.activate(tab.id);
    const terminal = tab.kind === 'terminal';
    const idx = workspace.tabs.findIndex((t) => t.id === tab.id);
    const items: MenuItem[] = [{ label: 'Rename…', icon: Pencil, run: () => void rename(tab) }];
    if (terminal) {
      items.push(
        { label: 'Duplicate', icon: Copy, run: () => workspace.duplicateTab(tab.id) },
        { separator: true },
        { label: 'Split right', icon: Columns2, shortcut: keymap.display('pane.splitRight') ?? undefined, run: () => workspace.splitActive('row') },
        { label: 'Split down', icon: Rows2, shortcut: keymap.display('pane.splitDown') ?? undefined, run: () => workspace.splitActive('col') },
        { separator: true },
        { label: 'Browse files', icon: FolderTree, shortcut: keymap.display('session.sftp') ?? undefined, disabled: !workspace.activeLeaf?.connectionId, run: openSftpForActive },
        { label: 'Reconnect', icon: RefreshCw, shortcut: keymap.display('session.reconnect') ?? undefined, disabled: !workspace.activeLeaf?.connectionId, run: () => void reconnectPane(tab.activePane) },
        { label: 'Move to new window', icon: AppWindow, disabled: tab.root.kind !== 'leaf', run: () => void tearOffActiveTab() },
      );
    }
    items.push(
      { separator: true },
      { label: 'Close tab', icon: X, shortcut: keymap.display('pane.close') ?? undefined, run: () => void closeTab(tab.id) },
      { label: 'Close other tabs', disabled: workspace.tabs.length < 2, run: () => void closeTabs(workspace.tabs.filter((t) => t.id !== tab.id).map((t) => t.id)) },
      { label: 'Close tabs to the right', disabled: idx >= workspace.tabs.length - 1, run: () => void closeTabs(workspace.tabs.slice(idx + 1).map((t) => t.id)) },
    );
    return items;
  }

  /** Live reordering: when the dragged tab's pointer crosses another tab's midpoint, swap places. */
  function reorderOver(targetId: string, x: number, rect: DOMRect) {
    const from = workspace.tabs.findIndex((t) => t.id === dragging);
    const to = workspace.tabs.findIndex((t) => t.id === targetId);
    if (from < 0 || to < 0 || from === to) return;
    const mid = rect.left + rect.width / 2;
    if ((from < to && x > mid) || (from > to && x < mid)) workspace.moveTab(from, to);
  }
</script>

<div class="tabs" role="tablist" aria-label="Open sessions">
  <div class="scroller" bind:this={scroller} onwheel={(e) => scroller && (scroller.scrollLeft += e.deltaY)}>
    {#each workspace.tabs as tab (tab.id)}
      {@const status = workspace.tabStatus(tab)}
      {@const active = tab.id === workspace.activeId}
      {@const title = workspace.tabTitle(tab)}
      <div
        class="tab"
        class:active
        class:dragging={dragging === tab.id}
        role="tab"
        tabindex={active ? 0 : -1}
        aria-selected={active}
        data-tab-id={tab.id}
        animate:flip={{ duration: 180 }}
        onpointerdown={(e) => e.button === 0 && workspace.activate(tab.id)}
        onauxclick={(e) => {
          if (e.button === 1) {
            e.preventDefault();
            void closeTab(tab.id);
          }
        }}
        onkeydown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') workspace.activate(tab.id);
        }}
        oncontextmenu={(e) => ui.showMenu(e, menuFor(tab))}
        use:tooltip={{ text: title, delay: 900 }}
        use:draggable={{
          payload: () => ({ type: 'tab', ids: [tab.id], label: title }),
          onstart: () => (dragging = tab.id),
          onend: () => (dragging = null),
        }}
        use:dropTarget={{
          accepts: (p) => p.type === 'tab',
          onmove: (_p, pt) => reorderOver(tab.id, pt.x, pt.rect),
          ondrop: () => {},
        }}
      >
        {#if tab.kind === 'sftp'}
          <span class="kind"><FolderTree size={14} /></span>
        {/if}
        {#if status}<i class="dot {status}" title={status}></i>{/if}
        <span class="title">{title}</span>
        {#if hasActivity(tab)}<i class="activity" aria-label="New output"></i>{/if}
        <button
          type="button"
          class="close"
          tabindex="-1"
          aria-label="Close tab"
          data-no-drag
          onpointerdown={(e) => e.stopPropagation()}
          onclick={(e) => {
            e.stopPropagation();
            void closeTab(tab.id);
          }}><X size={12} strokeWidth={2.4} /></button
        >
      </div>
    {/each}
  </div>
  <button type="button" class="new" aria-label="New connection" onclick={() => ui.openPalette('connect')} use:tooltip={{ text: 'New connection', shortcut: keymap.display('tab.new') }}>
    <Plus size={16} />
  </button>
</div>

<style>
  .tabs {
    display: flex;
    align-items: flex-end;
    align-self: stretch;
    min-width: 0;
    flex: 0 1 auto;
  }
  .scroller {
    display: flex;
    align-items: flex-end;
    align-self: stretch;
    min-width: 0;
    padding: 0 8px 0 8px;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
  }
  .scroller::-webkit-scrollbar {
    display: none;
  }
  .tab {
    position: relative;
    flex: 0 1 196px;
    min-width: 116px;
    max-width: 220px;
    height: calc(var(--titlebar-h) - 8px);
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px 0 12px;
    margin-right: 2px;
    border-radius: 9px 9px 0 0;
    color: var(--text-dim);
    font-size: var(--fs-md);
    font-weight: 500;
    white-space: nowrap;
    cursor: default;
    transition:
      background var(--dur-fast),
      color var(--dur-fast),
      opacity var(--dur);
  }
  .tab:hover:not(.active) {
    background: var(--bg-hover);
    color: var(--text);
  }
  .tab.dragging {
    opacity: 0.55;
  }
  .tab.active {
    z-index: 2;
    height: calc(var(--titlebar-h) - 7px);
    margin-bottom: -1px;
    color: var(--text);
    background:
      var(--grad-accent) top / 100% 2px no-repeat,
      var(--stage-bg, var(--bg-terminal));
  }
  /* Inverted corners that make the active tab flow into the stage below it. */
  .tab.active::before,
  .tab.active::after {
    content: '';
    position: absolute;
    bottom: 0;
    width: 9px;
    height: 9px;
    pointer-events: none;
  }
  .tab.active::before {
    left: -9px;
    background: radial-gradient(circle at 0 0, transparent 8.5px, var(--stage-bg, var(--bg-terminal)) 9px);
  }
  .tab.active::after {
    right: -9px;
    background: radial-gradient(circle at 100% 0, transparent 8.5px, var(--stage-bg, var(--bg-terminal)) 9px);
  }
  .kind {
    display: inline-flex;
    color: var(--accent);
    margin-right: -2px;
  }
  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dot {
    flex-shrink: 0;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-faint);
  }
  .dot.connected {
    background: var(--success);
    box-shadow: 0 0 8px color-mix(in srgb, var(--success) 60%, transparent);
  }
  .dot.connecting {
    background: var(--accent);
    animation: pulse-dot 1.1s var(--ease-in-out) infinite;
  }
  .dot.warning {
    background: var(--warning);
    animation: pulse-dot 1.1s var(--ease-in-out) infinite;
  }
  .dot.error {
    background: var(--danger);
  }
  .activity {
    flex-shrink: 0;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent-2);
    box-shadow: 0 0 8px var(--accent-2);
  }
  .close {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 20px;
    height: 20px;
    border-radius: 6px;
    color: var(--text-faint);
    opacity: 0;
    transition:
      opacity var(--dur-fast),
      background var(--dur-fast);
  }
  .tab:hover .close,
  .tab.active .close {
    opacity: 1;
  }
  .close:hover {
    background: var(--bg-active);
    color: var(--text);
  }
  .new {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    margin: 0 0 4px 2px;
    border-radius: var(--r-md);
    color: var(--text-dim);
    transition:
      background var(--dur-fast),
      color var(--dur-fast),
      transform var(--dur-fast);
  }
  .new:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .new:active {
    transform: scale(0.92);
  }
</style>
