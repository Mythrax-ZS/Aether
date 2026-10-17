//! Tokenising and `Include` expansion for ssh_config files.

use std::fs;
use std::path::{Path, PathBuf};

use super::{Cond, MatchKind, MatchTerm, Opt, Section, SshConfig};
use crate::util::{expand_tilde, glob_match};

const MAX_INCLUDE_DEPTH: usize = 16;
/// Real configs include a handful of small files. A file that includes itself through a wildcard, or a directory of
/// files that include each other, can otherwise multiply into millions of reads — so one config may pull in at most this
/// many files and this many bytes in total, and any file bigger than `MAX_CONFIG_FILE_BYTES` is not read at all.
const MAX_INCLUDED_FILES: usize = 128;
const MAX_INCLUDED_BYTES: usize = 4 * 1024 * 1024;
const MAX_CONFIG_FILE_BYTES: u64 = 1024 * 1024;
/// How many files one wildcard may match.
const MAX_GLOB_MATCHES: usize = 256;

/// Supplies the contents of `Include`d files. Abstracted so parsing is testable without a disk.
pub trait IncludeSource {
    /// Expands one `Include` argument (which may contain globs) into `(path, contents)`
    /// pairs in lexical order. Unreadable or missing files are simply omitted. The path is the file's identity: the
    /// same file must always come back under the same name, so that an include loop can be recognised.
    fn read(&self, pattern: &str) -> Vec<(String, String)>;
}

/// An include source with nothing in it, for configs that come from text alone.
pub struct NoIncludes;

impl IncludeSource for NoIncludes {
    fn read(&self, _pattern: &str) -> Vec<(String, String)> {
        Vec::new()
    }
}

/// Reads includes from the file system. Relative patterns are resolved against `base`
/// (OpenSSH uses `~/.ssh` for user configs).
pub struct FsIncludes {
    base: PathBuf,
}

impl FsIncludes {
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }
}

impl IncludeSource for FsIncludes {
    fn read(&self, pattern: &str) -> Vec<(String, String)> {
        let path = expand_tilde(pattern);
        let path = if path.is_absolute() || pattern.starts_with('~') { path } else { self.base.join(path) };
        expand_glob(&path)
            .into_iter()
            .take(MAX_GLOB_MATCHES)
            .filter_map(|p| {
                if fs::metadata(&p).ok()?.len() > MAX_CONFIG_FILE_BYTES {
                    tracing::warn!(file = %p.display(), "not reading an included ssh config file this large");
                    return None;
                }
                let text = fs::read_to_string(&p).ok()?;
                // Under its real name, so the same file reached by two routes (a link, another spelling) is one file.
                Some((identity_of(&p), text))
            })
            .collect()
    }
}

/// The name a config file goes by when checking whether an include loops back on itself.
pub(super) fn identity_of(path: &Path) -> String {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()).display().to_string()
}

/// Minimal glob expander: `*` and `?` in any path component.
fn expand_glob(pattern: &Path) -> Vec<PathBuf> {
    let mut results = vec![PathBuf::new()];
    for comp in pattern.components() {
        let name = comp.as_os_str().to_string_lossy().into_owned();
        let is_glob = name.contains(['*', '?']);
        let mut next = Vec::new();
        for base in &results {
            if is_glob {
                let dir = if base.as_os_str().is_empty() { Path::new(".") } else { base.as_path() };
                if let Ok(rd) = fs::read_dir(dir) {
                    let mut names: Vec<String> = rd
                        .filter_map(|e| e.ok())
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .filter(|n| glob_match(&name, n))
                        .collect();
                    names.sort();
                    next.extend(names.into_iter().map(|n| base.join(n)));
                }
            } else {
                next.push(base.join(comp.as_os_str()));
            }
        }
        results = next;
    }
    results.into_iter().filter(|p| p.is_file()).collect()
}

/// In-memory include source for tests.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct MemoryIncludes {
    files: Vec<(String, String)>,
}

#[cfg(test)]
impl MemoryIncludes {
    pub(crate) fn add(&mut self, name: &str, content: &str) {
        self.files.push((name.to_string(), content.to_string()));
    }
}

