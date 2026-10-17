import { describe, expect, it } from 'vitest';
import { firstFamily, withSymbolFallback } from './font';

describe('font stacks', () => {
  it('finds the first family however it is quoted', () => {
    expect(firstFamily("'JetBrains Mono Variable', monospace")).toBe('JetBrains Mono Variable');
    expect(firstFamily('"Fira Code", monospace')).toBe('Fira Code');
    expect(firstFamily('Consolas, monospace')).toBe('Consolas');
    expect(firstFamily('')).toBe('monospace');
  });

  it('puts the bundled symbols just before the generic family', () => {
    expect(withSymbolFallback("'JetBrains Mono Variable', Consolas, monospace")).toBe("'JetBrains Mono Variable', Consolas, 'Aether Symbols', monospace");
  });

  it('appends them when the stack has no generic family', () => {
    expect(withSymbolFallback("'Fira Code'")).toBe("'Fira Code', 'Aether Symbols'");
  });

  it('is idempotent and copes with an empty stack', () => {
    const once = withSymbolFallback('Consolas, monospace');
    expect(withSymbolFallback(once)).toBe(once);
    expect(withSymbolFallback('')).toBe("'Aether Symbols', monospace");
  });
});
