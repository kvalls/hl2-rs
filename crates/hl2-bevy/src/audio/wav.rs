//! 16-bit PCM WAV at the backend rate and Catmull-Rom sample-rate conversion.
use anyhow::{Context, Result};

/// rodio 0.22 keeps each player alive with mono 48 kHz silence spans of 512 samples. A
/// sound appended in another format has its first 512 samples resampled as if they were in
/// that filler format (the weapon-selection tick's attack played 2.2x fast and lost ~12 ms).
/// Sounds are therefore converted once at load to this rate; stereo one-shots also start
/// with one filler-length span of silence so the misread span is silent.
pub const BACKEND_RATE: u32 = 48_000;
pub const BACKEND_FILLER_SAMPLES: usize = 512;

#[inline]
fn catmull_rom(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    p1 + 0.5
        * t
        * (p2 - p0 + t * (2. * p0 - 5. * p1 + 4. * p2 - p3 + t * (3. * (p1 - p2) + p3 - p0)))
}

/// Resample interleaved samples with Catmull-Rom interpolation (each channel separately).
pub fn resample(samples: &[f32], channels: usize, from: u32, to: u32) -> Vec<f32> {
    let frames = samples.len() / channels;
    if from == to || frames == 0 {
        return samples.to_vec();
    }
    let out_frames = (frames as u64 * u64::from(to)).div_ceil(u64::from(from)) as usize;
    let max_frame = frames as isize - 1;
    let push_clamped = |out: &mut Vec<f32>, base: isize, t: f32| {
        let i0 = (base - 1).clamp(0, max_frame) as usize * channels;
        let i1 = base.clamp(0, max_frame) as usize * channels;
        let i2 = (base + 1).clamp(0, max_frame) as usize * channels;
        let i3 = (base + 2).clamp(0, max_frame) as usize * channels;
        for channel in 0..channels {
            out.push(catmull_rom(
                samples[i0 + channel],
                samples[i1 + channel],
                samples[i2 + channel],
                samples[i3 + channel],
                t,
            ));
        }
    };
    let mut out = Vec::with_capacity(out_frames * channels);
    let ratio = f64::from(from) / f64::from(to);
    let mut i = 0usize;
    while i < out_frames {
        let position = i as f64 * ratio;
        let floor = position.floor();
        if floor >= 1.0 {
            break;
        }
        push_clamped(&mut out, floor as isize, (position - floor) as f32);
        i += 1;
    }
    let interior_limit = frames.saturating_sub(2) as f64;
    while i < out_frames {
        let position = i as f64 * ratio;
        let floor = position.floor();
        if floor >= interior_limit {
            break;
        }
        let base = floor as usize;
        let t = (position - floor) as f32;
        let win = &samples[(base - 1) * channels..(base + 3) * channels];
        for channel in 0..channels {
            out.push(catmull_rom(
                win[channel],
                win[channels + channel],
                win[2 * channels + channel],
                win[3 * channels + channel],
                t,
            ));
        }
        i += 1;
    }
    while i < out_frames {
        let position = i as f64 * ratio;
        let floor = position.floor();
        push_clamped(&mut out, floor as isize, (position - floor) as f32);
        i += 1;
    }
    out
}

/// 16-bit PCM WAV at the backend rate; see [`BACKEND_RATE`].
#[cfg(test)]
pub fn backend_wav(data: Vec<u8>, looped: bool) -> Result<Vec<u8>> {
    let (channels, rate, samples) = hl2_simulation::sounds::wav_samples(&data)?;
    if rate == BACKEND_RATE && channels == 1 {
        return Ok(data);
    }
    let mut pcm = resample(&samples, usize::from(channels), rate, BACKEND_RATE);
    if channels != 1 && !looped {
        pcm.splice(0..0, std::iter::repeat_n(0., BACKEND_FILLER_SAMPLES));
    }
    wav_bytes(&pcm, channels)
}

/// 16-bit PCM WAV at the backend rate from decoded PCM samples; see [`BACKEND_RATE`].
pub fn backend_wav_from_pcm(
    channels: u16,
    rate: u32,
    samples: &[f32],
    raw_pcm_wav: Option<&[u8]>,
    looped: bool,
) -> Result<Vec<u8>> {
    if rate == BACKEND_RATE && channels == 1 {
        if let Some(raw) = raw_pcm_wav {
            return Ok(raw.to_vec());
        }
        return wav_bytes(samples, 1);
    }
    let mut pcm = resample(samples, usize::from(channels), rate, BACKEND_RATE);
    if channels != 1 && !looped {
        pcm.splice(0..0, std::iter::repeat_n(0., BACKEND_FILLER_SAMPLES));
    }
    wav_bytes(&pcm, channels)
}

