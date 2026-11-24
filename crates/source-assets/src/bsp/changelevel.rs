//! Fast entity-lump scanner for `trigger_changelevel` map targets.
use anyhow::{bail, Result};
use std::{borrow::Cow, collections::BTreeSet, path::Path};

/// Target map names (`"map"` values, lower-case) from a BSP's `trigger_changelevel` entities,
/// reading only the header and entity lump (lump 0) without allocating strings for other entities.
pub fn read_changelevel_targets(path: &Path) -> Result<BTreeSet<String>> {
    let raw = super::read_entity_lump(path)?;
    scan_changelevel_targets(&raw)
}

enum Token<'a> {
    BraceOpen,
    BraceClose,
    Text(Cow<'a, str>),
}

struct Scanner<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> Scanner<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, pos: 0 }
    }

    fn skip_ignored(&mut self) {
        let bytes = self.text.as_bytes();
        while self.pos < bytes.len() {
            let b = bytes[self.pos];
            if b.is_ascii_whitespace() {
                self.pos += 1;
                continue;
            }
            if b == b'/' && bytes.get(self.pos + 1) == Some(&b'/') {
                self.pos += 2;
                while self.pos < bytes.len() && bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
                if self.pos < bytes.len() {
                    self.pos += 1;
                }
                continue;
            }
            if b < 0x80 {
                break;
            }
            let ch = self.text[self.pos..].chars().next().unwrap();
            if ch.is_whitespace() || ch == '\u{feff}' {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
    }

    fn read_quoted(&mut self, capture: bool) -> Result<Cow<'a, str>> {
        let bytes = self.text.as_bytes();
        let start = self.pos;
        let mut has_escape = false;
        while self.pos < bytes.len() {
            match bytes[self.pos] {
                b'"' => {
                    let end = self.pos;
                    self.pos += 1;
                    if !capture {
                        return Ok(Cow::Borrowed(""));
                    }
                    if !has_escape {
                        return Ok(Cow::Borrowed(&self.text[start..end]));
                    }
                    return Ok(Cow::Owned(unescape_quoted(&self.text[start..end])));
                }
                b'\\' => {
                    self.pos += 1;
                    if self.pos < bytes.len() && matches!(bytes[self.pos], b'"' | b'\\') {
                        has_escape = true;
                        self.pos += 1;
                    }
                }
                b if b < 0x80 => self.pos += 1,
                _ => {
                    let ch = self.text[self.pos..].chars().next().unwrap();
                    self.pos += ch.len_utf8();
                }
            }
        }
        bail!("unterminated quoted KeyValues string");
    }

    fn next_token(&mut self, capture: bool) -> Result<Option<Token<'a>>> {
        self.skip_ignored();
        let bytes = self.text.as_bytes();
        if self.pos >= bytes.len() {
            return Ok(None);
        }
        match bytes[self.pos] {
            b'{' => {
                self.pos += 1;
                Ok(Some(Token::BraceOpen))
            }
            b'}' => {
                self.pos += 1;
                Ok(Some(Token::BraceClose))
            }
            b'"' => {
                self.pos += 1;
                Ok(Some(Token::Text(self.read_quoted(capture)?)))
            }
            _ => {
                let start = self.pos;
                while self.pos < bytes.len() {
                    let b = bytes[self.pos];
                    if b < 0x80 {
                        if b.is_ascii_whitespace() || b == b'{' || b == b'}' {
                            break;
                        }
                        self.pos += 1;
                    } else {
                        let ch = self.text[self.pos..].chars().next().unwrap();
                        if ch.is_whitespace() {
                            break;
                        }
                        self.pos += ch.len_utf8();
                    }
                }
                let slice = if capture {
                    &self.text[start..self.pos]
                } else {
                    ""
                };
                Ok(Some(Token::Text(Cow::Borrowed(slice))))
            }
        }
    }
}

fn unescape_quoted(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && matches!(chars.peek(), Some('"' | '\\')) {
            out.push(chars.next().unwrap());
        } else {
            out.push(c);
        }
    }
    out
}

