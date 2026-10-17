// Built-in themes. Aether Dark/Light have hand-tuned UI tokens in tokens.css; every other
// theme only specifies its terminal palette (and optionally an accent) and the UI chrome is
// derived from it (see apply.ts).

import type { TerminalPalette, Theme } from '$lib/bindings';

type Ansi = [
  black: string, red: string, green: string, yellow: string, blue: string, magenta: string, cyan: string, white: string,
];

function palette(
  base: { background: string; foreground: string; cursor: string; selection: string; cursorAccent?: string },
  normal: Ansi,
  bright: Ansi,
): TerminalPalette {
  return {
    background: base.background,
    foreground: base.foreground,
    cursor: base.cursor,
    cursorAccent: base.cursorAccent ?? base.background,
    selectionBackground: base.selection,
    selectionForeground: null,
    black: normal[0], red: normal[1], green: normal[2], yellow: normal[3],
    blue: normal[4], magenta: normal[5], cyan: normal[6], white: normal[7],
    brightBlack: bright[0], brightRed: bright[1], brightGreen: bright[2], brightYellow: bright[3],
    brightBlue: bright[4], brightMagenta: bright[5], brightCyan: bright[6], brightWhite: bright[7],
  };
}

function theme(id: string, name: string, kind: Theme['kind'], terminal: TerminalPalette, ui: Record<string, string> = {}, author: string | null = null): Theme {
  return { id, name, author, kind, ui, terminal, builtin: true };
}

