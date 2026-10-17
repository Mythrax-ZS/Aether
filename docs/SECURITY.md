# Security

This page describes what Aether protects, how, and where the limits are. If you think you have found
a vulnerability, please **do not open a public issue** — report it privately through the project's
security advisory page (or contact the maintainers directly) with steps to reproduce. You will get an
acknowledgement, and a fix or an explanation, as quickly as we can manage.

## What Aether is trying to protect

| Asset | From |
|---|---|
| Your credentials (passwords, key passphrases, private keys) | other users and programs on the machine, files left on disk, logs, backups |
| The identity of the servers you connect to | someone on the network pretending to be the server |
| Your PC and your files | a server (or a file it sends) that is hostile: names that try to escape the download folder, escape sequences that try to type for you, notifications that try to look like something else |
| The content of your sessions | the renderer being tricked into doing something other than what you asked |

Out of scope: malware already running as you (it can read what you can), a compromised server (it sees
what you type to it), and a hostile *administrator* of the PC.

## Server identity: host keys

Every connection — including each hop of a jump chain, and every automatic reconnect — verifies the
server's host key against known keys before any credential is sent.

- **Aether's own `known_hosts`** (in the config folder) holds the keys you accepted. Entries are plain
  host names so the *Known hosts* screen can list and remove them. Only real host names are ever
  written: a "name" containing `*`, `?`, `!`, a comma, a `|1|` prefix or whitespace — which another
  program would read as a pattern, a list or a hashed entry — is refused.
- **`~/.ssh/known_hosts` is also read** (setting: *Also trust ~/.ssh/known_hosts*), including hashed
  entries (`|1|salt|hash`), wildcards and `@revoked` markers. Aether **never writes** to that file.
  Files saved by a Windows shell redirect (UTF-16) or an editor (a byte-order mark, a legacy code page)
  still count. A file that exists but cannot be *read* (no permission, locked by another program) is
  never treated as an empty one: the connection stops with an explanation, under every policy, because
  nothing can be said about a key when the list of trusted keys can't be consulted — "first visit" would
  be false, and an impostor's key could pass for a new one. The file is also never overwritten, and
  edits to Aether's own file leave every other line exactly as it was.
- **Unknown server** (policy *Ask*, the default): a dialog shows the algorithm, the SHA-256
  fingerprint and its randomart, with *Cancel* / *Connect once* / *Trust and connect*. Enter and Space
  are ignored for the first half second of this dialog, so a keystroke you were making in the terminal
  behind it cannot accept a key you have not read. While the question is open the connection timeout
  waits for you; it starts again from zero once you answer.
- **Changed key:** the connection is blocked. The dialog compares the trusted and presented
  fingerprints side by side and offers *Cancel* as the default; replacing the saved key needs an
  explicit tick ("I've confirmed the new fingerprint with the server's administrator") and a separate
  button.
- **A key of a type you have not verified** — you trust a server's ed25519 key and it now presents an
  ECDSA one — is treated like a change, not like a first visit, and is **not** waved through by
  *Accept new*: that is exactly what someone in the middle would do to avoid the changed-key warning.
  Accepting it adds the new key next to the old one.
- **Reconnecting.** A key you accepted with *Connect once* is remembered for the rest of that session,
  so a dropped connection to the same server doesn't ask again. A *different* key on the way back is a
  change: the blocking dialog appears, and the passwords you typed for the old key are forgotten rather
  than sent to the new one.
- **If the key can't be recorded** (the file is locked, the disk is full), the connection stops with a
  message instead of carrying on — *Trust and connect* never silently means *Connect once*, and a
  retry asks again rather than being waved through. Replacing a changed key is a single write: a failure
  can't leave the host with no key on record.
- **Policies** (*Settings → SSH → Trust*):
  - *Ask* — prompt for unknown servers, block changed keys unless you confirm replacement.
  - *Accept new* — trust unknown servers automatically (and remember them); a changed key or an
    unfamiliar key type still blocks.
  - *Strict* — refuse anything not already verified, and refuse changed keys outright.
