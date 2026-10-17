// Keyboard accelerators: parsing "Ctrl+Shift+T", matching DOM events, and display.

export interface Accelerator {
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
  meta: boolean;
  /** Lower-case letter/digit, or a named key (`tab`, `arrowleft`, `f5`, `,`, ...). */
  key: string;
}

const ALIASES: Record<string, string> = {
  esc: 'escape',
  return: 'enter',
  del: 'delete',
  space: ' ',
  plus: '+',
  up: 'arrowup',
  down: 'arrowdown',
  left: 'arrowleft',
  right: 'arrowright',
};

const DISPLAY: Record<string, string> = {
  ' ': 'Space',
  arrowup: '↑',
  arrowdown: '↓',
  arrowleft: '←',
  arrowright: '→',
  escape: 'Esc',
  enter: 'Enter',
  tab: 'Tab',
  delete: 'Del',
  backspace: 'Backspace',
};

export function parseAccelerator(text: string): Accelerator | null {
  let rest = text.trim();
  let keyName: string;
  // The key itself may be a literal "+" ("Ctrl++"), so split from the right.
  if (rest === '+') {
    keyName = '+';
    rest = '';
  } else if (rest.endsWith('++')) {
    keyName = '+';
    rest = rest.slice(0, -2);
  } else {
    const idx = rest.lastIndexOf('+');
    keyName = idx >= 0 ? rest.slice(idx + 1) : rest;
    rest = idx >= 0 ? rest.slice(0, idx) : '';
  }
  keyName = keyName.trim();
  if (!keyName) return null;
  const modifiers = rest.split('+').map((p) => p.trim()).filter(Boolean);
  const acc: Accelerator = { ctrl: false, shift: false, alt: false, meta: false, key: '' };
  for (const part of modifiers) {
    switch (part.toLowerCase()) {
      case 'ctrl':
      case 'control':
      case 'cmdorctrl':
        acc.ctrl = true;
        break;
      case 'shift':
        acc.shift = true;
        break;
      case 'alt':
      case 'option':
        acc.alt = true;
        break;
      case 'meta':
      case 'win':
      case 'super':
      case 'cmd':
        acc.meta = true;
        break;
      default:
        return null;
    }
  }
  const last = keyName.toLowerCase();
  acc.key = ALIASES[last] ?? last;
  return acc;
}

/** The key an event represents, independent of Shift and keyboard layout quirks. */
export function eventKey(e: KeyboardEvent): string {
  if (e.code.startsWith('Digit')) return e.code.slice(5);
  if (e.code.startsWith('Key')) return e.code.slice(3).toLowerCase();
  if (e.code.startsWith('Numpad') && /\d/.test(e.code.slice(6))) return e.code.slice(6);
  const k = e.key.toLowerCase();
  return k === 'dead' || k === 'unidentified' ? e.code.toLowerCase() : k;
}

const MODIFIER_KEYS = new Set(['control', 'shift', 'alt', 'meta', 'altgraph', 'os']);

export function isModifierOnly(e: KeyboardEvent): boolean {
  return MODIFIER_KEYS.has(e.key.toLowerCase());
}

export function matchesAccelerator(e: KeyboardEvent, acc: Accelerator): boolean {
  if (e.ctrlKey !== acc.ctrl || e.altKey !== acc.alt || e.metaKey !== acc.meta) return false;
  const key = eventKey(e);
  // Keys that need Shift to type (+, ?, :) are matched without insisting on the Shift flag.
  const needsShift = /^[a-z0-9]$/.test(acc.key) || acc.key.length > 1;
  if (needsShift ? e.shiftKey !== acc.shift : false) return false;
  return key === acc.key;
}

/** Builds an accelerator from a key event (for the shortcut recorder). */
export function acceleratorFromEvent(e: KeyboardEvent): Accelerator | null {
  if (isModifierOnly(e)) return null;
  return { ctrl: e.ctrlKey, shift: e.shiftKey, alt: e.altKey, meta: e.metaKey, key: eventKey(e) };
}

/** The pieces of an accelerator as shown on key caps (`['Ctrl', 'Shift', 'T']`). */
export function acceleratorParts(acc: Accelerator): string[] {
  const parts: string[] = [];
  if (acc.ctrl) parts.push('Ctrl');
  if (acc.alt) parts.push('Alt');
  if (acc.shift) parts.push('Shift');
  if (acc.meta) parts.push('Win');
  parts.push(DISPLAY[acc.key] ?? (acc.key.length === 1 ? acc.key.toUpperCase() : acc.key[0].toUpperCase() + acc.key.slice(1)));
  return parts;
}

export function formatAccelerator(acc: Accelerator): string {
  return acceleratorParts(acc).join('+');
}

/** Normalises a stored string so it displays consistently ("ctrl+k" → "Ctrl+K"). */
export function normalizeAccelerator(text: string): string {
  const acc = parseAccelerator(text);
  return acc ? formatAccelerator(acc) : text;
}
