//! Valve KeyValues tokenization. Preserves duplicate entity output keys.
use anyhow::{bail, Context, Result};
use modkit_core::Entity;

#[derive(Clone, Debug)]
pub enum Value {
    Text(String),
    Block(Vec<Entry>),
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub key: String,
    pub value: Value,
}
impl Entry {
    pub fn text(&self) -> Option<&str> {
        if let Value::Text(s) = &self.value {
            Some(s)
        } else {
            None
        }
    }
    pub fn children(&self) -> &[Entry] {
        if let Value::Block(v) = &self.value {
            v
        } else {
            &[]
        }
    }
    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.children()
            .iter()
            .find(|e| e.key.eq_ignore_ascii_case(name))
    }
}
/// Nested scripts retain duplicate keys, including all rndwave alternatives.
pub fn parse(text: &str) -> Result<Vec<Entry>> {
    fn block(t: &[String], i: &mut usize, depth: usize) -> Result<Vec<Entry>> {
        if depth > 64 {
            bail!("KeyValues nesting limit");
        }
        let mut out = Vec::new();
        while *i < t.len() && t[*i] != "}" {
            let key = t[*i].clone();
            *i += 1;
            if key == "{" || *i >= t.len() || t[*i] == "}" {
                bail!("invalid KeyValues entry");
            }
            let value = if t[*i] == "{" {
                *i += 1;
                let nested = block(t, i, depth + 1)?;
                if t.get(*i).map(String::as_str) != Some("}") {
                    bail!("unclosed KeyValues block");
                }
                *i += 1;
                Value::Block(nested)
            } else {
                let v = Value::Text(t[*i].clone());
                *i += 1;
                v
            };
            out.push(Entry { key, value });
        }
        Ok(out)
    }
    let t = tokens(text)?;
    let mut i = 0;
    let result = block(&t, &mut i, 0)?;
    if i != t.len() {
        bail!("unexpected KeyValues closing brace");
    }
    Ok(result)
}

/// Source scheme/localization resources may use UTF-16 with a byte-order mark.
/// Invalid encoding is reported rather than silently dropping an incomplete code unit.
pub fn decode_text(data: &[u8]) -> Result<String> {
    if data.is_empty() {
        return Ok(String::new());
    }
    // UTF-8 with BOM: strip the 3-byte marker
    let data = if data.starts_with(&[0xef, 0xbb, 0xbf]) {
        &data[3..]
    } else {
        data
    };
    // UTF-16 with BOM (little-endian 0xFF 0xFE or big-endian 0xFE 0xFF)
    if data.starts_with(&[0xff, 0xfe]) || data.starts_with(&[0xfe, 0xff]) {
        let (pairs, remainder) = data[2..].as_chunks::<2>();
        if !remainder.is_empty() {
            bail!("truncated UTF-16 resource code unit");
        }
        let little = data[0] == 0xff;
        let units = pairs
            .iter()
            .map(|p| {
                if little {
                    u16::from_le_bytes(*p)
                } else {
                    u16::from_be_bytes(*p)
                }
            })
            .collect::<Vec<_>>();
        let decoded = String::from_utf16(&units).context("invalid UTF-16 resource text")?;
        return Ok(decoded.trim_end_matches('\0').to_owned());
    }
    // Incomplete UTF-16 BOM marker is rejected as invalid encoding
    if data.len() == 1 && (data[0] == 0xff || data[0] == 0xfe) {
        bail!("incomplete UTF-16 byte order mark");
    }
    // UTF-16 LE without BOM heuristic: even length >= 4 and alternating ASCII zero high bytes
    if data.len() >= 4 && data.len() % 2 == 0 && data[1] == 0 && data[3] == 0 {
        let (pairs, _) = data.as_chunks::<2>();
        let units: Vec<u16> = pairs.iter().map(|p| u16::from_le_bytes(*p)).collect();
        if let Ok(decoded) = String::from_utf16(&units) {
            return Ok(decoded.trim_end_matches('\0').to_owned());
        }
    }
    // Standard UTF-8 decoding
    match std::str::from_utf8(data) {
        Ok(s) => Ok(s
            .trim_start_matches('\u{feff}')
            .trim_end_matches('\0')
            .to_owned()),
        Err(_) => {
            // Tolerant Windows-1252 / ISO-8859-1 decoding for Source resources with 8-bit text
            // (e.g. copyright \xa9, en-dash \x96, smart quotes \x92..\x94)
            let decoded: String = data
                .iter()
                .map(|&b| match b {
                    0x80 => '€',
                    0x82 => '‚',
                    0x83 => 'ƒ',
                    0x84 => '„',
                    0x85 => '…',
                    0x86 => '†',
                    0x87 => '‡',
                    0x88 => 'ˆ',
                    0x89 => '‰',
                    0x8a => 'Š',
                    0x8b => '‹',
                    0x8c => 'Œ',
                    0x8e => 'Ž',
                    0x91 => '‘',
                    0x92 => '’',
                    0x93 => '“',
                    0x94 => '”',
                    0x95 => '•',
                    0x96 => '–',
                    0x97 => '—',
                    0x98 => '˜',
                    0x99 => '™',
                    0x9a => 'š',
                    0x9b => '›',
                    0x9c => 'œ',
                    0x9e => 'ž',
                    0x9f => 'Ÿ',
                    _ => b as char,
                })
                .collect();
            Ok(decoded
                .trim_start_matches('\u{feff}')
                .trim_end_matches('\0')
                .to_owned())
        }
    }
}

