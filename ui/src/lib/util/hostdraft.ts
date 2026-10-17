import type { GroupId, Host, Settings } from '$lib/bindings';

/** A blank host pre-filled from the user's defaults. */
export function blankHost(settings: Settings, groupId: GroupId | null = null): Host {
  const now = Date.now();
  return {
    id: crypto.randomUUID(),
    label: '',
    hostname: '',
    port: 22,
    username: '',
    groupId,
    tags: [],
    color: null,
    favorite: false,
    notes: '',
    auth: { methods: ['agent', 'public-key', 'keyboard-interactive', 'password'], key: null, identitiesOnly: false },
    jump: [],
    options: {
      connectTimeoutSecs: settings.ssh.connectTimeoutSecs,
      keepaliveIntervalSecs: settings.ssh.keepaliveIntervalSecs,
      keepaliveMax: settings.ssh.keepaliveMax,
      compression: false,
      agentForwarding: false,
      x11Forwarding: false,
      autoReconnect: settings.ssh.autoReconnect,
      term: settings.ssh.defaultTerm,
      legacyAlgorithms: false,
      proxyCommand: '',
    },
    forwards: [],
    startup: { command: '', env: [] },
    terminal: { themeId: null, fontFamily: null, fontSize: null },
    origin: { type: 'manual' },
    sort: 0,
    createdAt: now,
    updatedAt: now,
    lastConnectedAt: null,
    connectCount: 0,
  };
}

export interface HostProblems {
  hostname?: string;
  port?: string;
  key?: string;
  methods?: string;
}

/**
 * Whether `name` can only mean a machine: letters and digits (any script), `.`, `-`, `_`, `:` (IPv6) and `%` (its
 * zone id), never a leading `-`. Mirrors the backend rule — wildcards, commas and shell syntax are not addresses.
 */
export function isValidHostname(name: string): boolean {
  const bare = name.startsWith('[') && name.endsWith(']') ? name.slice(1, -1) : name;
  return bare.length > 0 && bare.length <= 255 && !bare.startsWith('-') && /^[\p{Alphabetic}\p{N}._:%-]+$/u.test(bare);
}

/** What's wrong with a host before it can be saved (empty object = fine). */
export function hostProblems(host: Host): HostProblems {
  const out: HostProblems = {};
  const name = host.hostname.trim();
  if (!name) out.hostname = 'Enter the server’s address.';
  else if (/\s/.test(name)) out.hostname = 'An address can’t contain spaces.';
  else if (name && !isValidHostname(name)) out.hostname = 'Use a host name or IP address: letters, digits, dots and dashes.';
  if (!Number.isInteger(host.port) || host.port < 1 || host.port > 65535) out.port = 'Use a port from 1 to 65535.';
  if (host.auth.methods.length === 0) out.methods = 'Choose at least one way to sign in.';
  const key = host.auth.key;
  if (key?.type === 'file' && !key.path.trim()) out.key = 'Choose a key file, or switch to the default keys.';
  if (key?.type === 'vault' && !key.keyId) out.key = 'Pick a saved key, or switch to the default keys.';
  return out;
}
