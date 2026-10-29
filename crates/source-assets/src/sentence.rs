//! Bounded speech phoneme data: the text VDAT chunk of owned WAV files (SDK CSentence
//! VERSION 1.0) and the flex settings (.vfe) that map phonemes to mouth controllers.
use crate::{bytes, f32le, i32le};
use anyhow::{bail, Context, Result};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Phoneme {
    pub code: u16,
    pub start: f32,
    pub end: f32,
}
/// Runtime sentence data: every word's phonemes in order plus emphasis samples.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sentence {
    pub phonemes: Vec<Phoneme>,
    pub emphasis: Vec<(f32, f32)>,
}
const MAX_VDAT: usize = 1 << 20;
const MAX_PHONEMES: usize = 1 << 14;
/// The VDAT chunk of a RIFF WAV, if present.
pub fn vdat_chunk(wav: &[u8]) -> Result<Option<&[u8]>> {
    if wav.len() < 12 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return Ok(None);
    }
    let mut at = 12usize;
    while at + 8 <= wav.len() {
        let id = &wav[at..at + 4];
        let size = usize::try_from(i32le(wav, at + 4)?).context("negative RIFF chunk size")?;
        let start = at + 8;
        let end = start.checked_add(size).context("RIFF chunk overflow")?;
        if id == b"VDAT" {
            if size > MAX_VDAT {
                bail!("VDAT chunk exceeds 1 MiB");
            }
            return Ok(Some(wav.get(start..end).context("VDAT outside WAV")?));
        }
        // Chunks are word aligned.
        at = end + (size & 1);
    }
    Ok(None)
}
/// CUtlBuffer text tokens: whitespace separated, with quoted strings.
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            out.push(chars.by_ref().take_while(|&c| c != '"').collect());
        } else {
            let mut token = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                token.push(c);
                chars.next();
            }
            out.push(token);
        }
    }
    out
}
fn number(token: Option<&String>) -> Result<f32> {
    let value: f32 = token.context("truncated VDAT")?.parse()?;
    if !value.is_finite() {
        bail!("non-finite VDAT number");
    }
    Ok(value)
}
impl Sentence {
    /// CSentence::InitFromBuffer for VERSION 1.0 text data (PLAINTEXT, WORDS, EMPHASIS,
    /// CLOSECAPTION and OPTIONS sections). Runtime phonemes are the word phonemes in order.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let text = String::from_utf8_lossy(data.split(|b| *b == 0).next().unwrap_or(data));
        let tokens = tokens(&text);
        let mut it = tokens.iter().peekable();
        if !it.next().is_some_and(|t| t.eq_ignore_ascii_case("VERSION")) {
            bail!("VDAT lacks VERSION");
        }
        if number(it.next())? != 1. {
            bail!("unsupported VDAT version");
        }
        let mut sentence = Self::default();
        while let Some(section) = it.next() {
            if section == "}" {
                break;
            }
            if !it.next().is_some_and(|t| t == "{") {
                break;
            }
            match section.to_ascii_uppercase().as_str() {
                "WORDS" => loop {
                    match it.next().map(|s| s.as_str()) {
                        Some("}") | None => break,
                        Some(w) if w.eq_ignore_ascii_case("WORD") => {}
                        Some(_) => bail!("malformed VDAT word"),
                    }
                    it.next(); // word text
                    number(it.next())?;
                    number(it.next())?;
                    if !it.next().is_some_and(|t| t == "{") {
                        bail!("malformed VDAT word phonemes");
                    }
                    loop {
                        let code = match it.next().map(|s| s.as_str()) {
                            Some("}") => break,
                            Some(code) => code.parse::<i64>()?,
                            None => bail!("truncated VDAT phonemes"),
                        };
                        it.next(); // phoneme name
                        let start = number(it.next())?;
                        let end = number(it.next())?;
                        number(it.next())?; // volume
                        sentence.phonemes.push(Phoneme {
                            code: u16::try_from(code).context("phoneme code range")?,
                            start,
                            end,
                        });
                        if sentence.phonemes.len() > MAX_PHONEMES {
                            bail!("VDAT phoneme budget exceeded");
                        }
                    }
                },
                "EMPHASIS" => {
                    while let Some(t) = it.next().filter(|t| *t != "}") {
                        let time = number(Some(t))?;
                        sentence.emphasis.push((time, number(it.next())?));
                        if sentence.emphasis.len() > MAX_PHONEMES {
                            bail!("VDAT emphasis budget exceeded");
                        }
                    }
                }
                // PLAINTEXT, CLOSECAPTION, OPTIONS and unknown sections are skipped.
                _ => {
                    let mut depth = 1;
                    for t in it.by_ref() {
                        if t == "{" {
                            depth += 1;
                        } else if t == "}" {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                    }
                }
            }
        }
        Ok(sentence)
    }
    /// CSentence::GetIntensity: Catmull-Rom over emphasis samples bounded by 0.5 at 0
    /// and at `length`; 0.5 without samples.
    pub fn intensity(&self, time: f32, length: f32) -> f32 {
        let n = self.emphasis.len() as isize;
        if n == 0 {
            return 0.5;
        }
        let sample = |i: isize| -> (f32, f32) {
            if i < 0 {
                (0., 0.5)
            } else if i >= n {
                (length, 0.5)
            } else {
                self.emphasis[i as usize]
            }
        };
        let mut i = -1;
        while i < n {
            let (s, e) = (sample(i), sample(i + 1));
            if time >= s.0 && time <= e.0 {
                break;
            }
            i += 1;
        }
        let (pre, start, end, next) = (
            sample((i - 1).max(-1)),
            sample(i.max(-1)),
            sample((i + 1).min(n)),
            sample((i + 2).min(n)),
        );
        let dt = (end.0 - start.0).clamp(0.01, 1.);
        let t = ((time - start.0) / dt).clamp(0., 1.);
        let (t2, t3) = (t * t, t * t * t);
        // Catmull_Rom_Spline (SDK mathlib) on the value component.
        let y = 0.5
            * (2. * start.1
                + (end.1 - pre.1) * t
                + (2. * pre.1 - 5. * start.1 + 4. * end.1 - next.1) * t2
                + (3. * start.1 - pre.1 - 3. * end.1 + next.1) * t3);
        y.clamp(0., 1.)
    }
}
/// A flex settings file (flexsettinghdr_t, "VFE"): settings indexed by phoneme code,
/// each a list of (controller name, weight).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlexSettings {
    index: Vec<i32>,
    settings: Vec<Vec<(String, f32)>>,
}
fn cstr(data: &[u8], at: usize) -> Result<String> {
    let tail = data.get(at..).context("VFE string outside file")?;
    let end = tail
        .iter()
        .take(256)
        .position(|b| *b == 0)
        .context("unterminated VFE string")?;
    Ok(String::from_utf8_lossy(&tail[..end]).into_owned())
}
fn count(data: &[u8], at: usize, limit: usize) -> Result<usize> {
    let n = usize::try_from(i32le(data, at)?)?;
    if n > limit {
        bail!("VFE table exceeds limit at {at}");
    }
    Ok(n)
}
impl FlexSettings {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if i32le(data, 0)? != (i32::from(b'V') << 16) + (i32::from(b'F') << 8) + i32::from(b'E') {
            bail!("not a VFE flex settings file");
        }
        let settings_n = count(data, 76, 4096)?;
        let settings_at = usize::try_from(i32le(data, 80)?)?;
        let index_n = count(data, 88, 1 << 16)?;
        let index_at = usize::try_from(i32le(data, 92)?)?;
        let keys_n = count(data, 96, 4096)?;
        let keys_at = usize::try_from(i32le(data, 100)?)?;
        bytes(data, settings_at, settings_n * 24)?;
        bytes(data, index_at, index_n * 4)?;
        bytes(data, keys_at, keys_n * 4)?;
        let keys = (0..keys_n)
            .map(|k| cstr(data, usize::try_from(i32le(data, keys_at + k * 4)?)?))
            .collect::<Result<Vec<_>>>()?;
        let index = (0..index_n)
            .map(|i| i32le(data, index_at + i * 4))
            .collect::<Result<Vec<_>>>()?;
        let mut settings = Vec::with_capacity(settings_n);
        for s in 0..settings_n {
            let at = settings_at + s * 24;
            let n = count(data, at + 8, 4096)?;
            let weights_at = usize::try_from(at as i64 + i64::from(i32le(data, at + 20)?))?;
            bytes(data, weights_at, n * 12)?;
            let mut weights = Vec::with_capacity(n);
            for w in 0..n {
                let key = usize::try_from(i32le(data, weights_at + w * 12)?)?;
                let name = keys.get(key).context("VFE key outside table")?;
                let weight = f32le(data, weights_at + w * 12 + 4)?;
                if !weight.is_finite() {
                    bail!("non-finite VFE weight");
                }
                weights.push((name.to_lowercase(), weight));
            }
            settings.push(weights);
        }
        if index.iter().any(|&i| i >= settings_n as i32 || i < -1) {
            bail!("VFE index outside settings");
        }
        Ok(Self { index, settings })
    }
    /// flexsettinghdr_t::pIndexedSetting for a phoneme code.
    pub fn indexed(&self, code: u16) -> Option<&[(String, f32)]> {
        let i = *self.index.get(usize::from(code))?;
        self.settings
            .get(usize::try_from(i).ok()?)
            .map(|s| s.as_slice())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const VDAT: &str = "VERSION 1.0\nPLAINTEXT\n{\nhey you\n}\nWORDS\n{\nWORD hey 0.100 0.400\n{\n104 h 0.100 0.200 1.000000\n101 e 0.200 0.400 1.000000\n}\nWORD you 0.450 0.700\n{\n106 y 0.450 0.700 1.000000\n}\n}\nEMPHASIS\n{\n0.300 0.900\n}\nOPTIONS\n{\nvoice_duck 1\n}\n";
    #[test]
    fn vdat_text_parses_runtime_phonemes_and_emphasis() {
        let mut wav = b"RIFF\0\0\0\0WAVEfmt \x02\0\0\0abVDAT".to_vec();
        wav.extend((VDAT.len() as i32).to_le_bytes());
        wav.extend(VDAT.as_bytes());
        let chunk = vdat_chunk(&wav).unwrap().unwrap();
        let s = Sentence::parse(chunk).unwrap();
        assert_eq!(
            s.phonemes,
            vec![
                Phoneme {
                    code: 104,
                    start: 0.1,
                    end: 0.2
                },
                Phoneme {
                    code: 101,
                    start: 0.2,
                    end: 0.4
                },
                Phoneme {
                    code: 106,
                    start: 0.45,
                    end: 0.7
                },
            ]
        );
        assert_eq!(s.emphasis, vec![(0.3, 0.9)]);
        // Bounded at 0.5 at both ends and passing through the authored sample.
        assert!((s.intensity(0., 1.) - 0.5).abs() < 1e-6);
        assert!((s.intensity(0.3, 1.) - 0.9).abs() < 1e-5);
        assert!((s.intensity(1., 1.) - 0.5).abs() < 1e-6);
        assert_eq!(Sentence::default().intensity(0.4, 1.), 0.5);
        assert!(vdat_chunk(b"RIFF\0\0\0\0WAVE").unwrap().is_none());
        assert!(Sentence::parse(b"VERSION 2.0").is_err());
    }
    #[test]
    #[ignore = "requires owned HL2 installation"]
    fn owned_phoneme_settings_parse() {
        let vfs = crate::vpk::Vfs::mount(&crate::install::discover().unwrap()).unwrap();
        for name in ["phonemes", "phonemes_weak", "phonemes_strong"] {
            let data = vfs
                .read(&format!("expressions/{name}.vfe"))
                .unwrap()
                .unwrap_or_else(|| panic!("{name}.vfe absent"));
            let settings = FlexSettings::parse(&data).unwrap();
            let indexed = (0..=u16::MAX)
                .filter(|&c| settings.indexed(c).is_some())
                .count();
            eprintln!("VFE {name}: {indexed} indexed phonemes");
            assert!(indexed > 0, "{name}");
            // Weak/strong classes may omit phonemes (SDK marks that class invalid).
            if name == "phonemes" {
                // 'aa' (0x251 in Source's phoneme table) opens the jaw.
                let open = settings.indexed(0x251).expect("aa setting");
                assert!(
                    open.iter().any(|(k, w)| k == "jaw_drop" && *w > 0.),
                    "{open:?}"
                );
            }
        }
    }
}
