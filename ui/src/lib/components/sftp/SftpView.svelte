<script lang="ts">
  import { ArrowLeft, ArrowRight } from '@lucide/svelte';
  import { onMount, untrack } from 'svelte';
  import { SftpController } from '$lib/sftp/controller.svelte';
  import type { FileItem } from '$lib/sftp/provider';
  import { filedrop } from '$lib/stores/filedrop.svelte';
  import { sessions } from '$lib/stores/sessions.svelte';
  import { settings } from '$lib/stores/settings.svelte';
  import { transfers } from '$lib/stores/transfers.svelte';
  import { workspace, type SftpTab } from '$lib/stores/workspace.svelte';
  import { tooltip } from '$lib/dom/tooltip';
  import ConflictDialog from './ConflictDialog.svelte';
  import FilePane from './FilePane.svelte';
  import PermissionsDialog from './PermissionsDialog.svelte';
  import TextEditorDialog from './TextEditorDialog.svelte';
  import TransferPanel from './TransferPanel.svelte';

  let { tab, visible }: { tab: SftpTab; visible: boolean } = $props();

  const ctl = new SftpController(untrack(() => tab.connectionId));
  const view = $derived(sessions.view(tab.connectionId));
  const connected = $derived(view?.state.type === 'connected');
  const hostLabel = $derived(tab.host.label || tab.host.hostname);

  let editing = $state<FileItem | null>(null);
  let permsFor = $state<FileItem[] | null>(null);

  onMount(() => {
    void ctl.init();
    // Files dropped from Explorer onto the server pane (or onto a folder in it) are uploaded.
    return filedrop.register((paths, x, y) => {
      if (!visible || workspace.activeId !== tab.id || paths.length === 0) return false;
      const target = document.elementFromPoint(x, y);
      if (!target?.closest('[data-pane="remote"]')) return false;
      const rowPath = target.closest<HTMLElement>('.row')?.dataset.path;
      const dir = rowPath ? ctl.remote.rows.find((r) => r.path === rowPath && r.isDir)?.path : undefined;
      void ctl.upload(paths, dir ?? ctl.remote.cwd);
      return true;
    });
  });

  // The "show hidden files" setting applies to both panes.
  $effect(() => {
    const show = settings.value.sftp.showHidden;
    ctl.local.showHidden = show;
    ctl.remote.showHidden = show;
  });

  // Reload the server side when the connection comes back after a drop.
  let wasConnected = false;
  $effect(() => {
    if (connected && !wasConnected) {
      wasConnected = true;
      if (untrack(() => ctl.remote.cwd)) void untrack(() => ctl.remote.refresh());
    } else if (!connected) wasConnected = false;
  });

  // Refresh what's on screen when transfers finish, so new files simply appear.
  let finishedSeen = 0;
  $effect(() => {
    const done = transfers.list.filter((t) => t.state === 'completed').length;
    if (done > finishedSeen) {
      finishedSeen = done;
      const timer = setTimeout(() => {
        void ctl.remote.refresh();
        void ctl.local.refresh();
      }, 300);
      return () => clearTimeout(timer);
    }
    finishedSeen = done;
  });

  // Which pane the Explorer drag is over, for the highlight.
  const osHover = $derived.by(() => {
    if (!filedrop.over || !visible) return null;
    const el = document.elementFromPoint(filedrop.x, filedrop.y);
    return el?.closest<HTMLElement>('[data-pane]')?.dataset.pane ?? null;
  });
</script>

<div class="sftp" class:hidden={!visible}>
  <div class="panes">
    <FilePane {ctl} side="local" title="This PC" />

    <div class="rail" aria-label="Transfer between the two sides">
      <button type="button" class="go up" aria-label="Upload the selected files" use:tooltip={{ text: 'Upload selection', placement: 'right' }} disabled={!connected || ctl.local.selected.length === 0} onclick={() => void ctl.sendSelection('local')}>
        <ArrowRight size={18} />
      </button>
      <button type="button" class="go down" aria-label="Download the selected files" use:tooltip={{ text: 'Download selection', placement: 'right' }} disabled={!connected || ctl.remote.selected.length === 0} onclick={() => void ctl.sendSelection('remote')}>
        <ArrowLeft size={18} />
      </button>
    </div>

    <FilePane
      {ctl}
      side="remote"
      title={hostLabel}
      offline={!connected}
      osDrop={osHover === 'remote'}
      onedit={(item) => (editing = item)}
      onperms={(items) => (permsFor = items)}
    />
  </div>

  <TransferPanel {ctl} />
</div>

<ConflictDialog {ctl} />
{#if editing}
  <TextEditorDialog connectionId={tab.connectionId} item={editing} onclose={() => (editing = null)} onsaved={() => void ctl.remote.refresh()} />
{/if}
{#if permsFor}
  <PermissionsDialog connectionId={tab.connectionId} items={permsFor} onclose={() => (permsFor = null)} ondone={() => { permsFor = null; void ctl.remote.refresh(); }} />
{/if}

<style>
  .sftp {
    position: absolute;
    inset: 0;
    display: grid;
    grid-template-rows: minmax(0, 1fr) auto;
    gap: 10px;
    padding: 12px;
    background: var(--bg-app);
  }
  .panes {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 44px minmax(0, 1fr);
    gap: 6px;
    min-height: 0;
  }
  .rail {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
  }
  .go {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 12px;
    background: var(--bg-raised);
    border: 1px solid var(--border-strong);
    color: var(--text-dim);
    transition:
      background var(--dur-fast),
      color var(--dur-fast),
      transform var(--dur) var(--ease-spring),
      border-color var(--dur-fast);
  }
  .go:hover:not(:disabled) {
    background: var(--grad-accent);
    border-color: transparent;
    color: var(--on-accent);
    transform: scale(1.1);
  }
  .go:active:not(:disabled) {
    transform: scale(0.94);
  }
  .go:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }
  .hidden {
    pointer-events: none;
  }
</style>
