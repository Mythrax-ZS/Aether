// A file-type icon and tint for each entry.

import {
  Download, File, FileArchive, FileAudio, FileCode, FileCog, FileImage, FileKey, FileText, FileTerminal, FileVideo, Folder,
  FolderSymlink, HardDrive, House, Link, Monitor, FileText as Docs,
} from '@lucide/svelte';
import type { Component } from 'svelte';
import { extension } from './paths';
import type { FileItem } from './provider';

export type Tone = 'folder' | 'code' | 'media' | 'archive' | 'doc' | 'key' | 'exec' | 'config' | 'plain' | 'drive';

export interface Glyph {
  icon: Component<{ size?: number }>;
  tone: Tone;
}

const CODE = new Set('js mjs cjs ts tsx jsx py rs go c h cc cpp hpp cs java kt swift rb php lua pl sh bash zsh fish ps1 bat cmd sql html css scss sass less vue svelte dart r scala hs ex exs erl clj'.split(' '));
const CONFIG = new Set('json yaml yml toml ini conf cfg env properties xml plist service timer'.split(' '));
const DOC = new Set('txt md markdown rst log csv tsv pdf doc docx rtf odt'.split(' '));
const IMAGE = new Set('png jpg jpeg gif webp svg ico bmp tif tiff avif heic'.split(' '));
const ARCHIVE = new Set('zip tar gz tgz bz2 xz 7z rar zst lz4 deb rpm iso'.split(' '));
const AUDIO = new Set('mp3 wav flac ogg m4a aac opus'.split(' '));
const VIDEO = new Set('mp4 mkv mov avi webm wmv flv'.split(' '));
const KEY = new Set('pem key pub crt cer pfx p12 gpg asc'.split(' '));

export function glyphFor(item: FileItem): Glyph {
  switch (item.special) {
    case 'drive':
      return { icon: HardDrive, tone: 'drive' };
    case 'home':
      return { icon: House, tone: 'folder' };
    case 'desktop':
      return { icon: Monitor, tone: 'folder' };
    case 'documents':
      return { icon: Docs, tone: 'folder' };
    case 'downloads':
      return { icon: Download, tone: 'folder' };
  }
  if (item.isDir) return { icon: item.isSymlink ? FolderSymlink : Folder, tone: 'folder' };
  if (item.isSymlink) return { icon: Link, tone: 'plain' };
  const ext = extension(item.name);
  if (CODE.has(ext)) return { icon: FileCode, tone: 'code' };
  if (CONFIG.has(ext) || item.name.startsWith('.')) return { icon: FileCog, tone: 'config' };
  if (IMAGE.has(ext)) return { icon: FileImage, tone: 'media' };
  if (AUDIO.has(ext)) return { icon: FileAudio, tone: 'media' };
  if (VIDEO.has(ext)) return { icon: FileVideo, tone: 'media' };
  if (ARCHIVE.has(ext)) return { icon: FileArchive, tone: 'archive' };
  if (KEY.has(ext)) return { icon: FileKey, tone: 'key' };
  if (DOC.has(ext)) return { icon: FileText, tone: 'doc' };
  // Unix executables have no extension but an execute bit.
  if (item.mode != null && (item.mode & 0o111) !== 0 && !ext) return { icon: FileTerminal, tone: 'exec' };
  return { icon: File, tone: 'plain' };
}

/** Files the built-in editor can open: small and text-like (the server decides about size). */
export function looksEditable(item: FileItem): boolean {
  if (item.isDir) return false;
  if (item.size > 1_000_000) return false;
  const ext = extension(item.name);
  if (IMAGE.has(ext) || AUDIO.has(ext) || VIDEO.has(ext) || ARCHIVE.has(ext) || KEY.has(ext) && ext !== 'pub') return false;
  return true;
}
