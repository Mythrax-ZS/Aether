<script lang="ts">
  import { initials } from '$lib/util/format';

  type Props = {
    name: string;
    /** `#rrggbb`; when missing a stable colour is derived from the name. */
    color?: string | null;
    size?: number;
    status?: 'connected' | 'connecting' | 'warning' | 'error' | null;
    rounded?: 'md' | 'full';
  };

  let { name, color = null, size = 28, status = null, rounded = 'md' }: Props = $props();

  // Same name → same colour, spread around the hue wheel but kept in the soft cyan→violet range
  // that matches the brand rather than rainbow noise.
  const hue = $derived.by(() => {
    let h = 2166136261;
    for (let i = 0; i < name.length; i++) h = Math.imul(h ^ name.charCodeAt(i), 16777619);
    return 175 + (Math.abs(h) % 150); // 175°–325°: teal → blue → violet → magenta
  });
  const base = $derived(color ?? `hsl(${hue} 72% 62%)`);
  const text = $derived(initials(name));
</script>

<span class="avatar {rounded}" style:--size="{size}px" style:--c={base} aria-hidden="true">
  <span class="txt">{text}</span>
  {#if status}<i class="status {status}"></i>{/if}
</span>

<style>
  .avatar {
    position: relative;
    display: inline-grid;
    place-items: center;
    flex-shrink: 0;
    width: var(--size);
    height: var(--size);
    color: color-mix(in srgb, var(--c) 82%, white);
    background:
      linear-gradient(145deg, color-mix(in srgb, var(--c) 30%, transparent), color-mix(in srgb, var(--c) 10%, transparent));
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--c) 34%, transparent);
    font-weight: 700;
    font-size: calc(var(--size) * 0.38);
    letter-spacing: 0.02em;
    line-height: 1;
  }
  :global(:root[data-theme-kind='light']) .avatar {
    color: color-mix(in srgb, var(--c) 78%, black);
  }
  .md {
    border-radius: calc(var(--size) * 0.3);
  }
  .full {
    border-radius: 50%;
  }
  .txt {
    transform: translateY(0.5px);
  }
  .status {
    position: absolute;
    right: -2px;
    bottom: -2px;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--text-faint);
    box-shadow: 0 0 0 2px var(--bg-sidebar);
  }
  .status.connected {
    background: var(--success);
  }
  .status.connecting {
    background: var(--accent);
    animation: pulse-dot 1.1s var(--ease-in-out) infinite;
  }
  .status.warning {
    background: var(--warning);
    animation: pulse-dot 1.1s var(--ease-in-out) infinite;
  }
  .status.error {
    background: var(--danger);
  }
</style>
