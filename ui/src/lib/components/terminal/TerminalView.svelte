<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { api } from '$lib/api';
  import type { TerminalSettings, TermSize, Theme } from '$lib/bindings';
  import { isAppShortcut } from '$lib/commands.svelte';
  import { zoomTerminal } from '$lib/actions';
  import { TerminalController } from '$lib/terminal/controller';
  import { terminalRegistry } from '$lib/terminal/registry';
  import { plainText, Throttle } from '$lib/terminal/safety';
  import { beep } from '$lib/util/beep';
  import { findTheme } from '$lib/themes/builtin';
  import { prompts } from '$lib/stores/prompts.svelte';
  import { settings } from '$lib/stores/settings.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import { workspace } from '$lib/stores/workspace.svelte';
  import type { Leaf } from '$lib/util/panes';

  type Props = {
    leaf: Leaf;
    /** The tab is on screen. Hidden terminals skip rendering work. */
    visible: boolean;
    /** This is the focused pane. */
    focused: boolean;
    onsearch?: (index: number, count: number) => void;
    onselection?: (has: boolean) => void;
    /** The xterm wrapper, once created (the pane uses it for the context menu and find bar). */
    controller?: TerminalController | undefined;
  };

  let { leaf, visible, focused, onsearch, onselection, controller = $bindable() }: Props = $props();

  let host: HTMLDivElement;
  let bellFlash = $state(0);
  const bellThrottle = new Throttle(500);
  const bellNotifyThrottle = new Throttle(15_000);
  /** The terminal has a real size, so a session can be opened for it. */
  let ready = $state<TermSize | null>(null);

  // Per-host overrides (theme/font) layered over the global terminal settings.
  const effective = $derived.by<TerminalSettings>(() => {
    const base = settings.value.terminal;
    const o = leaf.host?.terminal;
    if (!o) return base;
    return { ...base, fontFamily: o.fontFamily || base.fontFamily, fontSize: o.fontSize ?? base.fontSize };
  });
  const theme = $derived.by<Theme>(() => {
    const id = leaf.host?.terminal.themeId;
    return (id && findTheme(id, settings.customThemes)) || settings.terminalTheme;
  });

  onMount(() => {
    const info = () => workspace.paneInfo(leaf.id);
    const c = new TerminalController(host, effective, theme, {
      onReady: (size) => (ready = size),
      onTitle: (title) => (info().title = title.trim() || null),
      onCwd: (path) => (info().cwd = path),
      onBell: () => {
        // A program ringing the bell in a loop is a nuisance, not a signal: twice a second at most.
        if (!bellThrottle.allow()) return;
        const mode = settings.value.terminal.bell;
        if (mode === 'visual') bellFlash++;
        else if (mode === 'sound') beep();
        const tab = workspace.tabOfLeaf(leaf.id);
        if (!ui.windowFocused && tab && settings.value.notifications.onCommandFinished && bellNotifyThrottle.allow()) {
          void api.platform.notify('Terminal bell', plainText(workspace.tabTitle(tab), 80));
        }
      },
      onOutput: () => workspace.noteOutput(leaf.id),
      onState: (state) => (info().terminal = state),
      onCommandFinished: ({ exitCode, durationMs }) => {
        const n = settings.value.notifications;
        if (!n.onCommandFinished || durationMs < n.minCommandSecs * 1000) return;
        if (n.onlyWhenUnfocused && ui.windowFocused) return;
        const tab = workspace.tabOfLeaf(leaf.id);
        void api.platform.notify(exitCode === 0 || exitCode === null ? 'Command finished' : `Command failed (exit ${exitCode})`, tab ? workspace.tabTitle(tab) : undefined);
      },
      onNotify: (title, body) => {
        // Text chosen by a program on the server: allowed only if the person allows it, never while they are looking at
        // this very terminal, and always shown with the session it came from so it can't pass for something else.
        const n = settings.value.notifications;
        if (!n.fromServer) return;
        if (n.onlyWhenUnfocused && ui.windowFocused && visible && focused) return;
        const tab = workspace.tabOfLeaf(leaf.id);
        const source = tab ? plainText(workspace.tabTitle(tab), 60) : 'Terminal';
        void api.platform.notify(title ? `${source} — ${title}` : source, body);
      },
      onNotice: (message) => ui.info(message),
      onSelectionChange: (has) => onselection?.(has),
      onSearchResults: (i, n) => onsearch?.(i, n),
      onResize: (cols, rows) => (info().size = { cols, rows }),
      isAppShortcut,
      confirmPaste: async (text) => {
        const lines = text.replace(/[\r\n]+$/, '').split(/\r\n|\r|\n/);
        const preview = lines.slice(0, 5).join('\n') + (lines.length > 5 ? `\n… and ${lines.length - 5} more lines` : '');
        const r = await ui.confirmWithCheckbox({
          title: `Paste ${lines.length} lines?`,
          message: preview.length > 400 ? preview.slice(0, 400) + '…' : preview,
          confirmLabel: 'Paste',
          checkbox: 'Don’t ask again',
        });
        if (r.ok && r.checked) settings.patch({ terminal: { confirmMultilinePaste: false } });
        return r.ok;
      },
      onZoom: (delta) => zoomTerminal(delta),
    });
    controller = c;
    terminalRegistry.set(leaf.id, c);
    return () => {
      terminalRegistry.delete(leaf.id);
      c.dispose();
      controller = undefined;
    };
  });

  $effect(() => {
    controller?.apply(effective, theme);
  });

  $effect(() => {
    controller?.setVisible(visible);
  });

  // Whatever is modal (dialogs, palette, menus, prompts) owns the keyboard until it closes.
  const modalOpen = $derived(ui.dialogs.length > 0 || ui.palette.open || !!ui.menu || !!ui.confirmState || !!ui.textState || !!prompts.current);

  // The active pane takes the keyboard when it becomes active or its shell comes alive. Dialogs
  // opening or closing are deliberately *not* triggers: between two consecutive prompts there is
  // a moment with no dialog, and grabbing focus then would steal it from the next one.
  const alive = $derived(workspace.info[leaf.id]?.terminal?.type === 'running');
  $effect(() => {
    if (!visible || !focused || !controller) return;
    void alive;
    const c = controller;
    const t = setTimeout(() => {
      if (!untrack(() => modalOpen)) c.focus();
    }, 0);
    return () => clearTimeout(t);
  });

  // Open the session as soon as the terminal has a size — and again after a "try again" resets the pane.
  $effect(() => {
    const phase = workspace.info[leaf.id]?.phase;
    if (ready && controller && phase === 'idle' && leaf.open !== 'attach') void workspace.openLeaf(leaf.id, controller.size());
  });

  // Show the shell once the backend has one for this pane.
  $effect(() => {
    const id = leaf.terminalId;
    if (id && controller) void controller.attach(id);
  });

