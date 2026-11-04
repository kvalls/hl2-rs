//! WAV, MS ADPCM and compressed audio decoded for playback.
use anyhow::{bail, Context, Result};
use std::io::{Cursor, ErrorKind};
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error as DecodeError,
    formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

pub(super) const MAX_ENCODED_AUDIO: usize = 32 * 1024 * 1024;
const MAX_PCM_AUDIO: usize = 128 * 1024 * 1024;
pub(super) const MS_ADPCM_COEFFICIENTS: [(i16, i16); 7] = [
    (256, 0),
    (512, -256),
    (0, 0),
    (192, 64),
    (240, 0),
    (460, -208),
    (392, -232),
];

#[derive(Clone, Copy)]
pub(super) struct MsAdpcmHeader {
    channels: u16,
    sample_rate: u32,
    pub(super) frames: u32,
}

fn wav_u16(data: &[u8], at: usize) -> Result<u16> {
    let end = at.checked_add(2).context("WAV field offset overflow")?;
    Ok(u16::from_le_bytes(
        data.get(at..end)
            .context("truncated WAV field")?
            .try_into()?,
    ))
}

fn wav_u32(data: &[u8], at: usize) -> Result<u32> {
    let end = at.checked_add(4).context("WAV field offset overflow")?;
    Ok(u32::from_le_bytes(
        data.get(at..end)
            .context("truncated WAV field")?
            .try_into()?,
    ))
}

/// Select only genuine Microsoft ADPCM and validate fields the decoder ignores.
/// Symphonia 0.5.5 derives block duration and uses the seven standard predictors,
/// so custom coefficient tables must be rejected rather than decoded incorrectly.
/// A Microsoft ADPCM format chunk, checked: (channels, rate, block size, frames per block).
fn adpcm_format(chunk: &[u8]) -> Result<(u16, u32, usize, usize)> {
    let channels = wav_u16(chunk, 2)?;
    let rate = wav_u32(chunk, 4)?;
    let block = usize::from(wav_u16(chunk, 12)?);
    let bits = wav_u16(chunk, 14)?;
    let extra = usize::from(wav_u16(chunk, 16)?);
    let frames = usize::from(wav_u16(chunk, 18)?);
    let coefficients = usize::from(wav_u16(chunk, 20)?);
    if !(1..=2).contains(&channels) || !(8000..=192000).contains(&rate) {
        bail!("unsupported Microsoft ADPCM channel count or sample rate");
    }
    if bits != 4 || extra != 32 || chunk.len() != 18 + extra || coefficients != 7 {
        bail!("unsupported Microsoft ADPCM format extension");
    }
    for (index, expected) in MS_ADPCM_COEFFICIENTS.iter().enumerate() {
        let offset = 22 + index * 4;
        let pair = (
            wav_u16(chunk, offset)? as i16,
            wav_u16(chunk, offset + 2)? as i16,
        );
        if pair != *expected {
            bail!("unsupported Microsoft ADPCM coefficient table");
        }
    }
    let channel_count = usize::from(channels);
    let payload = block
        .checked_sub(7 * channel_count)
        .context("Microsoft ADPCM block is smaller than its preamble")?;
    if frames != payload * 2 / channel_count + 2 {
        bail!("inconsistent Microsoft ADPCM samples per block");
    }
    Ok((channels, rate, block, frames))
}
pub(super) fn ms_adpcm_header(data: &[u8]) -> Result<Option<MsAdpcmHeader>> {
    if data.get(..4) != Some(b"RIFF") || data.get(8..12) != Some(b"WAVE") {
        return Ok(None);
    }
    let mut at = 12usize;
    let mut format = None;
    let mut fact = None;
    while at.checked_add(8).is_some_and(|end| end <= data.len()) {
        let id = &data[at..at + 4];
        let len = wav_u32(data, at + 4)? as usize;
        let start = at + 8;
        let end = start.checked_add(len).context("WAV chunk size overflow")?;
        let chunk = data.get(start..end).context("truncated WAV chunk")?;
        match id {
            b"fmt " => {
                if wav_u16(chunk, 0)? != 2 {
                    return Ok(None);
                }
                if format.is_some() {
                    bail!("duplicate Microsoft ADPCM format chunk");
                }
                format = Some(adpcm_format(chunk)?);
            }
            b"fact" => {
                if fact.is_some() {
                    bail!("duplicate Microsoft ADPCM fact chunk");
                }
                fact = Some(wav_u32(chunk, 0)?);
            }
            b"data" => {
                let Some((channels, sample_rate, block, per_block)) = format else {
                    return Ok(None);
                };
                let frames = fact.context("Microsoft ADPCM requires a fact chunk before data")?;
                if len == 0 || !len.is_multiple_of(block) {
                    bail!("incomplete Microsoft ADPCM data block");
                }
                let available = (len / block)
                    .checked_mul(per_block)
                    .context("audio size overflow")?;
                let count = (frames as usize)
                    .checked_mul(usize::from(channels))
                    .context("audio size overflow")?;
                if count > MAX_PCM_AUDIO / 2 {
                    bail!("decoded audio exceeds 128 MiB limit");
                }
                if frames == 0 || frames as usize > available {
                    bail!("Microsoft ADPCM fact count exceeds decoded data");
                }
                // Some owned Source files omit the final odd metadata pad byte.
                // The entire audio data chunk must exist; trailing metadata is
                // not interpreted as audio or repaired in the installed file.
                let riff_end = (wav_u32(data, 4)? as usize)
                    .checked_add(8)
                    .context("WAV container size overflow")?;
                if riff_end < end || riff_end > data.len().saturating_add(1) {
                    bail!("truncated Microsoft ADPCM RIFF container");
                }
                return Ok(Some(MsAdpcmHeader {
                    channels,
                    sample_rate,
                    frames,
                }));
            }
            _ => (),
        }
        at = end
            .checked_add(len & 1)
            .context("WAV chunk size overflow")?;
    }
    if format.is_some() {
        bail!("Microsoft ADPCM has no data chunk");
    }
    Ok(None)
}

