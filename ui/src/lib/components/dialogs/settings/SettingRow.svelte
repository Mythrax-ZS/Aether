<script lang="ts">
  import type { Snippet } from 'svelte';

  type Props = {
    title: string;
    description?: string;
    /** Put the control under the text instead of beside it (wide controls). */
    stacked?: boolean;
    children: Snippet;
  };

  let { title, description, stacked = false, children }: Props = $props();
</script>

<div class="row setting-row" class:stacked>
  <div class="text">
    <div class="title">{title}</div>
    {#if description}<div class="desc">{description}</div>{/if}
  </div>
  <div class="control">{@render children()}</div>
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    min-height: 52px;
    padding: 10px 0;
  }
  .row.stacked {
    flex-direction: column;
    align-items: stretch;
    gap: 12px;
  }
  .text {
    flex: 1;
    min-width: 0;
  }
  .title {
    font-weight: 560;
  }
  .desc {
    margin-top: 2px;
    color: var(--text-dim);
    font-size: var(--fs-sm);
    line-height: 1.5;
  }
  .control {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .stacked .control {
    flex-shrink: 1;
  }
</style>
