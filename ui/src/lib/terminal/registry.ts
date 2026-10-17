// Lets commands (copy, paste, find, zoom, ...) reach the terminal that is currently focused.

import type { TerminalController } from './controller';

const controllers = new Map<string, TerminalController>();

export const terminalRegistry = {
  set(leafId: string, controller: TerminalController): void {
    controllers.set(leafId, controller);
  },
  delete(leafId: string): void {
    controllers.delete(leafId);
  },
  get(leafId: string | null | undefined): TerminalController | undefined {
    return leafId ? controllers.get(leafId) : undefined;
  },
  /** Every live terminal, for searching across all of them. */
  entries(): [string, TerminalController][] {
    return [...controllers.entries()];
  },
};
