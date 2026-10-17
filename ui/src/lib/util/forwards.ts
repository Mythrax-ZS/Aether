import type { ForwardKind, ForwardRule } from '$lib/bindings';

/** A sensible starting point for a new port-forward rule. */
export function newForwardRule(kind: ForwardKind = 'local'): ForwardRule {
  return {
    id: crypto.randomUUID(),
    kind,
    label: '',
    bindHost: kind === 'remote' ? 'localhost' : '127.0.0.1',
    bindPort: kind === 'dynamic' ? 1080 : 8080,
    destHost: 'localhost',
    destPort: 80,
    autoStart: true,
  };
}

/** One-line description in the words people use ("localhost:8080 → db:5432"). */
export function describeForward(r: ForwardRule): string {
  const bind = `${r.bindHost || '127.0.0.1'}:${r.bindPort}`;
  if (r.kind === 'dynamic') return `SOCKS5 on ${bind}`;
  return `${bind} → ${r.destHost}:${r.destPort}`;
}

/** Returns a problem with the rule, or null when it can be started. */
export function validateForward(r: ForwardRule): string | null {
  const okPort = (p: number, allowZero = false) => Number.isInteger(p) && p >= (allowZero ? 0 : 1) && p <= 65535;
  if (!r.bindHost.trim()) return 'Enter an address to listen on.';
  if (!okPort(r.bindPort, r.kind === 'remote')) return 'The listening port must be between 1 and 65535.';
  if (r.kind !== 'dynamic') {
    if (!r.destHost.trim()) return 'Enter where connections should be forwarded to.';
    if (!okPort(r.destPort)) return 'The destination port must be between 1 and 65535.';
  }
  return null;
}
