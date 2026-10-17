// Live connection state, driven by backend events.

import { api } from '$lib/api';
import type { ConnectionId, ForwardStatus, SessionState } from '$lib/bindings';

export interface SessionView {
  state: SessionState;
  latencyMs: number | null;
  forwards: ForwardStatus[];
  /** Pre-login banner text from the server, if any. */
  banner: string;
  /** Set once the session has been connected at least once (tells a failed first attempt from a lost link). */
  everConnected: boolean;
}

const connecting = (): SessionView => ({
  state: { type: 'connecting', stage: null, hop: null },
  latencyMs: null,
  forwards: [],
  banner: '',
  everConnected: false,
});

class SessionsStore {
  views = $state<Record<string, SessionView>>({});

  view(id: ConnectionId | null | undefined): SessionView | null {
    return id ? (this.views[id] ?? null) : null;
  }

  ensure(id: ConnectionId): SessionView {
    if (!this.views[id]) this.views[id] = connecting();
    return this.views[id];
  }

  remove(id: ConnectionId): void {
    delete this.views[id];
  }

  isConnected(id: ConnectionId | null | undefined): boolean {
    return !!id && this.views[id]?.state.type === 'connected';
  }

  async init(): Promise<void> {
    await Promise.all([
      api.on('session:state', (e) => {
        const v = this.ensure(e.connectionId);
        v.state = e.state;
        if (e.state.type === 'connected') v.everConnected = true;
        else v.latencyMs = null;
      }),
      api.on('session:latency', (e) => (this.ensure(e.connectionId).latencyMs = e.rttMs)),
      api.on('session:forwards', (e) => (this.ensure(e.connectionId).forwards = e.forwards)),
      api.on('session:banner', (e) => {
        const v = this.ensure(e.connectionId);
        v.banner = (v.banner ? v.banner + '\n' : '') + e.text;
      }),
    ]);
  }
}

export const sessions = new SessionsStore();
