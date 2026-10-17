//! Editing a remote text file without changing what the user didn't touch.
//!
//! A text box edits characters, not bytes: it has no byte-order mark, its lines end in `\n`, and it can only hold what
//! is valid UTF-8. A file can be anything else, so the round trip has to *refuse* what it can't carry (rather than
//! quietly repairing it) and *remember* what it can (the line ending, the BOM) to put back on save.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

const BOM: &[u8] = b"\xEF\xBB\xBF";

/// How a file ends its lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum LineEnding {
    /// `\n` — Unix.
    Lf,
    /// `\r\n` — Windows.
    Crlf,
}

/// What identifies a file's state, to notice that something else changed it between opening and saving.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileVersion {
    #[ts(type = "number")]
    pub size: u64,
    /// Modification time in milliseconds since the epoch.
    #[ts(type = "number | null")]
    pub modified: Option<i64>,
}

/// A text file as an editor holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RemoteText {
    /// The contents with `\n` line ends and no byte-order mark.
    pub text: String,
    /// The file is longer than what was read, so what is shown is only its start and must not be saved back.
    pub truncated: bool,
    /// What the file used. Put back on save, so editing one line doesn't rewrite the ending of every other.
    pub line_ending: LineEnding,
    /// The file used both endings; saving makes them all `line_ending`.
    pub mixed_line_endings: bool,
    /// The file began with a UTF-8 byte-order mark; it is written back.
    pub bom: bool,
    pub version: FileVersion,
}

/// Why a file can't be edited as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotEditable {
    /// Contains NUL bytes: not text.
    Binary,
    /// Not valid UTF-8 (a legacy code page, say). Saving would have to change bytes it doesn't understand.
    NotUtf8,
    /// UTF-16 (found by its byte-order mark).
    Utf16,
    /// A lone carriage return (classic Mac line ends) — a text box would turn them into something else.
    LoneCarriageReturns,
}

impl NotEditable {
    pub fn explain(self) -> &'static str {
        match self {
            Self::Binary => "This looks like a binary file, so it can't be shown as text.",
            Self::NotUtf8 => {
                "This file isn't UTF-8 text (it may use an older character set), so editing it here could damage it."
            }
            Self::Utf16 => "This file is UTF-16 text, which the editor can't save back safely.",
            Self::LoneCarriageReturns => {
                "This file uses old-style line endings (a bare carriage return), which the editor can't keep."
            }
        }
    }
}

/// Reads `bytes` as editable text, or says why it can't be.
pub fn decode(bytes: &[u8], truncated: bool, version: FileVersion) -> Result<RemoteText, NotEditable> {
    // UTF-16 first: its text is full of NUL bytes, and "UTF-16" is a better thing to say than "binary".
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err(NotEditable::Utf16);
    }
    if bytes.contains(&0) {
        return Err(NotEditable::Binary);
    }
    let (body, bom) = match bytes.strip_prefix(BOM) {
        Some(rest) => (rest, true),
        None => (bytes, false),
    };
    let text = if truncated {
        // Only a preview: the cut may fall inside a character, and nothing here will be saved.
        String::from_utf8_lossy(body).into_owned()
    } else {
        std::str::from_utf8(body).map_err(|_| NotEditable::NotUtf8)?.to_owned()
    };

    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    let text = if crlf > 0 { text.replace("\r\n", "\n") } else { text };
    if text.contains('\r') && !truncated {
        return Err(NotEditable::LoneCarriageReturns);
    }
    Ok(RemoteText {
        text,
        truncated,
        line_ending: if crlf > 0 && crlf >= lf { LineEnding::Crlf } else { LineEnding::Lf },
        mixed_line_endings: crlf > 0 && lf > 0,
        bom,
        version,
    })
}

