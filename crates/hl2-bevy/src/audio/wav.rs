//! 16-bit PCM WAV at the backend rate and Catmull-Rom sample-rate conversion.
use anyhow::{Context, Result};

/// rodio 0.22 keeps each player alive with mono 48 kHz silence spans of 512 samples. A
/// sound appended in another format has its first 512 samples resampled as if they were in
/// that filler format (the weapon-selection tick's attack played 2.2x fast and lost ~12 ms).
/// Sounds are therefore converted once at load to this rate; stereo one-shots also start
/// with one filler-length span of silence so the misread span is silent.
pub const BACKEND_RATE: u32 = 48_000;
pub const BACKEND_FILLER_SAMPLES: usize = 512;

/// Resample interleaved samples with Catmull-Rom interpolation (each channel separately).
pub fn resample(samples: &[f32], channels: usize, from: u32, to: u32) -> Vec<f32> {
    let frames = samples.len() / channels;
    if from == to || frames == 0 {
        return samples.to_vec();
    }
    let out_frames = (frames as u64 * u64::from(to)).div_ceil(u64::from(from)) as usize;
    let at = |frame: isize, channel: usize| -> f32 {
        samples[frame.clamp(0, frames as isize - 1) as usize * channels + channel]
    };
    let mut out = Vec::with_capacity(out_frames * channels);
    for i in 0..out_frames {
        let position = i as f64 * f64::from(from) / f64::from(to);
        let (base, t) = (
            position.floor() as isize,
            (position - position.floor()) as f32,
        );
        for channel in 0..channels {
            let [p0, p1, p2, p3] = [-1, 0, 1, 2].map(|d| at(base + d, channel));
            out.push(
                p1 + 0.5
                    * t
                    * (p2 - p0
                        + t * (2. * p0 - 5. * p1 + 4. * p2 - p3 + t * (3. * (p1 - p2) + p3 - p0))),
            );
        }
    }
    out
}

/// 16-bit PCM WAV at the backend rate; see [`BACKEND_RATE`].
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
