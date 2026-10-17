// An in-memory backend. It powers unit tests and the browser-only dev preview
// (`npm run ui:dev`, no Tauri) with a scripted demo shell. It is never used in the app.

import type {
  ConnectionId,
  ForwardRule,
  ForwardStatus,
  Group,
  Host,
  HostsData,
  KeyInfo,
  RecentTarget,
  TerminalId,
  TerminalState,
  Theme,
  TransferInfo,
} from '$lib/bindings';
import { defaultSettings } from '$lib/stores/defaults';
import {
  type Api,
  type EventMap,
  type FileDropEvent,
  type PlatformApi,
  type TerminalHandlers,
  ApiError,
} from './types';

const uuid = () => crypto.randomUUID();
const now = () => Date.now();

export function sampleHost(label: string, hostname: string, extra: Partial<Host> = {}): Host {
  const t = now();
  return {
    id: uuid(), label, hostname, port: 22, username: 'deploy', groupId: null, tags: [], color: null, favorite: false,
    notes: '', auth: { methods: ['agent', 'public-key', 'keyboard-interactive', 'password'], key: null, identitiesOnly: false },
    jump: [],
    options: {
      connectTimeoutSecs: 15, keepaliveIntervalSecs: 30, keepaliveMax: 3, compression: false, agentForwarding: false,
      x11Forwarding: false, autoReconnect: true, term: 'xterm-256color', legacyAlgorithms: false, proxyCommand: '',
    },
    forwards: [], startup: { command: '', env: [] }, terminal: { themeId: null, fontFamily: null, fontSize: null },
    origin: { type: 'manual' }, sort: 0, createdAt: t, updatedAt: t, lastConnectedAt: null, connectCount: 0, ...extra,
  };
}

class Bus {
  private handlers = new Map<string, Set<(p: unknown) => void>>();
  on(event: string, h: (p: unknown) => void) {
    if (!this.handlers.has(event)) this.handlers.set(event, new Set());
    this.handlers.get(event)!.add(h);
    return () => this.handlers.get(event)?.delete(h);
  }
  emit(event: string, payload: unknown) {
    this.handlers.get(event)?.forEach((h) => h(payload));
  }
}

const ESC = '\x1b';
const PROMPT = `${ESC}[38;5;114m❯${ESC}[0m `;

/** A tiny scripted shell so the dev preview and tests have something to type into. */
class DemoShell {
  line = '';
  constructor(
    private out: (s: string) => void,
    private host: Host,
  ) {}
  start() {
    this.out(
      `${ESC}[2mLast login: Thu Oct 16 18:02:11 2026 from 10.0.0.12${ESC}[0m\r\n` +
        `${ESC}[1;36m${this.host.username}${ESC}[0m@${ESC}[1;35m${this.host.label}${ESC}[0m ${ESC}[2m~${ESC}[0m\r\n${PROMPT}`,
    );
  }
  input(data: string) {
    for (const ch of data) {
      if (ch === '\r') {
        this.out('\r\n');
        this.run(this.line.trim());
        this.line = '';
        this.out(PROMPT);
      } else if (ch === '\x7f') {
        if (this.line) {
          this.line = this.line.slice(0, -1);
          this.out('\b \b');
        }
      } else if (ch >= ' ') {
        this.line += ch;
        this.out(ch);
      }
    }
  }
  private run(cmd: string) {
    const [name, ...rest] = cmd.split(/\s+/);
    switch (name) {
      case '':
        return;
      case 'ls':
        this.out(
          `${ESC}[1;34mdocs${ESC}[0m  ${ESC}[1;34msrc${ESC}[0m  ${ESC}[1;32mdeploy.sh${ESC}[0m  README.md  ${ESC}[1;31mbackup.tar.gz${ESC}[0m  ${ESC}[36mnotes.txt${ESC}[0m\r\n`,
        );
        return;
      case 'echo':
        this.out(rest.join(' ') + '\r\n');
        return;
      case 'whoami':
        this.out(this.host.username + '\r\n');
        return;
      case 'uname':
        this.out('Linux ' + this.host.label + ' 6.8.0-aether x86_64 GNU/Linux\r\n');
        return;
      case 'colors':
        for (let i = 0; i < 16; i++) this.out(`${ESC}[48;5;${i}m  ${ESC}[0m`);
        this.out('\r\n');
        return;
      case 'ligatures':
        this.out('=> -> != === >= <= :: |> <| ... www\r\n');
        return;
      default:
        this.out(`${name}: command not found\r\n`);
    }
  }
}

