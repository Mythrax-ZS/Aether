// The contract between the UI and the backend. `tauri.ts` implements it over IPC;
// `fake.ts` implements it in memory for tests and the browser-only dev preview.

import type {
  AppInfo,
  AttachedTerminal,
  ArgsEvent,
  BannerEvent,
  ConflictPolicy,
  ConnectionId,
  ErrorInfo,
  FileVersion,
  FontInfo,
  ForwardRule,
  ForwardStatus,
  ForwardsEvent,
  Group,
  GroupDeleteMode,
  GroupId,
  Host,
  HostId,
  HostsData,
  ImportSummary,
  KeyGenerateRequest,
  KeyId,
  KeyImportRequest,
  KeyImportSource,
  KeyInfo,
  KeyInspectResult,
  KnownHostEntry,
  LatencyEvent,
  LocalListing,
  LocalRoot,
  NewWindowRequest,
  OpenSessionRequest,
  OpenedSession,
  PlanSummary,
  PromptClosedEvent,
  PromptEvent,
  PromptId,
  PromptReply,
  QuickConnectRequest,
  RecentTarget,
  RemoteListing,
  RemoteText,
  SessionStateEvent,
  SessionState,
  SessionSummary,
  Settings,
  SettingsEvent,
  SshConfigPreview,
  TermSize,
  TerminalId,
  TerminalState,
  Theme,
  TrayConnectEvent,
  TransferEvent,
  TransferId,
  TransferInfo,
  WriteTextRequest,
  DiskUsage,
} from '$lib/bindings';

export type { ErrorInfo };

/** A rejected backend call. Always carries a user-presentable `info`. */
export class ApiError extends Error {
  readonly info: ErrorInfo;
  constructor(info: ErrorInfo) {
    super(`${info.title}: ${info.message}`);
    this.name = 'ApiError';
    this.info = info;
  }
}

/** Normalises whatever IPC rejected with into an [`ErrorInfo`]. */
export function toErrorInfo(e: unknown): ErrorInfo {
  if (e instanceof ApiError) return e.info;
  if (e && typeof e === 'object' && 'code' in e && 'title' in e && 'message' in e) return e as ErrorInfo;
  const text = e instanceof Error ? e.message : typeof e === 'string' ? e : JSON.stringify(e);
  return {
    code: 'internal',
    title: 'Something went wrong',
    message: 'Aether hit an unexpected problem.',
    hint: null,
    detail: text,
    retryable: false,
  };
}

export interface EventMap {
  'session:state': SessionStateEvent;
  'session:forwards': ForwardsEvent;
  'session:latency': LatencyEvent;
  'session:banner': BannerEvent;
  'prompt:request': PromptEvent;
  'prompt:closed': PromptClosedEvent;
  'transfer:update': TransferEvent;
  'settings:changed': SettingsEvent;
  'hosts:changed': null;
  'tray:connect': TrayConnectEvent;
  'app:args': ArgsEvent;
}

export interface TerminalHandlers {
  onData(bytes: Uint8Array): void;
  onState(state: TerminalState): void;
}

export interface FileDropEvent {
  type: 'enter' | 'over' | 'drop' | 'leave';
  paths: string[];
  /** CSS pixels relative to the window. */
  x: number;
  y: number;
}

export interface DialogFilter {
  name: string;
  extensions: string[];
}

export interface PlatformApi {
  readonly isTauri: boolean;
  readonly windowLabel: string;
  window: {
    minimize(): Promise<void>;
    toggleMaximize(): Promise<void>;
    close(): Promise<void>;
    isMaximized(): Promise<boolean>;
    onMaximizeChange(cb: (maximized: boolean) => void): Promise<() => void>;
    onFocusChange(cb: (focused: boolean) => void): Promise<() => void>;
    /**
     * Called when the window is asked to close, by any route (the X, Alt+F4, the taskbar). The window closes
     * once the handler returns unless it called `prevent()`; after that it closes only if asked again.
     */
    onCloseRequested(cb: (prevent: () => void) => void | Promise<void>): Promise<() => void>;
    isFocused(): Promise<boolean>;
    setTitle(title: string): Promise<void>;
    setZoom(factor: number): Promise<void>;
    setFullscreen(on: boolean): Promise<void>;
    isFullscreen(): Promise<boolean>;
  };
  dialog: {
    pickFiles(opts?: { title?: string; filters?: DialogFilter[]; multiple?: boolean; defaultPath?: string }): Promise<string[]>;
    pickFolder(opts?: { title?: string; defaultPath?: string }): Promise<string | null>;
    saveFile(opts?: { title?: string; defaultPath?: string; filters?: DialogFilter[] }): Promise<string | null>;
  };
  clipboard: { readText(): Promise<string>; writeText(text: string): Promise<void> };
  openUrl(url: string): Promise<void>;
  notify(title: string, body?: string): Promise<void>;
  onFileDrop(handler: (e: FileDropEvent) => void): Promise<() => void>;
}

export interface Api {
  readonly mode: 'tauri' | 'fake';
  platform: PlatformApi;

  app: {
    info(): Promise<AppInfo>;
    ready(): Promise<void>;
    quit(): Promise<void>;
    restart(): Promise<void>;
    newWindow(request: NewWindowRequest): Promise<string>;
    listFonts(): Promise<FontInfo[]>;
    openPath(path: string): Promise<void>;
    openLogs(): Promise<void>;
    openConfig(): Promise<void>;
    customCss(): Promise<string>;
  };