- A key listed as `@revoked` is always refused.

## Credentials

- **Nothing secret is written to the hosts file, the settings file, the workspace file or the logs.**
  Hosts store *which* key or method to use, never the secret.
- **Remembering is opt-in, and only offered where it can work.** The password, passphrase and
  key-import dialogs have an unchecked "Remember…" box — shown for saved hosts only (a quick-connect
  target has nowhere to keep it). A password typed into a keyboard-interactive prompt is saved only
  after the server has accepted it; a saved password that the server rejects is forgotten and asked for
  again. One-time codes are never saved or replayed.
- **Where remembered secrets live** (see `aether-vault`):
  - *Installed:* passwords and passphrases in **Windows Credential Manager**, private keys in files
    sealed with **DPAPI** (`CryptProtectData`, current-user scope, with application-specific entropy).
  - *Portable / custom folder:* DPAPI-sealed files in the data folder only — nothing is left in the
    machine's credential store. A sealed file is useless to other users, other machines, and anyone
    who copies it off disk. Sealed files are written under a unique temporary name, flushed to disk and
    then moved into place, so a crash leaves the old secret or the new one, never an empty file.
- **In memory:** secrets are held in a zeroizing wrapper that redacts itself in `Debug` output (a test
  guards this). What you type into a password box is wiped when the reply is dropped and prints as
  "N characters" if it ever reaches a log line. The copy the OS hands back from Credential Manager or
  DPAPI is scrubbed before it is released. Credentials typed during a connection are kept — per hop —
  only so an automatic reconnect can re-authenticate without asking again, and are dropped when the
  session closes (or when that hop's host key changes).
- **Across the UI boundary:** a secret travels once, from the prompt you typed it into to Rust. The UI
  is never sent a stored secret back; "is a password saved?" is answered with a yes / no.
- **Keys:** OpenSSH, PEM (PKCS#1 / PKCS#8) and PuTTY `.ppk` private keys, with passphrase prompts.
  When you import an encrypted key and type its passphrase, the passphrase is *verified* before it can
  be saved. Key generation uses a cryptographically secure random number generator. *Only use this key*
  (`IdentitiesOnly`) keeps the agent's other keys out of the login entirely.

## Network and protocol

- The SSH implementation is [`russh`](https://github.com/Eugeny/russh) with the `ring` crypto backend.
  The default algorithm set is modern: curve25519 and NIST ECDH key exchange; ChaCha20-Poly1305 and
  AES-GCM ciphers (AES-CTR with HMAC-SHA2 as a fallback); Ed25519, ECDSA and RSA-SHA2 host keys.
  Older algorithms — SHA-1 key exchange and `ssh-rsa` signatures, CBC ciphers, HMAC-SHA1 — are
  available only through a per-host **Allow legacy algorithms** switch, off by default (a server
  that needs them gets an error card that says so).
- Strict key exchange (the Terrapin mitigation) violations are treated as fatal protocol errors.
- **Keepalives** detect dead connections; an unresponsive peer is classified and surfaced rather than
  left hanging.
- **Port forwards** bind to `127.0.0.1` unless you type another address. A forward that the server
  refuses reports why, instead of failing silently.
- **ProxyCommand** (a host option, or imported from `ssh_config`) runs the command you configured on
  *this PC* when you connect — exactly like OpenSSH — so it is treated as code execution:
  - Saving a host with a new or changed ProxyCommand, and importing hosts that have one, shows a
    **native Windows dialog** (not part of the web page) with the command in it — the whole command,
    with line breaks, tabs, control characters and invisible characters written out, so nothing can hide
    in it, and a note when it contains characters beyond plain ASCII (which can look like letters). Hosts
    that share a command are listed under it once, with their names (escaped and shortened, so a name
    can't write lines of its own into the dialog). The window is a native message box, which doesn't
    scroll, so what doesn't fit is not shown *and not approved*: the limits are on what is printed (a
    control character prints as several), 800 characters per command, 2,000 in all and five distinct
    commands. Past them a save is refused, and an import goes ahead *without* the commands. Declining
    a save keeps the host as it was; declining an import imports the hosts without their commands.
  - An imported ProxyCommand is kept exactly as written (quotes and `||` included) and keeps its
    `%h %p %r %n %u %d` tokens, which are filled in when the connection is made.
  - What the host's side puts into the command line (`%h`, `%r`, `%n`) must be a plain name: letters,
    digits, dots, dashes, underscores and colons; anything with shell syntax in it (ampersands, pipes,
    semicolons, quotes, carets, percent signs, backticks, a leading `-`, look-alike Unicode) is refused
    rather than escaped. Substituted text is never expanded a second time.
  - The command starts in your user folder, with the current-directory program search turned off, so a
    stray `ssh.exe` next to the app or in a download folder is not picked up.
- **`ssh_config` import** reads at most 128 files / 4 MiB through `Include`, recognises include loops,
  and never runs `Match exec`. A host imported with a `ProxyJump` brings the jump hosts it needs, or
  links to the ones you already saved — it never silently loses its bastion. Two aliases for one machine
  that are reached differently (one through a bastion, one directly) stay two hosts; they are only
  merged when the way in is the same too.
- **Agent keys and `IdentitiesOnly`.** With `IdentitiesOnly yes`, an agent is asked to sign only for the
  keys your `IdentityFile` lines name (by their `.pub` files, as OpenSSH does it): the other keys it holds
  are never offered to the server, where each offer counts toward its attempt limit.

## Features that widen the trust boundary

These are **off by default** and enabled per host:

- **Agent forwarding.** A server you forward your agent to can ask it to sign things as long as the
  session lasts. Enable it only for servers you trust fully.
- **X11 forwarding.** Remote programs get access to your X server, which means they can read your
  screen and keyboard events while they run. Aether never sends your real `.Xauthority` cookie to the
  server: it hands the server a random one, accepts a remote program **only if it presents that
  cookie** (the forwarded display is reachable by every user on the server), and swaps in the real
  cookie locally. A program that opens the channel and says nothing is cut off after ten seconds. The
  host editor states the risk next to the switch.

## What a server can make the app do

A terminal is where a program's output and a person's input meet, so both directions are handled
carefully.

- **Output.** Window titles and desktop notifications requested by the server are reduced to plain,
  bounded text (no control codes or direction overrides), titles are applied at most ten times a
  second, and a notification arrives at most once every few seconds, prefixed with the name of the
  session it came from and — with the default *only when Aether isn't in front* — never while you are
  looking at that terminal. A setting turns server notifications off. The bell is rate-limited too.
- **Clipboard writes** (OSC 52) are on by default so `tmux` and `vim` can copy, but only for the real
  clipboard, only from the terminal in front, at most 100 000 characters, not more than a couple of
  times a second — and each one shows a notice. Reading the clipboard is never allowed. A setting turns
  writes off.
- **History is not news.** When a window starts showing a terminal (a torn-off tab), the output it
  missed is replayed so the screen is right — but its bells, notifications, clipboard writes and
  queries are not acted on a second time, and the terminal does not answer queries from the past (the
  shell would read the answers as typing).
- **Input.** Pasting removes control codes — an Escape in the clipboard can end a bracketed paste
  early and let the rest run as typed commands — and a trailing line break, so a copied command doesn't
  run before you've read it. Multi-line pastes ask first. Files dropped on a terminal are typed as
  single-quoted shell words, so a file called `a$(touch pwned).txt` stays a name.
- **Links** in terminal output open only as `http(s):` or `mailto:` URLs, in your default browser,
  and only with Ctrl+click.

## Files

- **SFTP names come from the server and are treated as hostile.** Each one is turned into a single
  safe path component before it touches your disk: path separators become underscores (so `..\..\x` is
  just an odd file name, not a way out of the folder), as do drive letters, UNC prefixes, trailing dots
  and spaces and every character Windows forbids; reserved device names are defused; the length is
  bounded and colliding names are made distinct. Downloads are written to `<name>.aetherpart` and renamed when complete, are verified
  to stay inside the folder you chose, and a resumed transfer only continues a file that *the same
  job* started.
- **The text editor** refuses files it can't carry through a text box and back (binary data, a legacy
  code page, UTF-16) instead of damaging them, remembers the line endings and byte-order mark and
  writes them back, and saves safely: the new contents are written next to the file — created with its
  permissions from the first byte, so a secret is never briefly world-readable — and given its owner,
  then swapped in, with the original put aside until the swap has succeeded; if a rename's answer is
  lost, Aether looks at what is where and puts things right (and if the link is gone, says where the
  original and the new contents are). Where a swap isn't possible — it would change who owns the file,
  the folder is closed to you, the server won't change permissions, or won't move the file (a mount
  point, a sticky folder) — the contents are written in place instead, as an editor does. A folder is
  never replaced by a file. If the file changed on the server since you opened it, saving asks before
  replacing it. A file reached through a symlink is edited, not replaced by a copy.
- **Local files.** Deletes go to the Recycle Bin. A rename never replaces an existing file (the check
  and the move are one operation). "Show in folder" only opens paths that exist.
- **Aether's own data files** (hosts, settings, keys, workspace) are written through a temporary file
  that is flushed and renamed, with the last *good* version kept as `.bak` — an empty or damaged file
  never replaces it. A file that is damaged is moved aside (`.corrupt-…`) and the backup used; a file
  that merely can't be *read* (locked by a scanner, no permission) is left alone and never overwritten.
  What you see on screen is never ahead of what is on disk: a save that fails leaves the list as it was.

## The application itself

- **No remote content.** The UI is bundled with the app; the page's Content-Security-Policy is
  `default-src 'self'` with no remote scripts, frames, objects or form targets, and `connect-src` is
  limited to the app's own IPC. Scripts may compile WebAssembly (for the inline-image decoder) but not
  evaluate JavaScript from strings.
- **Small capability set.** The webview may use the window, dialog, notification, clipboard and
  "open link" permissions it needs; there is no generic file-system, shell or HTTP plugin. File,
  SSH and secret access exist only as Aether's own typed commands, and the slow ones run off the
  thread that serves the UI.
- **Browser behaviour is switched off:** reload, print, view-source, find-in-page, pinch zoom, the
  default context menu, and navigation by dropping a file on the window.
- **Single instance.** A second launch hands its arguments to the running copy rather than starting
  another one.
- **Launch at sign-in** is registered only by the installed app, never by a portable copy (which could
  be moved or deleted), and starts hidden in the tray.
- **Updates.** Aether does not phone home; there is no telemetry and no automatic update check.

## Reviewing the code

The security-relevant code is small and concentrated:

| Concern | Where |
|---|---|
| Host-key verification, "connect once" pinning | `crates/aether-ssh/src/handler.rs`, `session.rs`, `crates/aether-core/src/known_hosts.rs` |
| Authentication flow | `crates/aether-ssh/src/auth.rs`, `keys.rs` |
| Secret storage | `crates/aether-vault/` |
| What the UI may do | `src-tauri/capabilities/default.json`, `src-tauri/tauri.conf.json` (CSP) |
| Native confirmations (ProxyCommand) | `src-tauri/src/commands/hosts.rs`, `crates/aether-ssh/src/transport.rs` |
| X11 cookie handling | `crates/aether-ssh/src/x11.rs` |
| Hostile server names, SFTP editing | `crates/aether-ssh/src/transfers.rs`, `sftp.rs`, `text.rs` |
| What a server can make the terminal do | `ui/src/lib/terminal/controller.ts`, `safety.ts` |
| Data files | `crates/aether-core/src/store/json_file.rs` |

Dependencies are pinned through `Cargo.lock` and `package-lock.json`; `npm audit` reports no known
vulnerabilities at the time of release, and `cargo audit` is worth running before every release.
