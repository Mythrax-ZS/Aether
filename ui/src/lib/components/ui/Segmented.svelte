<script lang="ts">
  import type { Component } from 'svelte';

  interface Item {
    value: string;
    label: string;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    icon?: Component<any>;
  }

  type Props = {
    value: string;
    options: Item[];
    size?: 'sm' | 'md';
    label?: string;
    onchange?: (value: string) => void;
  };

  let { value = $bindable(''), options, size = 'md', label, onchange }: Props = $props();

  function pick(v: string) {
    value = v;
    onchange?.(v);
  }

  function onKey(e: KeyboardEvent) {
    const i = options.findIndex((o) => o.value === value);
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown') pick(options[(i + 1) % options.length].value);
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') pick(options[(i - 1 + options.length) % options.length].value);
    else return;
    e.preventDefault();
  }
</script>

<div class="seg {size}" role="radiogroup" aria-label={label} onkeydown={onKey} tabindex="-1">
  {#each options as o (o.value)}
    {@const Icon = o.icon}
    <button type="button" role="radio" aria-checked={value === o.value} class:on={value === o.value} tabindex={value === o.value ? 0 : -1} onclick={() => pick(o.value)}>
      {#if Icon}<Icon size={14} />{/if}
      {o.label}
    </button>
  {/each}
</div>

<style>
  .seg {
    display: inline-flex;
    padding: 3px;
    gap: 2px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: calc(var(--r-md) + 2px);
  }
  button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 0 12px;
    color: var(--text-dim);
    font-weight: 550;
    border-radius: var(--r-md);
    transition:
      background var(--dur-fast),
      color var(--dur-fast),
      box-shadow var(--dur);
  }
  .md button {
    height: 26px;
    font-size: var(--fs-sm);
  }
  .sm button {
    height: 22px;
    font-size: var(--fs-xs);
    padding: 0 9px;
  }
  button:hover:not(.on) {
    color: var(--text);
    background: var(--bg-hover);
  }
  button.on {
    background: var(--bg-overlay);
    color: var(--text);
    box-shadow:
      0 1px 2px rgb(0 0 0 / 0.3),
      0 0 0 1px var(--border-strong);
  }
</style>