  hosts: {
    snapshot(): Promise<HostsData>;
    save(host: Host): Promise<Host>;
    delete(ids: HostId[]): Promise<void>;
    duplicate(id: HostId): Promise<Host>;
    favorite(id: HostId, favorite: boolean): Promise<void>;
    move(ids: HostId[], groupId: GroupId | null): Promise<void>;
    reorder(ids: HostId[]): Promise<void>;
    saveGroup(group: Group): Promise<Group>;
    deleteGroup(id: GroupId, mode: GroupDeleteMode): Promise<void>;
    collapseGroup(id: GroupId, collapsed: boolean): Promise<void>;
    recents(): Promise<RecentTarget[]>;
    clearRecents(): Promise<void>;
    passwordExists(id: HostId): Promise<boolean>;
    setPassword(id: HostId, password: string): Promise<void>;
    clearPassword(id: HostId): Promise<void>;
    sshConfigPreview(): Promise<SshConfigPreview>;
    sshConfigImport(aliases: string[], groupId: GroupId | null): Promise<ImportSummary>;
  };

  keys: {
    list(): Promise<KeyInfo[]>;
    inspect(source: KeyImportSource, passphrase?: string): Promise<KeyInspectResult>;
    import(request: KeyImportRequest): Promise<KeyInfo>;
    generate(request: KeyGenerateRequest): Promise<KeyInfo>;
    rename(id: KeyId, label: string): Promise<KeyInfo>;
    delete(id: KeyId): Promise<void>;
    publicKey(id: KeyId): Promise<string>;
    exportPublic(id: KeyId, path: string): Promise<void>;
    install(connectionId: ConnectionId, id: KeyId): Promise<void>;
    knownHosts(): Promise<KnownHostEntry[]>;
    /** Forgets one key of Aether's own file; `fingerprint` guards against the file having changed since the list was read. */
    removeKnownHost(line: number, fingerprint: string): Promise<number>;
  };

  settings: {
    get(): Promise<Settings>;
    patch(patch: Record<string, unknown>): Promise<Settings>;
    reset(): Promise<Settings>;
    customThemes(): Promise<Theme[]>;
    saveTheme(theme: Theme): Promise<Theme>;
    deleteTheme(id: string): Promise<void>;
    loadWorkspace(): Promise<unknown | null>;
    saveWorkspace(workspace: unknown): Promise<void>;
  };

  sessions: {
    open(request: OpenSessionRequest): Promise<OpenedSession>;
    quickConnect(request: QuickConnectRequest): Promise<OpenedSession>;
    adopt(connectionId: ConnectionId): Promise<void>;
    close(connectionId: ConnectionId): Promise<void>;
    reconnect(connectionId: ConnectionId): Promise<void>;
    list(): Promise<SessionSummary[]>;
    state(connectionId: ConnectionId): Promise<SessionState>;
    newTerminal(connectionId: ConnectionId, size: TermSize, host: Host): Promise<TerminalId>;
    closeTerminal(connectionId: ConnectionId, terminalId: TerminalId): Promise<void>;
    restartTerminal(connectionId: ConnectionId, terminalId: TerminalId): Promise<void>;
    startForward(connectionId: ConnectionId, rule: ForwardRule): Promise<ForwardStatus>;
    stopForward(connectionId: ConnectionId, ruleId: string): Promise<void>;
    forwards(connectionId: ConnectionId): Promise<ForwardStatus[]>;
    respondPrompt(id: PromptId, reply: PromptReply): Promise<boolean>;
  };

  terminal: {
    attach(id: TerminalId, handlers: TerminalHandlers): Promise<AttachedTerminal>;
    detach(id: TerminalId): Promise<void>;
    write(id: TerminalId, data: string): Promise<void>;
    writeBinary(id: TerminalId, data: Uint8Array): Promise<void>;
    resize(id: TerminalId, size: TermSize): Promise<void>;
    ack(id: TerminalId, bytes: number): void;
  };

  sftp: {
    list(connectionId: ConnectionId, path: string): Promise<RemoteListing>;
    home(connectionId: ConnectionId): Promise<string>;
    mkdir(connectionId: ConnectionId, path: string): Promise<void>;
    rename(connectionId: ConnectionId, from: string, to: string): Promise<void>;
    delete(connectionId: ConnectionId, paths: string[]): Promise<void>;
    chmod(connectionId: ConnectionId, path: string, mode: number, recursive: boolean): Promise<void>;
    diskUsage(connectionId: ConnectionId, path: string): Promise<DiskUsage | null>;
    /** Opens a text file for editing. Anything that can't be edited safely (binary, not UTF-8) is rejected. */
    readText(connectionId: ConnectionId, path: string): Promise<RemoteText>;
    /** Saves it atomically; rejects with a `conflict` error if the file changed on the server since `expected`. */
    writeText(connectionId: ConnectionId, request: WriteTextRequest): Promise<FileVersion>;
  };

  transfers: {
    planUpload(connectionId: ConnectionId, localPaths: string[], remoteDir: string): Promise<PlanSummary>;
    planDownload(connectionId: ConnectionId, remotePaths: string[], localDir: string): Promise<PlanSummary>;
    start(planId: string, policy: ConflictPolicy): Promise<TransferId>;
    discardPlan(planId: string): Promise<void>;
    list(): Promise<TransferInfo[]>;
    pause(id: TransferId): Promise<void>;
    resume(id: TransferId): Promise<void>;
    cancel(id: TransferId): Promise<void>;
    dismiss(id: TransferId): Promise<void>;
    clearFinished(): Promise<void>;
  };

  local: {
    roots(): Promise<LocalRoot[]>;
    list(path: string): Promise<LocalListing>;
    mkdir(path: string): Promise<void>;
    rename(from: string, to: string): Promise<void>;
    delete(paths: string[]): Promise<void>;
    exists(path: string): Promise<boolean>;
    defaultDownloadDir(): Promise<string>;
  };

  on<K extends keyof EventMap>(event: K, handler: (payload: EventMap[K]) => void): Promise<() => void>;
}
