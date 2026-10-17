<script lang="ts">
  import {
    ArrowLeft, ArrowRight, ArrowUp, ChevronDown, Copy, Download, FileEdit, FilePlus2, FolderOpen, FolderPlus, House, Lock,
    Monitor, RefreshCw, Search, Server, Shield, Trash2, Upload, Pencil, X, TriangleAlert,
  } from '@lucide/svelte';
  import { onMount, untrack } from 'svelte';
  import { fade } from 'svelte/transition';
  import { copyText } from '$lib/actions';
  import { draggable, dropTarget, type DragPayload } from '$lib/dom/dnd.svelte';
  import { tooltip } from '$lib/dom/tooltip';
  import { glyphFor, looksEditable } from '$lib/sftp/icons';
  import type { Side, SftpController } from '$lib/sftp/controller.svelte';
  import type { FileItem } from '$lib/sftp/provider';
  import { settings } from '$lib/stores/settings.svelte';
  import { ui, type MenuItem } from '$lib/stores/ui.svelte';
  import { formatBytes, formatDateTime, formatMode, plural } from '$lib/util/format';
  import type { SortKey } from '$lib/sftp/paths';
  import Button from '$lib/components/ui/Button.svelte';
  import IconButton from '$lib/components/ui/IconButton.svelte';
  import PathBar from './PathBar.svelte';

  type Props = {
    ctl: SftpController;
    side: Side;
    title: string;
    /** The connection behind this pane isn't usable right now. */
    offline?: boolean;
    /** Files from Explorer are hovering over this pane. */
    osDrop?: boolean;
    onedit?: (item: FileItem) => void;
    onperms?: (items: FileItem[]) => void;
  };

  let { ctl, side, title, offline = false, osDrop = false, onedit, onperms }: Props = $props();

  const pane = $derived(ctl.pane(side));
  const active = $derived(ctl.active === side);
  const remote = $derived(side === 'remote');
  const ops = $derived(pane.provider.ops);

  const rowH = $derived(settings.value.appearance.density === 'compact' ? 26 : 30);
  let listEl = $state<HTMLDivElement>();
  let viewH = $state(400);
  let scrollTop = $state(0);
  let pathBar = $state<PathBar>();
  let filterOpen = $state(false);
  let filterEl = $state<HTMLInputElement>();

  const creating = $derived(pane.editing?.path === null);
  const offset = $derived(creating ? 1 : 0);
  const total = $derived(pane.rows.length + offset);
  const first = $derived(Math.max(0, Math.floor(scrollTop / rowH) - 8));
  const last = $derived(Math.min(pane.rows.length, Math.ceil((scrollTop + viewH) / rowH) + 8));
  const slice = $derived(pane.rows.slice(first, last).map((item, k) => ({ item, index: first + k })));

  // Keep the keyboard cursor on screen.
  $effect(() => {
    const path = pane.cursor;
    const el = untrack(() => listEl);
    if (!path || !el) return;
    const idx = untrack(() => pane.rows.findIndex((r) => r.path === path));
    if (idx < 0) return;
    const top = (idx + untrack(() => offset)) * untrack(() => rowH);
    const bottom = top + untrack(() => rowH);
    if (top < el.scrollTop) el.scrollTop = top;
    else if (bottom > el.scrollTop + el.clientHeight) el.scrollTop = bottom - el.clientHeight;
  });

  // The new-folder row sits above the first item: make sure it can be seen.
  $effect(() => {
    if (creating) untrack(() => listEl && (listEl.scrollTop = 0));
  });

  // After a navigation, start at the top (unless something was selected on purpose).
  $effect(() => {
    void pane.cwd;
    untrack(() => {
      if (listEl && !pane.cursor) listEl.scrollTop = 0;
    });
  });

  onMount(() => {
    const el = listEl;
    if (!el) return;
    const ro = new ResizeObserver(() => (viewH = el.clientHeight));
    ro.observe(el);
    viewH = el.clientHeight;
    return () => ro.disconnect();
  });

  // ── opening things ─────────────────────────────────────────────────────────

  function activate(item: FileItem) {
    if (item.isDir) void pane.open(item.path);
    else if (!offline || !remote) void ctl.sendSelection(side, [item]);
  }

  function activateCursor() {
    const item = pane.rows.find((r) => r.path === pane.cursor);
    if (item) activate(item);
  }

  // ── drag and drop ──────────────────────────────────────────────────────────

  interface Dragged {
    side: Side;
    items: FileItem[];
  }

  function dragPayload(item: FileItem): DragPayload | null {
    if (item.special === 'drive') return null;
    if (!pane.selection.has(item.path)) pane.selectOnly(item.path);
    const items = $state.snapshot(pane.selected) as FileItem[];
    if (items.length === 0) return null;
    const data: Dragged = { side, items };
    return {
      type: 'file',
      ids: items.map((i) => i.path),
      label: items.length === 1 ? items[0].name : plural(items.length, 'item'),
      sublabel: remote ? 'from the server' : 'from this PC',
      data,
    };
  }

  function canDrop(p: DragPayload, target?: FileItem): boolean {
    if (p.type !== 'file' || offline) return false;
    const d = p.data as Dragged;
    if (d.side !== side) return !(target && !target.isDir);
    // Same side: only moving into another folder makes sense.
    if (!target?.isDir) return false;
    return !d.items.some((i) => i.path === target.path || (i.isDir && ops.within(target.path, i.path)));
  }

  function drop(p: DragPayload, target?: FileItem) {
    const d = p.data as Dragged;
    const dest = target ? target.path : pane.cwd;
    if (d.side !== side) void ctl.sendSelection(d.side, d.items, dest);
    else void ctl.moveInto(side, d.items, dest);
  }

  // ── menus ──────────────────────────────────────────────────────────────────

  function rowMenu(e: MouseEvent, item: FileItem) {
    if (!pane.selection.has(item.path)) pane.selectOnly(item.path);
    ctl.active = side;
    const sel = pane.selected;
    const one = sel.length === 1 ? sel[0] : null;
    const items: MenuItem[] = [];
    if (one?.isDir) items.push({ label: 'Open', icon: FolderOpen, shortcut: 'Enter', run: () => void pane.open(one.path) });
    items.push({
      label: remote ? 'Download' : 'Upload',
      icon: remote ? Download : Upload,
      disabled: offline || sel.some((s) => s.special === 'drive'),
      run: () => void ctl.sendSelection(side),
    });
    if (remote && one && looksEditable(one)) items.push({ label: 'Edit', icon: FileEdit, run: () => onedit?.(one) });
    items.push(
      { separator: true },
      { label: 'Rename', icon: Pencil, shortcut: 'F2', disabled: !one || !!one.special || offline, run: () => one && pane.beginRename(one) },
      { label: sel.length > 1 ? `Delete ${sel.length} items` : 'Delete', icon: Trash2, danger: true, shortcut: 'Del', disabled: offline || sel.some((s) => !!s.special), run: () => void pane.remove(sel) },
      { separator: true },
      { label: 'Copy path', icon: Copy, disabled: !one, run: () => one && void copyText(one.path, 'Path copied') },
    );
    if (remote) items.push({ label: 'Permissions…', icon: Shield, disabled: offline, run: () => onperms?.(sel) });
    items.push({ separator: true }, ...blankItems());
    ui.showMenu(e, items);
  }

  function blankItems(): MenuItem[] {
    return [
      { label: 'New folder', icon: FolderPlus, shortcut: 'F7', disabled: offline || pane.cwd === '', run: () => pane.beginNewFolder() },
      { label: 'Refresh', icon: RefreshCw, shortcut: 'F5', run: () => void pane.refresh() },
      {
        label: pane.showHidden ? 'Hide hidden files' : 'Show hidden files',
        icon: Lock,
        run: () => settings.patch({ sftp: { showHidden: !settings.value.sftp.showHidden } }),
      },
    ];
  }

  function blankMenu(e: MouseEvent) {
    if ((e.target as HTMLElement).closest('.row')) return;
    ctl.active = side;
    ui.showMenu(e, blankItems());
  }

  // ── keyboard ───────────────────────────────────────────────────────────────

  function onKey(e: KeyboardEvent) {
    if (pane.editing) return;
    const mod = e.ctrlKey || e.metaKey;
    const page = Math.max(1, Math.floor(viewH / rowH) - 1);
    switch (e.key) {
      case 'ArrowDown':
        if (e.altKey) activateCursor();
        else pane.move(1, e.shiftKey);
        break;
      case 'ArrowUp':
        if (e.altKey) void pane.up();
        else pane.move(-1, e.shiftKey);
        break;
      case 'ArrowLeft':
        if (e.altKey) void pane.back();
        else return;
        break;
      case 'ArrowRight':
        if (e.altKey) void pane.forward();
        else return;
        break;
      case 'PageDown':
        pane.move(page, e.shiftKey);
        break;
      case 'PageUp':
        pane.move(-page, e.shiftKey);
        break;
      case 'Home':
        pane.move('start', e.shiftKey);
        break;
      case 'End':
        pane.move('end', e.shiftKey);
        break;
      case 'Enter':
        activateCursor();
        break;
      case 'Backspace':
        void pane.up();
        break;
      case 'F5':
        void pane.refresh();
        break;
      case 'F2': {
        const one = pane.selected.length === 1 ? pane.selected[0] : null;
        if (one && !one.special && !offline) pane.beginRename(one);
        break;
      }
      case 'F7':
        if (!offline && pane.cwd !== '') pane.beginNewFolder();
        break;
      case 'F4':
        pathBar?.edit();
        break;
      case 'Delete':
        if (!offline && pane.selected.length && !pane.selected.some((s) => s.special)) void pane.remove(pane.selected);
        break;
      case 'Escape':
        if (pane.filter) {
          pane.filter = '';
          filterOpen = false;
        } else pane.clearSelection();
        break;
      case 'a':
      case 'A':
        if (mod) pane.selectAll();
        else return;
        break;
      case 'l':
      case 'L':
        if (mod) pathBar?.edit();
        else return;
        break;
      case 'f':
      case 'F':
        if (mod) {
          filterOpen = true;
          queueMicrotask(() => filterEl?.focus());
        } else return;
        break;
      case 'ContextMenu': {
        const row = listEl?.querySelector<HTMLElement>('.row.cursor');
        const r = row?.getBoundingClientRect();
        const item = pane.rows.find((x) => x.path === pane.cursor);
        if (item && r) rowMenu({ clientX: r.left + 40, clientY: r.bottom, preventDefault() {} } as unknown as MouseEvent, item);
        break;
      }
      default:
        if (e.key.length === 1 && !mod && !e.altKey) {
          pane.typeAhead(e.key);
          break;
        }
        return;
    }
    e.preventDefault();
    e.stopPropagation();
  }

  // ── inline rename ──────────────────────────────────────────────────────────

  function focusName(node: HTMLInputElement) {
    queueMicrotask(() => {
      node.focus();
      // Renaming selects the stem so typing keeps the extension; a new folder selects everything.
      const renaming = pane.editing?.path != null;
      const dot = node.value.lastIndexOf('.');
      node.setSelectionRange(0, renaming && dot > 0 ? dot : node.value.length);
    });
  }

  const SORTS: { key: SortKey; label: string; cls: string }[] = $derived([
    { key: 'name', label: 'Name', cls: 'c-name' },
    { key: 'size', label: 'Size', cls: 'c-size' },
    { key: 'modified', label: 'Modified', cls: 'c-date' },
    ...(remote ? [{ key: 'mode' as SortKey, label: 'Permissions', cls: 'c-mode' }] : []),
  ]);

  const selSummary = $derived(
    pane.selected.length
      ? `${plural(pane.selected.length, 'item')} selected${pane.selectedBytes ? ` · ${formatBytes(pane.selectedBytes)}` : ''}`
      : `${plural(pane.rows.length, 'item')}${pane.items.length !== pane.rows.length ? ` of ${pane.items.length}` : ''}`,
  );
