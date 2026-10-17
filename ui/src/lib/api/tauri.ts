// The real backend: Tauri IPC commands, events and native window/dialog APIs.

import { Channel, invoke, type InvokeArgs } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog';
import { readText, writeText } from '@tauri-apps/plugin-clipboard-manager';
import { isPermissionGranted, requestPermission, sendNotification } from '@tauri-apps/plugin-notification';
import { openUrl } from '@tauri-apps/plugin-opener';
import type { AttachedTerminal, TerminalState } from '$lib/bindings';
import { ApiError, toErrorInfo, type Api, type EventMap, type FileDropEvent, type PlatformApi } from './types';

async function call<T>(cmd: string, args?: InvokeArgs): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw new ApiError(toErrorInfo(e));
  }
}

/** Fire-and-forget for hot paths (keystrokes, flow-control acks). */
function fire(cmd: string, args: InvokeArgs): void {
  invoke(cmd, args).catch(() => {});
}

function toBytes(message: unknown): Uint8Array {
  if (message instanceof ArrayBuffer) return new Uint8Array(message);
  if (ArrayBuffer.isView(message)) return new Uint8Array(message.buffer, message.byteOffset, message.byteLength);
  if (Array.isArray(message)) return Uint8Array.from(message as number[]);
  return new Uint8Array(0);
}

function toBase64(bytes: Uint8Array): string {
  let binary = '';
  const step = 0x8000;
  for (let i = 0; i < bytes.length; i += step) {
    binary += String.fromCharCode(...bytes.subarray(i, i + step));
  }
  return btoa(binary);
}

function createPlatform(): PlatformApi {
  const win = getCurrentWindow();
  const webview = getCurrentWebview();
  return {
    isTauri: true,
    windowLabel: win.label,
    window: {
      minimize: () => win.minimize(),
      toggleMaximize: () => win.toggleMaximize(),
      close: () => win.close(),
      isMaximized: () => win.isMaximized(),
      onMaximizeChange: async (cb) => {
        // Maximising resizes the window; re-query rather than infer from the size.
        const un = await win.onResized(async () => cb(await win.isMaximized()));
        return un;
      },
      onFocusChange: (cb) => win.onFocusChanged((e) => cb(e.payload)),
      onCloseRequested: (cb) => win.onCloseRequested(async (event) => cb(() => event.preventDefault())),
      isFocused: () => win.isFocused(),
      setTitle: (title) => win.setTitle(title),
      setZoom: (factor) => webview.setZoom(factor),
      setFullscreen: (on) => win.setFullscreen(on),
      isFullscreen: () => win.isFullscreen(),
    },
    dialog: {
      async pickFiles(opts) {
        const r = await openDialog({
          title: opts?.title,
          multiple: opts?.multiple ?? true,
          filters: opts?.filters,
          defaultPath: opts?.defaultPath,
        });
        return r === null ? [] : Array.isArray(r) ? r : [r];
      },
      async pickFolder(opts) {
        const r = await openDialog({ title: opts?.title, directory: true, multiple: false, defaultPath: opts?.defaultPath });
        return typeof r === 'string' ? r : null;
      },
      saveFile: (opts) => saveDialog({ title: opts?.title, defaultPath: opts?.defaultPath, filters: opts?.filters }),
    },
    clipboard: {
      readText: async () => (await readText()) ?? '',
      writeText: (text) => writeText(text),
    },
    openUrl: (url) => openUrl(url),
    async notify(title, body) {
      let granted = await isPermissionGranted();
      if (!granted) granted = (await requestPermission()) === 'granted';
      if (granted) sendNotification({ title, body });
    },
    async onFileDrop(handler) {
      return webview.onDragDropEvent((event) => {
        const p = event.payload;
        const ratio = window.devicePixelRatio || 1;
        const out: FileDropEvent = {
          type: p.type,
          paths: 'paths' in p ? p.paths : [],
          x: 'position' in p ? p.position.x / ratio : 0,
          y: 'position' in p ? p.position.y / ratio : 0,
        };
        handler(out);
      });
    },
  };
}

