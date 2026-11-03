//! Localized strings from installed Source localization files (`*_english.txt`,
//! `merged_loc_all.txt`).
//!
//! Source merges every localization file it loads into one token table; the first file to define
//! a token wins (the search path order). Lookups are case-insensitive and accept an optional
//! leading `#`, matching how `.res` files and code reference tokens.
//!
//! Fallback policy (owner requirement): an unresolved token renders as its **key name** (without
//! the `#`), never as empty text, so missing translations stay visible and diagnosable.
//!
//! An app can supply text for tokens no loaded file defines ([`Fallback`]); it is checked after
//! the loaded files and before the key name.

use source_assets::keyvalues::{self, Entry};
use std::collections::HashMap;
use std::marker::PhantomData;

/// Text an app supplies for tokens that no loaded localization file defines.
pub trait Fallback {
    /// The text of `key` (without `#`, any case), if the app has one.
    fn lookup(key: &str) -> Option<&'static str>;
}

/// No app text: unresolved tokens render as their key name.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoFallback;

impl Fallback for NoFallback {
    fn lookup(_key: &str) -> Option<&'static str> {
        None
    }
}

/// Merged, case-insensitive localization table, with the app's [`Fallback`] text.
#[derive(Clone, Debug, Default)]
pub struct Localization<F = NoFallback> {
    tokens: HashMap<String, String>,
    /// Files that contributed tokens, in load order (diagnostics/reporting).
    sources: Vec<String>,
    fallback: PhantomData<F>,
}

impl<F: Fallback> Localization<F> {
    /// Adds every token of a localization resource. Existing tokens are kept (first file wins).
    ///
    /// Accepts both layouts found in the install:
    /// `"lang" { "Language" "English" "Tokens" { ... } }` (UTF-16 `*_english.txt`) and
    /// `"lang" { english { Tokens { ... } } }` (UTF-8 `hl2_complete/resource/merged_loc_all.txt`).
    /// Returns the number of new tokens, or `None` if the bytes are not a localization file.
    pub fn add_resource(&mut self, name: &str, bytes: &[u8]) -> Option<usize> {
        let roots = keyvalues::parse_resource(bytes).ok()?;
        let root = roots
            .into_iter()
            .find(|e| e.key.eq_ignore_ascii_case("lang"))?;
        let tokens = token_block(&root)?;
        let mut added = 0;
        for entry in tokens.children() {
            let Some(text) = entry.text() else { continue };
            if let std::collections::hash_map::Entry::Vacant(slot) =
                self.tokens.entry(entry.key.to_ascii_lowercase())
            {
                slot.insert(unescape(text));
                added += 1;
            }
        }
        self.sources.push(name.to_owned());
        Some(added)
    }

    /// Inserts a token if absent (tests and synthetic resources).
    pub fn insert(&mut self, token: &str, text: &str) {
        self.tokens
            .entry(token.trim_start_matches('#').to_ascii_lowercase())
            .or_insert_with(|| unescape(text));
    }

    /// The translated text of a token, if defined in loaded resources or the app's [`Fallback`].
    pub fn lookup(&self, token: &str) -> Option<&str> {
        let key = token.trim().trim_start_matches('#');
        if let Some(text) = self.tokens.get(&key.to_ascii_lowercase()) {
            return Some(text.as_str());
        }
        F::lookup(key)
    }

    /// The translated text of `token`, else the token's key name (never empty for a non-empty token).
    pub fn text(&self, token: &str) -> String {
        let key = token.trim().trim_start_matches('#');
        match self.lookup(key) {
            Some(text) if !text.is_empty() => text.to_owned(),
            _ => key.to_owned(),
        }
    }

