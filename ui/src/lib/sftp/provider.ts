// The two sides of the file browser behind one interface, so a single pane component can show
// either this PC or the server.

import { api } from '$lib/api';
import type { ConnectionId, DiskUsage } from '$lib/bindings';
import { THIS_PC, posix, win, type PathOps } from './paths';

export interface FileItem {
  name: string;
  path: string;
  isDir: boolean;
  isSymlink: boolean;
  size: number;
  modified: number | null;
  mode: number | null;
  owner: string | null;
  group: string | null;
  hidden: boolean;
  /** Drives and shortcuts shown at the very top of the local tree. */
  special?: 'drive' | 'home' | 'desktop' | 'documents' | 'downloads';
}

export interface Listing {
  path: string;
  parent: string | null;
  items: FileItem[];
}

export interface FileProvider {
  readonly side: 'local' | 'remote';
  readonly ops: PathOps;
  /** The label for the root of the tree ("This PC" / "/"). */
  readonly rootLabel: string;
  list(path: string): Promise<Listing>;
  home(): Promise<string>;
  mkdir(path: string): Promise<void>;
  rename(from: string, to: string): Promise<void>;
  remove(paths: string[]): Promise<void>;
  diskUsage(path: string): Promise<DiskUsage | null>;
}

export function localProvider(): FileProvider {
  return {
    side: 'local',
    ops: win,
    rootLabel: 'This PC',
    async list(path) {
      if (path === THIS_PC) {
        const roots = await api.local.roots();
        return {
          path: THIS_PC,
          parent: null,
          items: roots.map((r) => ({
            name: r.label,
            path: r.path,
            isDir: true,
            isSymlink: false,
            size: 0,
            modified: null,
            mode: null,
            owner: null,
            group: null,
            hidden: false,
            special: r.kind,
          })),
        };
      }
      const l = await api.local.list(path);
      return {
        path: l.path,
        parent: l.parent ?? win.parent(l.path),
        items: l.entries.map((e) => ({
          name: e.name,
          path: e.path,
          isDir: e.kind === 'dir',
          isSymlink: e.isSymlink,
          size: e.size,
          modified: e.modified,
          mode: null,
          owner: null,
          group: null,
          hidden: e.hidden,
        })),
      };
    },
    async home() {
      const roots = await api.local.roots();
      return roots.find((r) => r.kind === 'home')?.path ?? roots[0]?.path ?? THIS_PC;
    },
    mkdir: (path) => api.local.mkdir(path),
    rename: (from, to) => api.local.rename(from, to),
    remove: (paths) => api.local.delete(paths),
    diskUsage: async () => null,
  };
}

export function remoteProvider(connectionId: ConnectionId): FileProvider {
  return {
    side: 'remote',
    ops: posix,
    rootLabel: '/',
    async list(path) {
      const l = await api.sftp.list(connectionId, path);
      return {
        path: l.path,
        parent: posix.parent(l.path),
        items: l.entries.map((e) => ({
          name: e.name,
          path: e.path,
          isDir: e.kind === 'dir',
          isSymlink: e.isSymlink,
          size: e.size,
          modified: e.modified,
          mode: e.mode,
          owner: e.owner,
          group: e.group,
          hidden: e.name.startsWith('.'),
        })),
      };
    },
    home: () => api.sftp.home(connectionId),
    mkdir: (path) => api.sftp.mkdir(connectionId, path),
    rename: (from, to) => api.sftp.rename(connectionId, from, to),
    remove: (paths) => api.sftp.delete(connectionId, paths),
    diskUsage: (path) => api.sftp.diskUsage(connectionId, path).catch(() => null),
  };
}
