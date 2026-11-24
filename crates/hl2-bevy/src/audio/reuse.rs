//! Cache only immutable decoded content, never entity IDs, cue RNG, soundscape playback or sinks.
use super::{BACKEND_RATE, backend_wav_from_pcm, distance_variant_mixes_from_pcm, resample};
use crate::assets::{reuse::Reuse, work::Cancellation};
use anyhow::{Context, Result};
use source_assets::vpk::Vfs;
use std::sync::Arc;

pub(super) struct Decoded {
    pub waves: Vec<(String, Arc<[u8]>)>,
    pub summary: hl2_simulation::sounds::AudioSummary,
    pub sentence: Option<source_assets::sentence::Sentence>,
}
struct Mono {
    samples: Arc<[f32]>,
}
pub(crate) struct AudioReuse {
    waves: Reuse<Decoded>,
    mono: Reuse<Mono>,
}
impl Default for AudioReuse {
    fn default() -> Self {
        Self {
            waves: Reuse::new(192 * 1024 * 1024),
            mono: Reuse::new(256 * 1024 * 1024),
        }
    }
}
impl AudioReuse {
    pub(super) fn wave(
        &self,
        vfs: &Vfs,
        path: &str,
        looped: bool,
        distance: bool,
        cancel: &Cancellation,
    ) -> Result<Arc<Decoded>> {
        cancel.check()?;
        let encoded = vfs
            .read(&format!("sound/{path}"))?
            .context("owned sound absent")?;
        let mode = u8::from(looped) | (u8::from(distance) << 1);
        if let Some(value) = self.waves.get(path, mode, &encoded) {
            return Ok(value);
        }
        let sentence = match source_assets::sentence::vdat_chunk(&encoded) {
            Ok(Some(chunk)) => Some(source_assets::sentence::Sentence::parse(chunk)?),
            _ => None,
        };
        let identity: Arc<[u8]> = encoded.clone().into();
        let (channels, rate, samples, summary) = hl2_simulation::sounds::decode_pcm(path, encoded)?;
        let mixes = if distance {
            distance_variant_mixes_from_pcm(channels, rate, &samples)?
        } else {
            None
        };
        let raw_pcm_wav = (summary.codec == "wav").then_some(identity.as_ref());
        let waves = match mixes {
            Some(waves) => waves,
            None => vec![(
                path.to_owned(),
                backend_wav_from_pcm(channels, rate, &samples, raw_pcm_wav, looped)?,
            )],
        };
        let size = waves.iter().map(|(_, bytes)| bytes.len()).sum();
        let value = Arc::new(Decoded {
            waves: waves
                .into_iter()
                .map(|(key, bytes)| (key.replace("{path}", path), bytes.into()))
                .collect(),
            summary,
            sentence,
        });
        cancel.check()?;
        self.waves.insert(path, mode, identity, value.clone(), size);
        Ok(value)
    }
    pub(super) fn mono(&self, vfs: &Vfs, path: &str, cancel: &Cancellation) -> Result<Arc<[f32]>> {
        cancel.check()?;
        let encoded = vfs
            .read(&format!("sound/{path}"))?
            .context("owned sentence word absent")?;
        if let Some(value) = self.mono.get(path, 0, &encoded) {
            return Ok(value.samples.clone());
        }
        let identity: Arc<[u8]> = encoded.clone().into();
        let (channels, rate, samples, _) = hl2_simulation::sounds::decode_pcm(path, encoded)?;
        let samples: Arc<[f32]> = if channels <= 1 {
            resample(&samples, 1, rate, BACKEND_RATE).into()
        } else {
            let channels = usize::from(channels);
            let mono: Vec<f32> = samples
                .chunks(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                .collect();
            resample(&mono, 1, rate, BACKEND_RATE).into()
        };
        cancel.check()?;
        self.mono.insert(
            path,
            0,
            identity,
            Arc::new(Mono {
                samples: samples.clone(),
            }),
            samples.len() * 4,
        );
        Ok(samples)
    }
    pub(crate) fn report(&self) -> serde_json::Value {
        serde_json::json!({"waves":self.waves.report(),"vox_words":self.mono.report()})
    }
}

#[cfg(test)]
mod tests {
    use super::AudioReuse;
    use crate::assets::work::Cancellation;
    use source_assets::vpk::Vfs;
    use std::{fs, sync::Arc};
    #[test]
    fn decoded_samples_reuse_across_maps_without_crossing_expansion_overrides() {
        let root = std::env::temp_dir().join(format!(
            "hl2-audio-reuse-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for game in ["hl2", "ep2"] {
            fs::create_dir_all(root.join(game).join("sound")).unwrap();
        }
        let base = super::super::wav_bytes(&[0.25; 128], 1).unwrap();
        let expansion = super::super::wav_bytes(&[-0.5; 128], 1).unwrap();
        fs::write(root.join("hl2/sound/shared.wav"), &base).unwrap();
        fs::write(root.join("ep2/sound/shared.wav"), &expansion).unwrap();
        let base_vfs = Vfs::mount_game(&root, "hl2").unwrap();
        let expansion_vfs = Vfs::mount_game(&root, "ep2").unwrap();
        let cache = AudioReuse::default();
        let cancel = Cancellation::default();
        let first = cache
            .wave(&base_vfs, "shared.wav", false, false, &cancel)
            .unwrap();
        let second = cache
            .wave(&base_vfs, "shared.wav", false, false, &cancel)
            .unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let other = cache
            .wave(&expansion_vfs, "shared.wav", false, false, &cancel)
            .unwrap();
        assert_ne!(first.waves[0].1, other.waves[0].1);
        let near = cache.mono(&base_vfs, "shared.wav", &cancel).unwrap();
        let again = cache.mono(&base_vfs, "shared.wav", &cancel).unwrap();
        assert!(Arc::ptr_eq(&near, &again));
        let override_word = cache.mono(&expansion_vfs, "shared.wav", &cancel).unwrap();
        assert_ne!(near, override_word);
        cancel.cancel();
        assert!(
            cache
                .wave(&base_vfs, "shared.wav", false, false, &cancel)
                .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ms_adpcm_at_backend_rate_does_not_bypass_pcm_conversion() {
        let root = std::env::temp_dir().join(format!(
            "hl2-audio-adpcm-reuse-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("hl2/sound")).unwrap();
        let coeffs: [(i16, i16); 7] = [
            (256, 0),
            (512, -256),
            (0, 0),
            (192, 64),
            (240, 0),
            (460, -208),
            (392, -232),
        ];
        let block = [0u8, 16, 0, 232, 3, 132, 3, 0x12, 0xf8, 0x00];
        let block_size = block.len() as u16;
        let per_block = (block_size - 7) * 2 + 2;
        let mut fmt = Vec::new();
        fmt.extend(2u16.to_le_bytes());
        fmt.extend(1u16.to_le_bytes());
        fmt.extend(48000u32.to_le_bytes());
        fmt.extend((48000u32 * u32::from(block_size) / u32::from(per_block)).to_le_bytes());
        fmt.extend(block_size.to_le_bytes());
        fmt.extend(4u16.to_le_bytes());
        fmt.extend(32u16.to_le_bytes());
        fmt.extend(per_block.to_le_bytes());
        fmt.extend(7u16.to_le_bytes());
        for (a, b) in coeffs {
            fmt.extend(a.to_le_bytes());
            fmt.extend(b.to_le_bytes());
        }
        let mut adpcm = b"RIFF\0\0\0\0WAVE".to_vec();
        for (id, chunk) in [
            (b"fmt ", fmt),
            (b"fact", 6u32.to_le_bytes().to_vec()),
            (b"data", block.to_vec()),
        ] {
            adpcm.extend(id);
            adpcm.extend((chunk.len() as u32).to_le_bytes());
            adpcm.extend(&chunk);
        }
        let riff_len = adpcm.len() as u32 - 8;
        adpcm[4..8].copy_from_slice(&riff_len.to_le_bytes());
        fs::write(root.join("hl2/sound/adpcm48k.wav"), &adpcm).unwrap();
        let vfs = Vfs::mount_game(&root, "hl2").unwrap();
        let decoded = AudioReuse::default()
            .wave(&vfs, "adpcm48k.wav", false, false, &Cancellation::default())
            .unwrap();
        assert_eq!(decoded.summary.codec, "ms-adpcm");
        assert_ne!(decoded.waves[0].1.as_ref(), adpcm.as_slice());
        let (channels, rate, samples) =
            hl2_simulation::sounds::wav_samples(&decoded.waves[0].1).unwrap();
        assert_eq!((channels, rate, samples.len()), (1, 48000, 6));
        fs::remove_dir_all(root).unwrap();
    }
}
