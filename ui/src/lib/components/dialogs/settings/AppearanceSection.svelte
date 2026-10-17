<script lang="ts">
  import { Check, FolderOpen, Moon, Sun } from '@lucide/svelte';
  import { api } from '$lib/api';
  import type { Theme } from '$lib/bindings';
  import { settings } from '$lib/stores/settings.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Segmented from '$lib/components/ui/Segmented.svelte';
  import Switch from '$lib/components/ui/Switch.svelte';
  import SettingRow from './SettingRow.svelte';

  const a = $derived(settings.value.appearance);
  const followSystem = $derived(a.theme === 'system');
  const current = $derived(followSystem ? (settings.systemDark ? a.darkTheme : a.lightTheme) : a.theme);

  const dark = $derived(settings.themes.filter((t) => t.kind === 'dark'));
  const light = $derived(settings.themes.filter((t) => t.kind === 'light'));

  function pick(t: Theme) {
    if (followSystem) settings.patch({ appearance: t.kind === 'dark' ? { darkTheme: t.id } : { lightTheme: t.id } });
    else settings.patch({ appearance: { theme: t.id } });
  }

  function setFollow(on: boolean) {
    if (on) settings.patch({ appearance: { theme: 'system' } });
    else settings.patch({ appearance: { theme: current } });
  }

  const ACCENTS = ['#5cc8ff', '#7c9cff', '#9d8cff', '#d18cff', '#ff8ccf', '#ff6f8a', '#ff9f6b', '#f5c067', '#7fdc9b', '#4fd1c5'];
  const SCALES = [
    { value: '0.9', label: '90%' },
    { value: '1', label: '100%' },
    { value: '1.1', label: '110%' },
    { value: '1.25', label: '125%' },
    { value: '1.5', label: '150%' },
  ];
</script>

