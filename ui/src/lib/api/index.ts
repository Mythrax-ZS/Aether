import { createTauriApi } from './tauri';
import type { Api } from './types';

export * from './types';

const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/**
 * The backend. In the desktop app this is Tauri IPC; in a plain browser (dev preview and
 * tests) it is an in-memory fake with a demo shell.
 */
export const api: Api = inTauri ? createTauriApi() : (await import('./fake')).createFakeApi();
