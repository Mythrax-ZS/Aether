// Applying a theme: CSS variables for the chrome, an xterm palette for the terminal.

import type { ITheme } from '@xterm/xterm';
import type { TerminalPalette, Theme } from '$lib/bindings';

const TUNED = new Set(['aether-dark', 'aether-light']);

/** Variables we set last time, so switching themes cleans up after itself. */
let applied: string[] = [];

/** WCAG relative luminance of a `#rrggbb` colour (0 = black, 1 = white). */
export function luminance(hex: string): number {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return 0.5;
  const n = parseInt(m[1], 16);
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((v) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** Text colour that stays readable on top of `bg`. */
export function onColor(bg: string): string {
  return luminance(bg) > 0.42 ? '#071018' : '#ffffff';
}

const mix = (a: string, pct: number, b: string) => `color-mix(in srgb, ${a} ${pct}%, ${b})`;

/** Derives UI chrome variables from a terminal palette. */
export function deriveChrome(p: TerminalPalette, kind: Theme['kind']): Record<string, string> {
  const bg = p.background;
  const fg = p.foreground;
  const dark = kind === 'dark';
  const edge = dark ? '255 255 255' : '20 28 60';
  return {
    'bg-terminal': bg,
    'bg-app': dark ? mix(bg, 66, 'black') : mix(bg, 88, '#9aa3bd'),
    'bg-sidebar': dark ? mix(bg, 80, 'black') : mix(bg, 92, '#c4cadc'),
    'bg-surface': dark ? mix(bg, 94, 'white') : mix(bg, 100, 'white'),
    'bg-raised': dark ? mix(bg, 90, 'white') : 'white',
    'bg-overlay': dark ? mix(bg, 86, 'white') : 'white',
    'bg-input': dark ? mix(bg, 78, 'black') : mix(bg, 92, '#c4cadc'),
    'bg-hover': `rgb(${edge} / ${dark ? 0.05 : 0.055})`,
    'bg-active': `rgb(${edge} / ${dark ? 0.09 : 0.1})`,
    border: `rgb(${edge} / ${dark ? 0.08 : 0.12})`,
    'border-strong': `rgb(${edge} / ${dark ? 0.15 : 0.22})`,
    text: fg,
    'text-dim': mix(fg, 72, bg),
    'text-faint': mix(fg, 46, bg),
    success: p.green,
    warning: p.yellow,
    danger: p.red,
  };
}

function accentVars(accent: string): Record<string, string> {
  return {
    accent,
    'accent-strong': mix(accent, 85, 'black'),
    'on-accent': onColor(accent),
    'accent-soft': mix(accent, 15, 'transparent'),
    'accent-ring': mix(accent, 52, 'transparent'),
    'grad-accent': `linear-gradient(135deg, ${mix(accent, 100, 'white 25%')} 0%, ${accent} 55%, var(--accent-2) 100%)`,
    'grad-accent-soft': `linear-gradient(135deg, ${mix(accent, 18, 'transparent')}, ${mix('var(--accent-2)', 18, 'transparent')})`,
  };
}

/** Applies a theme to the document. `accentOverride` is the user's chosen accent, if any. */
export function applyTheme(theme: Theme, accentOverride: string | null = null): void {
  const root = document.documentElement;
  for (const name of applied) root.style.removeProperty(`--${name}`);
  applied = [];

  const base = TUNED.has(theme.id) ? theme.id : theme.kind === 'light' ? 'aether-light' : 'aether-dark';
  root.dataset.theme = base;
  root.dataset.themeKind = theme.kind;

  const vars: Record<string, string> = {};
  if (!TUNED.has(theme.id)) Object.assign(vars, deriveChrome(theme.terminal, theme.kind));
  Object.assign(vars, theme.ui);
  const accent = accentOverride ?? theme.ui.accent ?? (TUNED.has(theme.id) ? null : theme.terminal.blue);
  if (accent) Object.assign(vars, accentVars(accent));

  for (const [name, value] of Object.entries(vars)) {
    root.style.setProperty(`--${name}`, value);
    applied.push(name);
  }
}

export function toXtermTheme(p: TerminalPalette): ITheme {
  return {
    background: p.background,
    foreground: p.foreground,
    cursor: p.cursor,
    cursorAccent: p.cursorAccent,
    selectionBackground: p.selectionBackground,
    selectionForeground: p.selectionForeground ?? undefined,
    black: p.black, red: p.red, green: p.green, yellow: p.yellow,
    blue: p.blue, magenta: p.magenta, cyan: p.cyan, white: p.white,
    brightBlack: p.brightBlack, brightRed: p.brightRed, brightGreen: p.brightGreen, brightYellow: p.brightYellow,
    brightBlue: p.brightBlue, brightMagenta: p.brightMagenta, brightCyan: p.brightCyan, brightWhite: p.brightWhite,
  };
}