{#snippet card(t: Theme)}
  {@const p = t.terminal}
  {@const on = current === t.id}
  <button type="button" class="theme" class:on onclick={() => pick(t)} aria-pressed={on} aria-label="{t.name} theme">
    <span class="shot" style:background={p.background}>
      <span class="bar" style:background={p.black}></span>
      <span class="ln"><i style:background={p.green}></i><i style:background={p.foreground} style:opacity="0.8"></i></span>
      <span class="ln"><i style:background={p.cyan}></i><i style:background={p.yellow}></i><i style:background={p.foreground} style:opacity="0.5"></i></span>
      <span class="ln"><i style:background={p.magenta}></i><i style:background={p.blue}></i></span>
      <span class="sw">
        {#each [p.red, p.green, p.yellow, p.blue, p.magenta, p.cyan] as c, i (i)}<b style:background={c}></b>{/each}
      </span>
    </span>
    <span class="name">{t.name}{#if on}<span class="ck"><Check size={12} strokeWidth={3} /></span>{/if}</span>
  </button>
{/snippet}

<div class="fsec">
  <h3>Theme</h3>
  <SettingRow title="Match Windows" description="Switch between a light and a dark theme automatically.">
    <Switch checked={followSystem} aria="Match Windows light and dark mode" onchange={setFollow} />
  </SettingRow>

  {#if followSystem}
    <p class="fdesc"><Moon size={13} /> Dark mode uses <b>{settings.themes.find((t) => t.id === a.darkTheme)?.name}</b> · <Sun size={13} /> Light mode uses <b>{settings.themes.find((t) => t.id === a.lightTheme)?.name}</b>. Pick one of each below.</p>
  {/if}

  <div class="group-title"><Moon size={13} /> Dark</div>
  <div class="themes">{#each dark as t (t.id)}{@render card(t)}{/each}</div>
  <div class="group-title"><Sun size={13} /> Light</div>
  <div class="themes">{#each light as t (t.id)}{@render card(t)}{/each}</div>
  <p class="fdesc">Make your own: drop a theme file in the <b>themes</b> folder inside the config folder and it shows up here.</p>
</div>

<div class="fsec">
  <h3>Accent colour</h3>
  <div class="accents" role="radiogroup" aria-label="Accent colour">
    <button type="button" role="radio" aria-checked={a.accent === null} class="acc theme-default" class:on={a.accent === null} onclick={() => settings.patch({ appearance: { accent: null } })} aria-label="Theme default">
      {#if a.accent === null}<Check size={12} strokeWidth={3} />{:else}A{/if}
    </button>
    {#each ACCENTS as c (c)}
      <button type="button" role="radio" aria-checked={a.accent === c} class="acc" class:on={a.accent === c} style:--c={c} onclick={() => settings.patch({ appearance: { accent: c } })} aria-label={c}>
        {#if a.accent === c}<Check size={12} strokeWidth={3} />{/if}
      </button>
    {/each}
    <label class="custom" title="Pick any colour">
      <input type="color" value={a.accent ?? '#5cc8ff'} oninput={(e) => settings.patch({ appearance: { accent: e.currentTarget.value } })} aria-label="Custom accent colour" />
    </label>
  </div>
</div>

<div class="fsec">
  <h3>Interface</h3>
  <SettingRow title="Interface size" description="Scales menus, sidebar and dialogs. The terminal has its own font size.">
    <Segmented value={String(a.uiScale)} label="Interface size" options={SCALES} onchange={(v) => settings.patch({ appearance: { uiScale: Number(v) } })} />
  </SettingRow>
  <SettingRow title="Density" description="Compact fits more hosts in the sidebar.">
    <Segmented
      value={a.density}
      label="Density"
      options={[
        { value: 'comfortable', label: 'Comfortable' },
        { value: 'compact', label: 'Compact' },
      ]}
      onchange={(v) => settings.patch({ appearance: { density: v as 'comfortable' | 'compact' } })}
    />
  </SettingRow>
  <SettingRow title="Animations" description="Follow Windows’ reduced-motion setting, or force animations on or off.">
    <Segmented
      value={a.motion}
      label="Animations"
      options={[
        { value: 'system', label: 'Match Windows' },
        { value: 'full', label: 'Always' },
        { value: 'reduced', label: 'Reduced' },
      ]}
      onchange={(v) => settings.patch({ appearance: { motion: v as 'system' | 'full' | 'reduced' } })}
    />
  </SettingRow>
  <SettingRow title="Status bar" description="Connection, latency and encryption details along the bottom.">
    <Switch checked={a.showStatusBar} aria="Show the status bar" onchange={(v) => settings.patch({ appearance: { showStatusBar: v } })} />
  </SettingRow>
</div>

<div class="fsec">
  <h3>Customise</h3>
  <SettingRow title="Custom stylesheet" description="Load custom.css from the config folder after the built-in styles. Every colour, radius and spacing is a CSS variable, so small tweaks go a long way.">
    <Switch checked={a.customCss} aria="Load custom.css" onchange={(v) => settings.patch({ appearance: { customCss: v } })} />
  </SettingRow>
  <div><Button size="sm" onclick={() => void api.app.openConfig().catch((e) => ui.error(e))}>{#snippet icon()}<FolderOpen size={13} />{/snippet}Open config folder</Button></div>
</div>

<style>
  .group-title {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-sm);
    font-weight: 600;
    color: var(--text-dim);
  }
  .themes {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(138px, 1fr));
    gap: 10px;
  }
  .theme {
    display: flex;
    flex-direction: column;
    gap: 7px;
    padding: 6px;
    text-align: left;
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    transition:
      border-color var(--dur-fast),
      transform var(--dur) var(--ease-out),
      box-shadow var(--dur);
  }
  .theme:hover {
    transform: translateY(-2px);
    border-color: var(--border-strong);
  }
  .theme.on {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .shot {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 5px;
    height: 78px;
    padding: 18px 9px 8px;
    border-radius: 8px;
    overflow: hidden;
    box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.06);
  }
  .bar {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: 11px;
  }
  .ln {
    display: flex;
    gap: 4px;
  }
  .ln i {
    display: block;
    height: 4px;
    border-radius: 2px;
  }
  .ln i:nth-child(1) {
    width: 16px;
  }
  .ln i:nth-child(2) {
    width: 34px;
  }
  .ln i:nth-child(3) {
    width: 20px;
  }
  .sw {
    display: flex;
    gap: 3px;
    margin-top: auto;
  }
  .sw b {
    width: 9px;
    height: 4px;
    border-radius: 1px;
  }
  .name {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 4px 2px;
    font-size: var(--fs-sm);
    font-weight: 560;
  }
  .ck {
    display: grid;
    place-items: center;
    width: 17px;
    height: 17px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--on-accent);
  }
  .accents {
    display: flex;
    flex-wrap: wrap;
    gap: 9px;
    align-items: center;
  }
  .acc {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    background: var(--c);
    color: #0b0d13;
    box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.2);
    transition: transform var(--dur-fast) var(--ease-spring);
  }
  .acc:hover {
    transform: scale(1.15);
  }
  .acc.on {
    box-shadow:
      0 0 0 2px var(--bg-surface),
      0 0 0 4px var(--c, var(--accent));
  }
  .theme-default {
    background: var(--grad-accent);
    color: var(--on-accent);
    font-size: var(--fs-xs);
    font-weight: 800;
  }
  .custom {
    position: relative;
    display: block;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    overflow: hidden;
    background: conic-gradient(#ff6f8a, #f5c067, #7fdc9b, #5cc8ff, #9d8cff, #ff6f8a);
    cursor: pointer;
    transition: transform var(--dur-fast) var(--ease-spring);
  }
  .custom:hover {
    transform: scale(1.15);
  }
  .custom input {
    position: absolute;
    inset: -6px;
    width: 40px;
    height: 40px;
    opacity: 0;
    cursor: pointer;
  }
</style>
