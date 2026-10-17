<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api } from '$lib/api';
  import { confirmWindowClose, connectHostId, connectTarget } from '$lib/actions';
  import { installKeyHandler } from '$lib/commands.svelte';
  import { installLockdown } from '$lib/dom/lockdown';
  import { findTheme } from '$lib/themes/builtin';
  import { commandLine, isSshCommand } from '$lib/util/cmdline';
  import { whenPainted } from '$lib/util/reveal';
  import { filedrop } from '$lib/stores/filedrop.svelte';
  import { hosts } from '$lib/stores/hosts.svelte';
  import { prompts } from '$lib/stores/prompts.svelte';
  import { sessions } from '$lib/stores/sessions.svelte';
  import { settings } from '$lib/stores/settings.svelte';
  import { transfers } from '$lib/stores/transfers.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import { workspace } from '$lib/stores/workspace.svelte';
  import CommandPalette from '$lib/components/palette/CommandPalette.svelte';
  import ConfirmDialog from '$lib/components/dialogs/ConfirmDialog.svelte';
  import ContextMenu from '$lib/components/ui/ContextMenu.svelte';
  import DialogHost from '$lib/components/dialogs/DialogHost.svelte';
  import DragGhost from '$lib/components/ui/DragGhost.svelte';
  import DropOverlay from '$lib/components/shell/DropOverlay.svelte';
  import PromptDialog from '$lib/components/dialogs/PromptDialog.svelte';
  import Sidebar from '$lib/components/shell/Sidebar.svelte';
  import Stage from '$lib/components/shell/Stage.svelte';
  import StatusBar from '$lib/components/shell/StatusBar.svelte';
  import TextPromptDialog from '$lib/components/dialogs/TextPromptDialog.svelte';
  import Titlebar from '$lib/components/shell/Titlebar.svelte';
  import Toasts from '$lib/components/ui/Toasts.svelte';

  let booted = $state(false);
  /** Decorative animation pauses after this long without input. */
  const IDLE_AFTER_MS = 20_000;
  /** Tabs are on disk (so turning "restore" off must clear them). */
  let savedTabs = false;
  const isMain = api.platform.windowLabel === 'main';

  // The stage takes the colour of whatever is in front, so the active tab flows into it.
  const stageBg = $derived.by(() => {
    const tab = workspace.active;
    if (!tab) return 'var(--bg-app)';
    if (tab.kind === 'sftp') return 'var(--bg-surface)';
    const override = workspace.activeLeaf?.host?.terminal.themeId;
    return ((override && findTheme(override, settings.customThemes)) || settings.terminalTheme).terminal.background;
  });

  onMount(() => {
    const disposers: (() => void)[] = [installKeyHandler(), installLockdown()];
    let alive = true;

    const track = async <T extends () => void>(p: Promise<T>) => {
      const off = await p;
      if (alive) disposers.push(off);
      else off();
    };

    void boot(track);

    return () => {
      alive = false;
      disposers.forEach((d) => d());
    };
  });

  async function boot(track: <T extends () => void>(p: Promise<T>) => Promise<void>) {
    try {
      await Promise.all([settings.init(), hosts.init(), sessions.init(), prompts.init(), transfers.init(), filedrop.init()]);
      ui.sidebarOpen = !settings.value.appearance.sidebarCollapsed;

      // Window state → UI.
      const w = api.platform.window;
      ui.maximized = await w.isMaximized().catch(() => false);
      ui.windowFocused = await w.isFocused().catch(() => true);
      ui.fullscreen = await w.isFullscreen().catch(() => false);
      await track(w.onMaximizeChange((m) => (ui.maximized = m)));
      await track(w.onFocusChange((f) => (ui.windowFocused = f)));

      // Closing the window by any route asks first when live connections would be dropped.
      let asking = false;
      await track(
        w.onCloseRequested(async (prevent) => {
          // With "keep running in the tray" the main window is only hidden (the shell does that), so nothing is lost.
          if (isMain && settings.value.general.closeToTray) return prevent();
          if (asking) return prevent(); // a second Alt+F4 while the question is open
          asking = true;
          try {
            if (!(await confirmWindowClose())) prevent();
          } finally {
            asking = false;
          }
        }),
      );

      // Things other processes ask for.
      await track(api.on('tray:connect', (e) => connectHostId(e.hostId)));
      await track(
        api.on('app:args', (e) => {
          // `aether ssh -J bastion me@db` is one command, to be read as one — not as three destinations.
          if (isSshCommand(e.args)) return connectTarget(commandLine(e.args));
          for (const arg of e.args) {
            if (arg.startsWith('-')) continue;
            connectTarget(arg.replace(/^ssh:\/\//i, '').replace(/\/+$/, ''));
          }
        }),
      );

      // A tab dragged out of another window arrives with its session already running.
      const params = new URLSearchParams(location.search);
      const adopt = params.get('adopt');
      const terminal = params.get('terminal');
      if (adopt && terminal) {
        try {
          await api.sessions.adopt(adopt);
          const summary = (await api.sessions.list()).find((s) => s.id === adopt);
          if (summary) workspace.adopt(summary, terminal, params.get('host'));
        } catch (e) {
          ui.error(e, 'Couldn’t move the tab');
        }
      } else if (isMain && settings.value.general.startup === 'restore') {
        workspace.restore(await api.settings.loadWorkspace().catch(() => null));
      } else if (isMain) {
        // Not restoring: a snapshot left over from when the setting was on gets cleared (see below).
        const old = (await api.settings.loadWorkspace().catch(() => null)) as { tabs?: unknown[] } | null;
        if (old?.tabs?.length) savedTabs = true;
      }

      // Problems found at startup (e.g. a damaged file that was recovered).
      const info = await api.app.info().catch(() => null);
      for (const warning of info?.warnings ?? []) ui.toast({ kind: 'warning', title: warning.title, message: warning.message, hint: warning.hint, detail: warning.detail, timeout: 0 });

      if (!settings.value.general.onboardingCompleted && isMain && !adopt) ui.openDialog({ type: 'onboarding' });
    } catch (e) {
      console.error('startup failed', e);
      ui.error(e, 'Aether had trouble starting');
    } finally {
      booted = true;
      await tick();
      // Reveal the window only once there is something painted to show.
      whenPainted(() => void api.app.ready());
    }
  }

  // Remember the open tabs — main window only, and only when the user asked to restore them. Turning the
  // setting off clears what was saved, so a list of hosts doesn't linger on disk against their wishes.
  $effect(() => {
    if (!booted || !isMain) return;
    const restoring = settings.value.general.startup === 'restore';
    if (!restoring && !savedTabs) return;
    const snapshot = restoring ? workspace.snapshot() : { version: 1, tabs: [], active: 0 };
    const timer = setTimeout(() => {
      savedTabs = restoring;
      void api.settings.saveWorkspace(snapshot).catch(() => {});
    }, 700);
    return () => clearTimeout(timer);
  });

  // The taskbar and Alt+Tab show what you're looking at.
  $effect(() => {
    const tab = workspace.active;
    void api.platform.window.setTitle(tab ? `${workspace.tabTitle(tab)} — Aether` : 'Aether').catch(() => {});
  });

  // Lets the stylesheet rest animations while nobody is looking (see base.css): window not in front, or no input for a while.
  $effect(() => {
    document.documentElement.dataset.focus = ui.windowFocused ? 'in' : 'out';
  });
  $effect(() => {
    const root = document.documentElement;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const wake = () => {
      if (root.dataset.idle !== 'false') root.dataset.idle = 'false';
      clearTimeout(timer);
      timer = setTimeout(() => (root.dataset.idle = 'true'), IDLE_AFTER_MS);
    };
    const events = ['pointermove', 'pointerdown', 'keydown', 'wheel'] as const;
    for (const e of events) window.addEventListener(e, wake, { passive: true, capture: true });
    wake();
    return () => {
      clearTimeout(timer);
      for (const e of events) window.removeEventListener(e, wake, { capture: true });
    };
  });

  // Keep the live indicators honest when a session drops while you're elsewhere.
  $effect(() => {
    if (!booted || !settings.value.notifications.onDisconnect) return;
    for (const [id, view] of Object.entries(sessions.views)) {
      if (view.state.type !== 'disconnected' || !view.everConnected) continue;
      if (notified.has(id)) continue;
      notified.add(id);
      if (!ui.windowFocused) {
        const leaf = workspace.allLeaves().find((l) => l.connectionId === id);
        void api.platform.notify('Connection lost', leaf?.host?.label ?? leaf?.target ?? 'A session ended').catch(() => {});
      }
    }
  });
  const notified = new Set<string>();

  function onUnhandled(e: PromiseRejectionEvent) {
    console.error('unhandled rejection', e.reason);
  }
</script>

<svelte:window onunhandledrejection={onUnhandled} />

<div class="app" class:inactive={!ui.windowFocused} class:maximized={ui.maximized} class:fullscreen={ui.fullscreen} style:--stage-bg={stageBg}>
  <Titlebar />
  <Sidebar />
  <Stage />
  {#if settings.value.appearance.showStatusBar}<StatusBar />{/if}
</div>

<DialogHost />
<PromptDialog />
<ConfirmDialog />
<TextPromptDialog />
<CommandPalette />
<ContextMenu />
<DropOverlay />
<DragGhost />
<Toasts />

<style>
  .app {
    display: grid;
    height: 100%;
    grid-template-columns: auto minmax(0, 1fr);
    grid-template-rows: var(--titlebar-h) minmax(0, 1fr) auto;
    grid-template-areas:
      'titlebar titlebar'
      'sidebar stage'
      'status status';
    background: var(--bg-sidebar);
    color: var(--text);
  }
  /* Quietly dim the chrome when another window has focus. */
  .app.inactive :global(.titlebar) {
    opacity: 0.72;
  }
  .app :global(.titlebar) {
    transition: opacity var(--dur);
  }
</style>
