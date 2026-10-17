<script lang="ts">
  import { ui, type MenuItem } from '$lib/stores/ui.svelte';
  import MenuPanel from './MenuPanel.svelte';

  let previousFocus: HTMLElement | null = null;

  // Remember where focus was so closing the menu puts it back.
  $effect(() => {
    if (ui.menu) previousFocus = document.activeElement as HTMLElement | null;
  });

  function close(restore = true) {
    ui.hideMenu();
    if (restore && previousFocus && document.contains(previousFocus)) previousFocus.focus({ preventScroll: true });
  }

  function run(item: MenuItem) {
    close();
    // After the menu is gone and focus is restored, so dialogs opened by the action keep focus.
    queueMicrotask(() => item.run?.());
  }

  $effect(() => {
    if (!ui.menu) return;
    const hide = () => close(false);
    // Escape closes the menu wherever keyboard focus happens to be.
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      e.preventDefault();
      e.stopPropagation();
      close();
    };
    window.addEventListener('blur', hide);
    window.addEventListener('resize', hide);
    window.addEventListener('keydown', onKey, true);
    return () => {
      window.removeEventListener('blur', hide);
      window.removeEventListener('resize', hide);
      window.removeEventListener('keydown', onKey, true);
    };
  });
</script>

{#if ui.menu}
  {#key ui.menu}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="catcher"
      onpointerdown={() => close(false)}
      oncontextmenu={(e) => {
        e.preventDefault();
        close(false);
      }}
    ></div>
    <MenuPanel items={ui.menu.items} x={ui.menu.x} y={ui.menu.y} onrun={run} onclose={() => close()} />
  {/key}
{/if}

<style>
  .catcher {
    position: fixed;
    inset: 0;
    z-index: calc(var(--z-modal) + 49);
  }
</style>
