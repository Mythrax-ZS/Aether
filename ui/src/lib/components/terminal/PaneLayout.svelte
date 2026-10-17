<script lang="ts">
  import { scale } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { workspace, type TerminalTab } from '$lib/stores/workspace.svelte';
  import { clamp } from '$lib/util/format';
  import { layoutPanes, leaves, type Divider } from '$lib/util/panes';
  import Pane from './Pane.svelte';

  let { tab, visible }: { tab: TerminalTab; visible: boolean } = $props();

  let area = $state<HTMLDivElement>();
  let draggingId = $state<string | null>(null);

  const layout = $derived(layoutPanes(tab.root));
  const panes = $derived(leaves(tab.root));
  const multi = $derived(panes.length > 1);

  function startDrag(e: PointerEvent, d: Divider) {
    if (!area) return;
    e.preventDefault();
    draggingId = d.splitId;
    const rect = area.getBoundingClientRect();
    const row = d.dir === 'row';
    document.documentElement.style.cursor = row ? 'col-resize' : 'row-resize';
    const move = (ev: PointerEvent) => {
      // Pointer position inside the split's own area → its ratio.
      const start = row ? rect.left + d.parent.x * rect.width : rect.top + d.parent.y * rect.height;
      const size = row ? d.parent.w * rect.width : d.parent.h * rect.height;
      const pos = row ? ev.clientX : ev.clientY;
      workspace.setSplitRatio(tab.id, d.splitId, clamp((pos - start) / size, 0.1, 0.9));
    };
    const up = () => {
      draggingId = null;
      document.documentElement.style.cursor = '';
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  }

  function nudge(e: KeyboardEvent, d: Divider) {
    const step = e.shiftKey ? 0.05 : 0.02;
    const split = findSplit(d.splitId);
    if (!split) return;
    const dec = d.dir === 'row' ? 'ArrowLeft' : 'ArrowUp';
    const inc = d.dir === 'row' ? 'ArrowRight' : 'ArrowDown';
    if (e.key === dec) workspace.setSplitRatio(tab.id, d.splitId, split.ratio - step);
    else if (e.key === inc) workspace.setSplitRatio(tab.id, d.splitId, split.ratio + step);
    else return;
    e.preventDefault();
  }

  function findSplit(id: string) {
    const walk = (n: typeof tab.root): (typeof tab.root & { kind: 'split' }) | undefined => {
      if (n.kind === 'leaf') return undefined;
      return n.id === id ? n : (walk(n.a) ?? walk(n.b));
    };
    return walk(tab.root);
  }

  const pct = (n: number) => `${(n * 100).toFixed(4)}%`;
</script>

<div class="layout" bind:this={area} class:dragging={!!draggingId}>
  {#each panes as leaf (leaf.id)}
    {@const r = layout.rects.get(leaf.id)}
    {#if r}
      <div
        class="slot"
        style:left={pct(r.x)}
        style:top={pct(r.y)}
        style:width={pct(r.w)}
        style:height={pct(r.h)}
        in:scale={{ start: 0.985, duration: multi ? 200 : 0, easing: cubicOut }}
      >
        <Pane {leaf} {visible} focused={tab.activePane === leaf.id} {multi} />
      </div>
    {/if}
  {/each}

  {#each layout.dividers as d (d.splitId)}
    <!-- A focusable separator is an interactive widget (ARIA 1.2): arrow keys move it. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      class="divider {d.dir}"
      class:active={draggingId === d.splitId}
      role="separator"
      aria-orientation={d.dir === 'row' ? 'vertical' : 'horizontal'}
      tabindex="0"
      style:left={d.dir === 'row' ? pct(d.at) : pct(d.parent.x)}
      style:top={d.dir === 'row' ? pct(d.parent.y) : pct(d.at)}
      style:width={d.dir === 'row' ? undefined : pct(d.parent.w)}
      style:height={d.dir === 'row' ? pct(d.parent.h) : undefined}
      onpointerdown={(e) => startDrag(e, d)}
      ondblclick={() => workspace.setSplitRatio(tab.id, d.splitId, 0.5)}
      onkeydown={(e) => nudge(e, d)}
    ></div>
  {/each}
</div>

<style>
  .layout {
    position: absolute;
    inset: 0;
  }
  .slot {
    position: absolute;
    overflow: hidden;
  }
  .divider {
    position: absolute;
    z-index: 8;
    outline: none;
  }
  .divider.row {
    width: 9px;
    margin-left: -4px;
    cursor: col-resize;
  }
  .divider.col {
    height: 9px;
    margin-top: -4px;
    cursor: row-resize;
  }
  .divider::after {
    content: '';
    position: absolute;
    background: var(--border-strong);
    transition:
      background var(--dur-fast),
      box-shadow var(--dur);
  }
  .divider.row::after {
    left: 4px;
    top: 0;
    bottom: 0;
    width: 1px;
  }
  .divider.col::after {
    top: 4px;
    left: 0;
    right: 0;
    height: 1px;
  }
  .divider:hover::after,
  .divider:focus-visible::after,
  .divider.active::after {
    background: var(--accent);
    box-shadow: 0 0 10px var(--accent-ring);
  }
  .divider.row:hover::after,
  .divider.row.active::after {
    width: 2px;
    left: 3.5px;
  }
  .divider.col:hover::after,
  .divider.col.active::after {
    height: 2px;
    top: 3.5px;
  }
</style>
