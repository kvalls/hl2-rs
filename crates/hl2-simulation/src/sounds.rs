//! Shared bounded owned-audio decoding, Source cue selection and actor-aware requests.
use anyhow::{bail, Context, Result};
use source_assets::{
    keyvalues,
    sounds::{ActorGender, ActorRegistry, Wave},
    vpk::Vfs,
};
use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, ErrorKind};
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error as DecodeError,
    formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

const MAX_ENCODED_AUDIO: usize = 32 * 1024 * 1024;
const MAX_PCM_AUDIO: usize = 128 * 1024 * 1024;
const MS_ADPCM_COEFFICIENTS: [(i16, i16); 7] = [
    (256, 0),
    (512, -256),
    (0, 0),
    (192, 64),
    (240, 0),
    (460, -208),
    (392, -232),
];

#[derive(Clone, Copy)]
struct MsAdpcmHeader {
    channels: u16,
    sample_rate: u32,
    frames: u32,
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
fn ms_adpcm_header(data: &[u8]) -> Result<Option<MsAdpcmHeader>> {
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
                format = Some((channels, rate, block, frames));
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

#[derive(serde::Serialize)]
pub struct AudioSummary {
    codec: String,
    channels: u16,
    sample_rate: u32,
    frames: u32,
    seconds: f64,
}
impl AudioSummary {
    pub fn seconds(&self) -> f64 {
        self.seconds
    }
}

fn validate_wav(data: &[u8]) -> Result<()> {
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

fn playback_bytes(path: &str, data: Vec<u8>) -> Result<(Vec<u8>, &'static str)> {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoundActor {
    pub name: String,
    pub model: String,
    /// Speaking entity, for lip sync.
    pub entity: Option<usize>,
}

/// ambient_generic sound control (SDK CAmbientGeneric InputPlaySound/InputStopSound).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmbientControl {
    /// Stop the entity's current sound and start it again.
    Play(usize),
    Stop(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SoundRequest {
    pub name: String,
    pub actor: Option<SoundActor>,
    /// Set for ambient_generic inputs; the host applies the entity's own parameters.
    pub ambient: Option<AmbientControl>,
    /// Emit volume (EmitSound_t m_flVolume); None uses the script volume.
    pub volume: Option<f32>,
    /// World position for distance gain with the script soundlevel; None is unspatialized.
    pub origin: Option<glam::Vec3>,
    /// The emitting entity/handle, for StopSound and sound patches (see emitters below).
    pub emitter: Option<u64>,
    /// CBaseEntity::StopSound(name) on `emitter`: stop that emitter's playing sounds of
    /// this name instead of starting one.
    pub stop: bool,
    /// CSoundEnvelopeController operation on the (emitter, name) patch.
    pub patch: Option<PatchOp>,
}

/// CSoundEnvelopeController (SDK soundenvelope.cpp) operations on one looping patch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PatchOp {
    /// Play(patch, volume, pitch): start (or restart) looping at this volume/pitch.
    Play { volume: f32, pitch: f32 },
    /// SoundChangeVolume(patch, volume, seconds): linear ramp from the current value.
    Volume { volume: f32, seconds: f32 },
    /// SoundChangePitch(patch, pitch, seconds).
    Pitch { pitch: f32, seconds: f32 },
    /// SoundFadeOut(patch, seconds): ramp to silence, then stop.
    FadeOut { seconds: f32 },
    /// SoundDestroy: stop immediately.
    Destroy,
}

/// One CSoundPatch's volume/pitch envelope (linear ramps from the value at the time
/// of the change, as CSoundEnvelopeController's SoundChangeVolume/Pitch).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PatchEnvelope {
    volume: Ramp,
    pitch: Ramp,
    /// SoundFadeOut: stop once the volume ramp reaches silence.
    fading: bool,
    pub stopped: bool,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Ramp {
    from: f32,
    to: f32,
    start: f64,
    seconds: f32,
}
impl Ramp {
    fn at(&self, time: f64) -> f32 {
        if self.seconds <= 0. {
            return self.to;
        }
        let t = ((time - self.start) as f32 / self.seconds).clamp(0., 1.);
        self.from + (self.to - self.from) * t
    }
    fn set(value: f32, time: f64) -> Self {
        Self {
            from: value,
            to: value,
            start: time,
            seconds: 0.,
        }
    }
    fn change(&mut self, to: f32, seconds: f32, time: f64) {
        *self = Self {
            from: self.at(time),
            to,
            start: time,
            seconds: seconds.max(0.),
        };
    }
}
impl PatchEnvelope {
    pub fn new(volume: f32, pitch: f32, time: f64) -> Self {
        Self {
            volume: Ramp::set(volume, time),
            pitch: Ramp::set(pitch, time),
            fading: false,
            stopped: false,
        }
    }
    pub fn apply(&mut self, op: PatchOp, time: f64) {
        match op {
            PatchOp::Play { volume, pitch } => *self = Self::new(volume, pitch, time),
            PatchOp::Volume { volume, seconds } => self.volume.change(volume, seconds, time),
            PatchOp::Pitch { pitch, seconds } => self.pitch.change(pitch, seconds, time),
            PatchOp::FadeOut { seconds } => {
                self.volume.change(0., seconds, time);
                self.fading = true;
            }
            PatchOp::Destroy => self.stopped = true,
        }
    }
    /// (volume, pitch percent) at `time`; marks a finished fade-out as stopped.
    pub fn sample(&mut self, time: f64) -> (f32, f32) {
        let volume = self.volume.at(time);
        if self.fading && volume <= 0. {
            self.stopped = true;
        }
        (volume, self.pitch.at(time))
    }
}

/// Emitter id spaces (entities, projectiles and the player's weapons never collide).
pub fn entity_emitter(id: usize) -> u64 {
    id as u64
}
pub fn projectile_emitter(id: u64) -> u64 {
    (1 << 40) | id
}
/// The player's active weapon (one id per weapon class is enough for patches).
pub fn weapon_emitter(class: &str) -> u64 {
    let hash = class.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    });
    (2 << 40) | (hash & 0xff_ffff_ffff)
}

impl SoundRequest {
    /// StopSound(name) on an emitter.
    pub fn stop(name: &str, emitter: u64) -> Self {
        Self {
            emitter: Some(emitter),
            stop: true,
            ..name.into()
        }
    }
    /// A CSoundEnvelopeController operation on the (emitter, name) patch.
    pub fn patch(name: &str, emitter: u64, op: PatchOp) -> Self {
        Self {
            emitter: Some(emitter),
            patch: Some(op),
            ..name.into()
        }
    }
}

impl From<String> for SoundRequest {
    fn from(name: String) -> Self {
        Self {
            name,
            actor: None,
            ambient: None,
            volume: None,
            origin: None,
            emitter: None,
            stop: false,
            patch: None,
        }
    }
}

impl From<&str> for SoundRequest {
    fn from(name: &str) -> Self {
        name.to_owned().into()
    }
}

impl AsRef<str> for SoundRequest {
    fn as_ref(&self) -> &str {
        &self.name
    }
}

impl PartialEq<str> for SoundRequest {
    fn eq(&self, other: &str) -> bool {
        self.name == other
    }
}

impl PartialEq<&str> for SoundRequest {
    fn eq(&self, other: &&str) -> bool {
        self.name == *other
    }
}

fn eligible_waves(waves: &[Wave], gender: ActorGender) -> Vec<usize> {
    let matching = waves
        .iter()
        .enumerate()
        .filter_map(|(index, wave)| (wave.gender == gender).then_some(index))
        .collect::<Vec<_>>();
    if matching.is_empty() {
        // Retail has an explicit random fallback across all registered waves
        // when this gender has no match. NONE is not implicitly male/female.
        (0..waves.len()).collect()
    } else {
        matching
    }
}

pub fn playback_path(wave: &str) -> String {
    // PSkipSoundChars skips only Source's defined modifiers. `$` is a literal
    // template byte, not a modifier; removing it could hide unresolved paths.
    wave.trim_start_matches(['*', '?', '!', '#', '>', '<', '^', '@', ')', '}'])
        .replace('\\', "/")
        .to_ascii_lowercase()
        .trim_start_matches("sound/")
        .to_owned()
}

fn raw_wave(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains(".wav") || lower.contains(".mp3")
}

/// Sound script playback parameters (SDK CSoundParametersInternal). Intervals use their
/// midpoint; Source picks a random value inside them per emission.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptParams {
    pub volume: f32,
    pub pitch: f32,
    pub soundlevel: f32,
    /// Interval bounds (low, high) that each emission draws from.
    pub volume_range: (f32, f32),
    pub pitch_range: (f32, f32),
}
impl Default for ScriptParams {
    fn default() -> Self {
        Self {
            volume: 1.,
            pitch: 100.,
            soundlevel: 75.,
            volume_range: (1., 1.),
            pitch_range: (100., 100.),
        }
    }
}
fn interval_bounds(text: &str) -> Option<(f32, f32)> {
    let values = text
        .split(',')
        .map(|v| v.trim().parse::<f32>().ok().filter(|v| v.is_finite()))
        .collect::<Option<Vec<_>>>()?;
    match values[..] {
        [v] => Some((v, v)),
        [a, b] => Some((a, b)),
        _ => None,
    }
}
fn interval(text: &str) -> Option<f32> {
    let values = text
        .split(',')
        .map(|v| v.trim().parse::<f32>().ok().filter(|v| v.is_finite()))
        .collect::<Option<Vec<_>>>()?;
    match values[..] {
        [v] => Some(v),
        [a, b] => Some((a + b) * 0.5),
        _ => None,
    }
}
/// SNDLVL_* names and numbers, or ATTN_* values via SDK ATTN_TO_SNDLVL.
pub fn soundlevel_value(text: &str) -> Option<f32> {
    let upper = text.trim().to_ascii_uppercase();
    if let Some(name) = upper.strip_prefix("SNDLVL_") {
        return match name {
            "NONE" => Some(0.),
            "IDLE" => Some(60.),
            "STATIC" => Some(66.),
            "NORM" => Some(75.),
            "TALKING" => Some(80.),
            "GUNFIRE" => Some(140.),
            _ => name.strip_suffix("DB")?.parse::<f32>().ok(),
        };
    }
    interval(&upper)
}
fn attenuation_soundlevel(text: &str) -> Option<f32> {
    let attenuation = match text.trim().to_ascii_uppercase().as_str() {
        "ATTN_NONE" => 0.,
        "ATTN_NORM" => 0.8,
        "ATTN_IDLE" => 2.,
        "ATTN_STATIC" => 1.25,
        "ATTN_RICOCHET" => 1.5,
        "ATTN_GUNFIRE" => 0.27,
        other => interval(other)?,
    };
    Some(if attenuation == 0. {
        0.
    } else {
        (50. + 20. / attenuation).trunc()
    })
}
fn script_params(entry: &source_assets::keyvalues::Entry) -> ScriptParams {
    let mut params = ScriptParams::default();
    for child in entry.children() {
        let Some(text) = child.text() else { continue };
        let key = child.key.to_ascii_lowercase();
        match key.as_str() {
            "volume" if text.eq_ignore_ascii_case("VOL_NORM") => {
                params.volume = 1.;
                params.volume_range = (1., 1.);
            }
            "volume" => {
                params.volume = interval(text).unwrap_or(params.volume);
                params.volume_range = interval_bounds(text).unwrap_or(params.volume_range);
            }
            "pitch" => {
                let named = match text.to_ascii_uppercase().as_str() {
                    "PITCH_NORM" => Some(100.),
                    "PITCH_LOW" => Some(95.),
                    "PITCH_HIGH" => Some(120.),
                    _ => None,
                };
                params.pitch = named.or_else(|| interval(text)).unwrap_or(params.pitch);
                params.pitch_range = named
                    .map(|p| (p, p))
                    .or_else(|| interval_bounds(text))
                    .unwrap_or(params.pitch_range);
            }
            "soundlevel" => params.soundlevel = soundlevel_value(text).unwrap_or(params.soundlevel),
            "attenuation" => {
                params.soundlevel = attenuation_soundlevel(text).unwrap_or(params.soundlevel)
            }
            _ => {}
        }
    }
    params
}

