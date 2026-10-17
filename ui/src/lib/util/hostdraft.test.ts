import { describe, expect, it } from 'vitest';
import type { Settings } from '$lib/bindings';
import { blankHost, hostProblems, isValidHostname } from './hostdraft';

const settings = {
  ssh: { connectTimeoutSecs: 15, keepaliveIntervalSecs: 30, keepaliveMax: 3, autoReconnect: true, defaultTerm: 'xterm-256color' },
} as unknown as Settings;

describe('isValidHostname', () => {
  it('accepts names and addresses', () => {
    for (const ok of ['example.com', 'db-01.prod_internal', '10.0.0.5', '::1', '[fe80::1]', 'fe80::1%eth0', 'münchen.example', 'a']) {
      expect(isValidHostname(ok), ok).toBe(true);
    }
  });

  it('rejects anything that is not an address', () => {
    for (const bad of ['', '-oProxyCommand=calc', 'a b', '*', '*.example.com', 'a?b', 'a,b', 'host&calc', 'a/b', 'a\\b', '|1|x|y', '[]', 'a;b']) {
      expect(isValidHostname(bad), JSON.stringify(bad)).toBe(false);
    }
    expect(isValidHostname('a'.repeat(256))).toBe(false);
  });
});

describe('hostProblems', () => {
  it('asks for an address, and for a sensible one', () => {
    const host = blankHost(settings);
    expect(hostProblems(host).hostname).toMatch(/Enter/);
    host.hostname = 'bad host';
    expect(hostProblems(host).hostname).toMatch(/spaces/);
    host.hostname = '*.example.com';
    expect(hostProblems(host).hostname).toMatch(/host name or IP/);
    host.hostname = 'db.example.com';
    expect(hostProblems(host).hostname).toBeUndefined();
  });

  it('wants an address even when a proxy command is set', () => {
    const host = blankHost(settings);
    host.options.proxyCommand = 'ssh -W %h:%p jump';
    expect(hostProblems(host).hostname).toMatch(/Enter/);
  });
});