/// Published KeyValues::EvaluateConditional platform semantics for a desktop host.
/// `$WIN32` historically means IsPC, whereas `$WINDOWS` tests Windows itself.
/// `$DECK`/`$X360` are false here; unknown conditions stay false even when negated.
/// Source checks named platforms in order rather than evaluating a boolean grammar.
pub fn resource_condition(token: &str) -> Result<bool> {
    let token = token.trim();
    let expression = token
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .context("invalid KeyValues resource condition")?;
    let negate = expression.starts_with('!');
    let upper = expression.to_ascii_uppercase();
    let value = if upper.contains("$DECK") || upper.contains("$X360") {
        false
    } else if upper.contains("$WIN32") {
        true
    } else if upper.contains("$WINDOWS") {
        cfg!(target_os = "windows")
    } else if upper.contains("$OSX") {
        cfg!(target_os = "macos")
    } else if upper.contains("$LINUX") {
        cfg!(target_os = "linux")
    } else if upper.contains("$POSIX") {
        cfg!(unix)
    } else {
        return Ok(false);
    };
    Ok(value != negate)
}

/// Decode a resource and evaluate conditions before blocks or after values/blocks.
/// Duplicate active keys are preserved, including numbered font resolution ranges.
/// `#include` and `#base` remain entries; resolving their paths belongs to the VFS caller.
pub fn parse_resource(data: &[u8]) -> Result<Vec<Entry>> {
    parse_resource_with_condition(data, resource_condition)
}

/// Resolve conditions for a specific resource consumer without changing host-wide policy.
pub fn parse_resource_with_condition(
    data: &[u8],
    condition: impl Fn(&str) -> Result<bool>,
) -> Result<Vec<Entry>> {
    fn block(
        t: &[Token],
        i: &mut usize,
        depth: usize,
        condition: &impl Fn(&str) -> Result<bool>,
    ) -> Result<Vec<Entry>> {
        if depth > 64 {
            bail!("KeyValues resource nesting limit");
        }
        let mut entries = Vec::new();
        while *i < t.len() && !t[*i].control("}") {
            let key = &t[*i];
            if key.control("{") || key.condition() {
                bail!("invalid KeyValues resource key");
            }
            let key = key.value.clone();
            *i += 1;
            let mut keep = true;
            while t.get(*i).is_some_and(Token::condition) {
                keep &= condition(&t[*i].value)?;
                *i += 1;
            }
            let token = t.get(*i).context("missing KeyValues resource value")?;
            let value = if token.control("{") {
                *i += 1;
                let children = block(t, i, depth + 1, condition)?;
                if !t.get(*i).is_some_and(|v| v.control("}")) {
                    bail!("unclosed KeyValues resource block");
                }
                *i += 1;
                Value::Block(children)
            } else if token.control("}") {
                break;
            } else {
                *i += 1;
                Value::Text(token.value.clone())
            };
            while t.get(*i).is_some_and(Token::condition) {
                keep &= condition(&t[*i].value)?;
                *i += 1;
            }
            if keep {
                entries.push(Entry { key, value });
            }
        }
        Ok(entries)
    }
    let text = decode_text(data)?;
    let t = lexical_tokens(&text)?;
    let mut i = 0;
    let entries = block(&t, &mut i, 0, &condition)?;
    if i != t.len() {
        bail!("unexpected KeyValues resource closing brace");
    }
    Ok(entries)
}