pub struct Library {
    names: BTreeMap<String, Vec<Wave>>,
    params: BTreeMap<String, ScriptParams>,
    actors: ActorRegistry,
    available: HashMap<String, Vec<bool>>,
    rng: u64,
    /// Separate stream for volume/pitch draws so wave selection is unchanged.
    param_rng: u64,
    pub variants_played: BTreeMap<String, usize>,
    pub decoded: BTreeMap<String, AudioSummary>,
    pub errors: BTreeMap<String, String>,
    pub played: usize,
}
impl Library {
    /// Precache every registered variant, including unresolved actor genders.
    /// Runtime emission still uses alternatives/resolve's exact actor policy.
    pub fn all_alternatives(&self, name: &str) -> Result<Vec<String>> {
        if raw_wave(name) {
            return Ok(vec![playback_path(name)]);
        }
        self.names
            .get(&name.to_ascii_lowercase())
            .map(|waves| waves.iter().map(|wave| playback_path(&wave.path)).collect())
            .with_context(|| format!("sound script absent: {name}"))
    }
    pub fn new(vfs: &Vfs) -> Self {
        let mut names = BTreeMap::new();
        let mut params = BTreeMap::new();
        let mut errors = BTreeMap::new();
        let actors = match (|| -> Result<ActorRegistry> {
            let data = vfs
                .read("scripts/global_actors.txt")?
                .context("actor registry absent")?;
            ActorRegistry::parse(&data)
        })() {
            Ok(registry) => registry,
            Err(error) => {
                errors.insert("scripts/global_actors.txt".into(), format!("{error:#}"));
                ActorRegistry::default()
            }
        };
        let result = (|| -> Result<()> {
            let manifest = vfs
                .read("scripts/game_sounds_manifest.txt")?
                .context("sound manifest absent")?;
            let tokens = keyvalues::tokens(&String::from_utf8_lossy(&manifest))?;
            for pair in tokens
                .windows(2)
                .filter(|p| p[0].eq_ignore_ascii_case("precache_file"))
            {
                let Some(data) = vfs.read(&pair[1])? else {
                    continue;
                };
                for entry in keyvalues::parse(&String::from_utf8_lossy(&data))? {
                    let mut waves = Vec::new();
                    for child in entry.children() {
                        if child.key.eq_ignore_ascii_case("wave") {
                            if let Some(wave) = child.text() {
                                waves.extend(Wave::register(wave)?);
                            }
                        }
                        if child.key.eq_ignore_ascii_case("rndwave") {
                            for wave in child
                                .children()
                                .iter()
                                .filter(|e| e.key.eq_ignore_ascii_case("wave"))
                            {
                                if let Some(wave) = wave.text() {
                                    waves.extend(Wave::register(wave)?);
                                }
                            }
                        }
                    }
                    if !waves.is_empty() {
                        if waves.len() > 4096 {
                            bail!("sound script has more than 4096 wave alternatives");
                        }
                        params.insert(entry.key.to_lowercase(), script_params(&entry));
                        names.insert(entry.key.to_lowercase(), waves);
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("Sound manifest: {e:#}");
            errors.insert("scripts/game_sounds_manifest.txt".into(), format!("{e:#}"));
        }
        Self {
            names,
            params,
            actors,
            available: HashMap::new(),
            rng: 0x92ea79123,
            param_rng: 0x51ab_2c3d_4e5f_6071,
            variants_played: BTreeMap::new(),
            decoded: BTreeMap::new(),
            errors,
            played: 0,
        }
    }
    /// One emission's volume and pitch drawn inside the script intervals
    /// (CSoundParametersInternal random volume/pitch), plus the soundlevel.
    pub fn draw_params(&mut self, name: &str) -> (f32, f32, f32) {
        let p = self.params(name);
        let mut draw = |(low, high): (f32, f32)| {
            if high == low {
                return low;
            }
            self.param_rng ^= self.param_rng << 13;
            self.param_rng ^= self.param_rng >> 7;
            self.param_rng ^= self.param_rng << 17;
            low + (high - low) * ((self.param_rng >> 40) as f32 / (1u64 << 24) as f32)
        };
        let volume = draw(p.volume_range);
        let pitch = draw(p.pitch_range);
        (volume, pitch, p.soundlevel)
    }
    /// Script volume/pitch/soundlevel; raw waves use the SDK defaults.
    pub fn params(&self, name: &str) -> ScriptParams {
        self.params
            .get(&name.to_ascii_lowercase())
            .copied()
            .unwrap_or_default()
    }
    fn actor_gender(&self, request: &SoundRequest) -> Result<ActorGender> {
        self.actors
            .gender(request.actor.as_ref().map(|actor| actor.model.as_str()))
    }

    /// Enumerate every wave that this request can select, without consuming the
    /// emission shuffle state. Used for owned-asset validation and preloading.
    pub fn alternatives(&self, request: &SoundRequest) -> Result<Vec<String>> {
        if raw_wave(&request.name) {
            // Raw EmitSound does not call GenderExpandString, even with an actor.
            return Ok(vec![playback_path(&request.name)]);
        }
        let key = request.name.to_ascii_lowercase();
        if let Some(waves) = self.names.get(&key) {
            let gender = self.actor_gender(request)?;
            Ok(eligible_waves(waves, gender)
                .into_iter()
                .map(|index| playback_path(&waves[index].path))
                .collect())
        } else {
            bail!("sound script absent: {}", request.name);
        }
    }

    pub fn resolve(&mut self, request: &SoundRequest) -> Result<String> {
        if raw_wave(&request.name) {
            return Ok(playback_path(&request.name));
        }
        let key = request.name.to_ascii_lowercase();
        if let Some(waves) = self.names.get(&key) {
            let gender = self.actor_gender(request)?;
            let mut eligible = eligible_waves(waves, gender);
            let matching_gender = eligible.iter().any(|&i| waves[i].gender == gender);
            let available = self
                .available
                .entry(key)
                .or_insert_with(|| vec![true; waves.len()]);
            if matching_gender {
                // Native resets only this gender's exhausted bag. If no gender
                // matches, its all-wave fallback ignores availability flags.
                if !eligible.iter().any(|&i| available[i]) {
                    for &index in &eligible {
                        available[index] = true;
                    }
                }
                eligible.retain(|&index| available[index]);
            }
            self.rng ^= self.rng << 13;
            self.rng ^= self.rng >> 7;
            self.rng ^= self.rng << 17;
            let choice = eligible[self.rng as usize % eligible.len()];
            available[choice] = false;
            Ok(playback_path(&waves[choice].path))
        } else {
            Ok(self.alternatives(request)?.remove(0))
        }
    }
}

/// Source distance gain (retail engine `GetDistGainFromSoundLevel`, engine.dll 10216bd0 and
/// 10216a20) with the stock convars snd_refdb 60, snd_refdist 36, snd_gain 1, snd_gain_max 1,
/// snd_gain_min 0.01 and snd_foliage_db_loss 4. Soundlevel 0 (SNDLVL_NONE) plays everywhere.
pub fn dist_gain(soundlevel: f32, distance: f32) -> f32 {
    const REF_DB: f32 = 60.;
    const REF_DIST: f32 = 36.;
    const GAIN: f32 = 1.;
    const GAIN_MAX: f32 = 1.;
    const GAIN_MIN: f32 = 0.01;
    const FOLIAGE_DB_LOSS: f32 = 4.;
    if soundlevel == 0. {
        return GAIN;
    }
    let dist_mult = 10f32.powf(REF_DB * 0.05) / 10f32.powf(soundlevel * 0.05) / REF_DIST;
    if dist_mult == 0. {
        return GAIN;
    }
    let relative =
        dist_mult * distance * 10f32.powf(distance * (1. / 1200.) * FOLIAGE_DB_LOSS * 0.05);
    let mut gain = if f64::from(relative) > 0.1 {
        GAIN / relative
    } else {
        GAIN * 10.
    };
    if gain > 0.5 {
        // Soft knee above half gain; loud (> 90 dB) sounds compress less steeply.
        let level = (20. * (10f32.powf(REF_DB / 20.) / (REF_DIST * dist_mult)).log10()) as i32;
        let power = if level as f32 > 90. {
            2.5 - (level as f32 - 90.) * 1.7 * 0.02
        } else {
            2.5
        };
        let knee = -1. / (0.5f32.powf(power) * -0.5);
        gain = GAIN_MAX * (1. - 1. / (gain.powf(power) * knee));
    }
    if GAIN_MIN > gain {
        gain = (2. - GAIN_MIN * relative) * GAIN_MIN;
        if gain <= 0. {
            gain = 0.001;
        }
    }
    gain
}

/// Interleaved samples in -1..1 of PCM WAV bytes produced by [`decode`]:
/// (channels, sample rate, samples). 8-bit WAV is unsigned, as RIFF defines it.
pub fn wav_samples(data: &[u8]) -> Result<(u16, u32, Vec<f32>)> {
    let mut wav = hound::WavReader::new(Cursor::new(data))?;
    let spec = wav.spec();
    let samples = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, 32) => wav.samples::<f32>().collect::<Result<Vec<_>, _>>()?,
        (hound::SampleFormat::Int, bits @ 1..=32) => {
            let scale = 1. / (1u64 << (bits - 1)) as f32;
            wav.samples::<i32>()
                .map(|s| s.map(|s| s as f32 * scale))
                .collect::<Result<Vec<_>, _>>()?
        }
        _ => bail!("unsupported WAV sample format"),
    };
    Ok((spec.channels, spec.sample_rate, samples))
}

/// Decode bounded owned audio into backend-independent WAV bytes and metadata.
pub fn decode(path: &str, encoded: Vec<u8>) -> Result<(Vec<u8>, AudioSummary)> {
    let (data, codec) = playback_bytes(path, encoded)?;
    let wav = hound::WavReader::new(Cursor::new(&data))?;
    let spec = wav.spec();
    let summary = AudioSummary {
        codec: codec.to_owned(),
        channels: spec.channels,
        sample_rate: spec.sample_rate,
        frames: wav.duration(),
        seconds: f64::from(wav.duration()) / f64::from(spec.sample_rate),
    };
    Ok((data, summary))
}

/// The retained viewmodel event cursor, independent of rendering frame rate.
#[derive(Default)]
pub struct WeaponAnimationSounds {
    previous: Option<(String, String, f64, f32)>,
}
impl WeaponAnimationSounds {
    pub fn events(
        &mut self,
        world: &modkit_core::World,
        inventory: &crate::gameplay::Inventory,
        weapons: &BTreeMap<String, crate::gameplay::Weapon>,
        time: f64,
    ) -> Vec<modkit_core::animation::ClipEvent> {
        let Some(weapon) = weapons.get(&inventory.active) else {
            return vec![];
        };
        let Some(clip) = world
            .rigs
            .get(&format!("{}#0", weapon.viewmodel.to_lowercase()))
            .and_then(|rig| rig.clips.get(&inventory.animation))
        else {
            return vec![];
        };
        let elapsed = (time - inventory.animation_at).max(0.) as f32;
        let previous = self
            .previous
            .as_ref()
            .filter(|(weapon, animation, started, _)| {
                *weapon == inventory.active
                    && *animation == inventory.animation
                    && *started == inventory.animation_at
            })
            .map_or(-f32::EPSILON, |(_, _, _, elapsed)| *elapsed);
        let events = clip
            .events_between(previous, elapsed)
            .into_iter()
            .filter(|event| {
                (event.id == 5004 || event.name == "AE_CL_PLAYSOUND") && !event.options.is_empty()
            })
            .cloned()
            .collect();
        self.previous = Some((
            inventory.active.clone(),
            inventory.animation.clone(),
            inventory.animation_at,
            elapsed,
        ));
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speech_audio() -> Library {
        Library {
            params: BTreeMap::new(),
            names: [
                (
                    "mixed".into(),
                    ["vo/$gender01/a.wav", "vo/shared.wav"]
                        .into_iter()
                        .flat_map(|path| Wave::register(path).unwrap())
                        .collect(),
                ),
                (
                    "gendered".into(),
                    ["vo/$gender01/a.wav", "vo/$gender01/b.wav"]
                        .into_iter()
                        .flat_map(|path| Wave::register(path).unwrap())
                        .collect(),
                ),
                ("plain".into(), Wave::register("vo/shared.wav").unwrap()),
            ]
            .into(),
            actors: ActorRegistry::parse(br#"actors { "actor_a" "female" "actor_b" "male" }"#)
                .unwrap(),
            available: HashMap::new(),
            rng: 0x92ea79123,
            param_rng: 0x51ab_2c3d_4e5f_6071,
            variants_played: BTreeMap::new(),
            decoded: BTreeMap::new(),
            errors: BTreeMap::new(),
            played: 0,
        }
    }

    #[test]
    fn patch_envelopes_ramp_linearly_from_the_current_value_and_fade_out_stops() {
        let mut env = PatchEnvelope::new(0., 50., 0.);
        env.apply(
            PatchOp::Pitch {
                pitch: 100.,
                seconds: 0.5,
            },
            0.,
        );
        env.apply(
            PatchOp::Volume {
                volume: 0.8,
                seconds: 0.5,
            },
            0.,
        );
        assert_eq!(env.sample(0.25), (0.4, 75.));
        assert_eq!(env.sample(1.), (0.8, 100.));
        // A wind-down at 0.25 s starts from the ramp's value then.
        let mut env2 = PatchEnvelope::new(0., 50., 0.);
        env2.apply(
            PatchOp::Volume {
                volume: 0.8,
                seconds: 0.5,
            },
            0.,
        );
        env2.apply(
            PatchOp::Volume {
                volume: 0.,
                seconds: 1.,
            },
            0.25,
        );
        assert!((env2.sample(0.75).0 - 0.2).abs() < 1e-6);
        env.apply(PatchOp::FadeOut { seconds: 0.1 }, 2.);
        assert!(!env.stopped);
        env.sample(2.05);
        assert!(!env.stopped);
        env.sample(2.1);
        assert!(env.stopped);
    }

    fn speech_request(cue: &str, model: &str) -> SoundRequest {
        SoundRequest {
            actor: Some(SoundActor {
                name: "actor diagnostic name".into(),
                model: model.into(),
                entity: None,
            }),
            ..cue.into()
        }
    }

    #[test]
    fn preloading_all_genders_does_not_advance_emission_shuffle() {
        let mut audio = speech_audio();
        let mut control = speech_audio();
        assert_eq!(
            audio.all_alternatives("MIXED").unwrap(),
            ["vo/male01/a.wav", "vo/female01/a.wav", "vo/shared.wav"]
        );
        assert_eq!(
            audio.all_alternatives("^vo/$gender01/raw.wav").unwrap(),
            ["vo/$gender01/raw.wav"]
        );
        let request = speech_request("mixed", "models/actor_a.mdl");
        for _ in 0..8 {
            assert_eq!(
                audio.resolve(&request).unwrap(),
                control.resolve(&request).unwrap()
            );
        }
    }

    #[test]
    fn viewmodel_cues_fire_once_through_pause_and_animation_restart() {
        use crate::gameplay::{Inventory, Weapon};
        use modkit_core::{
            animation::{Clip, ClipEvent, Rig},
            World,
        };
        let sound = |cycle, id, name: &str, options: &str| ClipEvent {
            cycle,
            id,
            flags: 0,
            name: name.into(),
            options: options.into(),
        };
        let mut world = World::default();
        world.rigs.insert(
            "models/test.mdl#0".into(),
            Rig {
                clips: [(
                    "draw".into(),
                    Clip {
                        layer: Default::default(),
                        fps: 1.,
                        looping: false,
                        frames: vec![vec![], vec![]],
                        events: vec![
                            sound(0., 5004, "", "start.wav"),
                            sound(0.5, 7, "AE_CL_PLAYSOUND", "mid.wav"),
                            sound(0.6, 8, "other", "ignored.wav"),
                            sound(0.6, 5004, "", ""),
                        ],
                    },
                )]
                .into(),
                ..Default::default()
            },
        );
        let weapons = [(
            "weapon_test".into(),
            Weapon {
                viewmodel: "models/test.mdl".into(),
                ..Default::default()
            },
        )]
        .into();
        let mut inventory = Inventory::default();
        inventory.active = "weapon_test".into();
        inventory.animation = "draw".into();
        inventory.animation_at = 0.;
        let mut cursor = WeaponAnimationSounds::default();
        assert_eq!(
            cursor.events(&world, &inventory, &weapons, 0.)[0].options,
            "start.wav"
        );
        assert!(cursor.events(&world, &inventory, &weapons, 0.).is_empty());
        assert_eq!(
            cursor.events(&world, &inventory, &weapons, 0.5)[0].options,
            "mid.wav"
        );
        assert!(cursor.events(&world, &inventory, &weapons, 0.5).is_empty());
        assert!(cursor.events(&world, &inventory, &weapons, 0.9).is_empty());
        inventory.animation_at = 1.;
        assert_eq!(
            cursor.events(&world, &inventory, &weapons, 1.)[0].options,
            "start.wav"
        );
    }

    #[test]
    fn symbolic_selection_uses_model_registry_and_exact_gender_tags() -> Result<()> {
        let mut audio = speech_audio();
        assert_eq!(
            audio.alternatives(&speech_request("MIXED", r"models\ACTOR_A.mdl"))?,
            ["vo/female01/a.wav"]
        );
        assert_eq!(
            audio.alternatives(&speech_request("mixed", "models/actor_b.mdl"))?,
            ["vo/male01/a.wav"]
        );
        // Plain NONE waves are selected for an unknown actor when present.
        assert_eq!(
            audio.alternatives(&speech_request("mixed", "models/female_guess.mdl"))?,
            ["vo/shared.wav"]
        );
        // With no matching NONE wave, retail considers all registered genders.
        assert_eq!(
            audio.alternatives(&"gendered".into())?,
            [
                "vo/male01/a.wav",
                "vo/female01/a.wav",
                "vo/male01/b.wav",
                "vo/female01/b.wav"
            ]
        );
        assert_eq!(
            audio.alternatives(&speech_request("plain", "models/actor_a.mdl"))?,
            ["vo/shared.wav"]
        );
        // Raw EmitSound bypasses the actor/template resolver.
        assert_eq!(
            audio.alternatives(&speech_request(
                "^vo/$gender01/raw.wav",
                "models/actor_a.mdl"
            ))?,
            ["vo/$gender01/raw.wav"]
        );
        assert!(audio.alternatives(&"unregistered_symbolic".into()).is_err());
        audio
            .names
            .insert("vo/raw.wav".into(), Wave::register("vo/other.wav")?);
        assert_eq!(
            audio.resolve(&speech_request("vo/raw.wav", &"x".repeat(5000)))?,
            "vo/raw.wav"
        );
        assert_eq!(
            audio.resolve(&speech_request("$gender/raw.wav", "models/actor_a.mdl"))?,
            "$gender/raw.wav"
        );
        Ok(())
    }

    #[test]
    fn emitting_shuffle_exhausts_and_resets_each_gender_independently() -> Result<()> {
        let mut audio = speech_audio();
        let female = speech_request("gendered", "models/actor_a.mdl");
        let male = speech_request("gendered", "models/actor_b.mdl");
        let first = audio.resolve(&female)?;
        let second = audio.resolve(&female)?;
        assert_ne!(first, second);
        assert!(first.contains("female01") && second.contains("female01"));
        let male_first = audio.resolve(&male)?;
        let male_second = audio.resolve(&male)?;
        assert_ne!(male_first, male_second);
        assert!(male_first.contains("male01") && male_second.contains("male01"));
        assert!(audio.resolve(&female)?.contains("female01"));
        // NONE's fallback must work even after both gender bags are exhausted.
        for _ in 0..12 {
            assert!(audio
                .alternatives(&"gendered".into())?
                .contains(&audio.resolve(&"gendered".into())?));
        }
        Ok(())
    }

    fn adpcm_wave(channels: u16, block: &[u8], frames: u32) -> Vec<u8> {
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
        for (first, second) in MS_ADPCM_COEFFICIENTS {
            fmt.extend(first.to_le_bytes());
            fmt.extend(second.to_le_bytes());
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
    fn microsoft_adpcm_mono_decodes_nibbles_and_trims_block_padding_to_fact() {
        // Predictor0, delta16, previous samples900/1000, then signed nibbles
        // +1,+2,-1,-8,0,0. fact6 excludes the last two padded frames.
        let original = adpcm_wave(1, &[0, 16, 0, 232, 3, 132, 3, 0x12, 0xf8, 0x00], 6);
        assert!(validate_wav(&original).is_err());
        let (converted, codec) = playback_bytes("voice.wav", original).unwrap();
        assert_eq!(codec, "ms-adpcm");
        let mut wav = hound::WavReader::new(Cursor::new(converted)).unwrap();
        assert_eq!(wav.spec().channels, 1);
        assert_eq!(wav.spec().sample_rate, 22050);
        assert_eq!(wav.duration(), 6);
        assert_eq!(
            wav.samples::<i16>()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap(),
            [900, 1000, 1016, 1048, 1032, 904]
        );
    }

    #[test]
    fn microsoft_adpcm_stereo_preserves_channel_order_and_signed_samples() {
        let original = adpcm_wave(
            2,
            &[
                0, 0, 16, 0, 16, 0, 232, 3, 24, 252, 132, 3, 124, 252, 0x1f, 0x28,
            ],
            4,
        );
        let (converted, codec) = playback_bytes("stereo.wav", original).unwrap();
        assert_eq!(codec, "ms-adpcm");
        let mut wav = hound::WavReader::new(Cursor::new(converted)).unwrap();
        assert_eq!(wav.spec().channels, 2);
        assert_eq!(wav.duration(), 4);
        assert_eq!(
            wav.samples::<i16>()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap(),
            [900, -900, 1000, -1000, 1016, -1016, 1048, -1144]
        );
    }

    #[test]
    fn microsoft_adpcm_rejects_custom_predictors_bad_blocks_and_fact_counts() {
        let valid = adpcm_wave(1, &[0, 16, 0, 232, 3, 132, 3, 0x12, 0xf8, 0x00], 8);
        for (offset, bytes) in [
            (32, vec![6, 0]),                      // Preamble cannot fit the declared block.
            (38, vec![7, 0]),                      // samplesPerBlock disagrees with block size.
            (42, vec![255, 0]),                    // Custom coefficient table is unsupported.
            (78, vec![9, 0, 0, 0]),                // More authored frames than encoded data.
            (78, u32::MAX.to_le_bytes().to_vec()), // Refuse oversized decoded allocation.
            (22, vec![3, 0]),                      // Surround layout is unsupported.
            (24, vec![0, 0, 0, 0]),                // Invalid sample rate.
            (90, vec![7]), // Invalid predictor index inside an encoded block.
        ] {
            let mut malformed = valid.clone();
            malformed[offset..offset + bytes.len()].copy_from_slice(&bytes);
            assert!(
                playback_bytes("bad.wav", malformed).is_err(),
                "offset {offset}"
            );
        }
        let mut truncated = valid.clone();
        truncated.pop();
        assert!(playback_bytes("truncated.wav", truncated).is_err());
        let mut missing_fact = valid;
        missing_fact.drain(70..82);
        let len = missing_fact.len() as u32 - 8;
        missing_fact[4..8].copy_from_slice(&len.to_le_bytes());
        assert!(playback_bytes("missing-fact.wav", missing_fact).is_err());
    }

    #[test]
    fn encoded_audio_limit_is_enforced_before_probing() {
        assert!(playback_bytes("large.wav", vec![0; MAX_ENCODED_AUDIO + 1]).is_err());
    }

    #[test]
    fn script_params_read_names_intervals_and_attenuation() {
        let entries = source_assets::keyvalues::parse(
            r#"a { volume VOL_NORM pitch "95,105" soundlevel SNDLVL_75dB wave x.wav }
               b { volume 0.32 pitch PITCH_HIGH attenuation ATTN_STATIC wave y.wav }
               c { soundlevel SNDLVL_NONE wave z.wav }"#,
        )
        .unwrap();
        let p: Vec<ScriptParams> = entries.iter().map(script_params).collect();
        assert_eq!(
            p[0],
            ScriptParams {
                volume: 1.,
                pitch: 100.,
                soundlevel: 75.,
                volume_range: (1., 1.),
                pitch_range: (95., 105.),
            }
        );
        assert_eq!(
            p[1],
            ScriptParams {
                volume: 0.32,
                pitch: 120.,
                soundlevel: 66.,
                volume_range: (0.32, 0.32),
                pitch_range: (120., 120.),
            }
        );
        assert_eq!(p[2].soundlevel, 0.);
        assert_eq!(soundlevel_value("SNDLVL_TALKING"), Some(80.));
    }
    #[test]
    fn dist_gain_follows_the_retail_curve() {
        // SNDLVL_NONE is not attenuated.
        assert_eq!(dist_gain(0., 5000.), 1.);
        // At the 36-unit reference distance a 60 dB sound has relative distance ~1, which the
        // soft knee maps to 1 - 1/(2/0.5^2.5) = 1 - 0.5^2.5 / 2.
        let knee = 1. - 0.5f32.powf(2.5) / 2.;
        assert!((dist_gain(60., 36.) - knee).abs() < 0.01);
        // Gain at the knee boundary is continuous (0.5 on both sides).
        let at = |d: f32| dist_gain(60., d);
        let boundary = (1..10000)
            .map(|d| d as f32 * 0.1)
            .find(|&d| at(d) <= 0.5)
            .unwrap();
        assert!((at(boundary) - 0.5).abs() < 0.01);
        // Falls monotonically with distance; below 0.01 it ramps toward zero and is then
        // held at the 0.001 floor (a small step up from the ramp's last values, as in retail).
        let gains: Vec<f32> = (1..400).map(|i| at(i as f32 * 25.)).collect();
        let floor = gains.iter().position(|&g| g == 0.001).unwrap();
        assert!(gains[..floor].windows(2).all(|w| w[1] <= w[0] + 1e-6));
        assert!(gains[floor..].iter().all(|&g| g == 0.001));
        assert!(gains[floor - 1] < 0.001);
        assert_eq!(at(1e6), 0.001);
        // Louder sounds reach farther.
        assert!(dist_gain(90., 1000.) > dist_gain(75., 1000.));
    }
    #[test]
    fn wav_samples_reads_unsigned_8_bit_and_signed_16_bit() {
        for (bits, raw, expected) in [
            (8u16, vec![128u8, 255, 0], vec![0., 127. / 128., -1.]),
            (
                16,
                vec![0, 0, 0xff, 0x7f, 0, 0x80],
                vec![0., 32767. / 32768., -1.],
            ),
        ] {
            let mut cursor = Cursor::new(Vec::new());
            let mut writer = hound::WavWriter::new(
                &mut cursor,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 22050,
                    bits_per_sample: bits,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            if bits == 8 {
                for b in &raw {
                    writer.write_sample(*b as i8 ^ i8::MIN).unwrap();
                }
            } else {
                for c in raw.as_chunks::<2>().0 {
                    writer.write_sample(i16::from_le_bytes(*c)).unwrap();
                }
            }
            writer.finalize().unwrap();
            let (channels, rate, samples) = wav_samples(&cursor.into_inner()).unwrap();
            assert_eq!((channels, rate), (1, 22050));
            assert_eq!(samples, expected);
        }
    }
    #[test]
    fn valid_pcm_keeps_its_rate_channels_and_samples() {
        let mut cursor = Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(
            &mut cursor,
            hound::WavSpec {
                channels: 2,
                sample_rate: 22050,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for sample in [i16::MIN, i16::MAX, -123, 456] {
            writer.write_sample(sample).unwrap();
        }
        writer.finalize().unwrap();
        let original = cursor.into_inner();
        assert_eq!(
            playback_bytes("synthetic.WAV", original.clone()).unwrap().0,
            original
        );
    }

    #[test]
    fn invalid_compressed_audio_is_rejected_without_a_substitute() {
        assert!(playback_bytes("invalid.mp3", vec![0; 128]).is_err());
        assert!(playback_bytes("empty.mp3", vec![]).is_err());
        assert!(playback_bytes("unsupported.ogg", vec![]).is_err());
        assert!(playback_bytes("truncated.wav", b"RIFF".to_vec()).is_err());
    }

    #[test]
    fn nonfinite_pcm_and_surround_audio_are_rejected() {
        for (channels, sample) in [(1, f32::NAN), (3, 0.)] {
            let mut cursor = Cursor::new(Vec::new());
            let mut writer = hound::WavWriter::new(
                &mut cursor,
                hound::WavSpec {
                    channels,
                    sample_rate: 48000,
                    bits_per_sample: 32,
                    sample_format: hound::SampleFormat::Float,
                },
            )
            .unwrap();
            for _ in 0..channels {
                writer.write_sample(sample).unwrap();
            }
            writer.finalize().unwrap();
            assert!(playback_bytes("invalid.wav", cursor.into_inner()).is_err());
        }
    }

    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn installed_trainstation_music_decodes_to_nonempty_pcm() {
        let root = source_assets::install::discover().unwrap();
        let vfs = Vfs::mount(&root).unwrap();
        let path = "music/HL2_song26_trainstation1.mp3";
        let original = vfs.read(&format!("sound/{path}")).unwrap().unwrap();
        let converted = playback_bytes(path, original).unwrap().0;
        validate_wav(&converted).unwrap();
        let mut wav = hound::WavReader::new(Cursor::new(converted)).unwrap();
        let spec = wav.spec();
        let samples = wav
            .samples::<i16>()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert!(samples.len() > spec.sample_rate as usize * spec.channels as usize);
        assert!(samples.iter().any(|sample| sample.unsigned_abs() > 100));
        println!(
            "installed music: {spec:?}, {} interleaved samples",
            samples.len()
        );
    }

    #[test]
    #[ignore = "requires an owned installed English Half-Life 2 copy"]
    fn installed_gman_adpcm_voices_preserve_authored_fact_duration() {
        let root = source_assets::install::discover().unwrap();
        let vfs = Vfs::mount(&root).unwrap();
        for (path, expected_frames) in [
            ("vo/gman_misc/gman_riseshine.wav", 146740),
            ("vo/gman_misc/gman_02.wav", 461823),
        ] {
            let original = vfs.read(&format!("sound/{path}")).unwrap().unwrap();
            let header = ms_adpcm_header(&original).unwrap().unwrap();
            assert_eq!(header.frames, expected_frames);
            let (converted, codec) = playback_bytes(path, original).unwrap();
            assert_eq!(codec, "ms-adpcm");
            validate_wav(&converted).unwrap();
            let mut wav = hound::WavReader::new(Cursor::new(converted)).unwrap();
            assert_eq!(wav.spec().channels, 1);
            assert_eq!(wav.spec().sample_rate, 22050);
            assert_eq!(wav.duration(), expected_frames);
            assert!(wav
                .samples::<i16>()
                .any(|sample| sample.unwrap().unsigned_abs() > 100));
            println!("{path}: Microsoft ADPCM -> PCM16, {expected_frames} frames, mono 22050Hz");
        }
    }

    #[test]
    #[ignore = "requires an owned installed English Half-Life 2 copy"]
    fn installed_first_map_scene_voice_census() -> Result<()> {
        use source_assets::{
            bsp::Bsp,
            scenes::{Cache, EventType},
        };
        use std::collections::BTreeSet;
        let root = source_assets::install::discover()?;
        let mut vfs = Vfs::mount(&root)?;
        let bsp = Bsp::parse(
            &vfs.read("maps/d1_trainstation_01.bsp")?
                .context("first map absent")?,
        )?;
        vfs.mount_pak(bsp.lump(40))?;
        let world = bsp.world("d1_trainstation_01")?;
        let mut controller = crate::entities::Scene::new(&world);
        // Speakers created by point_templates exist by the time their scenes run.
        for (id, e) in world.entities.iter().enumerate() {
            if e.class() == "point_template" {
                controller.send(id, "ForceSpawn", "");
            }
        }
        controller.tick(&world, glam::Vec3::ZERO, 0.015);
        let cache = Cache::parse(
            vfs.read("scenes/scenes.image")?
                .context("scene cache absent")?,
        )?;
        let audio = Library::new(&vfs);
        let mut scene_paths = BTreeSet::new();
        let mut cues = BTreeSet::new();
        let mut definition_events = BTreeSet::new();
        let mut registered_paths = BTreeSet::new();
        let mut eligible_paths = BTreeSet::new();
        let mut scene_failures = BTreeMap::new();
        let mut actor_failures = BTreeMap::new();
        let mut cue_failures = BTreeMap::new();
        let mut decode_failures = BTreeMap::new();
        let mut template_requests = Vec::new();
        let mut references = 0;
        let mut speech_instances = 0;
        let mut resolved_actors = 0;
        let mut registered_alternatives = 0;
        for (entity_id, entity) in world
            .entities
            .iter()
            .enumerate()
            .filter(|(_, entity)| entity.class() == "logic_choreographed_scene")
        {
            let Some(path) = entity.get("SceneFile") else {
                scene_failures.insert(format!("entity {entity_id}"), "SceneFile absent".to_owned());
                continue;
            };
            references += 1;
            let path = path.replace('\\', "/").to_ascii_lowercase();
            scene_paths.insert(path.clone());
            let scene = match cache.scene(&path) {
                Ok(Some(scene)) => scene,
                result => {
                    scene_failures.insert(path, format!("{result:?}"));
                    continue;
                }
            };
            for (event_id, event) in scene
                .events
                .iter()
                .enumerate()
                .filter(|(_, event)| event.active() && event.kind == EventType::Speak)
            {
                speech_instances += 1;
                definition_events.insert((path.clone(), event_id));
                let cue = &event.parameters[0];
                let cue_key = cue.to_ascii_lowercase();
                if cues.insert(cue_key.clone()) {
                    if let Some(waves) = audio.names.get(&cue_key) {
                        registered_alternatives += waves.len();
                        registered_paths.extend(waves.iter().map(|wave| playback_path(&wave.path)));
                    } else if let Ok(paths) = audio.alternatives(&cue.as_str().into()) {
                        registered_alternatives += paths.len();
                        registered_paths.extend(paths);
                    } else {
                        cue_failures.insert(cue.clone(), "sound script absent".to_owned());
                    }
                }
                let actor_name = event
                    .actor
                    .and_then(|id| scene.actors.get(id))
                    .map(|actor| actor.name.as_str());
                let Some(request) = actor_name.and_then(|actor_name| {
                    controller.scene_sound_request(&world, entity_id, actor_name, usize::MAX, cue)
                }) else {
                    actor_failures.insert(
                        format!("{entity_id}:{path}:{event_id}:{cue}"),
                        actor_name.unwrap_or("<actor absent>").to_owned(),
                    );
                    continue;
                };
                resolved_actors += 1;
                match audio.alternatives(&request) {
                    Ok(paths) => {
                        let templates = audio.names.get(&cue_key).is_some_and(|waves| {
                            waves.iter().any(|wave| wave.gender != ActorGender::None)
                        });
                        if templates {
                            let actor = request.actor.as_ref().context("speech actor absent")?;
                            template_requests.push(serde_json::json!({"scene_entity":entity_id,"scene":path,"cue":cue,"actor":actor.name,"model":actor.model,"gender":format!("{:?}",audio.actor_gender(&request)?),"eligible_waves":paths}));
                        }
                        eligible_paths.extend(paths);
                    }
                    Err(error) => {
                        cue_failures.insert(cue.clone(), format!("{error:#}"));
                    }
                }
            }
        }
        let mut codecs = BTreeMap::<String, usize>::new();
        for path in &registered_paths {
            let result = (|| -> Result<&'static str> {
                let data = vfs
                    .read(&format!("sound/{path}"))?
                    .context("sound asset absent")?;
                let (decoded, codec) = playback_bytes(path, data)?;
                let wav = hound::WavReader::new(Cursor::new(decoded))?;
                if wav.duration() == 0 {
                    bail!("decoded voice has no frames");
                }
                Ok(codec)
            })();
            match result {
                Ok(codec) => {
                    *codecs.entry(codec.into()).or_default() += 1;
                }
                Err(error) => {
                    decode_failures.insert(path.clone(), format!("{error:#}"));
                }
            }
        }
        let report = serde_json::json!({
            "map":"d1_trainstation_01","actor_registry_entries":audio.actors.len(),
            "scene_references":references,"unique_scenes":scene_paths.len(),
            "active_speak_definitions":definition_events.len(),"active_speak_instances":speech_instances,
            "resolved_actor_instances":resolved_actors,"unique_cues":cues.len(),
            "registered_wave_alternatives":registered_alternatives,"registered_unique_files":registered_paths.len(),
            "eligible_unique_files":eligible_paths.len(),"decoded_files":codecs.values().sum::<usize>(),
            "codecs":codecs,"registry_errors":audio.errors,"scene_failures":scene_failures,
            "actor_failures":actor_failures,"cue_failures":cue_failures,"decode_failures":decode_failures,
            "template_requests":template_requests,
            "scope":"All active SPEAK definitions checked per scene entity through runtime actor lookup; every registered wave alternative decoded, including currently ineligible genders. No backend mixing, navigation or lip-sync verification."
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
        if !audio.errors.is_empty()
            || !scene_failures.is_empty()
            || !actor_failures.is_empty()
            || !cue_failures.is_empty()
            || !decode_failures.is_empty()
        {
            bail!("first-map speech census has explicit unresolved entries");
        }
        Ok(())
    }
}