/// 16-bit PCM WAV bytes at the backend rate.
/// Premixed mono versions of a distance-variant stereo wave (Source mixes left near and
/// right far by the channel's distmix while it plays). Keys use the `{path}` placeholder.
pub const DISTANCE_VARIANT_STEPS: usize = 16;

pub fn distance_variant_key(path: &str, distance: f32) -> String {
    let mix = hl2_simulation::sounds::distance_mix(distance);
    format!(
        "{path}#dv{}",
        (mix * DISTANCE_VARIANT_STEPS as f32).round() as usize
    )
}

/// Keyed backend WAV bytes.
pub type KeyedWaves = Vec<(String, Vec<u8>)>;

/// None for a mono wave (the mixer then plays it as it is).
#[cfg(test)]
pub fn distance_variant_mixes(data: &[u8]) -> Result<Option<KeyedWaves>> {
    let (channels, rate, samples) = hl2_simulation::sounds::wav_samples(data)?;
    if channels != 2 {
        return Ok(None);
    }
    (0..=DISTANCE_VARIANT_STEPS)
        .map(|step| {
            let mix = step as f32 / DISTANCE_VARIANT_STEPS as f32;
            let mono = hl2_simulation::sounds::distance_variant_mono(&samples, mix);
            Ok((
                format!("{{path}}#dv{step}"),
                wav_bytes(&resample(&mono, 1, rate, BACKEND_RATE), 1)?,
            ))
        })
        .collect::<Result<_>>()
        .map(Some)
}

/// None for a mono wave (the mixer then plays it as it is). Resamples stereo once.
pub fn distance_variant_mixes_from_pcm(
    channels: u16,
    rate: u32,
    samples: &[f32],
) -> Result<Option<KeyedWaves>> {
    if channels != 2 {
        return Ok(None);
    }
    let resampled = resample(samples, 2, rate, BACKEND_RATE);
    (0..=DISTANCE_VARIANT_STEPS)
        .map(|step| {
            let mix = step as f32 / DISTANCE_VARIANT_STEPS as f32;
            let mono = hl2_simulation::sounds::distance_variant_mono(&resampled, mix);
            Ok((format!("{{path}}#dv{step}"), wav_bytes(&mono, 1)?))
        })
        .collect::<Result<_>>()
        .map(Some)
}

pub fn wav_bytes(pcm: &[f32], channels: u16) -> Result<Vec<u8>> {
    let bytes = u32::try_from(pcm.len() * 2).context("converted audio exceeds WAV size")?;
    let mut wav = Vec::with_capacity(44 + pcm.len() * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&BACKEND_RATE.to_le_bytes());
    wav.extend_from_slice(&(BACKEND_RATE * u32::from(channels) * 2).to_le_bytes());
    wav.extend_from_slice(&(channels * 2).to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&bytes.to_le_bytes());
    for sample in pcm {
        wav.extend_from_slice(&((sample.clamp(-1., 1.) * 32767.).round() as i16).to_le_bytes());
    }
    Ok(wav)
}

#[cfg(test)]
mod tests {
    use super::{
        BACKEND_FILLER_SAMPLES, BACKEND_RATE, DISTANCE_VARIANT_STEPS, backend_wav_from_pcm,
        catmull_rom, distance_variant_mixes, distance_variant_mixes_from_pcm, resample, wav_bytes,
    };

    fn pcm16_wav(channels: u16, rate: u32, pcm: &[f32]) -> Vec<u8> {
        let data_bytes = (pcm.len() * 2) as u32;
        let mut wav = Vec::with_capacity(44 + pcm.len() * 2);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        wav.extend_from_slice(&(channels * 2).to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_bytes.to_le_bytes());
        for sample in pcm {
            wav.extend_from_slice(&((sample.clamp(-1., 1.) * 32767.).round() as i16).to_le_bytes());
        }
        wav
    }

