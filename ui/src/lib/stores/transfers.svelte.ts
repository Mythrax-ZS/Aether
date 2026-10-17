// File transfer queue state.

import { api } from '$lib/api';
import type { TransferId, TransferInfo } from '$lib/bindings';

class TransfersStore {
  items = $state<Record<string, TransferInfo>>({});

  list = $derived(Object.values(this.items).sort((a, b) => b.startedAt - a.startedAt));
  active = $derived(this.list.filter((t) => t.state === 'running' || t.state === 'queued'));
  finished = $derived(this.list.filter((t) => ['completed', 'failed', 'cancelled'].includes(t.state)));
  /** 0..1 across all running transfers, for the status bar and tab badge. */
  progress = $derived.by(() => {
    const running = this.active;
    const total = running.reduce((n, t) => n + t.bytesTotal, 0);
    return total > 0 ? running.reduce((n, t) => n + t.bytesDone, 0) / total : 0;
  });
  speed = $derived(this.active.reduce((n, t) => n + t.speedBps, 0));

  async init(): Promise<void> {
    for (const t of await api.transfers.list().catch(() => [])) this.items[t.id] = t;
    await api.on('transfer:update', (e) => (this.items[e.transfer.id] = e.transfer));
  }

  pause = (id: TransferId) => api.transfers.pause(id);
  resume = (id: TransferId) => api.transfers.resume(id);
  cancel = (id: TransferId) => api.transfers.cancel(id);

  async dismiss(id: TransferId): Promise<void> {
    await api.transfers.dismiss(id);
    delete this.items[id];
  }

  async clearFinished(): Promise<void> {
    await api.transfers.clearFinished();
    for (const t of this.finished) delete this.items[t.id];
  }
}

export const transfers = new TransfersStore();
