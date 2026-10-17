<script lang="ts">
  type Props = {
    value: string;
    label?: string;
    hint?: string;
    placeholder?: string;
    rows?: number;
    mono?: boolean;
    id?: string;
    disabled?: boolean;
    /** Takes the keyboard when it appears (also marks it as the field a dialog should start on). */
    autofocus?: boolean;
    oninput?: (e: Event) => void;
  };

  let {
    value = $bindable(''),
    label,
    hint,
    placeholder,
    rows = 4,
    mono = false,
    id = `ta-${Math.random().toString(36).slice(2, 8)}`,
    disabled = false,
    autofocus = false,
    oninput,
  }: Props = $props();

  function mount(node: HTMLTextAreaElement) {
    if (autofocus) queueMicrotask(() => node.focus({ preventScroll: true }));
  }
</script>

<div class="field">
  {#if label}<label for={id}>{label}</label>{/if}
  <textarea {id} bind:value {rows} {placeholder} {disabled} class:mono spellcheck="false" data-autofocus={autofocus || undefined} use:mount {oninput}></textarea>
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
  textarea {
    width: 100%;
    padding: 9px 11px;
    resize: vertical;
    min-height: 60px;
    background: var(--bg-input);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-md);
    outline: none;
    color: var(--text);
    font-size: var(--fs-md);
    line-height: 1.5;
    transition:
      border-color var(--dur-fast),
      box-shadow var(--dur);
  }
  textarea:hover {
    border-color: color-mix(in srgb, var(--accent) 30%, var(--border-strong));
  }
  textarea:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  textarea.mono {
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
  }
  textarea::placeholder {
    color: var(--text-faint);
  }
  .msg {
    font-size: var(--fs-xs);
    color: var(--text-faint);
  }
</style>
