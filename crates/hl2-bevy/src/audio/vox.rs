//! VOX sentences (the HEV suit's): preloaded word samples concatenated on request.
use super::reuse;
use super::wav::{BACKEND_RATE, resample};
use anyhow::Result;
use source_assets::vpk::Vfs;
use std::collections::{BTreeMap, HashMap};

/// VOX sentences the game can request (the HEV suit's), with their words' samples
/// preloaded as mono backend-rate PCM. Playback concatenates the words.
#[derive(Default)]
pub(super) struct Vox {
    pub(super) sentences: BTreeMap<String, Vec<source_assets::vox::Word>>,
    pub(super) groups: BTreeMap<String, Vec<String>>,
    pub(super) words: HashMap<String, std::sync::Arc<[f32]>>,
    pub(super) errors: BTreeMap<String, String>,
    pub(super) played: Vec<String>,
    pub(super) rng: u64,
}
impl Vox {
    pub(super) fn load(
        vfs: &Vfs,
        reuse: &reuse::AudioReuse,
        cancel: &crate::assets::work::Cancellation,
    ) -> Result<Self> {
        let mut vox = Self {
            rng: 0x6a09_e667_f3bc_c909,
            ..Self::default()
        };
        let text = match vfs.read("scripts/sentences.txt") {
            Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
            other => {
                vox.errors
                    .insert("scripts/sentences.txt".into(), format!("{:?}", other.err()));
                return Ok(vox);
            }
        };
        for (name, line) in source_assets::vox::sentences(&text) {
            // The suit's sentences and the metrocops' and combine soldiers' (CAI_Sentence
            // groups) are requested.
            if !name.starts_with("HEV_")
                && !name.starts_with("METROPOLICE_")
                && !name.starts_with("COMBINE_")
            {
                continue;
            }
            let words = source_assets::vox::words(&line);
            for word in &words {
                if vox.words.contains_key(&word.wave) || vox.errors.contains_key(&word.wave) {
                    continue;
                }
                cancel.check()?;
                let decoded = reuse.mono(vfs, &word.wave, cancel);
                match decoded {
                    Ok(samples) => {
                        vox.words.insert(word.wave.clone(), samples);
                    }
                    Err(error) => {
                        vox.errors.insert(word.wave.clone(), format!("{error:#}"));
                    }
                }
            }
            vox.groups
                .entry(source_assets::vox::group(&name).to_owned())
                .or_default()
                .push(name.clone());
            vox.sentences.insert(name, words);
        }
        Ok(vox)
    }
    pub(super) fn random(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }
    /// `!NAME` (a sentence) or `#GROUP` (a random member) to mono PCM. The channel
    /// pitch follows UTIL_EmitSoundSuit (100, or 98-104 half of the time); a word's own
    /// pitch replaces it (inferred: the retail mixer's use of word pitch is unread).
    pub(super) fn render(&mut self, request: &str) -> Option<(String, Vec<f32>)> {
        let mut name = request.get(1..)?.to_ascii_uppercase();
        if request.starts_with('#') {
            let members = self.groups.get(&name)?.clone();
            let pick = (self.random() % members.len().max(1) as u64) as usize;
            name = members.get(pick)?.clone();
        }
        let channel_pitch = if self.random() % 2 == 1 {
            98 + (self.random() % 7) as i32
        } else {
            100
        };
        let pcm = self.render_at(&name, channel_pitch)?;
        self.played.push(name.clone());
        Some((format!("!{name}"), pcm))
    }
    pub(super) fn render_at(&mut self, name: &str, channel_pitch: i32) -> Option<Vec<f32>> {
        let words = self.sentences.get(name)?.clone();
        let mut pcm = Vec::new();
        for word in words {
            let Some(samples) = self.words.get(&word.wave) else {
                continue;
            };
            let length = samples.len();
            let percent = |p: i32| (length as i64 * i64::from(p.clamp(0, 100)) / 100) as usize;
            let (start, end) = (percent(word.params.start), percent(word.params.end));
            if start >= end {
                continue;
            }
            if word.params.time != 0 {
                self.errors
                    .entry(format!("{name}: time compression"))
                    .or_insert_with(|| "not implemented; played uncompressed".into());
            }
            let pitch = word.params.pitch.unwrap_or(channel_pitch).clamp(1, 255) as u32;
            let gain = word.params.volume.clamp(0, 255) as f32 / 100.;
            let rate = BACKEND_RATE * pitch / 100;
            pcm.extend(
                resample(&samples[start..end], 1, rate, BACKEND_RATE)
                    .into_iter()
                    .map(|v| v * gain),
            );
        }
        Some(pcm)
    }
}
