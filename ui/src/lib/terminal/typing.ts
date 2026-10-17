// Telling the person's own input apart from xterm's automatic answers.
//
// xterm reports both through the same `onData`: what was typed or pasted, and its replies to queries it finds in the
// output (device attributes, cursor position, colours). While history is being replayed the replies belong to
// questions asked long ago and must not be sent — but the person's own keystrokes must.
//
// The marker is raised just before xterm handles something the person did, and drops once that is over. It does not
// wait for the event to finish travelling: xterm stops its key events from propagating, so a listener placed to lower
// the mark "after" them would simply never run, and the mark would stay up for good.

export class TypingMark {
  #active = false;
  #timer: ReturnType<typeof setTimeout> | undefined;

  /** Is xterm, right now, handling something the person did? */
  get active(): boolean {
    return this.#active;
  }

  /**
   * Input from the person has just arrived. The mark stays up until the current event has been handled (the next
   * turn of the event loop), or `settleMs` longer for input xterm reads off a timer (composed text).
   */
  begin(settleMs = 0): void {
    this.#active = true;
    clearTimeout(this.#timer);
    this.#timer = setTimeout(() => {
      this.#active = false;
      this.#timer = undefined;
    }, settleMs);
  }

  /** Runs `fn`, which hands xterm text on the person's behalf (a paste), with the mark up for exactly as long. */
  during<T>(fn: () => T): T {
    const before = this.#active;
    this.#active = true;
    try {
      return fn();
    } finally {
      // A timer from an earlier `begin` may still be pending and will lower it; otherwise restore what it was.
      this.#active = this.#timer === undefined ? before : this.#active;
    }
  }

  dispose(): void {
    clearTimeout(this.#timer);
    this.#timer = undefined;
    this.#active = false;
  }
}
