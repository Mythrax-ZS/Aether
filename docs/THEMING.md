# Theming

Aether has three layers you can change, from gentle to thorough:

1. **Pick a theme** — *Settings → Appearance*. Eleven are built in.
2. **Add your own theme** — a small JSON file dropped into the `themes` folder.
3. **Tweak the interface with CSS** — a `custom.css` loaded after everything else.

Your config folder (*Settings → General → Open config folder*) is
`%APPDATA%\Aether` for an installed copy and `AetherData\config` beside the program for the portable
one.

## Built-in themes

| Theme | Kind |
|---|---|
| Aether Dark (default) · Midnight · Graphite | dark |
| Nord · Dracula · Tokyo Night · Catppuccin Mocha · Gruvbox Dark · Solarized Dark | dark |
| Aether Light · Solarized Light | light |

*Match Windows* switches between a chosen dark and a chosen light theme as Windows does. A theme
also decides the terminal colours; individual hosts can override the theme and the font (*Edit host →
Terminal*).

## Writing a theme

Create `themes\my-theme.json`. Aether picks it up the next time the app starts, and it appears next
to the built-ins. Everything is validated when it loads; a file with a mistake is skipped (the reason is written to
the log — *Settings → General → Open logs*) rather than breaking the app.

```json
{
  "id": "midnight-rose",
  "name": "Midnight Rose",
  "author": "You",
  "kind": "dark",
  "ui": {
    "accent": "#ff7aa8",
    "accent-strong": "#ff5c93",
    "accent-2": "#b48cff",
    "bg-surface": "#14101a"
  },
  "terminal": {
    "background": "#120e17",
    "foreground": "#e8e0f0",
    "cursor": "#ff7aa8",
    "cursorAccent": "#120e17",
    "selectionBackground": "rgba(255, 122, 168, 0.28)",

    "black": "#1d1626",   "red": "#ff6b8a",   "green": "#8de0a8",   "yellow": "#f2cf8b",
    "blue": "#8aa4ff",    "magenta": "#d49cff", "cyan": "#7fe0ea",  "white": "#cfc6dc",

    "brightBlack": "#4a3f5a",   "brightRed": "#ff8fa7",   "brightGreen": "#a8f0c0",   "brightYellow": "#ffe0a3",
    "brightBlue": "#a9bdff",    "brightMagenta": "#e4b8ff", "brightCyan": "#a5f0f6",  "brightWhite": "#f6f1fb"
  }
}
```

### Fields

| Field | Meaning |
|---|---|
| `id` | Unique, letters / numbers / `-` / `_` only. If it matches a built-in's id, the built-in wins. |
| `name`, `author` | Shown in the theme picker (`author` is optional). |
| `kind` | `"dark"` or `"light"` — decides which side of *Match Windows* the theme can serve and the native control colours (scrollbars, form controls). |
| `terminal` | The 16 ANSI colours plus `background`, `foreground`, `cursor`, `cursorAccent`, `selectionBackground` and optionally `selectionForeground`. Required. |
| `ui` | Optional overrides for the interface's CSS variables (below), written **without** the leading `--`. |

Colour values may be `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, or CSS `rgb()`, `rgba()`, `hsl()`,
`hsla()`. Values in `ui` may not contain `;`, `{`, `}`, `<`, `>` or `url(` — they are placed into a
stylesheet, so they are kept inert.

## The interface variables

Every colour, radius and spacing in the UI is a CSS custom property defined in
[`ui/src/styles/tokens.css`](../ui/src/styles/tokens.css). The ones themes usually touch:

| Group | Variables |
|---|---|
| Surfaces | `bg-app`, `bg-sidebar`, `bg-surface`, `bg-raised`, `bg-overlay`, `bg-input`, `bg-terminal` |
| Interaction | `bg-hover`, `bg-active`, `border`, `border-strong`, `scrim` |
| Text | `text`, `text-dim`, `text-faint` |
| Accent | `accent`, `accent-strong`, `accent-2`, `on-accent`, `accent-soft`, `accent-ring`, `grad-accent` |
| Status | `success`, `warning`, `danger` |
| Shape & depth | `r-sm`, `r-md`, `r-lg`, `r-xl`, `shadow-md`, `shadow-lg` |
| Scrollbars | `scroll-thumb`, `scroll-thumb-hover` |
| Fonts | `font-ui`, `font-mono` |

A theme that sets only `accent` and the terminal palette already feels like its own thing; the
dark/light base supplies the rest.

## Custom CSS

Turn on **Settings → Appearance → Custom stylesheet**, then create `custom.css` in the config folder.
It is loaded after the built-in styles, so a plain override is enough:

```css
/* rounder, more compact sidebar rows */
:root {
  --r-md: 10px;
  --row-h: 28px;
}

/* a thin accent line under the active tab */
.tab.active {
  box-shadow: inset 0 -2px 0 var(--accent);
}
```

Tips:

- Prefer changing variables to overriding component selectors — variables are the stable surface;
  class names can change between versions.
- Open the developer tools of a debug build (`npm run dev`) to inspect which variables a part of the
  interface uses.
- If a stylesheet makes the app unusable, turn the setting off from *Settings → Appearance*, or
  rename / delete `custom.css` and restart.

## Terminal fonts and rendering

*Settings → Terminal* has the typeface (any installed font; fixed-width ones are listed first), size,
line height, letter spacing, weight, ligatures, cursor shape and blink, and minimum contrast.
**JetBrains Mono** ships with Aether, so the defaults — including programming ligatures — look the
same on every machine.
