<script lang="ts">
  import {
    ChevronRight, ClipboardCopy, Clock, Copy, Download, Folder, FolderOpen, FolderPlus, KeyRound, Moon, Pencil, Play, Plus, Search, Server,
    Settings, ShieldCheck, Star, Sun, Trash2, X, Zap,
  } from '@lucide/svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { fade } from 'svelte/transition';
  import {
    connectHost, connectHosts, connectTarget, copyAddress, deleteGroup, deleteHosts, moveHosts, toggleTheme,
  } from '$lib/actions';
  import { keymap } from '$lib/commands.svelte';
  import { scrollIntoViewWhen } from '$lib/dom/actions';
  import { dropTarget, type DragPayload } from '$lib/dom/dnd.svelte';
  import { tooltip } from '$lib/dom/tooltip';
  import type { Group, Host, HostId } from '$lib/bindings';
  import { hosts } from '$lib/stores/hosts.svelte';
  import { settings } from '$lib/stores/settings.svelte';
  import { ui, type MenuItem } from '$lib/stores/ui.svelte';
  import { workspace } from '$lib/stores/workspace.svelte';
  import { clamp, plural } from '$lib/util/format';
  import { groupPath, searchHosts, type GroupNode } from '$lib/util/tree';
  import IconButton from '$lib/components/ui/IconButton.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import HostRow from './HostRow.svelte';

  type SectionId = 'favorites' | 'recent' | 'hosts';
  type Row =
    | { kind: 'section'; key: string; id: SectionId; title: string; count: number }
    | { kind: 'group'; key: string; group: Group; depth: number; count: number }
    | { kind: 'host'; key: string; host: Host; depth: number; path: string | null; indices: number[] }
    | { kind: 'quick'; key: string; target: string }
    | { kind: 'none'; key: string };

  let query = $state('');
  let searchEl = $state<HTMLInputElement>();
  let listEl = $state<HTMLDivElement>();
  let closedSections = $state<Record<SectionId, boolean>>({ favorites: false, recent: false, hosts: false });
  const selection = new SvelteSet<HostId>();
  let anchor: HostId | null = null;
  let cursorKey = $state<string | null>(null);
  // The keyboard cursor is only drawn while the list has focus; after a click elsewhere it would look like a stray highlight.
  let listFocused = $state(false);
  const showCursor = (key: string) => listFocused && cursorKey === key;
  let resizing = $state(false);

  const width = $derived(settings.value.appearance.sidebarWidth);
  const searching = $derived(query.trim().length > 0);
  const hostCount = $derived(hosts.data.hosts.length);

  const looksLikeTarget = (q: string) =>
    /^[^\s@]+@[^\s@]+$/.test(q) || /^[\w-]+(\.[\w-]+)+(:\d{1,5})?$/.test(q) || /^\d{1,3}(\.\d{1,3}){3}(:\d{1,5})?$/.test(q) || /^\[[0-9a-f:]+\](:\d+)?$/i.test(q);

  const rows = $derived.by<Row[]>(() => {
    const out: Row[] = [];
    const q = query.trim();

    if (q) {
      if (looksLikeTarget(q)) out.push({ kind: 'quick', key: 'quick', target: q });
      const hits = searchHosts(hosts.data, q);
      for (const h of hits) out.push({ kind: 'host', key: `search:${h.item.id}`, host: h.item, depth: 0, path: h.path, indices: h.indices });
      if (hits.length === 0 && !looksLikeTarget(q)) out.push({ kind: 'none', key: 'none' });
      return out;
    }

    if (hosts.favorites.length) {
      out.push({ kind: 'section', key: 'sec:favorites', id: 'favorites', title: 'Favorites', count: hosts.favorites.length });
      if (!closedSections.favorites) for (const h of hosts.favorites) out.push({ kind: 'host', key: `fav:${h.id}`, host: h, depth: 0, path: null, indices: [] });
    }
    // Recent only helps once there is more than a handful of hosts to scroll through.
    if (hosts.recent.length && hostCount > 5) {
      out.push({ kind: 'section', key: 'sec:recent', id: 'recent', title: 'Recent', count: hosts.recent.length });
      if (!closedSections.recent) for (const h of hosts.recent.slice(0, 4)) out.push({ kind: 'host', key: `recent:${h.id}`, host: h, depth: 0, path: null, indices: [] });
    }
    out.push({ kind: 'section', key: 'sec:hosts', id: 'hosts', title: 'Hosts', count: hostCount });
    if (!closedSections.hosts) {
      const walk = (node: GroupNode, depth: number) => {
        out.push({ kind: 'group', key: `group:${node.group.id}`, group: node.group, depth, count: node.total });
        if (node.group.collapsed) return;
        node.children.forEach((c) => walk(c, depth + 1));
        for (const h of node.hosts) out.push({ kind: 'host', key: `tree:${h.id}`, host: h, depth: depth + 1, path: null, indices: [] });
      };
      hosts.tree.roots.forEach((n) => walk(n, 0));
      for (const h of hosts.tree.loose) out.push({ kind: 'host', key: `tree:${h.id}`, host: h, depth: 0, path: null, indices: [] });
    }
    return out;
  });

  const navigable = $derived(rows.filter((r) => r.kind !== 'none'));

  // Keep the cursor on a row that still exists.
  $effect(() => {
    if (cursorKey && !rows.some((r) => r.key === cursorKey)) cursorKey = null;
  });

  // Drop selected hosts that vanished (deleted elsewhere).
  $effect(() => {
    const ids = new Set(hosts.data.hosts.map((h) => h.id));
    for (const id of [...selection]) if (!ids.has(id)) selection.delete(id);
  });

  // ── selection ──────────────────────────────────────────────────────────────

  function selectHost(e: MouseEvent, row: Extract<Row, { kind: 'host' }>) {
    cursorKey = row.key;
    const id = row.host.id;
    if (e.shiftKey && anchor) {
      const order = rows.filter((r): r is Extract<Row, { kind: 'host' }> => r.kind === 'host').map((r) => r.host.id);
      const a = order.indexOf(anchor);
      const b = order.indexOf(id);
      if (a >= 0 && b >= 0) {
        if (!e.ctrlKey) selection.clear();
        for (const x of order.slice(Math.min(a, b), Math.max(a, b) + 1)) selection.add(x);
        return;
      }
    }
    if (e.ctrlKey || e.metaKey) {
      if (selection.has(id)) selection.delete(id);
      else selection.add(id);
    } else {
      selection.clear();
      selection.add(id);
    }
    anchor = id;
  }

  function selectedHosts(fallback?: Host): Host[] {
    const list = hosts.data.hosts.filter((h) => selection.has(h.id));
    if (fallback && !selection.has(fallback.id)) return [fallback];
    return list;
  }

  function dragFor(host: Host): DragPayload | null {
    const list = selectedHosts(host);
    return {
      type: 'host',
      ids: list.map((h) => h.id),
      label: list.length === 1 ? list[0].label || list[0].hostname : plural(list.length, 'host'),
      sublabel: list.length === 1 ? list[0].hostname : undefined,
    };
  }

  // ── menus ──────────────────────────────────────────────────────────────────

  function groupChoices(ids: HostId[], current: Set<string | null>): MenuItem[] {
    const items: MenuItem[] = [{ label: 'No group', icon: Server, disabled: current.size === 1 && current.has(null), run: () => void moveHosts(ids, null) }];
    const all = [...hosts.data.groups].map((g) => ({ g, path: groupPath(hosts.data, g.id) ?? g.name }));
    all.sort((a, b) => a.path.localeCompare(b.path, undefined, { sensitivity: 'base' }));
    for (const { g, path } of all) items.push({ label: path, icon: Folder, disabled: current.size === 1 && current.has(g.id), run: () => void moveHosts(ids, g.id) });
    items.push({ separator: true }, { label: 'New group…', icon: FolderPlus, run: () => ui.openDialog({ type: 'group-editor' }) });
    return items;
  }

  function hostMenu(e: MouseEvent, host: Host) {
    if (!selection.has(host.id)) {
      selection.clear();
      selection.add(host.id);
      anchor = host.id;
    }
    const list = selectedHosts();
    const many = list.length > 1;
    const ids = list.map((h) => h.id);
    const items: MenuItem[] = [
      { label: many ? `Connect to ${list.length} hosts` : 'Connect', icon: Play, shortcut: 'Enter', run: () => void connectHosts(list) },
      { separator: true },
      { label: 'Edit…', icon: Pencil, shortcut: 'F2', disabled: many, run: () => ui.editHost(host.id) },
      { label: 'Duplicate', icon: Copy, disabled: many, run: () => void hosts.duplicate(host.id).then((c) => ui.success(`Created “${c.label}”`)).catch((err) => ui.error(err)) },
      {
        label: list.every((h) => h.favorite) ? 'Remove from favorites' : 'Add to favorites',
        icon: Star,
        run: () => {
          const make = !list.every((h) => h.favorite);
          void Promise.all(list.filter((h) => h.favorite !== make).map((h) => hosts.toggleFavorite(h))).catch((err) => ui.error(err));
        },
      },
      { label: 'Copy address', icon: ClipboardCopy, disabled: many, run: () => void copyAddress(host) },
      { label: 'Move to group', icon: Folder, submenu: groupChoices(ids, new Set(list.map((h) => h.groupId))) },
      { separator: true },
      { label: many ? `Delete ${list.length} hosts…` : 'Delete…', icon: Trash2, danger: true, shortcut: 'Del', run: () => void deleteHosts(list) },
    ];
    ui.showMenu(e, items);
  }

  function groupMenu(e: MouseEvent, group: Group) {
    ui.showMenu(e, [
      { label: 'New host here…', icon: Plus, run: () => ui.newHost(group.id) },
      { label: 'New subgroup…', icon: FolderPlus, run: () => ui.openDialog({ type: 'group-editor', parentId: group.id }) },
      { separator: true },
      { label: 'Edit group…', icon: Pencil, run: () => ui.openDialog({ type: 'group-editor', groupId: group.id }) },
      { label: group.collapsed ? 'Expand' : 'Collapse', icon: ChevronRight, run: () => void hosts.setCollapsed(group, !group.collapsed) },
      { separator: true },
      { label: 'Delete group…', icon: Trash2, danger: true, run: () => void deleteGroup(group) },
    ]);
  }

  function addMenu(e: MouseEvent | { clientX: number; clientY: number }) {
    ui.showMenu(e, [
      { label: 'New host…', icon: Plus, shortcut: keymap.display('host.new') ?? undefined, run: () => ui.newHost() },
      { label: 'New group…', icon: FolderPlus, run: () => ui.openDialog({ type: 'group-editor' }) },
      { separator: true },
      { label: 'Import from ~/.ssh/config…', icon: Download, run: () => ui.openDialog({ type: 'import-ssh' }) },
    ]);
  }

  function openAddMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    addMenu({ clientX: r.left, clientY: r.bottom + 6 });
  }

  function emptyMenu(e: MouseEvent) {
    if ((e.target as HTMLElement).closest('.row, .grow, .sec')) return;
    addMenu(e);
    e.preventDefault();
  }

  // ── keyboard ───────────────────────────────────────────────────────────────

  function moveCursor(delta: number) {
    if (navigable.length === 0) return;
    const i = navigable.findIndex((r) => r.key === cursorKey);
    const next = clamp(i < 0 ? (delta > 0 ? 0 : navigable.length - 1) : i + delta, 0, navigable.length - 1);
    const row = navigable[next];
    cursorKey = row.key;
    if (row.kind === 'host') {
      selection.clear();
      selection.add(row.host.id);
      anchor = row.host.id;
    }
  }

  function onListKey(e: KeyboardEvent) {
    const row = rows.find((r) => r.key === cursorKey);
    switch (e.key) {
      case 'ArrowDown':
        moveCursor(1);
        break;
      case 'ArrowUp':
        moveCursor(-1);
        break;
      case 'Home':
        moveCursor(-9999);
        break;
      case 'End':
        moveCursor(9999);
        break;
      case 'ArrowRight':
        if (row?.kind === 'group' && row.group.collapsed) void hosts.setCollapsed(row.group, false);
        else if (row?.kind === 'section' && closedSections[row.id]) closedSections[row.id] = false;
        else return;
        break;
      case 'ArrowLeft':
        if (row?.kind === 'group' && !row.group.collapsed) void hosts.setCollapsed(row.group, true);
        else if (row?.kind === 'section' && !closedSections[row.id]) closedSections[row.id] = true;
        else return;
        break;
      case 'Enter':
        if (row?.kind === 'host') void connectHosts(selectedHosts(row.host));
        else if (row?.kind === 'group') void hosts.setCollapsed(row.group, !row.group.collapsed);
        else if (row?.kind === 'quick') connectTarget(row.target);
        else if (row?.kind === 'section') closedSections[row.id] = !closedSections[row.id];
        else return;
        break;
      case ' ':
        if (row?.kind === 'host') {
          if (selection.has(row.host.id)) selection.delete(row.host.id);
          else selection.add(row.host.id);
        } else return;
        break;
      case 'F2':
        if (row?.kind === 'host') ui.editHost(row.host.id);
        else if (row?.kind === 'group') ui.openDialog({ type: 'group-editor', groupId: row.group.id });
        else return;
        break;
      case 'Delete':
        if (selection.size) void deleteHosts(selectedHosts());
        else return;
        break;
      case 'Escape':
        if (selection.size) selection.clear();
        else if (query) query = '';
        else return;
        break;
      default:
        // Type-to-search, like a file manager.
        if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
          searchEl?.focus();
          return;
        }
        return;
    }
    e.preventDefault();
  }

  function onSearchKey(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      listEl?.focus();
      if (!cursorKey) moveCursor(1);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const first = rows.find((r) => r.kind === 'quick' || r.kind === 'host');
      if (first?.kind === 'quick') connectTarget(first.target);
      else if (first?.kind === 'host') connectHost(first.host);
    } else if (e.key === 'Escape') {
      if (query) {
        query = '';
        e.stopPropagation();
      } else searchEl?.blur();
    }
  }

  // ── resizing ───────────────────────────────────────────────────────────────

  function startResize(e: PointerEvent) {
    e.preventDefault();
    resizing = true;
    const move = (ev: PointerEvent) => settings.patch({ appearance: { sidebarWidth: clamp(Math.round(ev.clientX), 220, 480) } });
    const up = () => {
      resizing = false;
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  }

  const toggleSection = (id: SectionId) => (closedSections[id] = !closedSections[id]);
  const dropIntoGroup = (p: DragPayload, groupId: string | null) => void moveHosts(p.ids, groupId);
</script>

<aside class="sidebar" class:open={ui.sidebarOpen} class:resizing style:--w="{width}px" aria-label="Hosts" inert={!ui.sidebarOpen}>
  <div class="inner">
    <div class="head">
      <label class="search" class:active={searching}>
        <Search size={14} />
        <input
          bind:this={searchEl}
          bind:value={query}
          type="text"
          placeholder="Search hosts or connect"
          spellcheck="false"
          autocomplete="off"
          aria-label="Search hosts"
          onkeydown={onSearchKey}
        />
        {#if query}
          <button type="button" class="clear" aria-label="Clear search" onclick={() => { query = ''; searchEl?.focus(); }}><X size={13} /></button>
        {/if}
      </label>
      <IconButton label="Add" size={30} onclick={openAddMenu}><Plus size={17} /></IconButton>
    </div>

    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div
      class="list"
      role="tree"
      tabindex="0"
      aria-label="Saved hosts"
      bind:this={listEl}
      onkeydown={onListKey}
      oncontextmenu={emptyMenu}
      onfocusin={() => (listFocused = true)}
      onfocusout={(e) => {
        if (!listEl?.contains(e.relatedTarget as Node | null)) listFocused = false;
      }}
      onfocus={() => {
        if (!cursorKey && navigable.length) cursorKey = navigable[0].key;
      }}
    >
      {#each rows as row (row.key)}
        <div class="item" in:fade={{ duration: 110 }}>
          {#if row.kind === 'section'}
            <button
              type="button"
              class="sec"
              class:cursor={showCursor(row.key)}
              aria-expanded={!closedSections[row.id]}
              onclick={() => {
                cursorKey = row.key;
                toggleSection(row.id);
              }}
              use:scrollIntoViewWhen={cursorKey === row.key}
              use:dropTarget={row.id === 'hosts'
                ? { accepts: (p) => p.type === 'host', ondrop: (p) => dropIntoGroup(p, null) }
                : { accepts: () => false, ondrop: () => {} }}
            >
              <span class="chev" class:open={!closedSections[row.id]}><ChevronRight size={13} /></span>
              {#if row.id === 'favorites'}<Star size={12} />{:else if row.id === 'recent'}<Clock size={12} />{/if}
              <span class="sec-title">{row.title}</span>
              <span class="count">{row.count}</span>
            </button>
          {:else if row.kind === 'group'}
            {@const g = row.group}
            <div
              class="grow"
              class:cursor={showCursor(row.key)}
              role="treeitem"
              aria-selected="false"
              aria-expanded={!g.collapsed}
              tabindex="-1"
              style:--indent="{row.depth * 14}px"
              onclick={() => {
                cursorKey = row.key;
                void hosts.setCollapsed(g, !g.collapsed);
              }}
              onkeydown={() => {}}
              oncontextmenu={(e) => groupMenu(e, g)}
              use:scrollIntoViewWhen={cursorKey === row.key}
              use:dropTarget={{
                accepts: (p) => p.type === 'host' && !!p.ids.length,
                ondrop: (p) => dropIntoGroup(p, g.id),
                holdMs: 600,
                onhold: () => g.collapsed && void hosts.setCollapsed(g, false),
              }}
            >
              <span class="chev" class:open={!g.collapsed}><ChevronRight size={13} /></span>
              <span class="folder" style:color={g.color ?? undefined}>
                {#if g.collapsed}<Folder size={16} />{:else}<FolderOpen size={16} />{/if}
              </span>
              <span class="gname">{g.name}</span>
              <span class="count">{row.count}</span>
              <span class="gact" data-no-drag>
                <button
                  type="button"
                  class="mini"
                  aria-label="New host in {g.name}"
                  onclick={(e) => {
                    e.stopPropagation();
                    ui.newHost(g.id);
                  }}
                  use:tooltip={'New host here'}><Plus size={14} /></button
                >
              </span>
            </div>
          {:else if row.kind === 'host'}
            <HostRow
              host={row.host}
              depth={row.depth}
              selected={selection.has(row.host.id)}
              cursor={showCursor(row.key)}
              status={workspace.hostStatus.get(row.host.id)}
              indices={row.indices}
              path={row.path}
              onselect={(e) => selectHost(e, row)}
              onconnect={() => connectHost(row.host)}
              oncontext={(e) => hostMenu(e, row.host)}
              onfavorite={() => void hosts.toggleFavorite(row.host).catch((err) => ui.error(err))}
              drag={() => dragFor(row.host)}
            />
          {:else if row.kind === 'quick'}
            <button type="button" class="quick" class:cursor={showCursor(row.key)} onclick={() => connectTarget(row.target)}>
              <span class="qi"><Zap size={15} /></span>
              <span class="qt">Connect to <b>{row.target}</b></span>
              <kbd>↵</kbd>
            </button>
          {:else}
            {@const q = query.trim()}
            <p class="none">No hosts match “{q.length > 28 ? q.slice(0, 27) + '…' : q}”.<br /><span>Press Ctrl K to search commands too.</span></p>
          {/if}
        </div>
      {/each}

      {#if !searching && hosts.isEmpty}
        <div class="empty">
          <div class="art"><Server size={22} /></div>
          <h3>No hosts yet</h3>
          <p>Save the servers you use often and open them in one click.</p>
          <div class="cta">
            <Button variant="primary" size="sm" onclick={() => ui.newHost()}>{#snippet icon()}<Plus size={14} />{/snippet}Add host</Button>
            <Button size="sm" onclick={() => ui.openDialog({ type: 'import-ssh' })}>{#snippet icon()}<Download size={14} />{/snippet}Import ssh config</Button>
          </div>
        </div>
      {/if}
    </div>

    <div class="foot">
      <IconButton label="SSH keys" size={30} onclick={() => ui.openDialog({ type: 'keys' })}><KeyRound size={16} /></IconButton>
      <IconButton label="Known hosts" size={30} onclick={() => ui.openDialog({ type: 'known-hosts' })}><ShieldCheck size={16} /></IconButton>
      <span class="grow-space"></span>
      <IconButton label="Switch theme" size={30} onclick={toggleTheme}>
        {#if settings.activeTheme.kind === 'dark'}<Sun size={16} />{:else}<Moon size={16} />{/if}
      </IconButton>
      <IconButton label="Settings" shortcut={keymap.display('app.settings')} size={30} onclick={() => ui.openSettings()}><Settings size={16} /></IconButton>
    </div>
  </div>

  {#if ui.sidebarOpen}
    <!-- A focusable separator is an interactive widget (ARIA 1.2): arrow keys resize it. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      class="resizer"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize sidebar"
      tabindex="0"
      onpointerdown={startResize}
      ondblclick={() => settings.patch({ appearance: { sidebarWidth: 280 } })}
      onkeydown={(e) => {
        if (e.key === 'ArrowLeft') settings.patch({ appearance: { sidebarWidth: clamp(width - 16, 220, 480) } });
        if (e.key === 'ArrowRight') settings.patch({ appearance: { sidebarWidth: clamp(width + 16, 220, 480) } });
      }}
    ></div>
  {/if}
</aside>

<style>
  .sidebar {
    grid-area: sidebar;
    position: relative;
    z-index: var(--z-sidebar);
    width: 0;
    background: var(--bg-sidebar);
    overflow: hidden;
    transition: width var(--dur-slow) var(--ease-out);
  }
  .sidebar.open {
    width: var(--w);
  }
  .sidebar.resizing {
    transition: none;
  }
  .inner {
    display: flex;
    flex-direction: column;
    width: var(--w);
    height: 100%;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px 10px 12px;
  }
  .search {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 32px;
    padding: 0 8px 0 10px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    color: var(--text-faint);
    transition:
      border-color var(--dur-fast),
      box-shadow var(--dur);
  }
  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
    color: var(--accent);
  }
  .search input {
    flex: 1;
    min-width: 0;
    height: 100%;
    background: none;
    border: 0;
    outline: none;
    color: var(--text);
    font-size: var(--fs-md);
  }
  .search input::placeholder {
    color: var(--text-faint);
  }
  .clear {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: 5px;
    color: var(--text-faint);
  }
  .clear:hover {
    background: var(--bg-active);
    color: var(--text);
  }

  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    padding-bottom: 12px;
    outline: none;
    --host-row-h: 46px;
  }
  :global(:root[data-density='compact']) .list {
    --host-row-h: 36px;
  }

  .sec {
    display: flex;
    align-items: center;
    gap: 5px;
    width: calc(100% - 12px);
    height: 28px;
    margin: 6px 6px 0;
    padding: 0 8px 0 4px;
    border-radius: var(--r-sm);
    color: var(--text-faint);
    font-size: var(--fs-2xs);
    font-weight: 700;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    text-align: left;
    transition:
      background var(--dur-fast),
      color var(--dur-fast);
  }
  .sec:hover {
    color: var(--text-dim);
  }
  .sec.cursor,
  .grow.cursor,
  .quick.cursor {
    box-shadow: inset 0 0 0 1px var(--accent-ring);
  }
  .sec-title {
    flex: 1;
  }
  .count {
    font-size: var(--fs-2xs);
    font-weight: 600;
    letter-spacing: 0;
    color: var(--text-faint);
    font-variant-numeric: tabular-nums;
  }
  .chev {
    display: inline-flex;
    transition: transform var(--dur) var(--ease-out);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  :global([data-drop-over='true']).sec,
  :global([data-drop-over='true']).grow {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1px var(--accent-ring);
    color: var(--text);
  }

  .grow {
    display: flex;
    align-items: center;
    gap: 6px;
    height: var(--row-h);
    margin: 1px 6px;
    padding: 0 6px 0 calc(4px + var(--indent));
    border-radius: var(--r-md);
    color: var(--text-dim);
    font-weight: 560;
    cursor: default;
    transition: background var(--dur-fast);
  }
  .grow:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .folder {
    display: inline-flex;
    color: var(--accent-2);
  }
  .gname {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .gact {
    display: flex;
    opacity: 0;
    transition: opacity var(--dur-fast);
  }
  .grow:hover .gact {
    opacity: 1;
  }
  .mini {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 5px;
    color: var(--text-faint);
  }
  .mini:hover {
    background: var(--bg-active);
    color: var(--text);
  }

  .quick {
    display: flex;
    align-items: center;
    gap: 10px;
    width: calc(100% - 12px);
    height: 44px;
    margin: 2px 6px 6px;
    padding: 0 10px 0 8px;
    border-radius: var(--r-md);
    background: var(--grad-accent-soft);
    border: 1px solid color-mix(in srgb, var(--accent) 30%, transparent);
    color: var(--text);
    text-align: left;
  }
  .quick:hover {
    border-color: var(--accent-ring);
  }
  .qi {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 8px;
    background: var(--grad-accent);
    color: var(--on-accent);
  }
  .qt {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-dim);
  }
  .qt b {
    color: var(--text);
    font-weight: 650;
  }
  kbd {
    font: 600 11px var(--font-ui);
    color: var(--text-faint);
  }

  .none {
    padding: 28px 22px;
    text-align: center;
    color: var(--text-dim);
    line-height: 1.6;
  }
  .none span {
    color: var(--text-faint);
    font-size: var(--fs-sm);
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    margin: 24px 14px;
    padding: 24px 16px;
    text-align: center;
    border: 1px dashed var(--border-strong);
    border-radius: var(--r-lg);
    animation: fade-up 400ms var(--ease-out) both;
  }
  .art {
    display: grid;
    place-items: center;
    width: 46px;
    height: 46px;
    margin-bottom: 4px;
    border-radius: 14px;
    background: var(--grad-accent-soft);
    color: var(--accent);
  }
  .empty h3 {
    font-size: var(--fs-lg);
  }
  .empty p {
    color: var(--text-dim);
    font-size: var(--fs-sm);
    line-height: 1.5;
  }
  .cta {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 10px;
    width: 100%;
  }

  .foot {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 8px 10px;
    border-top: 1px solid var(--border);
  }
  .grow-space {
    flex: 1;
  }

  .resizer {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: 7px;
    cursor: col-resize;
    z-index: 3;
  }
  .resizer::after {
    content: '';
    position: absolute;
    top: 0;
    bottom: 0;
    right: 0;
    width: 2px;
    background: transparent;
    transition: background var(--dur-fast);
  }
  .resizer:hover::after,
  .resizing .resizer::after {
    background: var(--accent);
  }
</style>
