// Launch arguments, put back into the line they were typed as.

/**
 * Joins arguments into one line the quick-connect parser reads back as the same words: a word with spaces (or quotes)
 * in it is quoted. Backslashes are left alone — on Windows they are path separators.
 */
export function commandLine(args: readonly string[]): string {
  const word = (a: string): string => {
    if (a !== '' && !/[\s"']/.test(a)) return a;
    if (!a.includes('"')) return `"${a}"`;
    if (!a.includes("'")) return `'${a}'`;
    return `"${a.replaceAll('"', '')}"`;
  };
  return args.map(word).join(' ');
}

/** Is this launch a whole `ssh …` command (`aether ssh -J bastion me@db`) rather than a list of destinations? */
export function isSshCommand(args: readonly string[]): boolean {
  return args.length > 1 && /^ssh(\.exe)?$/i.test(args[0] ?? '');
}
