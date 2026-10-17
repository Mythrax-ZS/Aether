<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$lib/api';
  import type { CursorStyle, FontInfo } from '$lib/bindings';
  import { firstFamily, withSymbolFallback } from '$lib/terminal/font';
  import { ligatureRanges } from '$lib/terminal/ligatures';
  import { settings } from '$lib/stores/settings.svelte';
  import Input from '$lib/components/ui/Input.svelte';
  import Segmented from '$lib/components/ui/Segmented.svelte';
  import Select from '$lib/components/ui/Select.svelte';
  import Slider from '$lib/components/ui/Slider.svelte';
  import Switch from '$lib/components/ui/Switch.svelte';
  import SettingRow from './SettingRow.svelte';

  const t = $derived(settings.value.terminal);
  const theme = $derived(settings.terminalTheme.terminal);

  let fonts = $state<FontInfo[]>([]);
  onMount(() => void api.app.listFonts().then((f) => (fonts = f)).catch(() => {}));

  const current = $derived(firstFamily(t.fontFamily));

  // "JetBrains Mono" is bundled; map it to the variable-font family name the stylesheet registers.
  const BUNDLED = 'JetBrains Mono Variable';
  const FALLBACK = "'Cascadia Code', Consolas, 'Courier New', monospace";

  const fontOptions = $derived.by(() => {
    const mono = fonts.filter((f) => f.monospace).map((f) => f.family);
    const options = [
      { value: BUNDLED, label: 'JetBrains Mono (included)', group: 'Included' },
      ...mono.filter((f) => f !== 'JetBrains Mono').map((f) => ({ value: f, label: f, group: 'Installed monospace fonts' })),
    ];
    if (current && !options.some((o) => o.value === current)) options.push({ value: current, label: current, group: 'Current' });
    return options;
  });

  function setFont(family: string) {
    const stack = family === BUNDLED ? `'${BUNDLED}', ${FALLBACK}` : `'${family}', '${BUNDLED}', ${FALLBACK}`;
    settings.patch({ terminal: { fontFamily: stack } });
  }

  const SCROLLBACK = [1000, 5000, 10000, 20000, 50000, 100000].map((n) => ({ value: n, label: `${n.toLocaleString()} lines` }));

  // The live preview: real text in the chosen font and colours, with ligatures applied via spans.
  const SAMPLE = [
    { prompt: true, text: 'ssh deploy@prod-web-1' },
    { text: 'Last login: Thu Oct 16 18:02:11 from 10.0.0.12' },
    { prompt: true, text: 'git log --oneline | head -3' },
    { text: 'a1b2c3d => fix: retry when peer != ready' },
    { text: '9f8e7d6 -> feat: add 0xFF00 parser (>= v2.1)' },
  ];
  function segments(text: string): { text: string; lig: boolean }[] {
    if (!t.ligatures) return [{ text, lig: false }];
    const out: { text: string; lig: boolean }[] = [];
    let at = 0;
    for (const [s, e] of ligatureRanges(text)) {
      if (s > at) out.push({ text: text.slice(at, s), lig: false });
      out.push({ text: text.slice(s, e), lig: true });
      at = e;
    }
    if (at < text.length) out.push({ text: text.slice(at), lig: false });
    return out;
  }
</script>

