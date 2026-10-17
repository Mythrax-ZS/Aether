<script lang="ts">
  import { AlertTriangle, Check, ChevronDown, CircleAlert, Copy, Info, X } from '@lucide/svelte';
  import { flip } from 'svelte/animate';
  import { fade, fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { api } from '$lib/api';
  import { ui, type Toast } from '$lib/stores/ui.svelte';

  let expanded = $state<Record<number, boolean>>({});

  const ICON = { info: Info, success: Check, warning: AlertTriangle, error: CircleAlert } as const;

  async function copyDetail(t: Toast) {
    const text = [t.title, t.message, t.hint, t.detail].filter(Boolean).join('\n');
    await api.platform.clipboard.writeText(text).catch(() => {});
  }
</script>

<section class="toasts" aria-label="Notifications" aria-live="polite">
  {#each ui.toasts as t (t.id)}
    {@const Icon = ICON[t.kind]}
    <div
      class="toast {t.kind}"
      role={t.kind === 'error' ? 'alert' : 'status'}
      animate:flip={{ duration: 220, easing: cubicOut }}
      in:fly|global={{ x: 36, duration: 260, easing: cubicOut }}
      out:fade|global={{ duration: 160 }}
      style:--life="{t.timeout}ms"
    >
      <span class="badge"><Icon size={16} strokeWidth={2.4} /></span>
      <div class="content">
        <p class="title">{t.title}</p>
        {#if t.message}<p class="msg">{t.message}</p>{/if}
        {#if t.hint}<p class="hint">{t.hint}</p>{/if}
        {#if t.detail}
          <button type="button" class="more" onclick={() => (expanded[t.id] = !expanded[t.id])} aria-expanded={!!expanded[t.id]}>
            <span class="chev" class:open={expanded[t.id]}><ChevronDown size={13} /></span> Technical details
          </button>
          {#if expanded[t.id]}
            <div class="detail selectable">
              <pre>{t.detail}</pre>
              <button type="button" class="copy" aria-label="Copy details" onclick={() => copyDetail(t)}><Copy size={13} /></button>
            </div>
          {/if}
        {/if}
        {#if t.actions?.length}
          <div class="actions">
            {#each t.actions as a (a.label)}
              <button
                type="button"
                class="action"
                onclick={() => {
                  ui.dismissToast(t.id);
                  a.run();
                }}>{a.label}</button
              >
            {/each}
          </div>
        {/if}
      </div>
      <button type="button" class="close" aria-label="Dismiss" onclick={() => ui.dismissToast(t.id)}><X size={14} /></button>
      {#if t.timeout > 0}<i class="life"></i>{/if}
    </div>
  {/each}
</section>

<style>
  .toasts {
    position: fixed;
    right: 16px;
    bottom: calc(var(--statusbar-h) + 14px);
    z-index: var(--z-toast);
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: min(380px, calc(100vw - 32px));
    pointer-events: none;
  }
  .toast {
    --c: var(--accent);
    position: relative;
    display: flex;
    gap: 11px;
    align-items: flex-start;
    padding: 12px 10px 12px 12px;
    background: color-mix(in srgb, var(--bg-overlay) 96%, transparent);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-lg);
    box-shadow: var(--shadow-lg);
    overflow: hidden;
    pointer-events: auto;
  }
  .toast.success {
    --c: var(--success);
  }
  .toast.warning {
    --c: var(--warning);
  }
  .toast.error {
    --c: var(--danger);
  }
  .badge {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 26px;
    height: 26px;
    border-radius: 8px;
    color: var(--c);
    background: color-mix(in srgb, var(--c) 16%, transparent);
  }
  .content {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding-top: 2px;
  }
  .title {
    font-weight: 650;
    font-size: var(--fs-md);
  }
  .msg {
    color: var(--text-dim);
    font-size: var(--fs-sm);
    line-height: 1.45;
    overflow-wrap: anywhere;
  }
  .hint {
    color: var(--text);
    font-size: var(--fs-sm);
    line-height: 1.45;
    margin-top: 2px;
  }
  .more {
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    margin-top: 4px;
    font-size: var(--fs-xs);
    color: var(--text-faint);
  }
  .more:hover {
    color: var(--text-dim);
  }
  .chev {
    display: inline-flex;
    transition: transform var(--dur) var(--ease-out);
  }
  .chev.open {
    transform: rotate(180deg);
  }
  .detail {
    position: relative;
    margin-top: 4px;
    padding: 8px 28px 8px 9px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-sm);
  }
  .detail pre {
    margin: 0;
    max-height: 120px;
    overflow: auto;
    font: 11px/1.45 var(--font-mono);
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--text-dim);
  }
  .copy {
    position: absolute;
    top: 5px;
    right: 5px;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 5px;
    color: var(--text-faint);
  }
  .copy:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .actions {
    display: flex;
    gap: 6px;
    margin-top: 6px;
  }
  .action {
    height: 24px;
    padding: 0 10px;
    border-radius: 6px;
    font-size: var(--fs-sm);
    font-weight: 600;
    color: var(--c);
    background: color-mix(in srgb, var(--c) 14%, transparent);
  }
  .action:hover {
    background: color-mix(in srgb, var(--c) 24%, transparent);
  }
  .close {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 6px;
    color: var(--text-faint);
  }
  .close:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .life {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 2px;
    width: 100%;
    background: var(--c);
    opacity: 0.55;
    transform-origin: left;
    animation: life var(--life) linear forwards;
  }
  @keyframes life {
    to {
      transform: scaleX(0);
    }
  }
</style>
