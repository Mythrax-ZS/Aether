// One terminal: an xterm.js instance wired to a backend terminal.
//
// Responsibilities: rendering (WebGL with a DOM fallback), input, resize, output
// acknowledgements (flow control), copy/paste, search, hyperlinks, shell-integration
// escape sequences, and applying settings/themes live. Framework-free so the Svelte
// component stays small.

import { ClipboardAddon, type ClipboardSelectionType, type IClipboardProvider } from '@xterm/addon-clipboard';
import { FitAddon } from '@xterm/addon-fit';
import { ImageAddon } from '@xterm/addon-image';
import { SearchAddon, type ISearchOptions } from '@xterm/addon-search';
import { UnicodeGraphemesAddon } from '@xterm/addon-unicode-graphemes';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { WebglAddon } from '@xterm/addon-webgl';
import { Terminal } from '@xterm/xterm';
import '@xterm/xterm/css/xterm.css';
import { api } from '$lib/api';
import type { TerminalId, TerminalSettings, TerminalState, Theme, TermSize } from '$lib/bindings';
import { toXtermTheme } from '$lib/themes/apply';
import { firstFamily, SYMBOL_SAMPLE, withSymbolFallback } from './font';
import { ligatureRanges } from './ligatures';
import { hadHiddenControls, plainText, safePaste, Throttle } from './safety';
import { TypingMark } from './typing';

export interface FinishedCommand {
  exitCode: number | null;
  durationMs: number;
}

/** The longest clipboard write a server may ask for (characters). */
export const MAX_REMOTE_CLIPBOARD = 100_000;

export interface ControllerEvents {
  /** The pane has a real size and the fonts are ready: time to open the session. */
  onReady(size: TermSize): void;
  onTitle(title: string): void;
  onCwd(path: string): void;
  onBell(): void;
  onOutput(): void;
  onState(state: TerminalState): void;
  onCommandFinished(info: FinishedCommand): void;
  /** OSC 9 / 777 desktop notification requests from the remote side — already reduced to plain, short text. */
  onNotify(title: string, body: string): void;
  /** Something the app did with what the server sent or the clipboard held, worth telling the person (a toast). */
  onNotice(message: string): void;
  onSelectionChange(hasSelection: boolean): void;
  onSearchResults(index: number, count: number): void;
  onResize(cols: number, rows: number): void;
  /** True for keys the app handles itself (palette, tabs, splits, ...). */
  isAppShortcut(e: KeyboardEvent): boolean;
  /** Ask before pasting multiple lines; resolves to whether to go ahead. */
  confirmPaste(text: string): Promise<boolean>;
  /** Ctrl+wheel / Ctrl+± adjust the font size. */
  onZoom(delta: number): void;
}

const ACK_BATCH = 24 * 1024;
const ACK_DELAY_MS = 25;
/** The least contrast (WCAG ratio) kept between text and background on a light theme, whatever the setting says. */
const LIGHT_THEME_MIN_CONTRAST = 4.5;
/** How long after composed text ends xterm may still be reading it (it does so on a timer), in milliseconds. */
const COMPOSITION_SETTLE_MS = 30;

export class TerminalController {
  readonly term: Terminal;
  readonly #fit = new FitAddon();
  readonly #search = new SearchAddon({ highlightLimit: 2000 });
  #webgl: WebglAddon | undefined;
  #joiner: number | undefined;
  #disposed = false;
  #visible = true;
  #readyFired = false;
  #settings: TerminalSettings;

  #terminalId: TerminalId | null = null;
  // Replay: when a window starts showing a terminal, the backend first sends what it printed before. Replayed history is
  // drawn but not acted on — the bell must not ring, a notification must not pop up and the clipboard must not change
  // a second time, and the terminal must not answer the queries in it again (the shell would receive the answers as
  // keystrokes). `#replayLeft` counts the history bytes still to arrive (unknown, so unlimited, until the backend says
  // how many there are); `#replaying` counts history chunks that xterm has been given but not yet parsed.
  #replayLeft = 0;
  #replaying = 0;
  #received = 0;
  // Marks the moments xterm is handling something the *person* did (a key, a paste): xterm reports those and its own
  // answers to queries through the same `onData`, and during a replay only the former may be sent.
  readonly #typing = new TypingMark();
  readonly #notifyThrottle = new Throttle(4000);
  readonly #clipboardThrottle = new Throttle(400);
  #pendingTitle: string | null = null;
  #titleTimer: ReturnType<typeof setTimeout> | undefined;
  #unacked = 0;
  #ackTimer: ReturnType<typeof setTimeout> | undefined;
  #resizeTimer: ReturnType<typeof setTimeout> | undefined;
  #lastSent = { cols: 0, rows: 0 };
  #selectionTimer: ReturnType<typeof setTimeout> | undefined;
  #commandStartedAt: number | null = null;
  #observer: ResizeObserver;
  #searchOpts: ISearchOptions = {};
  #lastQuery = '';