</script>

<section
  class="pane"
  class:active
  class:offline
  class:os-drop={osDrop}
  data-pane={side}
  aria-label={title}
  onpointerdowncapture={() => (ctl.active = side)}
  use:dropTarget={{ accepts: (p) => canDrop(p), ondrop: (p) => drop(p) }}
>
  <header>
    <div class="title">
      <span class="side">{#if remote}<Server size={14} />{:else}<Monitor size={14} />{/if}</span>
      <span class="name">{title}</span>
      {#if remote}<span class="live" class:off={offline} aria-hidden="true"></span>{/if}
    </div>
    <div class="toolbar">
      <IconButton label="Back" shortcut="Alt+Left" size={26} disabled={!pane.canGoBack} onclick={() => void pane.back()}><ArrowLeft size={15} /></IconButton>
      <IconButton label="Forward" shortcut="Alt+Right" size={26} disabled={!pane.canGoForward} onclick={() => void pane.forward()}><ArrowRight size={15} /></IconButton>
      <IconButton label="Up one level" shortcut="Alt+Up" size={26} disabled={ops.parent(pane.cwd) === null} onclick={() => void pane.up()}><ArrowUp size={15} /></IconButton>
      <PathBar bind:this={pathBar} path={pane.cwd} crumbs={ops.crumbs(pane.cwd)} onnavigate={(p) => void pane.open(p)} />
      <IconButton label="Home" size={26} onclick={() => void pane.openHome()}><House size={15} /></IconButton>
      <IconButton label="Refresh" shortcut="F5" size={26} onclick={() => void pane.refresh()}><RefreshCw size={14} class={pane.loading ? 'spin' : ''} /></IconButton>
      <IconButton label="New folder" shortcut="F7" size={26} disabled={offline || pane.cwd === ''} onclick={() => pane.beginNewFolder()}><FolderPlus size={15} /></IconButton>
      <IconButton
        label="Filter"
        shortcut="Ctrl+F"
        size={26}
        active={filterOpen || !!pane.filter}
        onclick={() => {
          filterOpen = !filterOpen;
          if (filterOpen) queueMicrotask(() => filterEl?.focus());
          else pane.filter = '';
        }}><Search size={14} /></IconButton
      >
    </div>
    {#if filterOpen}
      <div class="filter" transition:fade={{ duration: 100 }}>
        <Search size={13} />
        <input
          bind:this={filterEl}
          bind:value={pane.filter}
          placeholder="Filter this folder"
          spellcheck="false"
          autocomplete="off"
          aria-label="Filter this folder"
          onkeydown={(e) => {
            if (e.key === 'Escape') {
              pane.filter = '';
              filterOpen = false;
              listEl?.focus();
              e.stopPropagation();
            }
          }}
        />
        {#if pane.filter}<button type="button" aria-label="Clear filter" onclick={() => (pane.filter = '')}><X size={12} /></button>{/if}
      </div>
    {/if}
  </header>

  <div class="cols" role="row">
    {#each SORTS as s (s.key)}
      <div class="colhead" role="columnheader" aria-sort={pane.sortKey === s.key ? (pane.ascending ? 'ascending' : 'descending') : 'none'}>
        <button type="button" class="col {s.cls}" class:on={pane.sortKey === s.key} onclick={() => pane.sortBy(s.key)}>
          {s.label}
          {#if pane.sortKey === s.key}<span class="arrow" class:desc={!pane.ascending}><ChevronDown size={12} /></span>{/if}
        </button>
      </div>
    {/each}
  </div>

  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    class="list"
    class:remote
    role="grid"
    tabindex="0"
    aria-label="{title} files"
    aria-rowcount={pane.rows.length}
    bind:this={listEl}
    onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}
    onkeydown={onKey}
    oncontextmenu={blankMenu}
    onclick={(e) => {
      if (e.target === e.currentTarget || (e.target as HTMLElement).classList.contains('spacer')) pane.clearSelection();
    }}
  >
    <div class="spacer" style:height="{total * rowH + 6}px">
      {#if creating && pane.editing}
        <div class="row editing" style:top="0px" style:height="{rowH}px">
          <div class="cell c-name">
            <span class="ico folder"><FolderPlus size={16} /></span>
            <input
              class="rename"
              class:bad={!!pane.editing.error}
              bind:value={pane.editing.value}
              use:focusName
              spellcheck="false"
              autocomplete="off"
              aria-label="New folder name"
              onkeydown={(e) => {
                e.stopPropagation();
                if (e.key === 'Enter') void pane.commitEdit();
                else if (e.key === 'Escape') pane.cancelEdit();
              }}
              onblur={() => void pane.commitEdit()}
            />
            {#if pane.editing.error}<span class="err">{pane.editing.error}</span>{/if}
          </div>
        </div>
      {/if}

      {#each slice as { item, index } (item.path)}
        {@const g = glyphFor(item)}
        {@const Icon = g.icon}
        {@const selected = pane.selection.has(item.path)}
        {@const renaming = pane.editing?.path === item.path}
        <div
          class="row"
          class:selected
          class:cursor={active && pane.cursor === item.path}
          class:hidden={item.hidden}
          role="row"
          tabindex="-1"
          aria-selected={selected}
          data-path={item.path}
          style:top="{(index + offset) * rowH}px"
          style:height="{rowH}px"
          onclick={(e) => pane.click(item.path, { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey })}
          ondblclick={() => activate(item)}
          onkeydown={() => {}}
          oncontextmenu={(e) => rowMenu(e, item)}
          use:draggable={{ payload: () => dragPayload(item), disabled: renaming }}
          use:dropTarget={{ accepts: (p) => !!item.isDir && canDrop(p, item), ondrop: (p) => drop(p, item), holdMs: 700, onhold: () => void pane.open(item.path) }}
        >
          <div class="cell c-name">
            <span class="ico {g.tone}"><Icon size={16} /></span>
            {#if renaming && pane.editing}
              <input
                class="rename"
                class:bad={!!pane.editing.error}
                bind:value={pane.editing.value}
                use:focusName
                spellcheck="false"
                autocomplete="off"
                aria-label="New name"
                onclick={(e) => e.stopPropagation()}
                ondblclick={(e) => e.stopPropagation()}
                onkeydown={(e) => {
                  e.stopPropagation();
                  if (e.key === 'Enter') void pane.commitEdit();
                  else if (e.key === 'Escape') pane.cancelEdit();
                }}
                onblur={() => void pane.commitEdit()}
              />
              {#if pane.editing.error}<span class="err">{pane.editing.error}</span>{/if}
            {:else}
              <span class="fname">{item.name}</span>
              {#if item.isSymlink}<span class="link" use:tooltip={'Symbolic link'}>↗</span>{/if}
            {/if}
          </div>
          <div class="cell c-size">{item.isDir ? '' : formatBytes(item.size)}</div>
          <div class="cell c-date">{item.special === 'drive' ? '' : formatDateTime(item.modified)}</div>
          {#if remote}
            <div class="cell c-mode" use:tooltip={item.owner ? `${item.owner}:${item.group ?? ''}` : null}>{item.mode != null ? formatMode(item.mode) : ''}</div>
          {/if}
        </div>
      {/each}
    </div>

    {#if pane.loading && pane.rows.length === 0}
      <div class="skeleton" aria-hidden="true">
        {#each Array(9) as _, i (i)}<div class="sk" style:--w="{38 + ((i * 17) % 40)}%"><i></i><b></b></div>{/each}
      </div>
    {:else if pane.error}
      <div class="state" role="alert">
        <span class="bad"><TriangleAlert size={22} /></span>
        <strong>{pane.error.title}</strong>
        <p>{pane.error.message}</p>
        {#if pane.error.hint}<p class="hint">{pane.error.hint}</p>{/if}
        <div class="row-actions">
          <Button size="sm" variant="primary" onclick={() => void pane.refresh()}>Try again</Button>
          {#if ops.parent(pane.cwd) !== null}<Button size="sm" onclick={() => void pane.up()}>Go up</Button>{/if}
        </div>
      </div>
    {:else if pane.rows.length === 0 && !pane.loading}
      <div class="state quiet">
        {#if pane.filter}
          <strong>No matches</strong>
          <p>Nothing here matches “{pane.filter}”.</p>
        {:else}
          <span class="soft"><FilePlus2 size={22} /></span>
          <strong>This folder is empty</strong>
          <p>{remote ? 'Drag files here, or drop them from Explorer, to upload.' : 'Nothing to show here.'}</p>
        {/if}
      </div>
    {/if}

    <div class="dropcue" aria-hidden="true">
      <span>{remote ? 'Drop to upload to' : 'Drop to download to'} <b>{ops.base(pane.cwd) || pane.provider.rootLabel}</b></span>
    </div>

    {#if offline}
      <div class="offline-cover" transition:fade={{ duration: 160 }}>
        <span class="pulse"></span>
        <strong>Connection unavailable</strong>
        <p>Files will be back as soon as the session reconnects.</p>
      </div>
    {/if}
  </div>

  <footer>
    <span>{selSummary}</span>
    {#if pane.disk}<span class="free">{formatBytes(pane.disk.free)} free of {formatBytes(pane.disk.total)}</span>{/if}
  </footer>
</section>

<style>
  .pane {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    overflow: hidden;
    transition: border-color var(--dur-fast);
  }
  .pane.active {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border-strong));
  }
  header {
    display: flex;
    flex-direction: column;
    gap: 7px;
    padding: 9px 10px 8px;
    border-bottom: 1px solid var(--border);
  }
  .title {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0 2px;
    font-size: var(--fs-xs);
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
  .active .title {
    color: var(--accent);
  }
  .side {
    display: inline-flex;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .live {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--success);
    box-shadow: 0 0 8px var(--success);
  }
  .live.off {
    background: var(--danger);
    box-shadow: none;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .filter {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 28px;
    padding: 0 8px;
    background: var(--bg-input);
    border: 1px solid var(--accent);
    border-radius: var(--r-md);
    box-shadow: 0 0 0 3px var(--accent-soft);
    color: var(--accent);
  }
  .filter input {
    flex: 1;
    min-width: 0;
    background: none;
    border: 0;
    outline: none;
    color: var(--text);
    cursor: text;
  }
  .filter button {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: 4px;
    color: var(--text-faint);
  }
  .filter button:hover {
    background: var(--bg-active);
    color: var(--text);
  }

  .cols {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 84px 128px;
    padding: 0 8px;
    border-bottom: 1px solid var(--border);
    background: color-mix(in srgb, var(--bg-surface) 70%, var(--bg-app));
  }
  .pane:has(.list.remote) .cols {
    grid-template-columns: minmax(0, 1fr) 84px 128px 96px;
  }
  /* Carries the ARIA header role and sort state; the button inside is the grid item. */
  .colhead {
    display: contents;
  }
  .col {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 26px;
    padding: 0 8px;
    font-size: var(--fs-xs);
    font-weight: 650;
    color: var(--text-faint);
    text-align: left;
  }
  .col:hover,
  .col.on {
    color: var(--text);
  }
  .c-size,
  .c-date,
  .c-mode {
    justify-content: flex-start;
  }
  .c-size {
    justify-content: flex-end;
  }
  .arrow {
    display: inline-flex;
    transition: transform var(--dur-fast);
  }
  .arrow.desc {
    transform: rotate(180deg);
  }

  .list {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    outline: none;
  }
  .spacer {
    position: relative;
    width: 100%;
  }
  .row {
    position: absolute;
    left: 4px;
    right: 4px;
    display: grid;
    grid-template-columns: minmax(0, 1fr) 84px 128px;
    align-items: center;
    padding: 0 4px;
    border-radius: var(--r-sm);
    color: var(--text);
    font-size: var(--fs-md);
    cursor: default;
  }
  .list.remote .row {
    grid-template-columns: minmax(0, 1fr) 84px 128px 96px;
  }
  .row:hover {
    background: var(--bg-hover);
  }
  .row.selected {
    background: var(--accent-soft);
  }
  .row.cursor {
    box-shadow: inset 0 0 0 1px var(--accent-ring);
  }
  .row.hidden .fname {
    opacity: 0.6;
  }
  .cell {
    min-width: 0;
    padding: 0 8px 0 4px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .c-name {
    display: flex;
    align-items: center;
    gap: 9px;
  }
  .row .c-size {
    text-align: right;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    font-size: var(--fs-sm);
  }
  .row .c-date {
    color: var(--text-dim);
    font-size: var(--fs-sm);
  }
  .row .c-mode {
    color: var(--text-faint);
    font: 11px var(--font-mono);
  }
  .fname {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .link {
    color: var(--text-faint);
    font-size: var(--fs-xs);
  }
  .ico {
    display: inline-flex;
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .ico.folder {
    color: var(--accent);
  }
  .ico.drive {
    color: var(--accent-2);
  }
  .ico.code {
    color: #6fd0e8;
  }
  .ico.config {
    color: var(--warning);
  }
  .ico.media {
    color: #e08ad8;
  }
  .ico.archive {
    color: #e0a35c;
  }
  .ico.key {
    color: var(--success);
  }
  .ico.exec {
    color: var(--success);
  }
  .ico.doc {
    color: var(--text);
  }
  .rename {
    flex: 1;
    min-width: 0;
    height: 22px;
    padding: 0 6px;
    background: var(--bg-input);
    border: 1px solid var(--accent);
    border-radius: 5px;
    outline: none;
    color: var(--text);
    font: inherit;
    cursor: text;
  }
  .rename.bad {
    border-color: var(--danger);
  }
  .err {
    flex-shrink: 0;
    color: var(--danger);
    font-size: var(--fs-xs);
  }

  .skeleton {
    position: absolute;
    inset: 6px 8px;
    pointer-events: none;
  }
  .sk {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 30px;
    padding: 0 6px;
  }
  .sk i {
    width: 16px;
    height: 16px;
    border-radius: 5px;
    background: var(--bg-active);
  }
  .sk b {
    width: var(--w);
    height: 9px;
    border-radius: 5px;
    background: linear-gradient(90deg, var(--bg-active), var(--bg-hover), var(--bg-active));
    background-size: 200% 100%;
    animation: shimmer 1.4s linear infinite;
  }
  .state {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 24px;
    text-align: center;
    pointer-events: none;
  }
  .state.quiet {
    color: var(--text-faint);
  }
  .state strong {
    color: var(--text);
  }
  .state p {
    max-width: 320px;
    color: var(--text-dim);
    font-size: var(--fs-sm);
    line-height: 1.5;
  }
  .state .hint {
    color: var(--text);
  }
  .state .bad {
    color: var(--danger);
  }
  .state .soft {
    color: var(--text-faint);
  }
  .row-actions {
    display: flex;
    gap: 8px;
    margin-top: 10px;
    pointer-events: auto;
  }
  .dropcue {
    position: absolute;
    inset: 6px;
    display: none;
    place-items: center;
    border: 2px dashed var(--accent);
    border-radius: var(--r-md);
    background: color-mix(in srgb, var(--bg-surface) 82%, transparent);
    color: var(--text-dim);
    pointer-events: none;
    z-index: 3;
  }
  .dropcue b {
    color: var(--accent);
  }
  .pane:global([data-drop-over='true']) .dropcue,
  .pane.os-drop .dropcue {
    display: grid;
  }
  .row:global([data-drop-over='true']) {
    background: var(--accent-soft);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .offline-cover {
    position: absolute;
    inset: 0;
    z-index: 4;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    background: color-mix(in srgb, var(--bg-surface) 88%, transparent);
    backdrop-filter: blur(3px);
    text-align: center;
  }
  .offline-cover p {
    color: var(--text-dim);
    font-size: var(--fs-sm);
  }
  .pulse {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--warning);
    animation: pulse-dot 1.1s var(--ease-in-out) infinite;
  }
  footer {
    display: flex;
    justify-content: space-between;
    gap: 10px;
    height: 26px;
    align-items: center;
    padding: 0 12px;
    border-top: 1px solid var(--border);
    font-size: var(--fs-xs);
    color: var(--text-faint);
    font-variant-numeric: tabular-nums;
  }
  :global(.spin) {
    animation: spin 0.9s linear infinite;
  }
</style>