function createFakePlatform(): PlatformApi {
  const noop = async () => {};
  return {
    isTauri: false,
    windowLabel: 'main',
    window: {
      minimize: noop,
      toggleMaximize: noop,
      close: noop,
      isMaximized: async () => false,
      onMaximizeChange: async () => () => {},
      onCloseRequested: async () => () => {},
      onFocusChange: async (cb) => {
        const f = () => cb(document.hasFocus());
        window.addEventListener('focus', f);
        window.addEventListener('blur', f);
        return () => {
          window.removeEventListener('focus', f);
          window.removeEventListener('blur', f);
        };
      },
      isFocused: async () => document.hasFocus(),
      setTitle: async (t) => void (document.title = t),
      setZoom: noop,
      setFullscreen: noop,
      isFullscreen: async () => !!document.fullscreenElement,
    },
    dialog: { pickFiles: async () => [], pickFolder: async () => null, saveFile: async () => null },
    clipboard: {
      readText: async () => navigator.clipboard.readText().catch(() => ''),
      writeText: async (t) => navigator.clipboard.writeText(t).catch(() => {}),
    },
    openUrl: async (url) => void window.open(url, '_blank', 'noopener'),
    notify: async (title, body) => console.info('[notify]', title, body),
    onFileDrop: async (_h: (e: FileDropEvent) => void) => () => {},
  };
}