#[derive(Debug)]
struct Token {
    value: String,
    quoted: bool,
}
impl Token {
    fn control(&self, value: &str) -> bool {
        !self.quoted && self.value == value
    }
    fn condition(&self) -> bool {
        !self.quoted && self.value.starts_with('[')
    }
}

pub fn tokens(text: &str) -> Result<Vec<String>> {
    Ok(lexical_tokens(text)?.into_iter().map(|t| t.value).collect())
}
fn lexical_tokens(text: &str) -> Result<Vec<Token>> {
    let clean = text.trim_start_matches('\u{feff}').trim_end_matches('\0');
    let mut chars = clean.chars().peekable();
    let mut out = Vec::new();
    while let Some(c) = chars.next() {
        if c.is_whitespace() || c == '\u{feff}' {
            continue;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            for v in chars.by_ref() {
                if v == '\n' {
                    break;
                }
            }
            continue;
        }
        if c == '{' || c == '}' {
            out.push(Token {
                value: c.to_string(),
                quoted: false,
            });
            continue;
        }
        let mut s = String::new();
        if c == '"' {
            let mut closed = false;
            while let Some(v) = chars.next() {
                if v == '"' {
                    closed = true;
                    break;
                }
                if v == '\\' {
                    match chars.peek() {
                        Some('"') | Some('\\') => {
                            s.push(chars.next().unwrap());
                        }
                        _ => s.push(v),
                    }
                } else {
                    s.push(v);
                }
            }
            if !closed {
                bail!("unterminated quoted KeyValues string");
            }
        } else {
            s.push(c);
            while let Some(&v) = chars.peek() {
                if v.is_whitespace() || v == '{' || v == '}' {
                    break;
                }
                s.push(chars.next().unwrap());
            }
        }
        out.push(Token {
            value: s,
            quoted: c == '"',
        });
    }
    Ok(out)
}
pub fn entities(data: &[u8]) -> Result<Vec<Entity>> {
    let text = std::str::from_utf8(data)?.trim_end_matches('\0');
    let t = tokens(text)?;
    let mut i = 0;
    let mut out = Vec::new();
    while i < t.len() {
        if t[i] != "{" {
            bail!("expected entity opening brace");
        }
        i += 1;
        let mut p = Vec::new();
        while i < t.len() && t[i] != "}" {
            if i + 1 >= t.len() || t[i + 1] == "}" || t[i] == "{" {
                bail!("invalid entity key/value");
            }
            p.push((t[i].clone(), t[i + 1].clone()));
            i += 2;
        }
        if i >= t.len() {
            bail!("unclosed entity");
        }
        i += 1;
        out.push(Entity { properties: p });
    }
    Ok(out)
}
/// Base-texture lookup also accepts unquoted VMT keys. Patch includes are resolved by the VFS.
pub fn value(text: &str, key: &str) -> Result<Option<String>> {
    let t = tokens(text)?;
    Ok(t.windows(2)
        .find(|p| p[0].eq_ignore_ascii_case(key) && p[1] != "{" && p[1] != "}")
        .map(|p| p[1].clone()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn comments_duplicates_and_slashes() {
        let e=entities(b"{\"classname\" \"logic_relay\" // hi\n\"OnTrigger\" \"a\" \"OnTrigger\" \"b\" \"model\" \"models\\props\\foo.mdl\"}\0").unwrap();
        assert_eq!(e[0].properties.len(), 4);
        assert_eq!(e[0].get("model"), Some("models\\props\\foo.mdl"));
    }
    #[test]
    fn nested_sound_variants_are_retained() {
        let entries =
            parse("Hit { channel CHAN_WEAPON rndwave { wave a.wav wave b.wav } }").unwrap();
        let waves = entries[0].get("rndwave").unwrap().children();
        assert_eq!(waves.len(), 2);
        assert_eq!(waves[1].text(), Some("b.wav"));
        assert!(parse("Hit { wave a.wav").is_err());
    }
    #[test]
    fn malformed_is_error() {
        assert!(entities(b"{\"x\"}").is_err());
        assert!(tokens("\"unterminated").is_err());
    }

    #[test]
    fn consumer_conditions_do_not_change_host_resource_policy() {
        let bytes = br#"Scheme { Fonts { QuickInfo { tall 28 [!$OSX] tall 50 [$OSX] } }
            MacOnly [$OSX] { visible 1 } }"#;
        let original = parse_resource(bytes).unwrap();
        let desktop = parse_resource_with_condition(bytes, |condition| match condition {
            "[$OSX]" => Ok(false),
            "[!$OSX]" => Ok(true),
            other => resource_condition(other),
        })
        .unwrap();
        let height = |entries: &[Entry]| {
            entries[0]
                .get("Fonts")
                .unwrap()
                .get("QuickInfo")
                .unwrap()
                .get("tall")
                .unwrap()
                .text()
                .unwrap()
                .to_owned()
        };
        assert_eq!(height(&desktop), "28");
        assert_eq!(
            height(&original),
            if cfg!(target_os = "macos") {
                "50"
            } else {
                "28"
            }
        );
        let again = parse_resource(bytes).unwrap();
        assert_eq!(height(&again), height(&original));
        assert_eq!(again[0].get("MacOnly").is_some(), cfg!(target_os = "macos"));
    }

    #[test]
    fn resource_conditions_select_desktop_variants_and_preserve_duplicate_ranges() {
        let entries = parse_resource(
            br#"Scheme {
            Fonts {
                Numbers { "1" { name HalfLife2 tall 70 [$DECK] tall 64 [!$DECK] } }
                Text { "1" { tall 8 yres "1 599" } "2" { tall 10 yres "600 767" } }
            }
            Hud [$DECK] { wide 200 }
            Hud [!$DECK] { wide 112 }
            Color "255 220 0 255" [$WIN32]
            Color "255 255 255 255" [$X360]
        }"#,
        )
        .unwrap();
        let scheme = &entries[0];
        assert_eq!(
            scheme.get("Hud").unwrap().get("wide").and_then(Entry::text),
            Some("112")
        );
        assert_eq!(
            scheme.get("Color").and_then(Entry::text),
            Some("255 220 0 255")
        );
        let fonts = scheme.get("Fonts").unwrap();
        assert_eq!(
            fonts.get("Numbers").unwrap().children()[0]
                .get("tall")
                .and_then(Entry::text),
            Some("64")
        );
        assert_eq!(fonts.get("Text").unwrap().children().len(), 2);
        assert!(resource_condition("[$WIN32]").unwrap());
        assert!(!resource_condition("[!$UNKNOWN]").unwrap());
        assert_eq!(
            resource_condition("[$WINDOWS]").unwrap(),
            cfg!(target_os = "windows")
        );
        assert_eq!(resource_condition("[$POSIX]").unwrap(), cfg!(unix));
    }

    #[test]
    fn resources_keep_quoted_brackets_and_braces_as_text() {
        let entries = parse_resource(br#"Root { literal "[$DECK]" brace "}" "{" value }"#).unwrap();
        assert_eq!(
            entries[0].get("literal").and_then(Entry::text),
            Some("[$DECK]")
        );
        assert_eq!(entries[0].get("brace").and_then(Entry::text), Some("}"));
        assert_eq!(entries[0].get("{").and_then(Entry::text), Some("value"));
    }

    #[test]
    fn utf16_resources_and_utf8_boms_preserve_non_ascii_localization() {
        let text = "lang { Tokens { HL2_Example \"Prisión 🦀\" } }\0";
        for little in [true, false] {
            let mut data = if little {
                vec![0xff, 0xfe]
            } else {
                vec![0xfe, 0xff]
            };
            for value in text.encode_utf16() {
                data.extend(if little {
                    value.to_le_bytes()
                } else {
                    value.to_be_bytes()
                });
            }
            let entries = parse_resource(&data).unwrap();
            assert_eq!(
                entries[0]
                    .get("Tokens")
                    .unwrap()
                    .get("HL2_Example")
                    .and_then(Entry::text),
                Some("Prisión 🦀")
            );
        }
        assert_eq!(decode_text(b"\xef\xbb\xbfHL2\0").unwrap(), "HL2");
        assert!(decode_text(&[0xff, 0xfe, 0x61]).is_err());
        assert!(decode_text(&[0xff, 0xfe, 0, 0xd8]).is_err());
        assert!(decode_text(&[0xff]).is_err());
    }

    #[test]
    fn decode_text_and_parse_resource_handle_bom_utf16_and_windows1252_patterns() {
        // UTF-8 with BOM
        let utf8_bom = b"\xef\xbb\xbf\"GameInfo\" { \"title\" \"HALF-LIFE'\" \"title2\" \"== episode one==\" }";
        let res = parse_resource(utf8_bom).expect("parse UTF-8 with BOM");
        assert_eq!(
            res[0].get("title").and_then(Entry::text),
            Some("HALF-LIFE'")
        );
        assert_eq!(
            res[0].get("title2").and_then(Entry::text),
            Some("== episode one==")
        );

        // UTF-16 LE with BOM
        let utf16_bom = [
            0xff, 0xfe, // BOM
            b'"', 0, b'G', 0, b'a', 0, b'm', 0, b'e', 0, b'I', 0, b'n', 0, b'f', 0, b'o', 0, b'"',
            0, b' ', 0, b'{', 0, b' ', 0, b'"', 0, b't', 0, b'i', 0, b't', 0, b'l', 0, b'e', 0,
            b'"', 0, b' ', 0, b'"', 0, b'H', 0, b'A', 0, b'L', 0, b'F', 0, b'-', 0, b'L', 0, b'I',
            0, b'F', 0, b'E', 0, b'\'', 0, b'"', 0, b' ', 0, b'}', 0,
        ];
        let res = parse_resource(&utf16_bom).expect("parse UTF-16 with BOM");
        assert_eq!(
            res[0].get("title").and_then(Entry::text),
            Some("HALF-LIFE'")
        );

        // UTF-16 LE without BOM
        let utf16_nobom = [
            b'"', 0, b'G', 0, b'a', 0, b'm', 0, b'e', 0, b'I', 0, b'n', 0, b'f', 0, b'o', 0, b'"',
            0, b' ', 0, b'{', 0, b' ', 0, b'"', 0, b't', 0, b'i', 0, b't', 0, b'l', 0, b'e', 0,
            b'"', 0, b' ', 0, b'"', 0, b'H', 0, b'A', 0, b'L', 0, b'F', 0, b'-', 0, b'L', 0, b'I',
            0, b'F', 0, b'E', 0, b'\'', 0, b'"', 0, b' ', 0, b'}', 0,
        ];
        let res = parse_resource(&utf16_nobom).expect("parse UTF-16 without BOM");
        assert_eq!(
            res[0].get("title").and_then(Entry::text),
            Some("HALF-LIFE'")
        );

        // Windows-1252 with en-dash (\x96) and copyright (\xa9)
        let win1252 =
            b"\"GameInfo\" { \"title\" \"HALF-LIFE'\" \"title2\" \"\x96 episode two \x96\" }";
        let res = parse_resource(win1252).expect("parse Windows-1252 text");
        assert_eq!(
            res[0].get("title2").and_then(Entry::text),
            Some("– episode two –")
        );
    }

    #[test]
    fn malformed_resources_fail_even_in_excluded_platform_blocks() {
        for text in [
            "Root { key value",
            "Root { key value [$WIN32 }",
            "Root { } }",
        ] {
            assert!(parse_resource(text.as_bytes()).is_err(), "{text}");
        }
        let nested = "Root { ".repeat(66) + &"}".repeat(66);
        assert!(parse_resource(nested.as_bytes()).is_err());
    }

    #[test]
    #[ignore = "requires owned install"]
    fn parse_ep2_hudlayout() {
        let game = crate::install::discover().expect("game install");
        let vfs = crate::vpk::Vfs::mount_game(&game, "ep2").expect("vfs");
        let bytes = vfs
            .read("scripts/HudLayout.res")
            .expect("read")
            .expect("exists");
        let entries = parse_resource(&bytes).expect("parse ep2 HudLayout.res");
        assert!(!entries.is_empty());
    }

    #[test]
    fn parse_resource_tolerates_trailing_key_before_closing_brace() {
        let text = b"\"Root\" { \"entry\" { \"a\" \"1\" \"trailing_key\" } }";
        let res = parse_resource(text).expect("parses with trailing key");
        assert_eq!(
            res[0].get("entry").unwrap().get("a").unwrap().text(),
            Some("1")
        );
    }
}