#[cfg(test)]
impl IncludeSource for MemoryIncludes {
    fn read(&self, pattern: &str) -> Vec<(String, String)> {
        self.files.iter().filter(|(name, _)| glob_match(pattern, name)).cloned().collect()
    }
}

// ── tokenizer ────────────────────────────────────────────────────────────────

/// Splits one config line into a lower-cased keyword and the rest of the line, as written. Returns `None` for blank
/// lines and comments.
pub(super) fn split_keyword(line: &str) -> Option<(String, &str)> {
    let line = line.trim_start();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let kw_end = line.find(|c: char| c.is_whitespace() || c == '=').unwrap_or(line.len());
    let keyword = line[..kw_end].to_ascii_lowercase();
    let mut rest = line[kw_end..].trim_start();
    if let Some(after) = rest.strip_prefix('=') {
        rest = after.trim_start();
    }
    Some((keyword, rest))
}

/// Splits one config line into a lower-cased keyword and its arguments. Returns `None`
/// for blank lines and comments.
#[cfg(test)]
pub(super) fn tokenize(line: &str) -> Option<(String, Vec<String>)> {
    let (keyword, rest) = split_keyword(line)?;
    Some((keyword, split_args(rest)))
}

fn split_args(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            '\\' if in_quotes && matches!(chars.peek(), Some('"') | Some('\\')) => {
                if let Some(escaped) = chars.next() {
                    cur.push(escaped);
                }
            }
            '#' if !in_quotes && !has_token => break,
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    args.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        args.push(cur);
    }
    args
}

// ── parser ───────────────────────────────────────────────────────────────────

struct Parser<'a> {
    sections: Vec<Section>,
    warnings: Vec<String>,
    includes: &'a dyn IncludeSource,
    depth: usize,
    /// The files being read right now, outermost first: a file asking for one of these is an include loop.
    active: Vec<String>,
    /// Totals over the whole config, against `MAX_INCLUDED_FILES` and `MAX_INCLUDED_BYTES`.
    included_files: usize,
    included_bytes: usize,
    /// Set once a limit was hit (and said so), so every later `Include` stays quiet instead of repeating itself.
    cut_off: bool,
}

pub(super) fn parse(text: &str, includes: &dyn IncludeSource) -> SshConfig {
    parse_named(text, includes, None)
}

/// Like [`parse`], for text that came from the file `root` — so that a file including itself is recognised at once.
pub(super) fn parse_named(text: &str, includes: &dyn IncludeSource, root: Option<String>) -> SshConfig {
    let mut p = Parser {
        sections: vec![Section { cond: Cond::Global, opts: Vec::new() }],
        warnings: Vec::new(),
        includes,
        depth: 0,
        active: root.into_iter().collect(),
        included_files: 0,
        included_bytes: 0,
        cut_off: false,
    };
    p.feed(text);
    SshConfig { sections: p.sections, warnings: p.warnings }
}