export function createFakeApi(): Api {
  const bus = new Bus();
  let settings = defaultSettings();
  settings.general.onboardingCompleted = true;
  const infra = sampleHost('bastion', 'bastion.example.com', { tags: ['infra'], favorite: true });
  const prod: Group = { id: uuid(), name: 'Production', parentId: null, color: null, sort: 0, collapsed: false };
  const lab: Group = { id: uuid(), name: 'Home lab', parentId: null, color: null, sort: 1, collapsed: false };
  const hostsData: HostsData = {
    version: 1,
    groups: [prod, lab],
    hosts: [
      infra,
      sampleHost('web-01', '10.0.1.11', { groupId: prod.id, tags: ['web', 'nginx'], favorite: true, color: '#5fd6a2' }),
      sampleHost('web-02', '10.0.1.12', { groupId: prod.id, tags: ['web'] }),
      sampleHost('db-primary', '10.0.2.5', { groupId: prod.id, tags: ['postgres'], color: '#ff6f8a' }),
      sampleHost('pi-hole', '192.168.1.2', { groupId: lab.id, username: 'pi' }),
      sampleHost('nas', '192.168.1.20', { groupId: lab.id, username: 'admin' }),
    ],
  };
  const recents: RecentTarget[] = [{ target: 'root@203.0.113.7', lastUsed: now() - 3600_000, count: 3 }];
  const keys: KeyInfo[] = [];
  const shells = new Map<TerminalId, { shell: DemoShell; handlers?: TerminalHandlers }>();
  const encoder = new TextEncoder();
  const sessions = new Map<ConnectionId, Host>();

  const platform = createFakePlatform();
  const fail = (msg: string): never => {
    throw new ApiError({ code: 'unsupported', title: 'Not available in the preview', message: msg, hint: null, detail: null, retryable: false });
  };

  const api: Api = {
    mode: 'fake',
    platform,
    app: {
      info: async () => ({
        name: 'Aether', version: '0.1.0', debug: true, storageMode: 'installed', configDir: 'C:\\Users\\you\\AppData\\Roaming\\Aether',
        logsDir: 'C:\\Users\\you\\AppData\\Local\\Aether\\logs', os: 'windows x86_64', vaultBackend: 'In-memory (preview)', warnings: [],
      }),
      ready: async () => {},
      quit: async () => {},
      restart: async () => {},
      newWindow: async () => 'win-1',
      listFonts: async () => [
        { family: 'JetBrains Mono Variable', monospace: true }, { family: 'Cascadia Code', monospace: true },
        { family: 'Consolas', monospace: true }, { family: 'Segoe UI', monospace: false },
      ],
      openPath: async () => {},
      openLogs: async () => {},
      openConfig: async () => {},
      customCss: async () => '',
    },
    hosts: {
      snapshot: async () => structuredClone(hostsData),
      save: async (host) => {
        const i = hostsData.hosts.findIndex((h) => h.id === host.id);
        const saved = { ...host, label: host.label || host.hostname, updatedAt: now() };
        if (i >= 0) hostsData.hosts[i] = saved;
        else hostsData.hosts.push(saved);
        bus.emit('hosts:changed', null);
        return structuredClone(saved);
      },
      delete: async (ids) => {
        hostsData.hosts = hostsData.hosts.filter((h) => !ids.includes(h.id));
        bus.emit('hosts:changed', null);
      },
      duplicate: async (id) => {
        const src = hostsData.hosts.find((h) => h.id === id) ?? fail('unknown host');
        const copy = { ...structuredClone(src), id: uuid(), label: `${src.label} copy`, favorite: false };
        hostsData.hosts.push(copy);
        bus.emit('hosts:changed', null);
        return copy;
      },
      favorite: async (id, favorite) => {
        const h = hostsData.hosts.find((x) => x.id === id);
        if (h) h.favorite = favorite;
        bus.emit('hosts:changed', null);
      },
      move: async (ids, groupId) => {
        hostsData.hosts.forEach((h) => ids.includes(h.id) && (h.groupId = groupId));
        bus.emit('hosts:changed', null);
      },
      reorder: async () => {},
      saveGroup: async (group) => {
        const i = hostsData.groups.findIndex((g) => g.id === group.id);
        if (i >= 0) hostsData.groups[i] = group;
        else hostsData.groups.push(group);
        bus.emit('hosts:changed', null);
        return group;
      },
      deleteGroup: async (id) => {
        hostsData.groups = hostsData.groups.filter((g) => g.id !== id);
        hostsData.hosts.forEach((h) => h.groupId === id && (h.groupId = null));
        bus.emit('hosts:changed', null);
      },
      collapseGroup: async (id, collapsed) => {
        const g = hostsData.groups.find((x) => x.id === id);
        if (g) g.collapsed = collapsed;
      },
      recents: async () => structuredClone(recents),
      clearRecents: async () => void (recents.length = 0),
      passwordExists: async () => false,
      setPassword: async () => {},
      clearPassword: async () => {},
      sshConfigPreview: async () => ({ found: false, path: null, candidates: [], warnings: [] }),
      sshConfigImport: async () => ({ added: 0, skipped: 0, unlinkedJumps: 0 }),
    },
    keys: {
      list: async () => structuredClone(keys),
      inspect: async () => fail('Key tools need the desktop app.'),
      import: async () => fail('Key tools need the desktop app.'),
      generate: async () => fail('Key tools need the desktop app.'),
      rename: async () => fail('Key tools need the desktop app.'),
      delete: async () => {},
      publicKey: async () => '',
      exportPublic: async () => {},
      install: async () => {},
      knownHosts: async () => [],
      removeKnownHost: async () => 0,
    },
    settings: {
      get: async () => structuredClone(settings),
      patch: async (patch) => {
        const merge = (t: Record<string, unknown>, p: Record<string, unknown>) => {
          for (const [k, v] of Object.entries(p)) {
            if (v && typeof v === 'object' && !Array.isArray(v) && typeof t[k] === 'object' && t[k] !== null && k !== 'keybindings') {
              merge(t[k] as Record<string, unknown>, v as Record<string, unknown>);
            } else t[k] = v;
          }
        };
        merge(settings as unknown as Record<string, unknown>, patch);
        bus.emit('settings:changed', { settings: structuredClone(settings) });
        return structuredClone(settings);
      },
      reset: async () => (settings = defaultSettings()),
      customThemes: async () => [] as Theme[],
      saveTheme: async (t) => t,
      deleteTheme: async () => {},
      loadWorkspace: async () => null,
      saveWorkspace: async () => {},
    },
    sessions: {
      async open({ hostId }) {
        const host = hostsData.hosts.find((h) => h.id === hostId) ?? fail('unknown host');
        return openFake(host, true);
      },
      async quickConnect({ target }) {
        const m = /^(?:(.+)@)?([^:@\s]+)(?::(\d+))?$/.exec(target.trim());
        if (!m) fail('Enter a host like user@example.com');
        const host = sampleHost(m![2], m![2], { username: m![1] ?? 'you', port: m![3] ? Number(m![3]) : 22 });
        return openFake(host, false);
      },
      adopt: async () => {},
      close: async (id) => void sessions.delete(id),
      reconnect: async () => {},
      list: async () => [],
      state: async () => ({ type: 'closed' }),
      newTerminal: async (connectionId, _size, host) => {
        const terminalId = uuid();
        shells.set(terminalId, { shell: new DemoShell((s) => emitTerminal(terminalId, s), sessions.get(connectionId) ?? host) });
        return terminalId;
      },
      closeTerminal: async (_c, t) => void shells.delete(t),
      restartTerminal: async () => {},
      startForward: async (_c, rule: ForwardRule) => forwardStatus(rule),
      stopForward: async () => {},
      forwards: async () => [],
      respondPrompt: async () => true,
    },
    terminal: {
      async attach(id, handlers) {
        const entry = shells.get(id);
        if (!entry) fail('unknown terminal');
        entry!.handlers = handlers;
        queueMicrotask(() => entry!.shell.start());
        return { state: { type: 'running' } satisfies TerminalState, replayBytes: 0 };
      },
      detach: async (id) => void (shells.get(id)!.handlers = undefined),
      write: async (id, data) => shells.get(id)?.shell.input(data),
      writeBinary: async () => {},
      resize: async () => {},
      ack: () => {},
    },
    sftp: {
      list: async (_c, path) => ({ path: path || '/home/deploy', entries: [] }),
      home: async () => '/home/deploy',
      mkdir: async () => {},
      rename: async () => {},
      delete: async () => {},
      chmod: async () => {},
      diskUsage: async () => null,
      readText: async () => ({
        text: '',
        truncated: false,
        lineEnding: 'lf',
        mixedLineEndings: false,
        bom: false,
        version: { size: 0, modified: null },
      }),
      writeText: async () => ({ size: 0, modified: null }),
    },
    transfers: {
      planUpload: async () => fail('File transfer needs the desktop app.'),
      planDownload: async () => fail('File transfer needs the desktop app.'),
      start: async () => fail('File transfer needs the desktop app.'),
      discardPlan: async () => {},
      list: async () => [] as TransferInfo[],
      pause: async () => {},
      resume: async () => {},
      cancel: async () => {},
      dismiss: async () => {},
      clearFinished: async () => {},
    },
    local: {
      roots: async () => [{ path: 'C:\\', label: 'Local Disk (C:)', kind: 'drive' }],
      list: async (path) => ({ path: path || 'C:\\', parent: null, entries: [] }),
      mkdir: async () => {},
      rename: async () => {},
      delete: async () => {},
      exists: async () => false,
      defaultDownloadDir: async () => 'C:\\Users\\you\\Downloads',
    },
    on: async <K extends keyof EventMap>(event: K, handler: (payload: EventMap[K]) => void) =>
      bus.on(event, handler as (p: unknown) => void),
  };

  function emitTerminal(id: TerminalId, text: string) {
    shells.get(id)?.handlers?.onData(encoder.encode(text));
  }

  function forwardStatus(rule: ForwardRule): ForwardStatus {
    return { id: rule.id, rule, state: 'listening', boundPort: rule.bindPort, activeConnections: 0, totalConnections: 0, bytesUp: 0, bytesDown: 0, refused: 0, lastError: null };
  }

  function openFake(host: Host, saved: boolean) {
    const connectionId = uuid();
    const terminalId = uuid();
    sessions.set(connectionId, host);
    shells.set(terminalId, { shell: new DemoShell((s) => emitTerminal(terminalId, s), host) });
    setTimeout(() => bus.emit('session:state', { connectionId, state: { type: 'connecting', stage: { type: 'authenticating', method: 'public key' }, hop: null } }), 10);
    setTimeout(
      () =>
        bus.emit('session:state', {
          connectionId,
          state: {
            type: 'connected',
            info: { target: `${host.username}@${host.hostname}:${host.port}`, via: [], serverKey: null, negotiated: null, connectedAt: now() },
          },
        }),
      700,
    );
    return { connectionId, terminalId, host, saved };
  }

  return api;
}
