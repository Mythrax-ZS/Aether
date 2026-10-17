# Changelog

All notable changes to Aether are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## 0.1.0 — first release

The first complete version: a terminal, a host manager, SFTP, tunnels, jump hosts and secure secret
storage, in one Windows application.

### Added

**Terminal**
- xterm.js renderer on WebGL (with a DOM fallback), true colour, programming ligatures, Unicode
  grapheme clusters and emoji, inline images (Sixel / iTerm2), hyperlinks, search with highlights,
  scrollback, copy-on-select, bracketed paste with a multi-line confirmation, zoom, bell modes.
- Shell integration: working directory (OSC 7), command-finished notifications (OSC 133), terminal
  notifications (OSC 9 / 777), opt-in clipboard writes (OSC 52).
- Tabs, split panes, multiple windows, tab tear-off that keeps the session running, restore tabs on
  start, per-host theme and font.

**Hosts and sign-in**
- Saved hosts with nested groups, tags, favourites, colours and notes; fuzzy search; quick connect
  (`user@host:port`); recents; system-tray quick connect.
- `~/.ssh/config` import (`Host`, `Match`, `Include`, `ProxyJump`, `ProxyCommand`, `IdentityFile`,
  forwards); `~/.ssh/known_hosts` is read (never written).
- Password, public key (files or the encrypted vault, with passphrase prompts), SSH agent (Windows
  OpenSSH agent and Pageant), keyboard-interactive / 2FA; per-host method order.
- Key manager: generate (Ed25519, ECDSA, RSA), import (OpenSSH, PEM, PuTTY `.ppk`), copy / export the
  public key, install a key on a server.

**Connections**
- `ProxyJump` chains with per-hop prompts and error attribution; `ProxyCommand`; agent forwarding;
  X11 forwarding (fake-cookie substitution); compression; optional legacy algorithms.
- Local, remote and dynamic (SOCKS5) forwards, saved per host, restored after a reconnect, with
  live statistics and explained refusals.
- Keepalives, automatic reconnect with back-off, a 2 MiB replay buffer per terminal and
  acknowledgement-based flow control.
- Host key verification with *Ask / Accept new / Strict* policies; a changed key is a hard block with a
  side-by-side comparison.

**SFTP**
- Dual-pane local / remote browser; drag and drop between the panes and from / to Explorer; transfer
  queue with pause, resume and cancel; per-file conflict resolution; rename, delete, new folder,
  `chmod`; a small text editor.

**Interface**
- Command palette with global search (hosts, commands, open tabs, and the output of every open
  terminal, with the match highlighted); eleven built-in themes, custom JSON themes and custom CSS;
  light / dark / follow Windows, with a 4.5:1 text-contrast floor in the terminal on light themes;
  UI scale and density; onboarding and empty states; notifications; customisable shortcuts;
  screen-reader mode, reduced-motion support and full keyboard operation.

**Distribution**
- Per-user NSIS installer, MSI, and a portable build that keeps everything beside the program — the web
  view's own profile and the window's remembered size and position included — and leaves no folders
  behind in the per-user locations when it is closed.
- *Launch at sign-in* (installed version only; starts hidden in the tray).

**Convenience**
- Quick connect understands a pasted `ssh` command — `ssh -J bastion -i ~/.ssh/work -p 2222 me@host` —
  including `-l`, `-o User= / Port= / ProxyJump= / IdentityFile=`, quoted paths, and options placed
  after the host.
- A jump written as the name of an `ssh_config` entry (`ProxyJump bastion`) uses that entry's host
  name, user, port, key and own jumps; importing a host brings the jump hosts it needs.
- The SFTP editor keeps a file's line endings and byte-order mark, saves through a temporary file created
  with the file's own permissions (and given its owner), writes in place where a swap isn't possible
  (another owner, a closed folder, a server that won't change permissions or move the file), and asks
  before overwriting a file that changed on the server since it was opened.
- `IdentitiesOnly` with `IdentityFile key.pub` selects that key in an agent that holds many (1Password,
  YubiKey, Pageant) instead of skipping the agent.
- An imported `ProxyCommand` keeps its quotes and tokens exactly as written; `%h %p %r %n %u %d` are filled
  in when the connection is made.
- Recents remember how a quick connection was opened — jump hosts and key included, or the `ssh_config`
  alias for a host that goes through a ProxyCommand.
- `aether ssh -J bastion me@db` from a second launch opens one connection, not one per word.

### Security
- Secrets live in Windows Credential Manager / DPAPI, are opt-in, zeroized in memory and absent from
  logs. Strict content-security policy, minimal Tauri capabilities, browser features disabled. See
  [docs/SECURITY.md](docs/SECURITY.md).
- **Host keys:** a key of a type you haven't verified is treated like a change (also under *Accept
  new*); *Connect once* is remembered for the session and a different key on reconnect blocks, with
  typed passwords forgotten; a key that can't be recorded stops the connection; Enter / Space are
  ignored for half a second after a trust dialog appears; the connection timeout waits while you read
  a fingerprint. Only real host names are written to `known_hosts`, and a `known_hosts` file that can't
  be read is never overwritten.
- **ProxyCommand** is confirmed in a native dialog when saved or imported — shown whole (once per
  distinct command, with the hosts that use it), with hidden characters written out and a note on
  look-alike characters, and not approved at all if it can't be shown in full — accepts only plain
  names for `%h %r %n`, and runs from your user folder without the current-directory program search.
- **known_hosts that can't be read** stops the connection under every policy, rather than being treated as
  empty; rewrites keep every other line byte for byte, and replacing a key is a single write.
- Prompts: an answer goes to the prompt it was meant for; no prompt accepts an answer in its first half
  second (keyboard or mouse), so a stray Enter or the second click of a double-click can't answer the
  next question too. An empty password can still be sent on purpose.
- A mistyped second-factor code no longer discards the password you asked to remember; one-time-password
  prompts are never answered with the saved password.
- Imported aliases that reach one machine differently are no longer merged into one.
- **X11:** remote programs must present the session's cookie — checked before anything connects to your
  X server — a program that never speaks is dropped, and only a few may wait to show it at once.
- **Terminal:** server-supplied titles and notifications are plain, bounded and rate-limited;
  clipboard writes (OSC 52) are capped, limited to the terminal in front, and announced; paste drops
  control codes and trailing newlines; replayed history triggers nothing; dropped file names are
  shell-quoted.
- **Files:** server-chosen file names are sanitised and confined to the download folder; local renames
  never replace files; data files keep the last good backup, are never overwritten when they couldn't
  be read, and the app never shows more than what was saved.
- **Secrets:** passwords entered over keyboard-interactive are remembered only after they work, saved
  ones that are rejected are forgotten, "remember" is offered only where it can be honoured, typed
  replies are wiped from memory and masked in logs, and DPAPI files are written atomically and flushed.

