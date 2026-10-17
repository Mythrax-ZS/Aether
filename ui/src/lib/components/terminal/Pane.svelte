<script lang="ts">
  import { onMount } from 'svelte';
  import { ClipboardPaste, Columns2, Copy, Eraser, Files, ListChecks, RefreshCw, Rows2, Search, X } from '@lucide/svelte';
  import { api } from '$lib/api';
  import { closeActivePane, openSftpForActive, reconnectPane } from '$lib/actions';
  import { keymap } from '$lib/commands.svelte';
  import type { TerminalController } from '$lib/terminal/controller';
  import { settings } from '$lib/stores/settings.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import { workspace } from '$lib/stores/workspace.svelte';
  import type { Leaf } from '$lib/util/panes';
  import ConnectionOverlay from './ConnectionOverlay.svelte';
  import FindBar from './FindBar.svelte';
  import TerminalView from './TerminalView.svelte';

  type Props = {
    leaf: Leaf;
    /** The tab is on screen. */
    visible: boolean;
    focused: boolean;
    /** The tab holds more than one pane. */
    multi: boolean;
  };

  let { leaf, visible, focused, multi }: Props = $props();

  let controller = $state<TerminalController>();
  let findOpen = $state(false);
  let search = $state({ index: -1, count: 0 });

  onMount(() => {
    const open = () => {
      if (visible && focused) findOpen = true;
    };
    window.addEventListener('aether:find', open);
    return () => window.removeEventListener('aether:find', open);
  });

  function focusMe() {
    workspace.focusPane(leaf.id);
  }

  function onKeyCapture(e: KeyboardEvent) {
    // A finished shell restarts on Enter (the pane stays so you can read what was printed).
    if (e.key !== 'Enter' || e.ctrlKey || e.altKey || e.shiftKey || e.metaKey) return;
    if (workspace.info[leaf.id]?.terminal?.type !== 'exited' || !leaf.connectionId || !leaf.terminalId) return;
    e.preventDefault();
    e.stopPropagation();
    void api.sessions.restartTerminal(leaf.connectionId, leaf.terminalId).catch((err) => ui.error(err));
  }

  function onContext(e: MouseEvent) {
    const c = controller;
    if (!c) return;
    e.preventDefault();
    focusMe();
    if (settings.value.terminal.rightClick === 'paste') {
      if (c.hasSelection()) c.copy();
      else void c.paste();
      return;
    }
    ui.showMenu(e, [
      { label: 'Copy', icon: Copy, shortcut: keymap.display('term.copy') ?? undefined, disabled: !c.hasSelection(), run: () => c.copy() },
      { label: 'Paste', icon: ClipboardPaste, shortcut: keymap.display('term.paste') ?? undefined, run: () => void c.paste() },
      { label: 'Select all', icon: ListChecks, shortcut: keymap.display('term.selectAll') ?? undefined, run: () => c.selectAll() },
      { separator: true },
      { label: 'Find…', icon: Search, shortcut: keymap.display('term.find') ?? undefined, run: () => (findOpen = true) },
      { label: 'Clear scrollback', icon: Eraser, shortcut: keymap.display('term.clear') ?? undefined, run: () => c.clear() },
      { separator: true },
      { label: 'Split right', icon: Columns2, shortcut: keymap.display('pane.splitRight') ?? undefined, run: () => workspace.splitActive('row') },
      { label: 'Split down', icon: Rows2, shortcut: keymap.display('pane.splitDown') ?? undefined, run: () => workspace.splitActive('col') },
      { separator: true },
      { label: 'Browse files', icon: Files, shortcut: keymap.display('session.sftp') ?? undefined, disabled: !leaf.connectionId, run: openSftpForActive },
      { label: 'Reconnect', icon: RefreshCw, shortcut: keymap.display('session.reconnect') ?? undefined, disabled: !leaf.connectionId, run: () => void reconnectPane(leaf.id) },
      { separator: true },
      { label: 'Close pane', icon: X, shortcut: keymap.display('pane.close') ?? undefined, run: () => void closeActivePane() },
    ]);
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="pane"
  class:focused={focused && multi}
  class:dim={multi && !focused}
  data-pane-id={leaf.id}
  onpointerdowncapture={focusMe}
  onkeydowncapture={onKeyCapture}
  oncontextmenu={onContext}
>
  <TerminalView
    {leaf}
    {visible}
    {focused}
    bind:controller
    onsearch={(index, count) => (search = { index, count })}
  />
  {#if findOpen}
    <FindBar {controller} index={search.index} count={search.count} onclose={() => (findOpen = false)} />
  {/if}
  <ConnectionOverlay {leaf} {visible} />
</div>

<style>
  .pane {
    position: absolute;
    inset: 0;
    overflow: hidden;
  }
  .pane.dim::after {
    content: '';
    position: absolute;
    inset: 0;
    z-index: 4;
    pointer-events: none;
    background: rgb(0 0 0 / 0.16);
  }
  :global(:root[data-theme-kind='light']) .pane.dim::after {
    background: rgb(255 255 255 / 0.28);
  }
  .pane.focused::before {
    content: '';
    position: absolute;
    inset: 0;
    z-index: 4;
    pointer-events: none;
    box-shadow: inset 0 0 0 1px var(--accent-ring);
  }
</style>