impl Parser<'_> {
    fn current_cond(&self) -> Cond {
        self.sections.last().map(|s| s.cond.clone()).unwrap_or(Cond::Global)
    }

    fn feed(&mut self, text: &str) {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        for (i, line) in text.lines().enumerate() {
            let line_no = i + 1;
            let Some((key, rest)) = split_keyword(line) else { continue };
            let args = split_args(rest);
            match key.as_str() {
                "host" => self.sections.push(Section { cond: Cond::Host(args), opts: Vec::new() }),
                "match" => {
                    let cond = self.parse_match(&args, line_no);
                    self.sections.push(Section { cond, opts: Vec::new() });
                }
                "include" => self.include(&args),
                _ => {
                    if let Some(section) = self.sections.last_mut() {
                        section.opts.push(Opt { key, args, raw: rest.trim_end().to_string(), line: line_no });
                    }
                }
            }
        }
    }

    fn include(&mut self, patterns: &[String]) {
        if self.depth >= MAX_INCLUDE_DEPTH {
            self.warnings.push("Include nesting is too deep; ignoring further includes".into());
            return;
        }
        let outer = self.current_cond();
        self.depth += 1;
        for pattern in patterns {
            if self.cut_off {
                break;
            }
            let files = self.includes.read(pattern);
            if files.is_empty() && !pattern.contains(['*', '?']) {
                self.warnings.push(format!("Included file `{pattern}` was not found"));
            }
            for (name, content) in files {
                if self.active.contains(&name) {
                    self.warnings.push(format!(
                        "`Include {pattern}` would loop back to a file that is already being read; skipped"
                    ));
                    continue;
                }
                if self.included_files >= MAX_INCLUDED_FILES || self.included_bytes + content.len() > MAX_INCLUDED_BYTES
                {
                    self.warnings.push(format!(
                        "Too many or too large included files (the limit is {MAX_INCLUDED_FILES} files and {} MiB); the rest are ignored",
                        MAX_INCLUDED_BYTES / (1024 * 1024)
                    ));
                    self.cut_off = true;
                    break;
                }
                self.included_files += 1;
                self.included_bytes += content.len();
                // An included file starts in the condition of the line that included it.
                self.sections.push(Section { cond: outer.clone(), opts: Vec::new() });
                self.active.push(name);
                self.feed(&content);
                self.active.pop();
            }
        }
        self.depth -= 1;
        // Back in the including file: options that follow belong to the outer condition.
        self.sections.push(Section { cond: outer, opts: Vec::new() });
    }

    fn parse_match(&mut self, args: &[String], line: usize) -> Cond {
        let mut terms = Vec::new();
        let mut i = 0;
        let mut unsupported = false;
        while i < args.len() {
            let mut kw = args[i].to_ascii_lowercase();
            let negate = if let Some(rest) = kw.strip_prefix('!') {
                kw = rest.to_string();
                true
            } else {
                false
            };
            i += 1;
            let mut list = || -> Vec<String> {
                let v = args.get(i).map(|a| a.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default();
                i += 1;
                v
            };
            let kind = match kw.as_str() {
                "all" => MatchKind::All,
                "canonical" | "final" => MatchKind::Never,
                "host" => MatchKind::Host(list()),
                "originalhost" => MatchKind::OriginalHost(list()),
                "user" => MatchKind::User(list()),
                "localuser" => MatchKind::LocalUser(list()),
                "exec" => {
                    let _ = list();
                    self.warnings.push(format!(
                        "line {line}: `Match exec` is not supported (Aether never runs commands from config files); the block is ignored"
                    ));
                    MatchKind::Exec
                }
                other => {
                    unsupported = true;
                    self.warnings
                        .push(format!("line {line}: unsupported Match criterion `{other}`; the block is ignored"));
                    break;
                }
            };
            terms.push(MatchTerm { negate, kind });
        }
        if unsupported || terms.is_empty() {
            terms = vec![MatchTerm { negate: false, kind: MatchKind::Never }];
        }
        Cond::Match(terms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizer_handles_quotes_equals_and_comments() {
        assert_eq!(tokenize("  # comment"), None);
        assert_eq!(tokenize(""), None);
        assert_eq!(tokenize("Port=22"), Some(("port".to_string(), vec!["22".to_string()])));
        assert_eq!(
            tokenize("IdentityFile \"a b\" c # note"),
            Some(("identityfile".to_string(), vec!["a b".to_string(), "c".to_string()]))
        );
        assert_eq!(
            tokenize("Host a#b"),
            Some(("host".to_string(), vec!["a#b".to_string()])),
            "# inside a token is part of it"
        );
        assert_eq!(
            tokenize(r#"SetEnv "A=say \"hi\"""#),
            Some(("setenv".to_string(), vec![r#"A=say "hi""#.to_string()]))
        );
    }

    #[test]
    fn fs_includes_expand_globs_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let confd = dir.path().join("conf.d");
        fs::create_dir_all(&confd).unwrap();
        fs::write(confd.join("b.conf"), "Host b\n  User bee\n").unwrap();
        fs::write(confd.join("a.conf"), "Host a\n  User ay\n").unwrap();
        fs::write(confd.join("ignored.txt"), "Host nope\n").unwrap();

        let inc = FsIncludes::new(dir.path());
        let got = inc.read("conf.d/*.conf");
        assert_eq!(got.len(), 2);
        assert!(got[0].0.ends_with("a.conf") && got[1].0.ends_with("b.conf"));

        let cfg = SshConfig::parse("Include conf.d/*.conf\n", &inc);
        assert_eq!(cfg.resolve("a", "me").user.as_deref(), Some("ay"));
        assert_eq!(cfg.resolve("b", "me").user.as_deref(), Some("bee"));
    }

    #[test]
    fn a_file_that_includes_itself_is_read_once() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("loop.conf"), "Include loop.conf\nHost x\n  User y\n").unwrap();
        let inc = FsIncludes::new(dir.path());
        let cfg = SshConfig::parse("Include loop.conf\n", &inc);
        assert!(cfg.warnings.iter().any(|w| w.contains("loop back")), "{:?}", cfg.warnings);
        assert_eq!(cfg.resolve("x", "me").user.as_deref(), Some("y"), "its own settings still count");
    }

    /// The config file that includes a folder of files, one of which is the config file itself (`Include *` in
    /// `~/.ssh/config` matches `config`): recognised straight away, whichever way the file is named.
    #[test]
    fn the_root_file_is_recognised_when_something_includes_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config");
        fs::write(&root, "Include *\nHost a\n  User ay\n").unwrap();
        fs::write(dir.path().join("extra"), "Host b\n  User bee\n").unwrap();
        let cfg = SshConfig::load_file(&root).unwrap();
        assert!(cfg.warnings.iter().any(|w| w.contains("loop back")), "{:?}", cfg.warnings);
        assert_eq!(cfg.resolve("a", "me").user.as_deref(), Some("ay"));
        assert_eq!(cfg.resolve("b", "me").user.as_deref(), Some("bee"));
    }

    /// Files that each include all of their neighbours: without a loop check this multiplies by the number of files
    /// at every level, up to the depth cap (here 6^16 reads). It must finish at once, within the file budget.
    #[test]
    fn a_folder_of_files_that_include_each_other_cannot_explode() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..6 {
            fs::write(dir.path().join(format!("{i}.conf")), format!("Include *.conf\nHost h{i}\n  User u{i}\n"))
                .unwrap();
        }
        let inc = FsIncludes::new(dir.path());
        let started = std::time::Instant::now();
        let cfg = SshConfig::parse("Include *.conf\n", &inc);
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "took {:?}", started.elapsed());
        for i in 0..6 {
            assert_eq!(cfg.resolve(&format!("h{i}"), "me").user, Some(format!("u{i}")), "every file is still read");
        }
    }

    #[test]
    fn the_amount_of_included_data_is_capped() {
        let mut inc = MemoryIncludes::default();
        for i in 0..(MAX_INCLUDED_FILES + 20) {
            inc.add(&format!("f{i}.conf"), &format!("Host h{i}\n  User u{i}\n"));
        }
        let cfg = SshConfig::parse("Include f*.conf\nHost last\n  User survivor\n", &inc);
        assert!(cfg.warnings.iter().any(|w| w.contains("Too many")), "{:?}", cfg.warnings);
        assert_eq!(cfg.resolve("h0", "me").user.as_deref(), Some("u0"));
        assert_eq!(cfg.resolve("h127", "me").user.as_deref(), Some("u127"));
        assert_eq!(cfg.resolve("h128", "me").user, None, "past the limit: ignored");
        assert_eq!(cfg.resolve("last", "me").user.as_deref(), Some("survivor"), "the file itself carries on");

        let mut big = MemoryIncludes::default();
        big.add("huge.conf", &format!("# {}\nHost h\n  User u\n", "x".repeat(MAX_INCLUDED_BYTES)));
        let cfg = SshConfig::parse("Include huge.conf\n", &big);
        assert!(cfg.warnings.iter().any(|w| w.contains("Too many or too large")), "{:?}", cfg.warnings);
        assert_eq!(cfg.resolve("h", "me").user, None);
    }

    #[test]
    fn oversized_files_are_not_read() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("big.conf"), vec![b'#'; (MAX_CONFIG_FILE_BYTES + 1) as usize]).unwrap();
        fs::write(dir.path().join("ok.conf"), "Host fine\n  User yes\n").unwrap();
        let inc = FsIncludes::new(dir.path());
        let found: Vec<String> = inc.read("*.conf").into_iter().map(|(n, _)| n).collect();
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].ends_with("ok.conf"));
        // The root config has a limit too.
        let root = dir.path().join("config");
        fs::write(&root, vec![b'#'; 5 * 1024 * 1024]).unwrap();
        assert!(SshConfig::load_file(&root).is_err());
    }
}
