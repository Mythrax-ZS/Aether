// Settings: loaded from the backend, edited optimistically, persisted with a short debounce,
// and applied to the document (theme, density, motion, accent, zoom, custom CSS).

import { api } from '$lib/api';
import type { Settings, Theme } from '$lib/bindings';
import { applyTheme } from '$lib/themes/apply';
import { BUILTIN_THEMES, DEFAULT_DARK, DEFAULT_LIGHT, findTheme } from '$lib/themes/builtin';
import { defaultSettings } from './defaults';

type Plain = Record<string, unknown>;

export type DeepPartial<T> = { [K in keyof T]?: T[K] extends object ? (T[K] extends unknown[] ? T[K] : DeepPartial<T[K]>) : T[K] };

const isObject = (v: unknown): v is Plain => !!v && typeof v === 'object' && !Array.isArray(v);

/** Merges `patch` into `target` in place. Arrays and `keybindings` are replaced, not merged. */
export function deepAssign(target: Plain, patch: Plain, topLevel = true): void {
  for (const [key, value] of Object.entries(patch)) {
    const keep = topLevel && key === 'keybindings';
    if (!keep && isObject(value) && isObject(target[key])) deepAssign(target[key] as Plain, value, false);
    else target[key] = value;
  }
}

class SettingsStore {
  value = $state<Settings>(defaultSettings());
  customThemes = $state<Theme[]>([]);
  loaded = $state(false);
  systemDark = $state(true);

  #pending: Plain = {};
  #timer: ReturnType<typeof setTimeout> | undefined;
  #customCssEl: HTMLStyleElement | null = null;

  // A custom theme can't take over a built-in's id (and duplicate ids would break keyed lists).
  themes = $derived<Theme[]>([...BUILTIN_THEMES, ...this.customThemes.filter((t) => !BUILTIN_THEMES.some((b) => b.id === t.id))]);

  /** The theme the app chrome is using right now. */
  activeTheme = $derived.by<Theme>(() => {
    const a = this.value.appearance;
    const id = a.theme === 'system' ? (this.systemDark ? a.darkTheme : a.lightTheme) : a.theme;
    return findTheme(id, this.customThemes) ?? findTheme(this.systemDark ? DEFAULT_DARK : DEFAULT_LIGHT)!;
  });

  /** The colours the terminal uses (may differ from the chrome theme). */
  terminalTheme = $derived.by<Theme>(() => {
    const id = this.value.terminal.theme;
    return (id && findTheme(id, this.customThemes)) || this.activeTheme;
  });

  async init(): Promise<void> {
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    this.systemDark = mq.matches;
    mq.addEventListener('change', (e) => (this.systemDark = e.matches));

    const [settings, themes] = await Promise.all([api.settings.get(), api.settings.customThemes().catch(() => [])]);
    this.value = settings;
    this.customThemes = themes;
    this.loaded = true;

    await api.on('settings:changed', (e) => {
      // Another window changed something; ignore echoes of our own pending edits.
      if (Object.keys(this.#pending).length === 0) this.value = e.settings;
    });

    $effect.root(() => {
      $effect(() => applyTheme(this.activeTheme, this.value.appearance.accent));
      $effect(() => {
        const a = this.value.appearance;
        const root = document.documentElement;
        root.dataset.density = a.density;
        if (a.motion === 'system') delete root.dataset.motion;
        else root.dataset.motion = a.motion === 'full' ? 'full' : 'reduced';
      });
      $effect(() => {
        void api.platform.window.setZoom(this.value.appearance.uiScale).catch(() => {});
      });
      $effect(() => {
        void this.#syncCustomCss(this.value.appearance.customCss);
      });
    });
  }

  async #syncCustomCss(enabled: boolean): Promise<void> {
    if (!enabled) {
      this.#customCssEl?.remove();
      this.#customCssEl = null;
      return;
    }
    const css = await api.app.customCss().catch(() => '');
    if (!this.#customCssEl) {
      this.#customCssEl = document.createElement('style');
      this.#customCssEl.id = 'aether-custom-css';
      document.head.append(this.#customCssEl);
    }
    this.#customCssEl.textContent = css;
  }

  /** Applies an edit immediately and saves it shortly after (so sliders feel instant). */
  patch(patch: DeepPartial<Settings>): void {
    deepAssign(this.value as unknown as Plain, patch as Plain);
    deepAssign(this.#pending, structuredClone(patch) as Plain);
    clearTimeout(this.#timer);
    this.#timer = setTimeout(() => void this.flush(), 250);
  }

  async flush(): Promise<void> {
    clearTimeout(this.#timer);
    const pending = this.#pending;
    if (Object.keys(pending).length === 0) return;
    this.#pending = {};
    try {
      const saved = await api.settings.patch(pending);
      // Only adopt the backend's view when the user hasn't typed more in the meantime.
      if (Object.keys(this.#pending).length === 0) this.value = saved;
    } catch (e) {
      // Keep the optimistic value; the caller surfaces errors where it matters.
      console.error('saving settings failed', e);
    }
  }

  async reset(): Promise<void> {
    this.value = await api.settings.reset();
  }

  async refreshThemes(): Promise<void> {
    this.customThemes = await api.settings.customThemes();
  }
}

export const settings = new SettingsStore();
