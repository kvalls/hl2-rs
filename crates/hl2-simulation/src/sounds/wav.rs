//! WAV, MS ADPCM and compressed audio decoded for playback.
use anyhow::{bail, Context, Result};
use std::io::{Cursor, ErrorKind};
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, conv::ConvertibleSample,
    errors::Error as DecodeError, formats::FormatOptions, io::MediaSourceStream,
    meta::MetadataOptions, probe::Hint, sample::Sample,
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
fn decode_compressed_samples<S>(
    data: Vec<u8>,
    extension: &str,
    expected: Option<MsAdpcmHeader>,
) -> Result<(u16, u32, Vec<S>)>
where
    S: Sample + ConvertibleSample,
{
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
    let mut scratch: Option<SampleBuffer<S>> = None;
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
        let samples = match &mut scratch {
            Some(buf) if buf.capacity() >= count => buf,
            _ => scratch.insert(SampleBuffer::<S>::new(
                decoded.capacity().max(decoded.frames()) as u64,
                current,
            )),
        };
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
    Ok((spec.channels.count() as u16, spec.rate, pcm))
}

fn compressed_to_wav(
    data: Vec<u8>,
    extension: &str,
    expected: Option<MsAdpcmHeader>,
) -> Result<Vec<u8>> {
    let (channels, sample_rate, pcm) = decode_compressed_samples::<i16>(data, extension, expected)?;
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(
        &mut cursor,
        hound::WavSpec {
            channels,
            sample_rate,
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

fn decode_wav_pcm(data: &[u8]) -> Result<(u16, u32, Vec<f32>, AudioSummary)> {
    let mut wav = hound::WavReader::new(Cursor::new(data))?;
    let spec = wav.spec();
    if !(1..=2).contains(&spec.channels) || spec.sample_rate == 0 {
        bail!("unsupported audio channel count or sample rate");
    }
    let frames = wav.duration();
    let cap = (wav.len() as usize).min(data.len());
    let mut samples = Vec::with_capacity(cap);
    match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, 32) => {
            for sample in wav.samples::<f32>() {
                let sample = sample?;
                if !sample.is_finite() {
                    bail!("nonfinite audio sample");
                }
                samples.push(sample);
            }
        }
        (hound::SampleFormat::Int, bits @ 1..=32) => {
            let scale = 1. / (1u64 << (bits - 1)) as f32;
            for sample in wav.samples::<i32>() {
                samples.push(sample? as f32 * scale);
            }
        }
        _ => bail!("unsupported WAV sample format"),
    }
    let summary = AudioSummary {
        codec: "wav".to_owned(),
        channels: spec.channels,
        sample_rate: spec.sample_rate,
        frames,
        seconds: f64::from(frames) / f64::from(spec.sample_rate),
    };
    Ok((spec.channels, spec.sample_rate, samples, summary))
}

fn decode_compressed_pcm(
    data: Vec<u8>,
    extension: &str,
    expected: Option<MsAdpcmHeader>,
    codec: &str,
) -> Result<(u16, u32, Vec<f32>, AudioSummary)> {
    let (channels, sample_rate, samples) =
        decode_compressed_samples::<f32>(data, extension, expected)?;
    let frames = u32::try_from(samples.len() / usize::from(channels))
        .context("audio frame count overflow")?;
    let summary = AudioSummary {
        codec: codec.to_owned(),
        channels,
        sample_rate,
        frames,
        seconds: f64::from(frames) / f64::from(sample_rate),
    };
    Ok((channels, sample_rate, samples, summary))
}

enum AudioKind {
    PcmWav,
    MsAdpcm(MsAdpcmHeader),
    Mp3,
}

fn classify_audio(path: &str, data: &[u8]) -> Result<AudioKind> {
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
        "wav" => Ok(match ms_adpcm_header(data)? {
            Some(header) => AudioKind::MsAdpcm(header),
            None => AudioKind::PcmWav,
        }),
        "mp3" => Ok(AudioKind::Mp3),
        _ => bail!("only WAV and MP3 audio are supported currently"),
    }
}

/// Decode bounded owned audio directly into interleaved normalized f32 samples and metadata:
/// `(channels, sample_rate, samples, summary)`.
pub fn decode_pcm(path: &str, data: Vec<u8>) -> Result<(u16, u32, Vec<f32>, AudioSummary)> {
    match classify_audio(path, &data)? {
        AudioKind::PcmWav => decode_wav_pcm(&data),
        AudioKind::MsAdpcm(header) => decode_compressed_pcm(data, "wav", Some(header), "ms-adpcm"),
        AudioKind::Mp3 => decode_compressed_pcm(data, "mp3", None, "mp3"),
    }
}