export const BUILTIN_THEMES: Theme[] = [
  theme(
    'aether-dark', 'Aether Dark', 'dark',
    palette(
      { background: '#0c0e15', foreground: '#d9deec', cursor: '#7fd4ff', selection: 'rgba(110, 140, 255, 0.30)' },
      ['#1b2030', '#ff6b81', '#7ee0a2', '#f5c76b', '#6aa5ff', '#b891ff', '#5fd7e8', '#cdd3e4'],
      ['#4a5270', '#ff8da0', '#a0f0bf', '#ffdc92', '#8dbcff', '#d0b3ff', '#8be8f4', '#f2f5fc'],
    ),
    {}, 'Aether',
  ),
  theme(
    'aether-light', 'Aether Light', 'light',
    palette(
      { background: '#fbfcfe', foreground: '#252a3d', cursor: '#0a84d6', selection: 'rgba(10, 132, 214, 0.22)' },
      ['#3a4058', '#d3224f', '#14925d', '#a86a00', '#1f66d1', '#8f3fd0', '#0a8aa0', '#c8cde0'],
      ['#6b7390', '#e8456d', '#25ad76', '#c58410', '#3d84ef', '#a95fe6', '#1aa7bf', '#e9ecf5'],
    ),
    {}, 'Aether',
  ),
  theme(
    'midnight', 'Midnight', 'dark',
    palette(
      { background: '#070a13', foreground: '#c8d2f0', cursor: '#82aaff', selection: 'rgba(130, 170, 255, 0.28)' },
      ['#171c2e', '#ff7a93', '#8bd5a0', '#e8c37a', '#7aa2f7', '#c099ff', '#6bd1e8', '#c0caf0'],
      ['#3c4566', '#ff98ab', '#a9e6bb', '#f4d79a', '#9bbaff', '#d6b6ff', '#94e1f2', '#e8eeff'],
    ),
    { accent: '#82aaff' }, 'Aether',
  ),
  theme(
    'graphite', 'Graphite', 'dark',
    palette(
      { background: '#16181c', foreground: '#d4d7de', cursor: '#e6e8ee', selection: 'rgba(160, 170, 190, 0.28)' },
      ['#23262c', '#ef6b73', '#84cc9a', '#e5b567', '#6fa2e0', '#b48ad8', '#63c4cf', '#c3c7d1'],
      ['#555b66', '#ff8f96', '#a3e0b5', '#f2cb89', '#92bdf0', '#cba8ea', '#8ad8e1', '#eef0f5'],
    ),
    { accent: '#8fb4e8' }, 'Aether',
  ),
  theme(
    'nord', 'Nord', 'dark',
    palette(
      { background: '#2e3440', foreground: '#d8dee9', cursor: '#d8dee9', selection: '#434c5e' },
      ['#3b4252', '#bf616a', '#a3be8c', '#ebcb8b', '#81a1c1', '#b48ead', '#88c0d0', '#e5e9f0'],
      ['#4c566a', '#bf616a', '#a3be8c', '#ebcb8b', '#81a1c1', '#b48ead', '#8fbcbb', '#eceff4'],
    ),
    { accent: '#88c0d0' }, 'Arctic Ice Studio',
  ),
  theme(
    'dracula', 'Dracula', 'dark',
    palette(
      { background: '#282a36', foreground: '#f8f8f2', cursor: '#f8f8f2', selection: '#44475a' },
      ['#21222c', '#ff5555', '#50fa7b', '#f1fa8c', '#bd93f9', '#ff79c6', '#8be9fd', '#f8f8f2'],
      ['#6272a4', '#ff6e6e', '#69ff94', '#ffffa5', '#d6acff', '#ff92df', '#a4ffff', '#ffffff'],
    ),
    { accent: '#bd93f9' }, 'Zeno Rocha',
  ),
  theme(
    'tokyo-night', 'Tokyo Night', 'dark',
    palette(
      { background: '#1a1b26', foreground: '#c0caf5', cursor: '#c0caf5', selection: '#283457' },
      ['#15161e', '#f7768e', '#9ece6a', '#e0af68', '#7aa2f7', '#bb9af7', '#7dcfff', '#a9b1d6'],
      ['#414868', '#f7768e', '#9ece6a', '#e0af68', '#7aa2f7', '#bb9af7', '#7dcfff', '#c0caf5'],
    ),
    { accent: '#7aa2f7' }, 'enkia',
  ),
  theme(
    'catppuccin-mocha', 'Catppuccin Mocha', 'dark',
    palette(
      { background: '#1e1e2e', foreground: '#cdd6f4', cursor: '#f5e0dc', selection: '#585b70' },
      ['#45475a', '#f38ba8', '#a6e3a1', '#f9e2af', '#89b4fa', '#f5c2e7', '#94e2d5', '#bac2de'],
      ['#585b70', '#f38ba8', '#a6e3a1', '#f9e2af', '#89b4fa', '#f5c2e7', '#94e2d5', '#a6adc8'],
    ),
    { accent: '#cba6f7' }, 'Catppuccin',
  ),
  theme(
    'gruvbox-dark', 'Gruvbox Dark', 'dark',
    palette(
      { background: '#282828', foreground: '#ebdbb2', cursor: '#ebdbb2', selection: '#504945' },
      ['#282828', '#cc241d', '#98971a', '#d79921', '#458588', '#b16286', '#689d6a', '#a89984'],
      ['#928374', '#fb4934', '#b8bb26', '#fabd2f', '#83a598', '#d3869b', '#8ec07c', '#ebdbb2'],
    ),
    { accent: '#fabd2f' }, 'Pavel Pertsev',
  ),
  theme(
    'solarized-dark', 'Solarized Dark', 'dark',
    palette(
      { background: '#002b36', foreground: '#93a1a1', cursor: '#93a1a1', selection: '#073642' },
      ['#073642', '#dc322f', '#859900', '#b58900', '#268bd2', '#d33682', '#2aa198', '#eee8d5'],
      ['#586e75', '#cb4b16', '#93a1a1', '#839496', '#839496', '#6c71c4', '#93a1a1', '#fdf6e3'],
    ),
    { accent: '#2aa198' }, 'Ethan Schoonover',
  ),
  theme(
    'solarized-light', 'Solarized Light', 'light',
    palette(
      { background: '#fdf6e3', foreground: '#586e75', cursor: '#586e75', selection: '#eee8d5' },
      ['#073642', '#dc322f', '#859900', '#b58900', '#268bd2', '#d33682', '#2aa198', '#eee8d5'],
      ['#002b36', '#cb4b16', '#586e75', '#657b83', '#839496', '#6c71c4', '#93a1a1', '#fdf6e3'],
    ),
    { accent: '#268bd2' }, 'Ethan Schoonover',
  ),
];

export const DEFAULT_DARK = 'aether-dark';
export const DEFAULT_LIGHT = 'aether-light';

export function findTheme(id: string, custom: Theme[] = []): Theme | undefined {
  // Built-ins win a clash, matching the picker (which hides a custom theme that reuses a built-in id).
  return BUILTIN_THEMES.find((t) => t.id === id) ?? custom.find((t) => t.id === id);
}
