<script lang="ts">
  import type { Snippet } from 'svelte';
  import { Eye, EyeOff } from '@lucide/svelte';

  type Props = {
    value: string | number;
    label?: string;
    hint?: string;
    error?: string | null;
    placeholder?: string;
    type?: 'text' | 'password' | 'number' | 'search';
    mono?: boolean;
    disabled?: boolean;
    autofocus?: boolean;
    select?: boolean;
    id?: string;
    name?: string;
    min?: number;
    max?: number;
    step?: number;
    size?: 'sm' | 'md';
    autocomplete?: 'off' | 'on' | 'username' | 'current-password' | 'new-password' | 'one-time-code';
    class?: string;
    icon?: Snippet;
    suffix?: Snippet;
    oninput?: (e: Event) => void;
    onkeydown?: (e: KeyboardEvent) => void;
    onblur?: () => void;
    input?: HTMLInputElement | null;
  };

  let {
    value = $bindable(''),
    label,
    hint,
    error = null,
    placeholder,
    type = 'text',
    mono = false,
    disabled = false,
    autofocus = false,
    select = false,
    id = `in-${Math.random().toString(36).slice(2, 8)}`,
    name,
    min,
    max,
    step,
    size = 'md',
    autocomplete = 'off',
    class: extra = '',
    icon,
    suffix,
    oninput,
    onkeydown,
    onblur,
    input = $bindable(null),
  }: Props = $props();

  let reveal = $state(false);
  const actualType = $derived(type === 'password' && reveal ? 'text' : type);

  // Takes focus when it appears (a dialog's focus trap picks the same field via `data-autofocus`,
  // so the two never disagree — and this also covers fields that appear after the dialog opened).
  function mount(node: HTMLInputElement) {
    if (!autofocus) return;
    queueMicrotask(() => {
      node.focus({ preventScroll: true });
      if (select) node.select();
    });
  }
</script>

<div class="field {extra}" class:error={!!error}>
  {#if label}<label for={id}>{label}</label>{/if}
  <div class="control {size}" class:has-icon={!!icon}>
    {#if icon}<span class="lead">{@render icon()}</span>{/if}
    <input
      {id}
      {name}
      type={actualType}
      class:mono
      bind:value
      bind:this={input}
      {placeholder}
      {disabled}
      {min}
      {max}
      {step}
      {autocomplete}
      spellcheck="false"
      data-autofocus={autofocus || undefined}
      data-select={select || undefined}
      use:mount
      {oninput}
      {onkeydown}
      {onblur}
    />
    {#if type === 'password'}
      <button type="button" class="reveal" tabindex="-1" aria-label={reveal ? 'Hide' : 'Show'} onclick={() => (reveal = !reveal)}>
        {#if reveal}<EyeOff size={15} />{:else}<Eye size={15} />{/if}
      </button>
    {/if}
    {#if suffix}<span class="tail">{@render suffix()}</span>{/if}
  </div>
  {#if error}<p class="msg err">{error}</p>{:else if hint}<p class="msg">{hint}</p>{/if}
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
    display: flex;
    align-items: center;
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
  .error .control {
    border-color: var(--danger);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--danger) 16%, transparent);
  }
  input {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0 11px;
    background: transparent;
    border: 0;
    outline: none;
    color: var(--text);
    font-size: var(--fs-md);
    cursor: text;
  }
  input::placeholder {
    color: var(--text-faint);
  }
  input:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  input.mono {
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
  }
  .has-icon input {
    padding-left: 4px;
  }
  .lead,
  .tail {
    display: inline-flex;
    align-items: center;
    color: var(--text-faint);
  }
  .lead {
    padding-left: 10px;
  }
  .tail {
    padding-right: 8px;
  }
  .reveal {
    display: grid;
    place-items: center;
    width: 30px;
    height: 100%;
    color: var(--text-faint);
  }
  .reveal:hover {
    color: var(--text);
  }
  .msg {
    font-size: var(--fs-xs);
    color: var(--text-faint);
  }
  .msg.err {
    color: var(--danger);
  }
</style>
