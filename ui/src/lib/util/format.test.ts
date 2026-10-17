import { describe, expect, it } from 'vitest';
import {
  baseName, formatBytes, formatDuration, formatMode, formatRelative, formatSpeed, initials, modeToOctal, parseOctal, plural,
} from './format';

describe('formatBytes', () => {
  it('scales units', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(1023)).toBe('1023 B');
    expect(formatBytes(1024)).toBe('1.0 KB');
    expect(formatBytes(1536)).toBe('1.5 KB');
    expect(formatBytes(5 * 1024 * 1024)).toBe('5.0 MB');
    expect(formatBytes(250 * 1024 * 1024)).toBe('250 MB');
  });
  it('is defensive', () => {
    expect(formatBytes(-1)).toBe('—');
    expect(formatBytes(NaN)).toBe('—');
    expect(formatSpeed(0)).toBe('—');
    expect(formatSpeed(2048)).toBe('2.0 KB/s');
  });
});

describe('formatDuration', () => {
  it('reads naturally', () => {
    expect(formatDuration(5)).toBe('5s');
    expect(formatDuration(75)).toBe('1m 15s');
    expect(formatDuration(3725)).toBe('1h 02m');
  });
});

describe('formatRelative', () => {
  const now = Date.UTC(2026, 9, 16, 12, 0, 0);
  it('buckets time', () => {
    expect(formatRelative(null)).toBe('never');
    expect(formatRelative(now - 10_000, now)).toBe('just now');
    expect(formatRelative(now - 5 * 60_000, now)).toBe('5 min ago');
    expect(formatRelative(now - 3 * 3_600_000, now)).toBe('3 hours ago');
    expect(formatRelative(now - 1 * 3_600_000, now)).toBe('1 hour ago');
    expect(formatRelative(now - 26 * 3_600_000, now)).toBe('yesterday');
    expect(formatRelative(now - 3 * 86_400_000, now)).toBe('3 days ago');
  });
});

describe('permissions', () => {
  it('formats rwx strings', () => {
    expect(formatMode(0o755)).toBe('rwxr-xr-x');
    expect(formatMode(0o644)).toBe('rw-r--r--');
    expect(formatMode(0o4755)).toBe('rwsr-xr-x');
    expect(formatMode(0o1777)).toBe('rwxrwxrwt');
    expect(formatMode(null)).toBe('—');
  });
  it('round-trips octal', () => {
    expect(modeToOctal(0o755)).toBe('755');
    expect(modeToOctal(0o100644)).toBe('644');
    expect(parseOctal('0755')).toBe(0o755);
    expect(parseOctal('755')).toBe(0o755);
    expect(parseOctal('789')).toBeNull();
    expect(parseOctal('7')).toBeNull();
  });
});

describe('misc', () => {
  it('baseName handles both separators', () => {
    expect(baseName('/home/me/file.txt')).toBe('file.txt');
    expect(baseName('C:\\Users\\me\\doc.pdf')).toBe('doc.pdf');
    expect(baseName('/home/me/')).toBe('me');
  });
  it('plural and initials', () => {
    expect(plural(1, 'file')).toBe('1 file');
    expect(plural(3, 'file')).toBe('3 files');
    expect(initials('web-01')).toBe('WE');
    expect(initials('db-01')).toBe('DB');
    expect(initials('Test server')).toBe('TS');
    expect(initials('staging-api')).toBe('SA');
    expect(initials('bastion')).toBe('BA');
    expect(initials('10.0.1.20')).toBe('10');
    expect(initials('deploy@host')).toBe('DH');
    expect(initials('')).toBe('?');
    expect(initials('---')).toBe('?');
  });
});