/// The bytes to store for `text` as the editor holds it.
pub fn encode(text: &str, line_ending: LineEnding, bom: bool) -> Vec<u8> {
    // A text box hands over `\n` only; be strict about it anyway so a stray `\r` can't double up.
    let unified = text.replace("\r\n", "\n").replace('\r', "\n");
    let body = match line_ending {
        LineEnding::Lf => unified,
        LineEnding::Crlf => unified.replace('\n', "\r\n"),
    };
    let mut out = Vec::with_capacity(body.len() + if bom { 3 } else { 0 });
    if bom {
        out.extend_from_slice(BOM);
    }
    out.extend_from_slice(body.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: FileVersion = FileVersion { size: 0, modified: None };

    fn read(bytes: &[u8]) -> RemoteText {
        decode(bytes, false, V).unwrap()
    }

    #[test]
    fn plain_unix_text_round_trips_untouched() {
        let original = b"first\nsecond\n\nlast without newline";
        let t = read(original);
        assert_eq!((t.line_ending, t.mixed_line_endings, t.bom), (LineEnding::Lf, false, false));
        assert_eq!(encode(&t.text, t.line_ending, t.bom), original);
    }

    /// The reason this module exists: a Windows file edited in a text box must not come back with Unix endings.
    #[test]
    fn windows_line_endings_survive_an_edit() {
        let original = b"[section]\r\nkey=value\r\n";
        let t = read(original);
        assert_eq!(t.line_ending, LineEnding::Crlf);
        assert_eq!(t.text, "[section]\nkey=value\n", "the editor sees plain newlines");
        assert_eq!(encode(&t.text, t.line_ending, t.bom), original, "unedited: byte for byte");
        let edited = t.text.replace("value", "changed");
        assert_eq!(encode(&edited, t.line_ending, t.bom), b"[section]\r\nkey=changed\r\n");
    }

    #[test]
    fn a_byte_order_mark_is_kept_out_of_the_text_and_put_back() {
        let original = b"\xEF\xBB\xBFhello\n";
        let t = read(original);
        assert!(t.bom);
        assert_eq!(t.text, "hello\n");
        assert_eq!(encode(&t.text, t.line_ending, t.bom), original);
    }

    #[test]
    fn mixed_endings_are_reported_and_resolved_to_the_dominant_one() {
        let t = read(b"a\r\nb\r\nc\n");
        assert!(t.mixed_line_endings);
        assert_eq!(t.line_ending, LineEnding::Crlf);
        let t = read(b"a\nb\nc\r\n");
        assert!(t.mixed_line_endings);
        assert_eq!(t.line_ending, LineEnding::Lf);
        assert_eq!(read(b"no newline at all").line_ending, LineEnding::Lf);
        assert!(!read(b"x\r\ny\r\n").mixed_line_endings);
    }

    #[test]
    fn what_cannot_be_carried_is_refused_not_repaired() {
        assert_eq!(decode(b"abc\0def", false, V), Err(NotEditable::Binary));
        assert_eq!(decode(b"caf\xe9 au lait", false, V), Err(NotEditable::NotUtf8), "Latin-1 is not UTF-8");
        assert_eq!(decode(b"\xFF\xFEh\0i\0", false, V), Err(NotEditable::Utf16));
        assert_eq!(decode(b"old\rmac\rlines", false, V), Err(NotEditable::LoneCarriageReturns));
        for problem in [NotEditable::Binary, NotEditable::NotUtf8, NotEditable::Utf16, NotEditable::LoneCarriageReturns]
        {
            assert!(!problem.explain().is_empty());
        }
    }

    #[test]
    fn a_preview_tolerates_a_cut_in_the_middle_of_a_character() {
        let bytes = "héllo".as_bytes();
        let cut = &bytes[..2]; // 'h' and the first half of 'é'
        let t = decode(cut, true, V).unwrap();
        assert!(t.truncated && t.text.starts_with('h'));
        assert!(decode(cut, false, V).is_err(), "but a whole file that is invalid is not editable");
    }

    #[test]
    fn stray_carriage_returns_in_edited_text_cannot_double_up() {
        assert_eq!(encode("a\r\nb\rc\n", LineEnding::Crlf, false), b"a\r\nb\r\nc\r\n");
        assert_eq!(encode("a\r\nb\n", LineEnding::Lf, false), b"a\nb\n");
    }

    #[test]
    fn non_ascii_text_is_stored_as_utf8() {
        let t = read("naïve — 日本語\n".as_bytes());
        assert_eq!(t.text, "naïve — 日本語\n");
        assert_eq!(encode(&t.text, t.line_ending, t.bom), "naïve — 日本語\n".as_bytes());
    }
}