</script>

<div class="wrap" style:background={theme.terminal.background}>
  <div class="term" bind:this={host}></div>
  {#key bellFlash}
    {#if bellFlash > 0}<div class="bell"></div>{/if}
  {/key}
</div>

<style>
  .wrap {
    position: absolute;
    inset: 0;
    padding: 6px 4px 4px 10px;
    overflow: hidden;
  }
  .term {
    width: 100%;
    height: 100%;
  }
  .term :global(.xterm) {
    height: 100%;
  }
  .term :global(.xterm-viewport) {
    background: transparent !important;
    /* xterm draws its own scrollbar; the native one underneath must stay out of sight. */
    scrollbar-width: none;
  }
  .term :global(.xterm-viewport::-webkit-scrollbar) {
    display: none;
  }
  /* Match the app's scrollbars (thin, rounded, quiet until hovered). */
  .term :global(.xterm .scrollbar.vertical .slider) {
    border-radius: 4px;
    background: var(--scroll-thumb) !important;
  }
  .term :global(.xterm .scrollbar.vertical .slider:hover),
  .term :global(.xterm .scrollbar.vertical .slider.active) {
    background: var(--scroll-thumb-hover) !important;
  }
  .bell {
    position: absolute;
    inset: 0;
    pointer-events: none;
    animation: bell-flash 380ms ease-out forwards;
  }
</style>
