<script lang="ts">
  import { PanelLeft, Search } from '@lucide/svelte';
  import { toggleSidebar } from '$lib/actions';
  import { keymap } from '$lib/commands.svelte';
  import { tooltip } from '$lib/dom/tooltip';
  import { settings } from '$lib/stores/settings.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import Kbd from '$lib/components/ui/Kbd.svelte';
  import Logo from '$lib/components/ui/Logo.svelte';
  import TabStrip from './TabStrip.svelte';
  import WindowControls from './WindowControls.svelte';

  // The left cluster is as wide as the sidebar so the first tab lines up with the stage.
  const leftWidth = $derived(ui.sidebarOpen ? settings.value.appearance.sidebarWidth : 96);
</script>

<header class="titlebar" data-tauri-drag-region>
  <div class="left" style:width="{leftWidth}px" data-tauri-drag-region>
    <span class="logo" data-tauri-drag-region><Logo size={22} /></span>
    <button
      type="button"
      class="toggle"
      class:off={!ui.sidebarOpen}
      aria-label="Toggle sidebar"
      aria-pressed={ui.sidebarOpen}
      onclick={toggleSidebar}
      use:tooltip={{ text: ui.sidebarOpen ? 'Hide sidebar' : 'Show sidebar', shortcut: keymap.display('app.sidebar'), placement: 'bottom' }}
    >
      <PanelLeft size={17} />
    </button>
  </div>

  <TabStrip />

  <div class="spacer" data-tauri-drag-region></div>

  <button type="button" class="palette" onclick={() => ui.openPalette()} aria-label="Open command palette">
    <Search size={14} />
    <span class="hint">Search or connect</span>
    <Kbd keys={keymap.display('palette.open')} subtle />
  </button>

  {#if !ui.fullscreen}<WindowControls />{/if}
</header>

<style>
  .titlebar {
    grid-area: titlebar;
    position: relative;
    z-index: var(--z-titlebar);
    display: flex;
    align-items: flex-end;
    height: var(--titlebar-h);
    background: var(--bg-sidebar);
    user-select: none;
  }
  .left {
    display: flex;
    align-items: center;
    align-self: stretch;
    flex-shrink: 0;
    gap: 6px;
    padding-left: 14px;
    overflow: hidden;
    transition: width var(--dur-slow) var(--ease-out);
  }
  .logo {
    display: grid;
    place-items: center;
    filter: drop-shadow(0 2px 8px rgb(111 140 255 / 0.35));
  }
  .toggle {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: var(--r-md);
    color: var(--text-dim);
    transition:
      background var(--dur-fast),
      color var(--dur-fast);
  }
  .toggle:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .toggle:not(.off) {
    color: var(--text);
  }
  .spacer {
    flex: 1;
    align-self: stretch;
    min-width: 24px;
  }
  .palette {
    display: inline-flex;
    flex-shrink: 0;
    align-items: center;
    align-self: center;
    white-space: nowrap;
    gap: 8px;
    height: 28px;
    padding: 0 8px 0 10px;
    margin-left: 8px;
    border-radius: var(--r-md);
    background: var(--bg-input);
    border: 1px solid var(--border);
    color: var(--text-faint);
    font-size: var(--fs-sm);
    transition:
      border-color var(--dur-fast),
      color var(--dur-fast);
  }
  .palette:hover {
    border-color: var(--border-strong);
    color: var(--text-dim);
  }
  @media (max-width: 1000px) {
    .hint {
      display: none;
    }
  }
</style>
