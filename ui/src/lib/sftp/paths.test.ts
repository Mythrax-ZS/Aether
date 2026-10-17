import { describe, expect, it } from 'vitest';
import { THIS_PC, findByPrefix, invalidName, posix, range, sortEntries, uniqueName, win } from './paths';

describe('posix paths', () => {
  it('joins with a single separator', () => {
    expect(posix.join('/', 'etc')).toBe('/etc');
    expect(posix.join('/var/log', 'syslog')).toBe('/var/log/syslog');
    expect(posix.join('/var/log/', 'syslog')).toBe('/var/log/syslog');
  });

  it('finds parents up to the root', () => {
    expect(posix.parent('/var/log/nginx')).toBe('/var/log');
    expect(posix.parent('/var')).toBe('/');
    expect(posix.parent('/')).toBeNull();
    expect(posix.parent('/var/log/')).toBe('/var');
  });

  it('builds breadcrumbs', () => {
    expect(posix.crumbs('/home/me/src')).toEqual([
      { name: '/', path: '/' },
      { name: 'home', path: '/home' },
      { name: 'me', path: '/home/me' },
      { name: 'src', path: '/home/me/src' },
    ]);
    expect(posix.crumbs('/')).toEqual([{ name: '/', path: '/' }]);
  });

  it('detects containment without matching look-alike siblings', () => {
    expect(posix.within('/a/b', '/a')).toBe(true);
    expect(posix.within('/a', '/a')).toBe(true);
    expect(posix.within('/ab', '/a')).toBe(false);
  });
});

describe('windows paths', () => {
  it('joins and normalises separators', () => {
    expect(win.join('C:\\Users', 'me')).toBe('C:\\Users\\me');
    expect(win.join('C:\\', 'Users')).toBe('C:\\Users');
  });

  it('goes up to the drive list from a drive root', () => {
    expect(win.parent('C:\\Users\\me')).toBe('C:\\Users');
    expect(win.parent('C:\\Users')).toBe('C:\\');
    expect(win.parent('C:\\')).toBe(THIS_PC);
    expect(win.parent(THIS_PC)).toBeNull();
  });

  it('builds breadcrumbs starting at This PC', () => {
    expect(win.crumbs('C:\\Users\\me')).toEqual([
      { name: 'This PC', path: '' },
      { name: 'C:', path: 'C:\\' },
      { name: 'Users', path: 'C:\\Users' },
      { name: 'me', path: 'C:\\Users\\me' },
    ]);
    expect(win.crumbs(THIS_PC)).toEqual([{ name: 'This PC', path: '' }]);
  });

  it('compares case-insensitively', () => {
    expect(win.within('c:\\users\\me', 'C:\\Users')).toBe(true);
    expect(win.within('C:\\UsersX', 'C:\\Users')).toBe(false);
  });
});

describe('sorting', () => {
  const e = (name: string, isDir = false, size = 0, modified: number | null = null) => ({ name, isDir, size, modified });

  it('puts folders first and sorts names naturally', () => {
    const sorted = sortEntries([e('file10'), e('file2'), e('Zed', true), e('alpha', true)], 'name', true);
    expect(sorted.map((x) => x.name)).toEqual(['alpha', 'Zed', 'file2', 'file10']);
  });

  it('reverses within each group only', () => {
    const sorted = sortEntries([e('b'), e('a'), e('d', true), e('c', true)], 'name', false);
    expect(sorted.map((x) => x.name)).toEqual(['d', 'c', 'b', 'a']);
  });

  it('sorts by size and breaks ties by name', () => {
    const sorted = sortEntries([e('b', false, 5), e('a', false, 5), e('c', false, 1)], 'size', true);
    expect(sorted.map((x) => x.name)).toEqual(['c', 'a', 'b']);
  });

  it('treats missing dates as oldest', () => {
    const sorted = sortEntries([e('new', false, 0, 200), e('none'), e('old', false, 0, 100)], 'modified', true);
    expect(sorted.map((x) => x.name)).toEqual(['none', 'old', 'new']);
  });
});

describe('helpers', () => {
  it('makes inclusive ranges either way round', () => {
    expect(range(2, 4)).toEqual([2, 3, 4]);
    expect(range(4, 2)).toEqual([2, 3, 4]);
    expect(range(3, 3)).toEqual([3]);
  });

  it('type-ahead wraps around', () => {
    const names = ['alpha', 'beta', 'bravo', 'charlie'];
    expect(findByPrefix(names, 'b', 0)).toBe(1);
    expect(findByPrefix(names, 'b', 2)).toBe(2);
    expect(findByPrefix(names, 'a', 3)).toBe(0);
    expect(findByPrefix(names, 'z', 0)).toBe(-1);
    expect(findByPrefix(names, '', 0)).toBe(-1);
  });

  it('numbers colliding names before the extension', () => {
    expect(uniqueName('a.txt', new Set())).toBe('a.txt');
    expect(uniqueName('a.txt', new Set(['a.txt']))).toBe('a (2).txt');
    expect(uniqueName('a.txt', new Set(['a.txt', 'a (2).txt']))).toBe('a (3).txt');
    expect(uniqueName('Makefile', new Set(['Makefile']))).toBe('Makefile (2)');
    expect(uniqueName('.env', new Set(['.env']))).toBe('.env (2)');
  });

  it('rejects names the target filesystem would refuse', () => {
    expect(invalidName('', 'remote')).not.toBeNull();
    expect(invalidName('a/b', 'remote')).not.toBeNull();
    expect(invalidName('a:b', 'remote')).toBeNull();
    expect(invalidName('a:b', 'local')).not.toBeNull();
    expect(invalidName('ok name.txt', 'local')).toBeNull();
    expect(invalidName('trailing.', 'local')).not.toBeNull();
    expect(invalidName('..', 'remote')).not.toBeNull();
  });
});
