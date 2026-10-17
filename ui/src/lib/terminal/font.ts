// Font stacks for the terminal.

const SYMBOLS = "'Aether Symbols'";

/** One character from the symbols font (✔ → ● ⚠ ➜), enough to make the browser fetch it. */
export const SYMBOL_SAMPLE = '✔→●⚠➜';
const GENERIC = /^(?:monospace|ui-monospace|serif|sans-serif|system-ui|cursive|fantasy)$/i;

/** The first family of a stack, unquoted ("'JetBrains Mono', monospace" → JetBrains Mono). */
export function firstFamily(stack: string): string {
  const m = /^\s*(?:'([^']+)'|"([^"]+)"|([^,]+))/.exec(stack);
  return (m?.[1] ?? m?.[2] ?? m?.[3] ?? 'monospace').trim();
}

/**
 * The user's stack with the bundled symbols font as the last stop before the generic family, so check marks, arrows and
 * dingbats come out the same on every machine instead of depending on the system's per-glyph fallback.
 */
export function withSymbolFallback(stack: string): string {
  if (/Aether Symbols/i.test(stack)) return stack;
  const parts = stack.split(',').map((p) => p.trim()).filter(Boolean);
  if (parts.length === 0) return `${SYMBOLS}, monospace`;
  const last = parts[parts.length - 1];
  if (GENERIC.test(last)) parts.splice(parts.length - 1, 0, SYMBOLS);
  else parts.push(SYMBOLS);
  return parts.join(', ');
}