pub(super) fn playback_bytes(path: &str, data: Vec<u8>) -> Result<(Vec<u8>, &'static str)> {
    match classify_audio(path, &data)? {
        AudioKind::PcmWav => {
            validate_wav(&data)?;
            Ok((data, "wav"))
        }
        AudioKind::MsAdpcm(header) => {
            Ok((compressed_to_wav(data, "wav", Some(header))?, "ms-adpcm"))
        }
        AudioKind::Mp3 => Ok((compressed_to_wav(data, "mp3", None)?, "mp3")),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{decode, wav_samples};
    use super::{decode_pcm, MAX_ENCODED_AUDIO, MS_ADPCM_COEFFICIENTS};
    use std::io::Cursor;

    fn assert_matches_decode(path: &str, encoded: Vec<u8>) {
        let (wav_bytes, expected_summary) = decode(path, encoded.clone()).unwrap();
        let (expected_ch, expected_rate, expected_samples) = wav_samples(&wav_bytes).unwrap();
        let (ch, rate, samples, summary) = decode_pcm(path, encoded).unwrap();
        assert_eq!((ch, rate), (expected_ch, expected_rate));
        assert_eq!(samples, expected_samples);
        assert_eq!(summary.codec, expected_summary.codec);
        assert_eq!(summary.channels, expected_summary.channels);
        assert_eq!(summary.sample_rate, expected_summary.sample_rate);
        assert_eq!(summary.frames, expected_summary.frames);
        assert_eq!(summary.seconds, expected_summary.seconds);
    }

    fn build_adpcm(channels: u16, block: &[u8], frames: u32) -> Vec<u8> {
        let block_size = block.len() as u16;
        let per_block = (block_size - 7 * channels) * 2 / channels + 2;
        let mut fmt = Vec::new();
        fmt.extend(2u16.to_le_bytes());
        fmt.extend(channels.to_le_bytes());
        fmt.extend(22050u32.to_le_bytes());
        fmt.extend((22050u32 * u32::from(block_size) / u32::from(per_block)).to_le_bytes());
        fmt.extend(block_size.to_le_bytes());
        fmt.extend(4u16.to_le_bytes());
        fmt.extend(32u16.to_le_bytes());
        fmt.extend(per_block.to_le_bytes());
        fmt.extend(7u16.to_le_bytes());
        for (a, b) in MS_ADPCM_COEFFICIENTS {
            fmt.extend(a.to_le_bytes());
            fmt.extend(b.to_le_bytes());
        }
        let mut wav = b"RIFF\0\0\0\0WAVE".to_vec();
        for (id, chunk) in [
            (b"fmt ", fmt),
            (b"fact", frames.to_le_bytes().to_vec()),
            (b"data", block.to_vec()),
        ] {
            wav.extend(id);
            wav.extend((chunk.len() as u32).to_le_bytes());
            wav.extend(&chunk);
            if chunk.len() & 1 != 0 {
                wav.push(0);
            }
        }
        let len = wav.len() as u32 - 8;
        wav[4..8].copy_from_slice(&len.to_le_bytes());
        wav
    }

    #[test]
    fn decode_pcm_matches_decode_and_wav_samples_for_pcm_formats() {
        for (bits, format, values) in [
            (8u16, hound::SampleFormat::Int, vec![-128i32, -1, 0, 127]),
            (16, hound::SampleFormat::Int, vec![-32768, -123, 0, 32767]),
            (
                24,
                hound::SampleFormat::Int,
                vec![-8_388_608, -123_456, 0, 8_388_607],
            ),
        ] {
            let mut cursor = Cursor::new(Vec::new());
            let mut writer = hound::WavWriter::new(
                &mut cursor,
                hound::WavSpec {
                    channels: 2,
                    sample_rate: 44100,
                    bits_per_sample: bits,
                    sample_format: format,
                },
            )
            .unwrap();
            for s in values {
                writer.write_sample(s).unwrap();
            }
            writer.finalize().unwrap();
            assert_matches_decode("pcm.wav", cursor.into_inner());
        }

        let mut cursor = Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(
            &mut cursor,
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        for s in [-1.0f32, -0.25, 0.0, 0.75, 1.0] {
            writer.write_sample(s).unwrap();
        }
        writer.finalize().unwrap();
        assert_matches_decode("float32.wav", cursor.into_inner());
    }

    #[test]
    fn decode_pcm_matches_decode_and_wav_samples_for_ms_adpcm() {
        let mono = build_adpcm(1, &[0, 16, 0, 232, 3, 132, 3, 0x12, 0xf8, 0x00], 6);
        assert_matches_decode("voice.wav", mono);
        let stereo = build_adpcm(
            2,
            &[
                0, 0, 16, 0, 16, 0, 232, 3, 24, 252, 132, 3, 124, 252, 0x1f, 0x28,
            ],
            4,
        );
        assert_matches_decode("stereo.wav", stereo);
    }

    #[test]
    fn decode_pcm_enforces_limits_and_rejects_invalid_inputs() {
        assert!(decode_pcm("large.wav", vec![0; MAX_ENCODED_AUDIO + 1]).is_err());
        assert!(decode_pcm("unsupported.ogg", vec![]).is_err());
        assert!(decode_pcm("invalid.mp3", vec![0; 128]).is_err());

        let mut cursor = Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(
            &mut cursor,
            hound::WavSpec {
                channels: 1,
                sample_rate: 48000,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .unwrap();
        writer.write_sample(f32::NAN).unwrap();
        writer.finalize().unwrap();
        assert!(decode_pcm("nan.wav", cursor.into_inner()).is_err());
    }
}