export function createTauriApi(): Api {
  return {
    mode: 'tauri',
    platform: createPlatform(),

    app: {
      info: () => call('app_info'),
      ready: () => call('app_ready'),
      quit: () => call('app_quit'),
      restart: () => call('app_restart'),
      newWindow: (request) => call('window_new', { request }),
      listFonts: () => call('list_fonts'),
      openPath: (path) => call('open_path', { path }),
      openLogs: () => call('open_logs_dir'),
      openConfig: () => call('open_config_dir'),
      customCss: () => call('custom_css_get'),
    },

    hosts: {
      snapshot: () => call('hosts_snapshot'),
      save: (host) => call('host_save', { host }),
      delete: (ids) => call('hosts_delete', { ids }),
      duplicate: (id) => call('host_duplicate', { id }),
      favorite: (id, favorite) => call('host_favorite', { id, favorite }),
      move: (ids, groupId) => call('hosts_move', { ids, groupId }),
      reorder: (ids) => call('hosts_reorder', { ids }),
      saveGroup: (group) => call('group_save', { group }),
      deleteGroup: (id, mode) => call('group_delete', { id, mode }),
      collapseGroup: (id, collapsed) => call('group_collapse', { id, collapsed }),
      recents: () => call('recents_list'),
      clearRecents: () => call('recents_clear'),
      passwordExists: (id) => call('host_password_exists', { id }),
      setPassword: (id, password) => call('host_password_set', { id, password }),
      clearPassword: (id) => call('host_password_clear', { id }),
      sshConfigPreview: () => call('ssh_config_preview'),
      sshConfigImport: (aliases, groupId) => call('ssh_config_import', { aliases, groupId }),
    },

    keys: {
      list: () => call('keys_list'),
      inspect: (source, passphrase) => call('key_inspect', { source, passphrase: passphrase ?? null }),
      import: (request) => call('key_import', { request }),
      generate: (request) => call('key_generate', { request }),
      rename: (id, label) => call('key_rename', { id, label }),
      delete: (id) => call('key_delete', { id }),
      publicKey: (id) => call('key_public', { id }),
      exportPublic: (id, path) => call('key_export_public', { id, path }),
      install: (connectionId, id) => call('key_install', { connectionId, id }),
      knownHosts: () => call('known_hosts_list'),
      removeKnownHost: (line, fingerprint) => call('known_host_remove', { line, fingerprint }),
    },

    settings: {
      get: () => call('settings_get'),
      patch: (patch) => call('settings_patch', { patch }),
      reset: () => call('settings_reset'),
      customThemes: () => call('themes_custom'),
      saveTheme: (theme) => call('theme_save', { theme }),
      deleteTheme: (id) => call('theme_delete', { id }),
      loadWorkspace: () => call('workspace_load'),
      saveWorkspace: (workspace) => call('workspace_save', { workspace }),
    },

    sessions: {
      open: (request) => call('session_open', { request }),
      quickConnect: (request) => call('session_quick_connect', { request }),
      adopt: (connectionId) => call('session_adopt', { connectionId }),
      close: (connectionId) => call('session_close', { connectionId }),
      reconnect: (connectionId) => call('session_reconnect', { connectionId }),
      list: () => call('sessions_list'),
      state: (connectionId) => call('session_state', { connectionId }),
      newTerminal: (connectionId, size, host) => call('terminal_new', { connectionId, size, host }),
      closeTerminal: (connectionId, terminalId) => call('terminal_close', { connectionId, terminalId }),
      restartTerminal: (connectionId, terminalId) => call('terminal_restart', { connectionId, terminalId }),
      startForward: (connectionId, rule) => call('forward_start', { connectionId, rule }),
      stopForward: (connectionId, ruleId) => call('forward_stop', { connectionId, ruleId }),
      forwards: (connectionId) => call('forwards_list', { connectionId }),
      respondPrompt: (promptId, reply) => call('prompt_respond', { promptId, reply }),
    },

    terminal: {
      attach(terminalId, handlers) {
        const onData = new Channel<unknown>();
        onData.onmessage = (message) => handlers.onData(toBytes(message));
        const onState = new Channel<TerminalState>();
        onState.onmessage = (state) => handlers.onState(state);
        return call<AttachedTerminal>('terminal_attach', { terminalId, onData, onState });
      },
      detach: (terminalId) => call('terminal_detach', { terminalId }),
      write: (terminalId, data) => call('terminal_write', { terminalId, data }),
      writeBinary: (terminalId, data) => call('terminal_write_binary', { terminalId, data: toBase64(data) }),
      resize: (terminalId, size) => call('terminal_resize', { terminalId, size }),
      ack: (terminalId, bytes) => fire('terminal_ack', { terminalId, bytes }),
    },

    sftp: {
      list: (connectionId, path) => call('sftp_list', { connectionId, path }),
      home: (connectionId) => call('sftp_home', { connectionId }),
      mkdir: (connectionId, path) => call('sftp_mkdir', { connectionId, path }),
      rename: (connectionId, from, to) => call('sftp_rename', { connectionId, from, to }),
      delete: (connectionId, paths) => call('sftp_delete', { connectionId, paths }),
      chmod: (connectionId, path, mode, recursive) => call('sftp_chmod', { connectionId, path, mode, recursive }),
      diskUsage: (connectionId, path) => call('sftp_disk_usage', { connectionId, path }),
      readText: (connectionId, path) => call('sftp_read_text', { connectionId, path }),
      writeText: (connectionId, request) => call('sftp_write_text', { connectionId, request }),
    },

    transfers: {
      planUpload: (connectionId, localPaths, remoteDir) => call('transfer_plan_upload', { connectionId, localPaths, remoteDir }),
      planDownload: (connectionId, remotePaths, localDir) => call('transfer_plan_download', { connectionId, remotePaths, localDir }),
      start: (planId, policy) => call('transfer_start', { planId, policy }),
      discardPlan: (planId) => call('transfer_discard_plan', { planId }),
      list: () => call('transfers_list'),
      pause: (id) => call('transfer_pause', { id }),
      resume: (id) => call('transfer_resume', { id }),
      cancel: (id) => call('transfer_cancel', { id }),
      dismiss: (id) => call('transfer_dismiss', { id }),
      clearFinished: () => call('transfers_clear_finished'),
    },

    local: {
      roots: () => call('local_roots'),
      list: (path) => call('local_list', { path }),
      mkdir: (path) => call('local_mkdir', { path }),
      rename: (from, to) => call('local_rename', { from, to }),
      delete: (paths) => call('local_delete', { paths }),
      exists: (path) => call('local_exists', { path }),
      defaultDownloadDir: () => call('local_default_download_dir'),
    },

    on: async <K extends keyof EventMap>(event: K, handler: (payload: EventMap[K]) => void) =>
      listen<EventMap[K]>(event, (e) => handler(e.payload)),
  };
}
