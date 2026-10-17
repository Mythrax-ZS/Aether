<script lang="ts">
  import { Play, Star } from '@lucide/svelte';
  import type { Host } from '$lib/bindings';
  import { draggable, type DragPayload } from '$lib/dom/dnd.svelte';
  import { scrollIntoViewWhen } from '$lib/dom/actions';
  import { tooltip } from '$lib/dom/tooltip';
  import { highlightRuns } from '$lib/util/fuzzy';
  import type { LinkStatus } from '$lib/stores/workspace.svelte';
  import Avatar from '$lib/components/ui/Avatar.svelte';

  type Props = {
    host: Host;
    depth?: number;
    selected?: boolean;
    cursor?: boolean;
    status?: LinkStatus;
    indices?: number[];
    path?: string | null;
    onselect: (e: MouseEvent) => void;
    onconnect: () => void;
    oncontext: (e: MouseEvent) => void;
    onfavorite: () => void;
    drag: () => DragPayload | null;
  };

  let { host, depth = 0, selected = false, cursor = false, status, indices = [], path = null, onselect, onconnect, oncontext, onfavorite, drag }: Props = $props();

  const name = $derived(host.label || host.hostname);
  const address = $derived(`${host.username ? host.username + '@' : ''}${host.hostname}${host.port !== 22 ? ':' + host.port : ''}`);
  const runs = $derived(highlightRuns(name, indices));
</script>

<!-- The list owns the keyboard (arrows move the cursor, Enter connects), so rows take pointer events only. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_click_events_have_key_events -->
<div
  class="row"
  class:selected
  class:cursor
  role="treeitem"
  aria-selected={selected}
  tabindex="-1"
  style:--indent="{depth * 14}px"
  onclick={onselect}
  ondblclick={onconnect}
  oncontextmenu={oncontext}
  use:scrollIntoViewWhen={cursor}
  use:draggable={{ payload: () => drag() }}
>
  <Avatar name={name} color={host.color} size={28} {status} />
  <div class="text">
    <div class="name">
      {#each runs as r, i (i)}{#if r.hit}<mark>{r.text}</mark>{:else}{r.text}{/if}{/each}
    </div>
    <div class="addr">{#if path}<span class="path">{path}</span> · {/if}{address}</div>
  </div>
  <div class="actions" data-no-drag>
    <button
      type="button"
      class="act star"
      class:on={host.favorite}
      aria-label={host.favorite ? 'Remove from favorites' : 'Add to favorites'}
      aria-pressed={host.favorite}
      onclick={(e) => {
        e.stopPropagation();
        onfavorite();
      }}
      use:tooltip={host.favorite ? 'Unfavorite' : 'Favorite'}
    >
      <Star size={14} fill={host.favorite ? 'currentColor' : 'none'} />
    </button>
    <button
      type="button"
      class="act go"
      aria-label="Connect to {name}"
      onclick={(e) => {
        e.stopPropagation();
        onconnect();
      }}
      use:tooltip={'Connect'}
    >
      <Play size={13} fill="currentColor" />
    </button>
  </div>
</div>

<style>
  .row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 10px;
    height: var(--host-row-h, 46px);
    margin: 0 6px;
    padding: 0 6px 0 calc(8px + var(--indent));
    border-radius: var(--r-md);
    color: var(--text);
    cursor: default;
    transition: background var(--dur-fast);
  }
  .row:hover {
    background: var(--bg-hover);
  }
  .row.selected {
    background: var(--accent-soft);
  }
  .row.selected::before {
    content: '';
    position: absolute;
    left: -6px;
    top: 9px;
    bottom: 9px;
    width: 3px;
    border-radius: 0 3px 3px 0;
    background: var(--grad-accent);
  }
  .row.cursor {
    box-shadow: inset 0 0 0 1px var(--accent-ring);
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
    line-height: 1.3;
  }
  .name {
    font-weight: 560;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name mark {
    background: none;
    color: var(--accent);
    font-weight: 700;
  }
  .addr {
    font-size: var(--fs-xs);
    color: var(--text-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .path {
    color: var(--text-dim);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .act {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: var(--r-sm);
    color: var(--text-faint);
    opacity: 0;
    transition:
      opacity var(--dur-fast),
      background var(--dur-fast),
      color var(--dur-fast);
  }
  .row:hover .act,
  .row.cursor .act,
  .act.star.on {
    opacity: 1;
  }
  .act:hover {
    background: var(--bg-active);
    color: var(--text);
  }
  .star.on {
    color: var(--warning);
  }
  .go:hover {
    background: var(--accent-soft);
    color: var(--accent);
  }
</style>