<div class="preview" style:background={theme.background} style:color={theme.foreground} style:font-family={withSymbolFallback(t.fontFamily)} style:font-size="{t.fontSize}px" style:line-height={t.lineHeight} style:letter-spacing="{t.letterSpacing}px" style:font-weight={t.fontWeight} style:font-variant-ligatures={t.ligatures ? 'normal' : 'none'} aria-hidden="true">
  {#each SAMPLE as line, i (i)}
    <div>
      {#if line.prompt}<span style:color={theme.green}>{'❯ '}</span>{/if}{#each segments(line.text) as seg, k (k)}<span>{seg.text}</span>{/each}{#if i === SAMPLE.length - 1}<span class="cursor" class:block={t.cursorStyle === 'block'} class:bar={t.cursorStyle === 'bar'} class:under={t.cursorStyle === 'underline'} class:blink={t.cursorBlink} style:background={theme.cursor}></span>{/if}
    </div>
  {/each}
</div>

<div class="fsec">
  <h3>Font</h3>
  <SettingRow title="Typeface" description="Monospace fonts installed on this PC. Fonts with ligatures (JetBrains Mono, Cascadia Code, Fira Code) make => and != look like single symbols.">
    <div class="w260"><Select value={current} options={fontOptions} size="sm" label="" onchange={setFont} /></div>
  </SettingRow>
  <SettingRow title="Size">
    <Slider value={t.fontSize} min={8} max={28} step={0.5} label="Font size" format={(v) => `${v} px`} onchange={(v) => settings.patch({ terminal: { fontSize: v } })} />
  </SettingRow>
  <SettingRow title="Line height">
    <Slider value={t.lineHeight} min={1} max={1.8} step={0.05} label="Line height" format={(v) => v.toFixed(2)} onchange={(v) => settings.patch({ terminal: { lineHeight: v } })} />
  </SettingRow>
  <SettingRow title="Letter spacing">
    <Slider value={t.letterSpacing} min={-1} max={3} step={0.25} label="Letter spacing" format={(v) => `${v} px`} onchange={(v) => settings.patch({ terminal: { letterSpacing: v } })} />
  </SettingRow>
  <SettingRow title="Weight">
    <Segmented
      value={String(t.fontWeight)}
      label="Font weight"
      options={[
        { value: '300', label: 'Light' },
        { value: '400', label: 'Regular' },
        { value: '500', label: 'Medium' },
      ]}
      onchange={(v) => settings.patch({ terminal: { fontWeight: Number(v) } })}
    />
  </SettingRow>
  <SettingRow title="Ligatures" description="Draw combinations like =>, !=, === and -> as joined symbols.">
    <Switch checked={t.ligatures} aria="Ligatures" onchange={(v) => settings.patch({ terminal: { ligatures: v } })} />
  </SettingRow>
</div>

<div class="fsec">
  <h3>Cursor</h3>
  <SettingRow title="Shape">
    <Segmented
      value={t.cursorStyle}
      label="Cursor shape"
      options={[
        { value: 'bar', label: 'Bar' },
        { value: 'block', label: 'Block' },
        { value: 'underline', label: 'Underline' },
      ]}
      onchange={(v) => settings.patch({ terminal: { cursorStyle: v as CursorStyle } })}
    />
  </SettingRow>
  <SettingRow title="Blink">
    <Switch checked={t.cursorBlink} aria="Blinking cursor" onchange={(v) => settings.patch({ terminal: { cursorBlink: v } })} />
  </SettingRow>
</div>

<div class="fsec">
  <h3>Behaviour</h3>
  <SettingRow title="Scrollback" description="Lines kept above the screen. More history uses more memory.">
    <div class="w160"><Select value={t.scrollback} options={SCROLLBACK} size="sm" label="" onchange={(v) => settings.patch({ terminal: { scrollback: Number(v) } })} /></div>
  </SettingRow>
  <SettingRow title="Copy on select" description="Selecting text copies it straight away.">
    <Switch checked={t.copyOnSelect} aria="Copy on select" onchange={(v) => settings.patch({ terminal: { copyOnSelect: v } })} />
  </SettingRow>
  <SettingRow title="Right click">
    <Segmented
      value={t.rightClick}
      label="Right click action"
      options={[
        { value: 'menu', label: 'Show menu' },
        { value: 'paste', label: 'Copy / paste' },
      ]}
      onchange={(v) => settings.patch({ terminal: { rightClick: v as 'menu' | 'paste' } })}
    />
  </SettingRow>
  <SettingRow title="Ask before pasting several lines" description="A multi-line paste can run commands you didn’t mean to — worth a glance first.">
    <Switch checked={t.confirmMultilinePaste} aria="Confirm multi-line paste" onchange={(v) => settings.patch({ terminal: { confirmMultilinePaste: v } })} />
  </SettingRow>
  <SettingRow title="Bell">
    <Segmented
      value={t.bell}
      label="Bell"
      options={[
        { value: 'none', label: 'Off' },
        { value: 'visual', label: 'Flash' },
        { value: 'sound', label: 'Sound' },
      ]}
      onchange={(v) => settings.patch({ terminal: { bell: v as 'none' | 'visual' | 'sound' } })}
    />
  </SettingRow>
  <SettingRow title="Jump to the prompt when you type" description="Scroll back to the bottom on a keypress.">
    <Switch checked={t.scrollOnKeypress} aria="Scroll on keypress" onchange={(v) => settings.patch({ terminal: { scrollOnKeypress: v } })} />
  </SettingRow>
  <SettingRow title="Follow new output" description="Keep scrolling to the bottom as output arrives, even if you scrolled up.">
    <Switch checked={t.scrollOnOutput} aria="Scroll on output" onchange={(v) => settings.patch({ terminal: { scrollOnOutput: v } })} />
  </SettingRow>
  <SettingRow title="Smooth scrolling">
    <Switch checked={t.smoothScroll} aria="Smooth scrolling" onchange={(v) => settings.patch({ terminal: { smoothScroll: v } })} />
  </SettingRow>
  <SettingRow title="Ctrl+K opens the command palette in the terminal" description="Turn off to send Ctrl+K to the shell (readline’s “kill to end of line”). Ctrl+Shift+P always works.">
    <Switch checked={t.paletteInTerminal} aria="Ctrl+K palette in terminal" onchange={(v) => settings.patch({ terminal: { paletteInTerminal: v } })} />
  </SettingRow>
</div>

<div class="fsec">
  <h3>Advanced</h3>
  <SettingRow title="Renderer" description="Auto uses the GPU when available and falls back to a slower, always-working renderer.">
    <Segmented
      value={t.renderer}
      label="Renderer"
      options={[
        { value: 'auto', label: 'Auto' },
        { value: 'webgl', label: 'GPU' },
        { value: 'dom', label: 'Compatible' },
      ]}
      onchange={(v) => settings.patch({ terminal: { renderer: v as 'auto' | 'webgl' | 'dom' } })}
    />
  </SettingRow>
  <SettingRow title="Minimum contrast" description="Nudges hard-to-read colour pairs toward readable. Off leaves colours alone, except on light themes, which always keep at least 4.5:1.">
    <Slider value={t.minimumContrast} min={1} max={7} step={0.5} label="Minimum contrast" format={(v) => (v === 1 ? 'Off' : `${v}:1`)} onchange={(v) => settings.patch({ terminal: { minimumContrast: v } })} />
  </SettingRow>
  <SettingRow title="Emoji and combined characters" description="Show family emoji, flags and accented letters as single cells.">
    <Switch checked={t.unicodeGraphemes} aria="Unicode grapheme clusters" onchange={(v) => settings.patch({ terminal: { unicodeGraphemes: v } })} />
  </SettingRow>
  <SettingRow title="Inline images" description="Sixel and iTerm2 images (e.g. from imgcat). Applies to new tabs.">
    <Switch checked={t.inlineImages} aria="Inline images" onchange={(v) => settings.patch({ terminal: { inlineImages: v } })} />
  </SettingRow>
  <SettingRow title="Let programs write to the clipboard" description="tmux and vim can copy to your clipboard through the connection. Programs can never read it back.">
    <Switch checked={t.osc52 === 'write-only'} aria="Allow clipboard writes from the server" onchange={(v) => settings.patch({ terminal: { osc52: v ? 'write-only' : 'deny' } })} />
  </SettingRow>
  <SettingRow title="Screen reader mode" description="Mirrors terminal output into the page so assistive technology can read it. Slower.">
    <Switch checked={t.screenReaderMode} aria="Screen reader mode" onchange={(v) => settings.patch({ terminal: { screenReaderMode: v } })} />
  </SettingRow>
  <SettingRow title="Word separators" description="Characters that end a word when you double-click.">
    <div class="w260"><Input size="sm" mono value={t.wordSeparators} oninput={(e) => settings.patch({ terminal: { wordSeparators: (e.currentTarget as HTMLInputElement).value } })} /></div>
  </SettingRow>
  <SettingRow title="Terminal colours" description="Use a different colour theme in terminals than in the rest of the app.">
    <div class="w220">
      <Select
        value={t.theme ?? ''}
        options={[{ value: '', label: 'Same as the app' }, ...settings.themes.map((x) => ({ value: x.id, label: x.name, group: x.kind === 'dark' ? 'Dark' : 'Light' }))]}
        size="sm"
        label=""
        onchange={(v) => settings.patch({ terminal: { theme: v || null } })}
      />
    </div>
  </SettingRow>
</div>

<style>
  .preview {
    position: sticky;
    top: -8px;
    z-index: 3;
    margin-bottom: 22px;
    padding: 14px 16px;
    border-radius: var(--r-lg);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-md);
    overflow: hidden;
    white-space: pre;
    user-select: none;
  }
  .cursor {
    display: inline-block;
    vertical-align: text-bottom;
    margin-left: 1px;
  }
  .cursor.block {
    width: 0.6em;
    height: 1.2em;
    opacity: 0.85;
  }
  .cursor.bar {
    width: 2px;
    height: 1.2em;
  }
  .cursor.under {
    width: 0.6em;
    height: 2px;
  }
  .cursor.blink {
    animation: pulse-dot 1.1s steps(2, jump-none) infinite;
  }
  .w160 {
    width: 160px;
  }
  .w220 {
    width: 220px;
  }
  .w260 {
    width: 260px;
  }
</style>
