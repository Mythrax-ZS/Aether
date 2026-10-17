// Questions the backend asked mid-connection: host key trust, passwords, 2FA codes.

import { api } from '$lib/api';
import type { PromptEvent, PromptReply } from '$lib/bindings';

class PromptsStore {
  queue = $state<PromptEvent[]>([]);
  /** The prompt currently shown (they're answered one at a time). */
  current = $derived<PromptEvent | null>(this.queue[0] ?? null);

  async init(): Promise<void> {
    await Promise.all([
      api.on('prompt:request', (e) => this.queue.push(e)),
      api.on('prompt:closed', (e) => (this.queue = this.queue.filter((p) => p.id !== e.id))),
    ]);
  }

  /**
   * Answers a prompt. `id` is the prompt the answer was meant for: if it is gone (closed, timed out) the answer is
   * dropped rather than handed to whichever prompt happens to be first in line now. Without an id it is the one shown.
   */
  async respond(reply: PromptReply, id?: string): Promise<void> {
    const at = id === undefined ? 0 : this.queue.findIndex((p) => p.id === id);
    if (at < 0 || at >= this.queue.length) return;
    const [prompt] = this.queue.splice(at, 1);
    try {
      const delivered = await api.sessions.respondPrompt(prompt.id, reply);
      // `false` means the connection stopped waiting (closed or timed out) — nothing more to do.
      if (!delivered) console.warn('prompt was no longer waiting', prompt.id);
    } catch (e) {
      console.error('could not deliver the prompt answer', e);
    }
  }

  cancel(id?: string): Promise<void> {
    return this.respond({ type: 'cancel' }, id);
  }

  /** Drops any prompts for a connection that was closed from the UI. */
  forget(connectionId: string): void {
    this.queue = this.queue.filter((p) => p.connectionId !== connectionId);
  }
}

export const prompts = new PromptsStore();
