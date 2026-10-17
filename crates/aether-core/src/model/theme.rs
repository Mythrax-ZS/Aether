//! Colour themes. Built-in themes live in the UI; user themes are JSON files in the
//! `themes/` folder and share this schema (documented in `docs/THEMING.md`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{CoreError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum ThemeKind {
    Dark,
    Light,
}

/// The 16 ANSI colours plus the surface colours of a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TerminalPalette {
    pub background: String,
    pub foreground: String,
    pub cursor: String,
    pub cursor_accent: String,
    pub selection_background: String,
    pub selection_foreground: Option<String>,
    pub black: String,
    pub red: String,
    pub green: String,
    pub yellow: String,
    pub blue: String,
    pub magenta: String,
    pub cyan: String,
    pub white: String,
    pub bright_black: String,
    pub bright_red: String,
    pub bright_green: String,
    pub bright_yellow: String,
    pub bright_blue: String,
    pub bright_magenta: String,
    pub bright_cyan: String,
    pub bright_white: String,
}

impl TerminalPalette {
    fn all_colors(&self) -> [(&'static str, &str); 21] {
        [
            ("background", &self.background),
            ("foreground", &self.foreground),
            ("cursor", &self.cursor),
            ("cursorAccent", &self.cursor_accent),
            ("selectionBackground", &self.selection_background),
            ("black", &self.black),
            ("red", &self.red),
            ("green", &self.green),
            ("yellow", &self.yellow),
            ("blue", &self.blue),
            ("magenta", &self.magenta),
            ("cyan", &self.cyan),
            ("white", &self.white),
            ("brightBlack", &self.bright_black),
            ("brightRed", &self.bright_red),
            ("brightGreen", &self.bright_green),
            ("brightYellow", &self.bright_yellow),
            ("brightBlue", &self.bright_blue),
            ("brightMagenta", &self.bright_magenta),
            ("brightCyan", &self.bright_cyan),
            ("brightWhite", &self.bright_white),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub author: Option<String>,
    pub kind: ThemeKind,
    /// Overrides for the UI's CSS custom properties, keyed without the leading `--`
    /// (e.g. `"bg-surface": "#12141c"`).
    pub ui: BTreeMap<String, String>,
    pub terminal: TerminalPalette,
    /// `true` for themes that ship with Aether (not stored on disk, not editable).
    #[serde(default)]
    pub builtin: bool,
}

/// Accepts `#rgb`, `#rrggbb`, `#rrggbbaa`, and CSS `rgb()/rgba()/hsl()/hsla()` forms.
fn is_css_color(value: &str) -> bool {
    let v = value.trim();
    if let Some(hex) = v.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    let lower = v.to_ascii_lowercase();
    ["rgb(", "rgba(", "hsl(", "hsla("].iter().any(|p| lower.starts_with(p))
        && lower.ends_with(')')
        && !lower.contains(';')
}

impl Theme {
    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty() || !self.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(CoreError::invalid("theme id", "use only letters, numbers, dashes and underscores"));
        }
        if self.name.trim().is_empty() {
            return Err(CoreError::invalid("theme name", "give the theme a name"));
        }
        for (name, value) in self.terminal.all_colors() {
            if !is_css_color(value) {
                return Err(CoreError::invalid(
                    format!("terminal colour `{name}`"),
                    format!("`{value}` is not a valid colour"),
                ));
            }
        }
        for (key, value) in &self.ui {
            if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                return Err(CoreError::invalid("theme variable", format!("`{key}` is not a valid variable name")));
            }
            // These values are injected into a stylesheet, so keep them inert.
            if value.contains([';', '{', '}', '<', '>']) || value.to_ascii_lowercase().contains("url(") {
                return Err(CoreError::invalid(
                    format!("theme variable `{key}`"),
                    "values may not contain ; { } < > or url()",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Theme {
        let c = |s: &str| s.to_string();
        Theme {
            id: "test-theme".into(),
            name: "Test".into(),
            author: None,
            kind: ThemeKind::Dark,
            ui: BTreeMap::new(),
            terminal: TerminalPalette {
                background: c("#000000"),
                foreground: c("#ffffff"),
                cursor: c("#ffffff"),
                cursor_accent: c("#000"),
                selection_background: c("rgba(255, 255, 255, 0.3)"),
                selection_foreground: None,
                black: c("#000"),
                red: c("#f00"),
                green: c("#0f0"),
                yellow: c("#ff0"),
                blue: c("#00f"),
                magenta: c("#f0f"),
                cyan: c("#0ff"),
                white: c("#fff"),
                bright_black: c("#111"),
                bright_red: c("#f11"),
                bright_green: c("#1f1"),
                bright_yellow: c("#ff1"),
                bright_blue: c("#11f"),
                bright_magenta: c("#f1f"),
                bright_cyan: c("#1ff"),
                bright_white: c("#eee"),
            },
            builtin: false,
        }
    }

    #[test]
    fn valid_theme_passes() {
        assert!(sample().validate().is_ok());
    }

    #[test]
    fn rejects_bad_colours_and_css_injection() {
        let mut t = sample();
        t.terminal.red = "not-a-colour".into();
        assert!(t.validate().is_err());

        let mut t = sample();
        t.ui.insert("bg".into(), "#fff; } body { display:none".into());
        assert!(t.validate().is_err());

        let mut t = sample();
        t.ui.insert("bg".into(), "url(https://evil.example/x.png)".into());
        assert!(t.validate().is_err());
    }

    #[test]
    fn rejects_unsafe_ids() {
        let mut t = sample();
        t.id = "../escape".into();
        assert!(t.validate().is_err());
    }
}