    /// Resolves `#Token` references; other text (e.g. `"1280 x 720"`) is returned as-is.
    pub fn localize(&self, text: &str) -> String {
        if text.trim_start().starts_with('#') {
            self.text(text)
        } else {
            text.to_owned()
        }
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    pub fn sources(&self) -> &[String] {
        &self.sources
    }
}

/// Finds `Tokens` directly under `lang`, or under its first language block.
fn token_block(root: &Entry) -> Option<&Entry> {
    root.get("Tokens").or_else(|| {
        root.children()
            .iter()
            .filter(|e| !e.children().is_empty())
            .find_map(|language| language.get("Tokens"))
    })
}

/// Unescapes common Source localization escape sequences (`\n`, `\r`, `\t`, `\"`, `\\`),
/// leaving non-escape backslashes intact.
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('n') => {
                    chars.next();
                    out.push('\n');
                }
                Some('r') => {
                    chars.next();
                    out.push('\r');
                }
                Some('t') => {
                    chars.next();
                    out.push('\t');
                }
                Some('\\') => {
                    chars.next();
                    out.push('\\');
                }
                Some('"') => {
                    chars.next();
                    out.push('"');
                }
                _ => {
                    out.push('\\');
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xfe];
        for unit in text.encode_utf16() {
            bytes.extend(unit.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn unescape_handles_linebreaks_tabs_quotes_and_preserves_literal_backslashes() {
        assert_eq!(unescape("No escapes here"), "No escapes here");
        assert_eq!(unescape("Line 1\\nLine 2"), "Line 1\nLine 2");
        assert_eq!(unescape("Line 1\\r\\nLine 2"), "Line 1\r\nLine 2");
        assert_eq!(unescape("Col 1\\tCol 2"), "Col 1\tCol 2");
        assert_eq!(unescape("He said \\\"hi\\\""), "He said \"hi\"");
        assert_eq!(unescape("Path\\\\to\\\\file"), "Path\\to\\file");
        // Literal backslashes that are not escape sequences stay intact:
        assert_eq!(unescape("C:\\Games\\Half-Life"), "C:\\Games\\Half-Life");
        assert_eq!(unescape("Trailing backslash\\"), "Trailing backslash\\");
        assert_eq!(unescape("Single \\ in middle"), "Single \\ in middle");
    }

    #[test]
    fn localized_tokens_unescape_line_breaks() {
        let mut loc = Localization::<NoFallback>::default();
        loc.insert("GameUI_CustomWarning", "Warning line 1\\nWarning line 2");
        assert_eq!(
            loc.text("#GameUI_CustomWarning"),
            "Warning line 1\nWarning line 2"
        );
        let resource =
            b"\"lang\" { english { Tokens { \"GameUI_Multiline\" \"One\\nTwo\\nThree\" } } }";
        loc.add_resource("test.txt", resource);
        assert_eq!(loc.text("#GameUI_Multiline"), "One\nTwo\nThree");
    }

    #[test]
    fn utf16_gameui_layout_and_merged_layout_both_load() {
        let mut loc = Localization::<NoFallback>::default();
        let gameui = utf16(
            "\"lang\" { \"Language\" \"English\" \"Tokens\" { \"GameUI_Back\" \"Back\" \"GameUI_Subtitles\" \"Subtitles (dialog only)\" [$WIN32] \"GameUI_Subtitles\" \"Subtitles\" [$X360] } }",
        );
        assert_eq!(loc.add_resource("gameui_english.txt", &gameui), Some(2));
        let merged = b"\"lang\" { english { Tokens { \"HL2_GET_CROWBAR_NAME\" \"Trusty Hardware\" \"GameUI_Back\" \"Other\" } } }";
        assert_eq!(loc.add_resource("merged_loc_all.txt", merged), Some(1));
        assert_eq!(loc.text("#GameUI_Back"), "Back", "first file wins");
        assert_eq!(loc.text("gameui_subtitles"), "Subtitles (dialog only)");
        assert_eq!(loc.text("HL2_GET_CROWBAR_NAME"), "Trusty Hardware");
        assert_eq!(loc.sources().len(), 2);
    }

    #[test]
    fn unresolved_tokens_render_as_their_key_name_never_empty() {
        let mut loc = Localization::<NoFallback>::default();
        loc.insert("GameUI_Empty", "");
        assert_eq!(loc.text("#GameUI_NoWindowBorder"), "GameUI_NoWindowBorder");
        assert_eq!(loc.text("GameUI_NoWindowBorder"), "GameUI_NoWindowBorder");
        assert_eq!(
            loc.text("#GameUI_Empty"),
            "GameUI_Empty",
            "empty translations fall back too"
        );
        assert_eq!(loc.localize("#Missing_Token"), "Missing_Token");
        assert!(!loc.localize("#Missing_Token").is_empty());
        assert_eq!(loc.localize("1280 x 720"), "1280 x 720");
    }

    #[test]
    fn non_localization_files_are_rejected() {
        let mut loc = Localization::<NoFallback>::default();
        assert_eq!(loc.add_resource("x", b"\"GameMenu\" { }"), None);
        assert_eq!(loc.add_resource("x", b"{ broken"), None);
        assert!(loc.is_empty());
    }
}
