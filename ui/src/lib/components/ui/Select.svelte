<script lang="ts">
  import { ChevronDown } from '@lucide/svelte';

  export interface Option<T extends string | number> {
    value: T;
    label: string;
    disabled?: boolean;
    group?: string;
  }

  type Props = {
    value: string | number;
    options: Option<string | number>[];
    label?: string;
    hint?: string;
    disabled?: boolean;
    size?: 'sm' | 'md';
    id?: string;
    class?: string;
    onchange?: (value: string) => void;
  };

  let {
    value = $bindable(''),
    options,
    label,
    hint,
    disabled = false,
    size = 'md',
    id = `sel-${Math.random().toString(36).slice(2, 8)}`,
    class: extra = '',
    onchange,
  }: Props = $props();

  const groups = $derived.by(() => {
    const out = new Map<string, Option<string | number>[]>();
    for (const o of options) {
      const g = o.group ?? '';
      if (!out.has(g)) out.set(g, []);
      out.get(g)!.push(o);
    }
    return [...out.entries()];
  });
</script>

<div class="field {extra}">
  {#if label}<label for={id}>{label}</label>{/if}
  <div class="control {size}">
    <select {id} bind:value {disabled} onchange={(e) => onchange?.(e.currentTarget.value)}>
      {#each groups as [group, items] (group)}
        {#if group}
          <optgroup label={group}>
            {#each items as o (o.value)}<option value={o.value} disabled={o.disabled}>{o.label}</option>{/each}
          </optgroup>
        {:else}
          {#each items as o (o.value)}<option value={o.value} disabled={o.disabled}>{o.label}</option>{/each}
        {/if}
      {/each}
    </select>
    <span class="chev"><ChevronDown size={15} /></span>
  </div>
  {#if hint}<p class="msg">{hint}</p>{/if}
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  label {
    font-size: var(--fs-sm);
    font-weight: 550;
    color: var(--text-dim);
  }
  .control {
    position: relative;
    background: var(--bg-input);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-md);
    transition:
      border-color var(--dur-fast),
      box-shadow var(--dur);
  }
  .control.md {
    height: 34px;
  }
  .control.sm {
    height: 28px;
  }
  .control:hover {
    border-color: color-mix(in srgb, var(--accent) 30%, var(--border-strong));
  }
  .control:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  select {
    appearance: none;
    width: 100%;
    height: 100%;
    padding: 0 30px 0 11px;
    background: transparent;
    border: 0;
    outline: none;
    color: var(--text);
    font-size: var(--fs-md);
    cursor: pointer;
  }
  option,
  optgroup {
    background: var(--bg-overlay);
    color: var(--text);
  }
  .chev {
    position: absolute;
    right: 9px;
    top: 50%;
    translate: 0 -50%;
    pointer-events: none;
    color: var(--text-faint);
    display: flex;
  }
  .msg {
    font-size: var(--fs-xs);
    color: var(--text-faint);
  }
</style>
