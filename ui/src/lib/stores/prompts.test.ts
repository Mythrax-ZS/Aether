import { beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '$lib/api';
import type { PromptEvent } from '$lib/bindings';
import { prompts } from './prompts.svelte';

const prompt = (id: string): PromptEvent => ({ id, connectionId: 'conn', payload: { type: 'secret' } }) as never;

describe('answering prompts', () => {
  beforeEach(() => {
    prompts.queue = [];
    vi.restoreAllMocks();
  });

  it('sends an answer to the prompt it was meant for, wherever that is in the queue', async () => {
    const send = vi.spyOn(api.sessions, 'respondPrompt').mockResolvedValue(true);
    prompts.queue = [prompt('a'), prompt('b')];
    await prompts.respond({ type: 'cancel' }, 'b');
    expect(send).toHaveBeenCalledExactlyOnceWith('b', { type: 'cancel' });
    expect(prompts.queue.map((p) => p.id)).toEqual(['a']);
  });

  it('drops an answer for a prompt that is gone instead of handing it to the next one', async () => {
    // Prompt A timed out while the person was clicking; B took its place in the same spot on screen.
    const send = vi.spyOn(api.sessions, 'respondPrompt').mockResolvedValue(true);
    prompts.queue = [prompt('b')];
    await prompts.respond({ type: 'answers', answers: ['hunter2'], remember: false }, 'a');
    expect(send).not.toHaveBeenCalled();
    expect(prompts.queue.map((p) => p.id)).toEqual(['b']);
  });

  it('answers the one in front when no prompt is named', async () => {
    const send = vi.spyOn(api.sessions, 'respondPrompt').mockResolvedValue(true);
    prompts.queue = [prompt('a'), prompt('b')];
    await prompts.cancel();
    expect(send).toHaveBeenCalledExactlyOnceWith('a', { type: 'cancel' });
    expect(prompts.queue.map((p) => p.id)).toEqual(['b']);
  });

  it('does nothing when there is nothing to answer', async () => {
    const send = vi.spyOn(api.sessions, 'respondPrompt').mockResolvedValue(true);
    await prompts.cancel();
    await prompts.cancel('missing');
    expect(send).not.toHaveBeenCalled();
  });

  it('survives the backend refusing the delivery', async () => {
    vi.spyOn(api.sessions, 'respondPrompt').mockRejectedValue(new Error('gone'));
    vi.spyOn(console, 'error').mockImplementation(() => {});
    prompts.queue = [prompt('a')];
    await expect(prompts.cancel('a')).resolves.toBeUndefined();
    expect(prompts.queue).toEqual([]);
  });
});
