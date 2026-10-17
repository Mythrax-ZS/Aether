<script lang="ts">
  type Props = {
    value: number;
    min: number;
    max: number;
    step?: number;
    label?: string;
    /** How to print the current value ("13.5 px"). */
    format?: (v: number) => string;
    width?: number;
    onchange?: (v: number) => void;
  };

  let { value = $bindable(0), min, max, step = 1, label, format = (v) => String(v), width = 200, onchange }: Props = $props();

  const pct = $derived(((value - min) / (max - min)) * 100);

  function onInput(e: Event) {
    value = Number((e.currentTarget as HTMLInputElement).value);
    onchange?.(value);
  }
</script>

<div class="slider" style:width="{width}px">
  <input type="range" {min} {max} {step} {value} aria-label={label} style:--pct="{pct}%" oninput={onInput} />
  <output>{format(value)}</output>
</div>

<style>
  .slider {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  input {
    flex: 1;
    min-width: 0;
    height: 18px;
    margin: 0;
    background: transparent;
    appearance: none;
    cursor: pointer;
  }
  input::-webkit-slider-runnable-track {
    height: 4px;
    border-radius: 2px;
    background: linear-gradient(90deg, var(--accent) var(--pct), var(--bg-active) var(--pct));
  }
  input::-webkit-slider-thumb {
    appearance: none;
    width: 14px;
    height: 14px;
    margin-top: -5px;
    border-radius: 50%;
    background: #fff;
    border: 0;
    box-shadow:
      0 1px 4px rgb(0 0 0 / 0.5),
      0 0 0 3px color-mix(in srgb, var(--accent) 30%, transparent);
    transition: transform var(--dur-fast) var(--ease-spring);
  }
  input:hover::-webkit-slider-thumb {
    transform: scale(1.2);
  }
  input:focus-visible {
    outline-offset: 4px;
  }
  output {
    min-width: 54px;
    text-align: right;
    font-size: var(--fs-sm);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
</style>