    fn naive_resample(samples: &[f32], channels: usize, from: u32, to: u32) -> Vec<f32> {
        let frames = samples.len() / channels;
        if from == to || frames == 0 {
            return samples.to_vec();
        }
        let out_frames = (frames as u64 * u64::from(to)).div_ceil(u64::from(from)) as usize;
        let at = |frame: isize, channel: usize| -> f32 {
            samples[frame.clamp(0, frames as isize - 1) as usize * channels + channel]
        };
        let ratio = f64::from(from) / f64::from(to);
        let mut out = Vec::with_capacity(out_frames * channels);
        for i in 0..out_frames {
            let position = i as f64 * ratio;
            let floor = position.floor();
            let (base, t) = (floor as isize, (position - floor) as f32);
            for channel in 0..channels {
                let [p0, p1, p2, p3] = [-1, 0, 1, 2].map(|d| at(base + d, channel));
                out.push(catmull_rom(p0, p1, p2, p3, t));
            }
        }
        out
    }

    #[test]
    fn resample_matches_naive_across_boundary_and_interior_frames() {
        for channels in [1usize, 2] {
            for len in [1usize, 2, 3, 4, 5, 257] {
                let input: Vec<f32> = (0..len * channels)
                    .map(|i| ((i as f32 * 0.37).sin() * 0.8).clamp(-1., 1.))
                    .collect();
                for (from, to) in [(22050, BACKEND_RATE), (44100, BACKEND_RATE), (48000, 22050)] {
                    assert_eq!(
                        resample(&input, channels, from, to),
                        naive_resample(&input, channels, from, to)
                    );
                }
            }
        }
    }

    #[test]
    fn distance_variant_mixes_from_pcm_matches_per_mix_resampling() {
        let frames = 2048usize;
        let stereo: Vec<f32> = (0..frames)
            .flat_map(|i| {
                let near = (i as f32 * std::f32::consts::TAU * 440. / 22050.).sin() * 0.6;
                let far = (i as f32 * std::f32::consts::TAU * 1100. / 22050.).sin() * 0.4;
                [near, far]
            })
            .collect();
        let wav = pcm16_wav(2, 22050, &stereo);
        let (_, _, decoded_samples) = hl2_simulation::sounds::wav_samples(&wav).unwrap();
        let expected = distance_variant_mixes(&wav).unwrap().unwrap();
        let actual = distance_variant_mixes_from_pcm(2, 22050, &decoded_samples)
            .unwrap()
            .unwrap();
        assert_eq!(expected.len(), DISTANCE_VARIANT_STEPS + 1);
        assert_eq!(actual.len(), expected.len());
        for ((expected_key, expected_wav), (actual_key, actual_wav)) in
            expected.iter().zip(actual.iter())
        {
            assert_eq!(actual_key, expected_key);
            let (ec, er, es) = hl2_simulation::sounds::wav_samples(expected_wav).unwrap();
            let (ac, ar, as_) = hl2_simulation::sounds::wav_samples(actual_wav).unwrap();
            assert_eq!((ac, ar, as_.len()), (ec, er, es.len()));
            for (index, (a, e)) in as_.iter().zip(es.iter()).enumerate() {
                assert!(
                    (a - e).abs() <= 1e-4,
                    "{actual_key} sample {index}: {a} vs {e}"
                );
            }
        }
        assert!(
            distance_variant_mixes_from_pcm(1, 22050, &stereo[..frames])
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn backend_wav_from_pcm_uses_raw_fast_path_only_when_provided() {
        let mono_48k = vec![0.25f32; 64];
        let raw = b"RAW-48K-MONO".to_vec();
        assert_eq!(
            backend_wav_from_pcm(1, BACKEND_RATE, &mono_48k, Some(&raw), false).unwrap(),
            raw
        );
        assert_eq!(
            backend_wav_from_pcm(1, BACKEND_RATE, &mono_48k, None, false).unwrap(),
            wav_bytes(&mono_48k, 1).unwrap()
        );
        let stereo = pcm16_wav(2, 44100, &[0.5f32; 2 * 441]);
        let (_, _, stereo_samples) = hl2_simulation::sounds::wav_samples(&stereo).unwrap();
        let one_shot =
            backend_wav_from_pcm(2, 44100, &stereo_samples, Some(&stereo), false).unwrap();
        let (_, _, decoded) = hl2_simulation::sounds::wav_samples(&one_shot).unwrap();
        assert_eq!(decoded.len(), BACKEND_FILLER_SAMPLES + 2 * 480);
        assert!(decoded[..BACKEND_FILLER_SAMPLES].iter().all(|&s| s == 0.));
    }
}
