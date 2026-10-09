//! scripts/sentences.txt and VOX sentence words, following the retail engine.dll parser
//! (build 19307283: tokenizer 100bb370, VOX_LoadSound 100bc9d0, word parameters
//! 100bb4b0). The directory is the text up to its last '/'; words split on " ,.({";
//! ',' and '.' (not at the end of the line) insert the `_comma` / `_period` words; a
//! word's own "(v p s e t)" applies to that word only, while a token that is only
//! "(...)" changes the defaults of the following words. Defaults: volume 100, channel
//! pitch, start 0, end 100, no time compression. "{...}" (Len, closecaption) is skipped.
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WordParams {
    /// Percent of the channel volume.
    pub volume: i32,
    /// Percent playback rate; None plays at the channel's pitch (the -1 default).
    pub pitch: Option<i32>,
    /// Start and end of the wave, percent of its length.
    pub start: i32,
    pub end: i32,
    /// Time compression percent (0 is none).
    pub time: i32,
}
impl Default for WordParams {
    fn default() -> Self {
        Self {
            volume: 100,
            pitch: None,
            start: 0,
            end: 100,
            time: 0,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word {
    /// Wave path below sound/, e.g. "hl1/fvox/boop.wav".
    pub wave: String,
    pub params: WordParams,
}

/// The tokenizer limit (CVOXWORDMAX 32 entries including the directory slot).
const MAX_WORDS: usize = 31;

/// Sentence text (after the name) to its words.
pub fn words(text: &str) -> Vec<Word> {
    // Captions/lengths are not words.
    let mut cleaned = String::with_capacity(text.len());
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => cleaned.push(c),
            _ => {}
        }
    }
    let text = cleaned.trim_end();
    let (directory, rest) = match text.rfind('/') {
        Some(slash) => (&text[..=slash], &text[slash + 1..]),
        None => ("vox/", text),
    };
    // Tokens: word text (with attached "(...)"), or the inserted punctuation words.
    let bytes = rest.as_bytes();
    let mut tokens: Vec<String> = Vec::new();
    let mut i = 0;
    let delimiter = |b: u8| matches!(b, b' ' | b',' | b'.' | b'(' | b'{');
    let skip = |b: u8| matches!(b, b' ' | b',' | b'.' | b'\t' | b'\r' | b'\n');
    while i < bytes.len() && skip(bytes[i]) {
        i += 1;
    }
    while i < bytes.len() && tokens.len() < MAX_WORDS {
        let start = i;
        while i < bytes.len() && !delimiter(bytes[i]) {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b'(' {
            while i < bytes.len() && bytes[i] != b')' {
                i += 1;
            }
            i = (i + 1).min(bytes.len());
        }
        tokens.push(rest[start..i].to_owned());
        if i < bytes.len()
            && matches!(bytes[i], b',' | b'.')
            && i + 1 < bytes.len()
            && !matches!(bytes[i + 1], b'\n' | b'\r')
            && tokens.len() < MAX_WORDS
        {
            tokens.push(
                if bytes[i] == b'.' {
                    "_period"
                } else {
                    "_comma"
                }
                .to_owned(),
            );
        }
        if i < bytes.len() {
            i += 1;
        }
        while i < bytes.len() && skip(bytes[i]) {
            i += 1;
        }
    }
    let mut defaults = WordParams::default();
    let mut result = Vec::new();
    for token in tokens {
        let mut params = defaults;
        let (name, list) = match token.find('(') {
            Some(open) if token.ends_with(')') => {
                (&token[..open], Some(&token[open + 1..token.len() - 1]))
            }
            _ => (token.as_str(), None),
        };
        if let Some(list) = list {
            let list = list.as_bytes();
            let mut j = 0;
            while j < list.len() {
                let key = list[j];
                if !matches!(key, b'v' | b'p' | b's' | b'e' | b't') {
                    j += 1;
                    continue;
                }
                let digits = list[j + 1..]
                    .iter()
                    .take_while(|b| b.is_ascii_digit())
                    .take(7)
                    .count();
                if digits == 0 {
                    break;
                }
                let value: i32 = std::str::from_utf8(&list[j + 1..j + 1 + digits])
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                match key {
                    b'v' => params.volume = value,
                    b'p' => params.pitch = Some(value),
                    b's' => params.start = value,
                    b'e' => params.end = value,
                    _ => params.time = value,
                }
                j += 1 + digits;
            }
            if name.is_empty() {
                defaults = params;
                continue;
            }
        }
        if name.is_empty() {
            continue;
        }
        result.push(Word {
            wave: format!("{directory}{name}.wav"),
            params,
        });
    }
    result
}

/// Every sentence of scripts/sentences.txt by upper-case name.
pub fn sentences(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with("//") {
                return None;
            }
            let (name, rest) = line.split_once(|c: char| c.is_whitespace())?;
            Some((name.to_ascii_uppercase(), rest.trim().to_owned()))
        })
        .collect()
}
/// A sentence's group: its name without trailing digits (HEV_DEAD0 -> HEV_DEAD).
pub fn group(name: &str) -> &str {
    name.trim_end_matches(|c: char| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(text: &str) -> Vec<(String, i32, Option<i32>)> {
        words(text)
            .into_iter()
            .map(|w| (w.wave, w.params.volume, w.params.pitch))
            .collect()
    }

    #[test]
    fn owned_hev_lines_follow_retail_parameter_scope() {
        let dmg = summary(
            "hl1/fvox/(p160) boop, boop, boop, (p100) minor_fracture {Len 3.67 closecaption HEV.minor_fracture}",
        );
        let w = |name: &str, pitch| (format!("hl1/fvox/{name}.wav"), 100, Some(pitch));
        assert_eq!(
            dmg,
            [
                w("boop", 160),
                w("_comma", 160),
                w("boop", 160),
                w("_comma", 160),
                w("boop", 160),
                w("_comma", 160),
                w("minor_fracture", 100),
            ]
        );
        // A word's own parameters do not change the following words.
        let heal = summary("hl1/fvox/(p140) boop, (p100) wound_sterilized, blip(p130 v50), hiss");
        assert_eq!(heal[3], ("hl1/fvox/_comma.wav".into(), 100, Some(100)));
        assert_eq!(heal[4], ("hl1/fvox/blip.wav".into(), 50, Some(130)));
        assert_eq!(heal[6], ("hl1/fvox/hiss.wav".into(), 100, Some(100)));
        // No pitch anywhere: the channel pitch.
        let dead = summary("hl1/fvox/beep beep, flatline {Len 6.29 closecaption HEV.Flatline}");
        assert_eq!(dead.len(), 4);
        assert!(dead.iter().all(|(_, v, p)| *v == 100 && p.is_none()));
        assert_eq!(dead[3].0, "hl1/fvox/flatline.wav");
        // A trailing period at the end of the line inserts nothing.
        assert_eq!(summary("vox/hello world.").len(), 2);
    }

    #[test]
    fn sentence_file_lines_and_groups() {
        let file =
            "// HEV Suit\n\nHEV_DEAD0 hl1/fvox/beep, flatline\nhev_dead1 hl1/fvox/flatline\n";
        let all = sentences(file);
        assert_eq!(all.len(), 2);
        assert_eq!(all["HEV_DEAD1"], "hl1/fvox/flatline");
        assert_eq!(group("HEV_DEAD1"), "HEV_DEAD");
        assert_eq!(group("HEV_AAx"), "HEV_AAx");
    }
}