  constructor(
    private readonly container: HTMLElement,
    settings: TerminalSettings,
    theme: Theme,
    private readonly events: ControllerEvents,
  ) {
    this.#settings = settings;
    this.term = new Terminal({
      allowProposedApi: true,
      cols: 100,
      rows: 30,
      ...this.#options(settings, theme),
      theme: toXtermTheme(theme.terminal),
      customGlyphs: true,
      // A symbol from a fallback font that is wider than one cell is squeezed to fit instead of spilling into the next cell.
      rescaleOverlappingGlyphs: true,
      drawBoldTextInBrightColors: false,
      macOptionIsMeta: false,
      rightClickSelectsWord: false,
      scrollOnEraseInDisplay: true,
      cursorInactiveStyle: 'outline',
    });

    this.term.loadAddon(this.#fit);
    this.term.loadAddon(this.#search);
    this.term.loadAddon(
      new WebLinksAddon((e, uri) => {
        // Ctrl/Cmd+click, so selecting text by clicking never launches a browser.
        if (e.ctrlKey || e.metaKey || e.button === 1) void api.platform.openUrl(uri).catch(() => {});
      }),
    );
    if (settings.unicodeGraphemes) {
      this.term.loadAddon(new UnicodeGraphemesAddon());
      this.term.unicode.activeVersion = '15-graphemes';
    }
    if (settings.inlineImages) this.term.loadAddon(new ImageAddon({ sixelSupport: true, iipSupport: true }));
    this.term.loadAddon(new ClipboardAddon(undefined, this.#clipboardProvider()));

    this.#search.onDidChangeResults((r) => this.events.onSearchResults(r.resultIndex, r.resultCount));
    this.#wire();
    this.#observer = new ResizeObserver(() => this.fit());
    void this.#start();
  }

  #options(s: TerminalSettings, theme: Theme) {
    return {
      fontFamily: withSymbolFallback(s.fontFamily),
      fontSize: s.fontSize,
      lineHeight: s.lineHeight,
      letterSpacing: s.letterSpacing,
      fontWeight: s.fontWeight as never,
      fontWeightBold: s.fontWeightBold as never,
      cursorStyle: s.cursorStyle,
      cursorBlink: s.cursorBlink,
      scrollback: s.scrollback,
      // Thin text on a light background looks washed out, and pale ANSI colours on white can vanish: light themes get a floor.
      minimumContrastRatio: theme.kind === 'light' ? Math.max(s.minimumContrast, LIGHT_THEME_MIN_CONTRAST) : s.minimumContrast,
      smoothScrollDuration: s.smoothScroll ? 90 : 0,
      scrollOnUserInput: s.scrollOnKeypress,
      wordSeparator: s.wordSeparators,
      screenReaderMode: s.screenReaderMode,
    };
  }

  #clipboardProvider(): IClipboardProvider {
    return {
      // Remote programs may copy to the clipboard (tmux, vim) but never read it back. And even writing is bounded:
      // only the real clipboard (not the X "primary" selection), only while this terminal is the one in front, only
      // for live output (not history being replayed), not too much, not too often — and never silently.
      writeText: async (selection: ClipboardSelectionType, text: string) => {
        if (this.#settings.osc52 !== 'write-only') return;
        if (selection !== 'c' || this.#replaying > 0 || !this.#visible || !document.hasFocus()) return;
        if (text.length > MAX_REMOTE_CLIPBOARD) {
          this.events.onNotice(`A program on the server tried to copy ${text.length.toLocaleString()} characters; that’s more than Aether allows.`);
          return;
        }
        if (!this.#clipboardThrottle.allow()) return;
        await api.platform.clipboard.writeText(text);
        this.events.onNotice(`Copied ${text.length.toLocaleString()} characters from the server to your clipboard.`);
      },
      readText: async () => '',
    };
  }

  async #start(): Promise<void> {
    // Measure with the real font: opening before it loads would cache wrong cell metrics.
    const family = firstFamily(this.#settings.fontFamily);
    const size = this.#settings.fontSize;
    try {
      await Promise.race([
        // The symbols font too: a glyph drawn into the texture atlas before its font arrives is cached in the fallback
        // font and never redrawn.
        Promise.all([document.fonts.load(`${size}px "${family}"`), document.fonts.load(`${size}px "Aether Symbols"`, SYMBOL_SAMPLE)]),
        new Promise((r) => setTimeout(r, 1500)),
      ]);
    } catch {
      /* system fonts and missing fonts both land here: fine */
    }
    if (this.#disposed) return;
    this.term.open(this.container);
    this.#watchTyping();
    this.#applyRenderer();
    this.#applyLigatures();
    this.#observer.observe(this.container);
    this.fit();
  }

  /**
   * Raises the typing mark for the person's own input. xterm listens on its textarea in the capture phase, so this
   * listens one level up in the capture phase, which runs first. Composed text (an input method) is read by xterm off a
   * timer, so that mark lingers a little.
   */
  #watchTyping(): void {
    const textarea = this.term.textarea;
    const root = this.term.element;
    if (!textarea || !root) return;
    for (const type of ['keydown', 'keypress', 'input', 'compositionend', 'paste'] as const) {
      root.addEventListener(
        type,
        (e) => {
          if (e.target === textarea) this.#typing.begin(type === 'compositionend' ? COMPOSITION_SETTLE_MS : 0);
        },
        true,
      );
    }
  }

  /** Types text as if the person had: pasted (in bracketed-paste mode where the program asked for it). */
  insert(text: string): void {
    this.#typing.during(() => this.term.paste(text));
  }

  #wire(): void {
    const t = this.term;
    // What xterm sends upstream: keystrokes and pastes — and its answers to queries found in the output (device
    // attributes, cursor position, colours). Answers to *replayed* queries were given long ago and must not be given
    // again: the shell would read them as typing.
    t.onData((data) => {
      // While history is replaying, only the person's own typing goes through: whatever else xterm has to say now is an
      // answer to something in the replay.
      if (this.#terminalId && (this.#replaying <= 0 || this.#typing.active)) void api.terminal.write(this.#terminalId, data).catch(() => {});
    });
    t.onBinary((data) => {
      if (!this.#terminalId || (this.#replaying > 0 && !this.#typing.active)) return;
      const bytes = Uint8Array.from(data, (c) => c.charCodeAt(0));
      void api.terminal.writeBinary(this.#terminalId, bytes).catch(() => {});
    });
    // A title is text the server chose for a tab: shown as plain, bounded text, and no more often than ten times a
    // second (a program that sets it in a loop would otherwise redraw the tab strip for every line). The last one wins.
    t.onTitleChange((title) => {
      this.#pendingTitle = plainText(title, 120);
      if (this.#titleTimer) return;
      this.#titleTimer = setTimeout(() => {
        this.#titleTimer = undefined;
        if (this.#pendingTitle !== null) this.events.onTitle(this.#pendingTitle);
        this.#pendingTitle = null;
      }, 100);
    });
    t.onBell(() => {
      if (this.#replaying <= 0) this.events.onBell();
    });
    t.onResize(({ cols, rows }) => {
      this.events.onResize(cols, rows);
      this.#scheduleResize();
    });
    t.onSelectionChange(() => {
      const has = t.hasSelection();
      this.events.onSelectionChange(has);
      if (has && this.#settings.copyOnSelect) {
        // Wait for the drag to settle so we don't hammer the clipboard mid-selection.
        clearTimeout(this.#selectionTimer);
        this.#selectionTimer = setTimeout(() => {
          const text = t.getSelection();
          if (text) void api.platform.clipboard.writeText(text).catch(() => {});
        }, 120);
      }
    });

    t.attachCustomKeyEventHandler((e) => this.#onKey(e));
    t.attachCustomWheelEventHandler((e) => {
      if (e.ctrlKey) {
        e.preventDefault();
        this.events.onZoom(e.deltaY < 0 ? 1 : -1);
        return false;
      }
      return true;
    });

    // Shell integration: OSC 7 (cwd), OSC 133 (prompt marks), OSC 9/777 (notifications). The server controls every byte
    // of these, so a handler must never throw into the parser (a malformed escape would otherwise stop the output),
    // and the ones with side effects wait for live output.
    const guarded = (handler: (data: string) => void) => (data: string) => {
      try {
        handler(data);
      } catch (e) {
        console.warn('ignored a malformed terminal escape sequence', e);
      }
      return true;
    };
    t.parser.registerOscHandler(
      7,
      guarded((data) => {
        const m = /^file:\/\/[^/]*(\/.*)$/.exec(data);
        if (!m) return;
        let path = m[1];
        try {
          path = decodeURIComponent(path);
        } catch {
          /* a stray % in a path: use it as it is */
        }
        // The path ends up in the UI: plain text, one line, bounded.
        this.events.onCwd(plainText(path, 1024));
      }),
    );
    t.parser.registerOscHandler(
      133,
      guarded((data) => {
        if (this.#replaying > 0) {
          this.#commandStartedAt = null; // history: a command that "finished" long ago is not news
          return;
        }
        const [kind, arg] = data.split(';');
        if (kind === 'C') this.#commandStartedAt = Date.now();
        if (kind === 'D' && this.#commandStartedAt) {
          const code = arg !== undefined && arg !== '' ? Number.parseInt(arg, 10) : null;
          this.events.onCommandFinished({ exitCode: Number.isNaN(code) ? null : code, durationMs: Date.now() - this.#commandStartedAt });
          this.#commandStartedAt = null;
        }
      }),
    );
    t.parser.registerOscHandler(
      9,
      guarded((data) => {
        if (!data.startsWith('4;')) this.#notify('', data); // 9;4 is a progress report, not a message
      }),
    );
    t.parser.registerOscHandler(
      777,
      guarded((data) => {
        const [kind, title, ...body] = data.split(';');
        if (kind === 'notify') this.#notify(title ?? '', body.join(';'));
      }),
    );
  }

  /** A desktop notification asked for by the server: live only, rare, and reduced to plain short text. */
  #notify(title: string, body: string): void {
    if (this.#replaying > 0 || !this.#notifyThrottle.allow()) return;
    const cleanTitle = plainText(title, 80);
    const cleanBody = plainText(body, 300);
    if (!cleanTitle && !cleanBody) return;
    this.events.onNotify(cleanTitle, cleanBody);
  }

  /** Returns false to keep xterm from handling the key. */
  #onKey(e: KeyboardEvent): boolean {
    if (e.type !== 'keydown') return !this.events.isAppShortcut(e);
    if (this.events.isAppShortcut(e)) return false;

    const ctrl = e.ctrlKey && !e.altKey && !e.metaKey;
    const key = e.key.toLowerCase();
    // Copy: Ctrl+C when something is selected (otherwise it is ^C), Ctrl+Shift+C always.
    if (ctrl && key === 'c' && (e.shiftKey || this.term.hasSelection())) {
      this.copy();
      return false;
    }
    // Paste: Ctrl+V, Ctrl+Shift+V, Shift+Insert.
    if ((ctrl && key === 'v') || (e.shiftKey && !e.ctrlKey && e.key === 'Insert')) {
      void this.paste();
      return false;
    }
    return true;
  }

  // ── lifecycle ──────────────────────────────────────────────────────────────

  /** Starts showing a backend terminal (replaying what it already printed). */
  async attach(id: TerminalId): Promise<void> {
    if (this.#terminalId === id) return;
    if (this.#terminalId) {
      // Switching to a different session in the same pane (retry): start from a clean screen.
      await this.detach();
      this.term.reset();
      this.#unacked = 0;
    }
    this.#terminalId = id;
    // Until the backend says how much history there is, everything that arrives counts as history.
    this.#replayLeft = Number.POSITIVE_INFINITY;
    this.#received = 0;
    try {
      const attached = await api.terminal.attach(id, {
        onData: (bytes) => this.write(bytes),
        onState: (s) => this.events.onState(s),
      });
      // The history is the first `replayBytes` bytes of the stream; some may already have arrived.
      this.#replayLeft = Math.max(0, attached.replayBytes - this.#received);
      this.events.onState(attached.state);
      // Tell the server our size now that someone is listening.
      this.#lastSent = { cols: 0, rows: 0 };
      this.#scheduleResize(0);
    } catch (e) {
      // Nothing was replayed: whatever comes next is live.
      this.#replayLeft = 0;
      console.error('terminal attach failed', e);
    }
  }

  async detach(): Promise<void> {
    const id = this.#terminalId;
    this.#terminalId = null;
    if (id) await api.terminal.detach(id).catch(() => {});
  }

  write(bytes: Uint8Array): void {
    // History (replayed on attach) is drawn like anything else but is not "activity", and while xterm parses it nothing
    // it contains is acted on — see the notes at `#replayLeft`.
    const history = this.#replayLeft > 0;
    if (history) this.#replaying++;
    else this.events.onOutput();
    this.#replayLeft -= bytes.length;
    this.#received += bytes.length;
    this.term.write(bytes, () => {
      if (history) this.#replaying = Math.max(0, this.#replaying - 1);
      this.#unacked += bytes.length;
      if (this.#unacked >= ACK_BATCH) this.#flushAck();
      else if (!this.#ackTimer) this.#ackTimer = setTimeout(() => this.#flushAck(), ACK_DELAY_MS);
    });
  }

  /** Writes text that came from the UI, not the server (status messages). */
  notice(text: string): void {
    this.term.write(text);
  }

  #flushAck(): void {
    clearTimeout(this.#ackTimer);
    this.#ackTimer = undefined;
    if (this.#unacked > 0 && this.#terminalId) api.terminal.ack(this.#terminalId, this.#unacked);
    this.#unacked = 0;
  }

  #scheduleResize(delay = 40): void {
    clearTimeout(this.#resizeTimer);
    this.#resizeTimer = setTimeout(() => {
      const id = this.#terminalId;
      const { cols, rows } = this.term;
      if (!id || (cols === this.#lastSent.cols && rows === this.#lastSent.rows)) return;
      this.#lastSent = { cols, rows };
      void api.terminal.resize(id, this.size()).catch(() => {});
    }, delay);
  }

  size(): TermSize {
    const rect = this.container.getBoundingClientRect();
    return {
      cols: this.term.cols,
      rows: this.term.rows,
      pixelWidth: Math.round(rect.width * devicePixelRatio),
      pixelHeight: Math.round(rect.height * devicePixelRatio),
    };
  }

  // ── layout ─────────────────────────────────────────────────────────────────

  fit(): void {
    if (this.#disposed || !this.#visible || !this.term.element) return;
    if (!this.container.clientWidth || !this.container.clientHeight) return;
    try {
      const dims = this.#fit.proposeDimensions();
      if (dims && Number.isFinite(dims.cols) && Number.isFinite(dims.rows) && dims.cols > 0 && dims.rows > 0) {
        if (dims.cols !== this.term.cols || dims.rows !== this.term.rows) this.term.resize(dims.cols, dims.rows);
      }
    } catch {
      /* the renderer isn't ready yet; the next resize will retry */
    }
    if (!this.#readyFired && this.term.cols > 0) {
      this.#readyFired = true;
      this.events.onReady(this.size());
    }
  }

  /** Hidden terminals skip layout work and release their GPU context. */
  setVisible(visible: boolean): void {
    if (this.#visible === visible) return;
    this.#visible = visible;
    this.#applyRenderer();
    if (visible) requestAnimationFrame(() => this.fit());
  }

  focus(): void {
    if (this.#visible && !this.#disposed) this.term.focus();
  }

  // ── settings ───────────────────────────────────────────────────────────────

  apply(settings: TerminalSettings, theme: Theme): void {
    const prev = this.#settings;
    this.#settings = settings;
    const opts = this.#options(settings, theme);
    for (const [k, v] of Object.entries(opts)) {
      if ((this.term.options as Record<string, unknown>)[k] !== v) (this.term.options as Record<string, unknown>)[k] = v;
    }
    this.term.options.theme = toXtermTheme(theme.terminal);
    if (prev.renderer !== settings.renderer) this.#applyRenderer();
    if (prev.ligatures !== settings.ligatures) this.#applyLigatures();
    // Font metrics changed: refit and let the WebGL atlas rebuild.
    if (prev.fontFamily !== settings.fontFamily || prev.fontSize !== settings.fontSize || prev.lineHeight !== settings.lineHeight) {
      this.#webgl?.clearTextureAtlas();
      requestAnimationFrame(() => this.fit());
    }
  }

  #applyRenderer(): void {
    if (this.#disposed || !this.term.element) return;
    const wantWebgl = this.#visible && this.#settings.renderer !== 'dom';
    if (wantWebgl && !this.#webgl) {
      try {
        const gl = new WebglAddon();
        gl.onContextLoss(() => {
          gl.dispose();
          if (this.#webgl === gl) this.#webgl = undefined;
        });
        this.term.loadAddon(gl);
        this.#webgl = gl;
      } catch (e) {
        console.warn('WebGL renderer unavailable, using DOM renderer', e);
        this.#webgl = undefined;
      }
    } else if (!wantWebgl && this.#webgl) {
      this.#webgl.dispose();
      this.#webgl = undefined;
    }
  }

  #applyLigatures(): void {
    if (this.#joiner !== undefined) {
      this.term.deregisterCharacterJoiner(this.#joiner);
      this.#joiner = undefined;
    }
    if (this.#settings.ligatures) this.#joiner = this.term.registerCharacterJoiner(ligatureRanges);
  }

  // ── clipboard ──────────────────────────────────────────────────────────────

  copy(): void {
    const text = this.term.getSelection();
    if (text) void api.platform.clipboard.writeText(text).catch(() => {});
    if (!this.#settings.copyOnSelect) this.term.clearSelection();
  }

  async paste(): Promise<void> {
    let raw = '';
    try {
      raw = await api.platform.clipboard.readText();
    } catch {
      return;
    }
    // What goes to the shell is the clipboard minus control codes (an ESC can end a bracketed paste early and let the
    // rest run as typed commands) and minus a trailing line break (which would run the command before it was read).
    const text = safePaste(raw);
    if (!text) return;
    if (hadHiddenControls(raw)) this.events.onNotice('Hidden control characters were removed from the pasted text.');
    if (this.#settings.confirmMultilinePaste && /[\r\n]/.test(text)) {
      if (!(await this.events.confirmPaste(text))) return;
    }
    this.insert(text);
  }

  selectAll(): void {
    this.term.selectAll();
  }

  hasSelection(): boolean {
    return this.term.hasSelection();
  }

  getSelection(): string {
    return this.term.getSelection();
  }

  clear(): void {
    this.term.clear();
  }

  scrollToBottom(): void {
    this.term.scrollToBottom();
  }

  // ── search ─────────────────────────────────────────────────────────────────

  find(query: string, forward: boolean, options: { caseSensitive: boolean; regex: boolean; wholeWord: boolean }): boolean {
    this.#lastQuery = query;
    this.#searchOpts = {
      ...options,
      decorations: {
        matchBackground: '#ffd16655',
        matchBorder: '#ffd16699',
        matchOverviewRuler: '#ffd166',
        activeMatchBackground: '#5cc8ff88',
        activeMatchBorder: '#5cc8ff',
        activeMatchColorOverviewRuler: '#5cc8ff',
      },
    };
    if (!query) {
      this.#search.clearDecorations();
      this.events.onSearchResults(-1, 0);
      return false;
    }
    // Match marks beside the scrollbar are only worth the pixels while searching.
    if (!this.term.options.overviewRuler?.width) this.term.options.overviewRuler = { width: 8 };
    return forward ? this.#search.findNext(query, this.#searchOpts) : this.#search.findPrevious(query, this.#searchOpts);
  }

  clearSearch(): void {
    this.#search.clearDecorations();
    this.#lastQuery = '';
    this.events.onSearchResults(-1, 0);
    this.term.options.overviewRuler = { width: 0 };
  }

  get lastQuery(): string {
    return this.#lastQuery;
  }

  /** Plain-text search through the whole buffer (scrollback included), newest lines first. */
  searchBuffer(query: string, limit = 40): { line: number; text: string; start: number }[] {
    const q = query.toLowerCase();
    if (!q) return [];
    const buffer = this.term.buffer.active;
    const out: { line: number; text: string; start: number }[] = [];
    for (let y = buffer.length - 1; y >= 0 && out.length < limit; y--) {
      const text = buffer.getLine(y)?.translateToString(true);
      if (!text) continue;
      const start = text.toLowerCase().indexOf(q);
      if (start >= 0) out.push({ line: y, text, start });
    }
    return out;
  }

  /** Scrolls a buffer line into view and highlights the match. */
  revealLine(line: number, start: number, length: number): void {
    this.term.scrollToLine(Math.max(0, line - Math.floor(this.term.rows / 3)));
    this.term.select(start, line, length);
  }

  // ── teardown ───────────────────────────────────────────────────────────────

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    clearTimeout(this.#ackTimer);
    clearTimeout(this.#resizeTimer);
    clearTimeout(this.#selectionTimer);
    clearTimeout(this.#titleTimer);
    this.#typing.dispose();
    this.#observer.disconnect();
    void this.detach();
    this.#webgl?.dispose();
    this.term.dispose();
  }
}