pub(super) fn scan_changelevel_targets(data: &[u8]) -> Result<BTreeSet<String>> {
    let clean = std::str::from_utf8(data)?
        .trim_start_matches('\u{feff}')
        .trim_end_matches('\0');
    let mut scanner = Scanner::new(clean);
    let mut targets = BTreeSet::new();
    while let Some(tok) = scanner.next_token(false)? {
        if !matches!(tok, Token::BraceOpen) {
            bail!("expected entity opening brace");
        }
        let mut is_changelevel: Option<bool> = None;
        let mut map_value: Option<Cow<'_, str>> = None;
        loop {
            let Some(key_tok) = scanner.next_token(true)? else {
                bail!("unclosed entity");
            };
            let key = match key_tok {
                Token::BraceClose => break,
                Token::BraceOpen => bail!("invalid entity key/value"),
                Token::Text(s) => s,
            };
            let is_classname = is_changelevel.is_none() && key.eq_ignore_ascii_case("classname");
            let is_map = map_value.is_none() && key.eq_ignore_ascii_case("map");
            let Some(val_tok) = scanner.next_token(is_classname || is_map)? else {
                bail!("invalid entity key/value");
            };
            let val = match val_tok {
                Token::BraceOpen | Token::BraceClose => bail!("invalid entity key/value"),
                Token::Text(s) => s,
            };
            if is_classname {
                is_changelevel = Some(val.eq_ignore_ascii_case("trigger_changelevel"));
            } else if is_map {
                map_value = Some(val);
            }
        }
        if is_changelevel == Some(true) {
            if let Some(map) = map_value {
                let normalized = map.trim().to_ascii_lowercase();
                if !normalized.is_empty() {
                    targets.insert(normalized);
                }
            }
        }
    }
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference_targets(data: &[u8]) -> Result<BTreeSet<String>> {
        let entities = crate::keyvalues::entities(data)?;
        Ok(entities
            .iter()
            .filter(|e| e.class().eq_ignore_ascii_case("trigger_changelevel"))
            .filter_map(|e| e.get("map"))
            .map(|m| m.trim().to_ascii_lowercase())
            .filter(|m| !m.is_empty())
            .collect())
    }

    #[test]
    fn scans_changelevel_targets_matching_keyvalues_entities() {
        let lump = br#"
        {
            "classname" "worldspawn"
            "map" "should_ignore"
            "OnMapSpawn" "cmd,Command,\"quoted\",0,-1"
        }
        // comment with { braces }
        {
            "origin" "10 20 30"
            "ClassName" "Trigger_Changelevel"
            "map" " D1_Trainstation_03 "
            "map" "duplicate_key_ignored"
        }
        {
            "classname" "trigger_changelevel"
            "map" "   "
        }
        {
            "classname" "trigger_changelevel"
            "classname" "info_player_start"
            "map" "d1_trainstation_04"
        }
        "#;
        assert_eq!(
            scan_changelevel_targets(lump).unwrap(),
            reference_targets(lump).unwrap()
        );
        assert_eq!(
            scan_changelevel_targets(lump).unwrap(),
            ["d1_trainstation_03", "d1_trainstation_04"]
                .map(String::from)
                .into()
        );
    }

    #[test]
    fn rejects_malformed_entity_lumps_like_keyvalues() {
        assert!(scan_changelevel_targets(br#"{"x"}"#).is_err());
        assert!(scan_changelevel_targets(br#"{"classname" "unterminated"#).is_err());
        assert!(scan_changelevel_targets(br#""stray""#).is_err());
        assert!(scan_changelevel_targets(br#"{"classname" "trigger_changelevel""#).is_err());
    }

    #[test]
    fn reads_changelevel_targets_from_bsp_file() {
        let lump = br#"{"classname" "trigger_changelevel" "map" "d1_canals_01"}"#;
        let mut bsp = vec![0u8; 24 + lump.len()];
        bsp[..4].copy_from_slice(b"VBSP");
        bsp[4..8].copy_from_slice(&20u32.to_le_bytes());
        bsp[8..12].copy_from_slice(&24u32.to_le_bytes());
        bsp[12..16].copy_from_slice(&(lump.len() as u32).to_le_bytes());
        bsp[24..].copy_from_slice(lump);

        let path =
            std::env::temp_dir().join(format!("hl2-rs-changelevel-{}.bsp", std::process::id()));
        std::fs::write(&path, &bsp).unwrap();
        let targets = read_changelevel_targets(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(targets, ["d1_canals_01".to_owned()].into());
    }

    #[test]
    #[ignore = "requires an owned Half-Life 2 installation"]
    fn owned_install_bsps_match_keyvalues_entities_exactly() {
        let root = crate::install::discover().expect("owned install");
        let mut checked = 0usize;
        for game in crate::install::games(&root) {
            let folder = root.join(&game).join("maps");
            for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.extension().is_none_or(|e| e != "bsp") {
                    continue;
                }
                if entry.metadata().is_ok_and(|m| m.len() == 0) {
                    continue;
                }
                let expected = crate::bsp::read_entities(&path).map(|entities| {
                    entities
                        .iter()
                        .filter(|e| e.class().eq_ignore_ascii_case("trigger_changelevel"))
                        .filter_map(|e| e.get("map"))
                        .map(|m| m.trim().to_ascii_lowercase())
                        .filter(|m| !m.is_empty())
                        .collect::<BTreeSet<String>>()
                });
                let actual = read_changelevel_targets(&path);
                match (expected, actual) {
                    (Ok(exp), Ok(act)) => assert_eq!(act, exp, "{}", path.display()),
                    (Err(_), Err(_)) => {}
                    (exp, act) => panic!("{}: expected {exp:?}, got {act:?}", path.display()),
                }
                checked += 1;
            }
        }
        assert!(checked > 0);
    }
}
