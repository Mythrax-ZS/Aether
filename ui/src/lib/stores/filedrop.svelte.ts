// Files dragged in from Explorer. Tauri reports real paths (the browser never would), so one
// global listener routes drops to whichever view wants them: an SFTP tab uploads, a terminal
// pastes the paths.

import { api } from '$lib/api';

type DropHandler = (paths: string[], x: number, y: number) => boolean;

class FileDropStore {
  /** Files are being dragged over the window. */
  over = $state(false);
  count = $state(0);
  /** Pointer position while hovering, in CSS pixels. */
  x = $state(0);
  y = $state(0);

  #handlers: DropHandler[] = [];
  #started = false;

  /** Registers a handler; the most recently registered gets the first chance. Returns an unregister function. */
  register(handler: DropHandler): () => void {
    this.#handlers.unshift(handler);
    return () => {
      this.#handlers = this.#handlers.filter((h) => h !== handler);
    };
  }

  async init(): Promise<void> {
    if (this.#started) return;
    this.#started = true;
    await api.platform.onFileDrop((e) => {
      this.x = e.x;
      this.y = e.y;
      switch (e.type) {
        case 'enter':
          this.over = true;
          this.count = e.paths.length;
          break;
        case 'over':
          this.over = true;
          break;
        case 'leave':
          this.over = false;
          break;
        case 'drop':
          this.over = false;
          for (const h of this.#handlers) if (h(e.paths, e.x, e.y)) break;
          break;
      }
    });
  }
}

export const filedrop = new FileDropStore();