#[derive(Clone, serde::Serialize)]
pub struct AudioSummary {
    pub codec: String,
    pub channels: u16,
    pub sample_rate: u32,
    pub frames: u32,
    pub seconds: f64,
}
impl AudioSummary {
    pub fn seconds(&self) -> f64 {
        self.seconds
    }
}

pub(super) fn validate_wav(data: &[u8]) -> Result<()> {
    let mut wav = hound::WavReader::new(Cursor::new(data))?;
    let spec = wav.spec();
    if !(1..=2).contains(&spec.channels) || spec.sample_rate == 0 {
        bail!("unsupported audio channel count or sample rate");
    }
    if spec.sample_format == hound::SampleFormat::Float {
        for sample in wav.samples::<f32>() {
            if !sample?.is_finite() {
                bail!("nonfinite audio sample");
            }
        }
    } else {
        for sample in wav.samples::<i32>() {
            sample?;
        }
    }
    Ok(())
}

// The playback backend accepts PCM WAV bytes. Decode compressed audio in memory,
// retaining channel order/rate; no converted game files are written.
fn compressed_to_wav(
    data: Vec<u8>,
    extension: &str,
    expected: Option<MsAdpcmHeader>,
) -> Result<Vec<u8>> {
    let stream = MediaSourceStream::new(Box::new(Cursor::new(data)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(extension);
    let options = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };
    let mut format = symphonia::default::get_probe()
        .format(&hint, stream, &options, &MetadataOptions::default())?
        .format;
    let track = format
        .default_track()
        .context("compressed audio has no audio track")?;
    let track_id = track.id;
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let mut spec = None;
    let mut pcm = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(DecodeError::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(error.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder.decode(&packet)?;
        let current = *decoded.spec();
        let channels = current.channels.count();
        if !(1..=2).contains(&channels) || !(8000..=192000).contains(&current.rate) {
            bail!("unsupported compressed audio channel count or sample rate");
        }
        if expected.is_some_and(|header| {
            usize::from(header.channels) != channels || header.sample_rate != current.rate
        }) {
            bail!("Microsoft ADPCM decoded format differs from its header");
        }
        if spec.is_some_and(|previous| previous != current) {
            bail!("compressed audio changes channel layout or sample rate");
        }
        spec = Some(current);
        let count = decoded
            .frames()
            .checked_mul(channels)
            .context("audio size overflow")?;
        let total = pcm
            .len()
            .checked_add(count)
            .context("audio size overflow")?;
        if total > MAX_PCM_AUDIO / 2 {
            bail!("decoded audio exceeds 128 MiB limit");
        }
        let mut samples = SampleBuffer::<i16>::new(decoded.capacity() as u64, current);
        samples.copy_interleaved_ref(decoded);
        pcm.extend_from_slice(samples.samples());
    }
    if pcm.is_empty() {
        bail!("compressed audio has no decoded audio samples");
    }
    if let Some(header) = expected {
        let samples = (header.frames as usize)
            .checked_mul(usize::from(header.channels))
            .context("audio size overflow")?;
        if pcm.len() < samples {
            bail!("Microsoft ADPCM decoded fewer samples than its fact count");
        }
        // Compressed blocks include padding beyond the authored sample count.
        pcm.truncate(samples);
    }
    let spec = spec.context("compressed audio has no decoded audio")?;
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(
        &mut cursor,
        hound::WavSpec {
            channels: spec.channels.count() as u16,
            sample_rate: spec.rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for sample in pcm {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    Ok(cursor.into_inner())
}

pub(super) fn playback_bytes(path: &str, data: Vec<u8>) -> Result<(Vec<u8>, &'static str)> {
    if data.len() > MAX_ENCODED_AUDIO {
        bail!("encoded audio exceeds 32 MiB limit");
    }
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "wav" => {
            if let Some(header) = ms_adpcm_header(&data)? {
                Ok((compressed_to_wav(data, "wav", Some(header))?, "ms-adpcm"))
            } else {
                validate_wav(&data)?;
                Ok((data, "wav"))
            }
        }
        "mp3" => Ok((compressed_to_wav(data, "mp3", None)?, "mp3")),
        _ => bail!("only WAV and MP3 audio are supported currently"),
    }
}
