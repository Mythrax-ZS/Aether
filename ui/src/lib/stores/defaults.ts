import type { Settings } from '$lib/bindings';

// Mirrors the Rust defaults (crates/aether-core/src/model/settings.rs); the backend is the
// source of truth at runtime, this exists so the UI can render before settings load.
export function defaultSettings(): Settings {
  return {
    version: 1,
    general: { startup: 'welcome', closeToTray: false, launchAtLogin: false, confirmCloseActive: true, onboardingCompleted: false },
    appearance: {
      theme: 'aether-dark', lightTheme: 'aether-light', darkTheme: 'aether-dark', accent: null, uiScale: 1,
      sidebarWidth: 264, sidebarCollapsed: false, motion: 'system', density: 'comfortable',
      showStatusBar: true, customCss: false,
    },
    terminal: {
      fontFamily: "'JetBrains Mono Variable', 'Cascadia Code', Consolas, monospace", fontSize: 13.5, lineHeight: 1.25,
      letterSpacing: 0, fontWeight: 400, fontWeightBold: 700, ligatures: true, cursorStyle: 'bar', cursorBlink: true,
      scrollback: 20000, copyOnSelect: true, rightClick: 'menu', bell: 'visual', wordSeparators: " ()[]{}'\"`,;|<>",
      osc52: 'write-only', minimumContrast: 1, smoothScroll: true, renderer: 'auto', scrollOnOutput: false,
      scrollOnKeypress: true, theme: null, confirmMultilinePaste: true, unicodeGraphemes: true, inlineImages: true,
      screenReaderMode: false, paletteInTerminal: true,
    },
    ssh: {
      defaultUsername: '', keepaliveIntervalSecs: 30, keepaliveMax: 3, connectTimeoutSecs: 15, autoReconnect: true,
      reconnectMaxAttempts: 12, hostKeyPolicy: 'ask', agent: 'auto', identityFiles: ['~/.ssh/id_ed25519'],
      readUserKnownHosts: true, useSshConfig: true, defaultTerm: 'xterm-256color', tcpNodelay: true, x11Display: 'localhost:0',
    },
    sftp: { showHidden: false, concurrentTransfers: 3, overwrite: 'ask', preserveTimes: true, downloadDir: null },
    notifications: { onDisconnect: true, onCommandFinished: true, minCommandSecs: 10, onlyWhenUnfocused: true, fromServer: true },
    keybindings: {},
  };
}
