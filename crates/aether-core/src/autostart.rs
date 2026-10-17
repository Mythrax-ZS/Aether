//! "Start at sign-in" for desktops that follow the freedesktop autostart convention: a `.desktop` file in
//! `<config>/autostart/` is launched when the person signs in. Windows, macOS and Linux go through the Tauri autostart
//! plugin; this is for the systems it has nothing for (the BSDs).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "aether.desktop";

/// The sign-in entry of one program: present or not.
pub struct Autostart {
    file: PathBuf,
    entry: String,
}

impl Autostart {
    /// `config_home` is the user's XDG configuration folder (`~/.config`); the entry starts `exe` with `args`.
    pub fn new(config_home: &Path, exe: &Path, args: &[&str]) -> Self {
        let command: Vec<String> = std::iter::once(exe.to_string_lossy().into_owned())
            .chain(args.iter().map(ToString::to_string))
            .map(|arg| exec_arg(&arg))
            .collect();
        Self { file: config_home.join("autostart").join(FILE_NAME), entry: desktop_entry(&command.join(" ")) }
    }

    pub fn is_enabled(&self) -> io::Result<bool> {
        self.file.try_exists()
    }

    pub fn enable(&self) -> io::Result<()> {
        if let Some(folder) = self.file.parent() {
            fs::create_dir_all(folder)?;
        }
        fs::write(&self.file, &self.entry)
    }

    /// Removes the entry. One that is already gone is not an error.
    pub fn disable(&self) -> io::Result<()> {
        match fs::remove_file(&self.file) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

fn desktop_entry(exec: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName=Aether\nComment=SSH client\nExec={exec}\nTerminal=false\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

/// One argument of an `Exec=` line, quoted the way the Desktop Entry specification asks for.
fn exec_arg(arg: &str) -> String {
    const RESERVED: &[char] =
        &[' ', '\t', '\n', '\r', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(', ')', '`'];
    let arg = arg.replace('%', "%%");
    if !arg.contains(RESERVED) {
        return arg;
    }
    let mut quoted = String::from("\"");
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    // The whole value is a string in the file format too, where a backslash is written twice and a line break can't
    // appear as itself (it would start another key).
    quoted.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(exe: &str, args: &[&str]) -> Autostart {
        Autostart::new(Path::new("/cfg"), Path::new(exe), args)
    }

    fn exec_of(a: &Autostart) -> &str {
        a.entry.lines().find_map(|l| l.strip_prefix("Exec=")).expect("an Exec line")
    }

    #[test]
    fn a_plain_command_is_left_alone() {
        let a = entry("/usr/local/bin/aether", &["--background"]);
        assert_eq!(exec_of(&a), "/usr/local/bin/aether --background");
    }

    #[test]
    fn a_path_with_spaces_is_quoted() {
        let a = entry("/home/me/My Apps/aether", &["--background"]);
        assert_eq!(exec_of(&a), "\"/home/me/My Apps/aether\" --background");
    }

    #[test]
    fn reserved_characters_are_escaped_for_both_layers() {
        // quote, backtick, dollar and backslash get a backslash for the command line, and every backslash is
        // doubled again for the file format.
        assert_eq!(exec_arg(r#"a"b"#), r#""a\\"b""#);
        assert_eq!(exec_arg("a`b"), r#""a\\`b""#);
        assert_eq!(exec_arg("$HOME"), r#""\\$HOME""#);
        assert_eq!(exec_arg(r"a\b"), r#""a\\\\b""#);
    }

    #[test]
    fn a_percent_sign_is_not_a_field_code() {
        assert_eq!(exec_arg("50%"), "50%%");
    }

    #[test]
    fn a_line_break_in_the_path_cannot_start_another_key() {
        let a = entry("/tmp/a\nTryExec=/bin/sh", &[]);
        assert_eq!(a.entry.lines().count(), 8, "{}", a.entry);
        assert!(exec_of(&a).starts_with('"') && exec_of(&a).contains("\\n"));
    }

    #[test]
    fn the_entry_names_the_program_and_runs_without_a_terminal() {
        let a = entry("/usr/local/bin/aether", &[]);
        assert!(a.entry.starts_with("[Desktop Entry]\nType=Application\n"));
        assert!(a.entry.contains("\nName=Aether\n") && a.entry.contains("\nTerminal=false\n"));
    }

    #[test]
    fn enabling_and_disabling_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let a = Autostart::new(tmp.path(), Path::new("/usr/local/bin/aether"), &["--background"]);
        assert!(!a.is_enabled().unwrap());

        a.enable().unwrap();
        assert!(a.is_enabled().unwrap());
        let written = fs::read_to_string(tmp.path().join("autostart").join(FILE_NAME)).unwrap();
        assert_eq!(written, a.entry);

        a.disable().unwrap();
        assert!(!a.is_enabled().unwrap());
        a.disable().expect("removing what is already gone is fine");
    }

    #[test]
    fn enabling_replaces_an_entry_that_points_somewhere_else() {
        let tmp = tempfile::tempdir().unwrap();
        let old = Autostart::new(tmp.path(), Path::new("/old/place/aether"), &[]);
        old.enable().unwrap();
        let new = Autostart::new(tmp.path(), Path::new("/new/place/aether"), &[]);
        new.enable().unwrap();
        let written = fs::read_to_string(tmp.path().join("autostart").join(FILE_NAME)).unwrap();
        assert!(written.contains("Exec=/new/place/aether\n") && !written.contains("/old/place"));
    }
}
