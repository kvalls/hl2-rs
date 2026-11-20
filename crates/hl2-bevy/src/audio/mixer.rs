//! One audio source for every game sound (DESIGN I). Voices read the 16-bit PCM of loaded
//! backend WAVs in place; the source mixes them a block at a time under one lock, so the
//! output thread's cost per sound is a few multiply-adds per frame instead of a chain of
//! library sources per sample.
use super::wav::BACKEND_RATE;
use bevy::audio::{ChannelCount, Decodable, Sample, SampleRate, Source};
use bevy::prelude::{Asset, Component, TypePath};
use std::num::NonZero;
use std::ops::Range;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// Frames mixed per lock: 256 at 48 kHz is 5.3 ms, about the library's control poll.
const BLOCK: usize = 256;
/// Stereo out; mono voices play on both channels.
const CHANNELS: usize = 2;

/// The 16-bit PCM of a backend WAV, read in place.
#[derive(Clone)]
pub struct Pcm {
    bytes: Arc<[u8]>,
    data: Range<usize>,
    channels: usize,
}

impl Pcm {
    /// The PCM of a 16-bit WAV at the backend rate with one or two channels.
    pub fn parse(bytes: Arc<[u8]>) -> Option<Self> {
        let u16_at = |at: usize| Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?));
        let u32_at = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
        if bytes.get(0..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
            return None;
        }
        let (mut at, mut channels, mut data) = (12, None, None);
        while at + 8 <= bytes.len() {
            let size = u32_at(at + 4)? as usize;
            let body = at + 8..(at + 8 + size).min(bytes.len());
            match bytes.get(at..at + 4)? {
                b"fmt " => {
                    let (format, count) = (u16_at(body.start)?, u16_at(body.start + 2)?);
                    let (rate, bits) = (u32_at(body.start + 4)?, u16_at(body.start + 14)?);
                    let usable = format == 1 && bits == 16 && rate == BACKEND_RATE;
                    channels = (usable && matches!(count, 1 | 2)).then_some(usize::from(count));
                }
                b"data" => data = Some(body),
                _ => {}
            }
            at += 8 + size + size % 2;
        }
        Some(Self {
            bytes,
            data: data?,
            channels: channels?,
        })
    }
    fn frames(&self) -> usize {
        self.data.len() / 2 / self.channels
    }
    fn sample(&self, frame: usize, channel: usize) -> f32 {
        let at = self.data.start + (frame * self.channels + channel.min(self.channels - 1)) * 2;
        f32::from(i16::from_le_bytes([self.bytes[at], self.bytes[at + 1]])) / 32768.
    }
}

struct VoiceState {
    pcm: Pcm,
    /// Source frames played (fractional while pitch-shifted).
    position: f64,
    speed: f32,
    /// The gain reached at the end of the last block, and the one asked for.
    gain: f32,
    target: f32,
    paused: bool,
    looped: bool,
    finished: bool,
}

impl VoiceState {
    /// Adds this voice's next block to `out`, ramping its gain across the block.
    fn mix_into(&mut self, out: &mut [f32]) {
        let frames = self.pcm.frames();
        let step = f64::from(self.speed.max(0.));
        if self.paused || self.finished || frames == 0 {
            return;
        }
        let (from, to) = (self.gain, self.target);
        self.gain = to;
        if from == 0. && to == 0. {
            // Silent: only the position moves on (the same end and loop points).
            self.position += step * BLOCK as f64;
            self.wrap_or_end(frames);
            return;
        }
        for (i, frame) in out.as_chunks_mut::<CHANNELS>().0.iter_mut().enumerate() {
            if self.position >= frames as f64 && !self.wrap_or_end(frames) {
                return;
            }
            let gain = from + (to - from) * (i + 1) as f32 / BLOCK as f32;
            let index = self.position as usize;
            let t = (self.position - index as f64) as f32;
            let next = if index + 1 < frames {
                index + 1
            } else if self.looped {
                0
            } else {
                index
            };
            for (channel, sample) in frame.iter_mut().enumerate() {
                let (a, b) = (
                    self.pcm.sample(index, channel),
                    self.pcm.sample(next, channel),
                );
                *sample += gain * (a + (b - a) * t);
            }
            self.position += step;
        }
        self.wrap_or_end(frames);
    }
    /// Past the end a looping voice starts over and a one-shot ends; false once ended.
    fn wrap_or_end(&mut self, frames: usize) -> bool {
        if self.position < frames as f64 {
            return true;
        }
        if self.looped {
            self.position %= frames as f64;
            return true;
        }
        self.finished = true;
        false
    }
}

#[derive(Default)]
struct Shared {
    voices: Vec<(u64, VoiceState)>,
    next_id: u64,
}

impl Shared {
    fn mix(&mut self, out: &mut [f32]) {
        out.fill(0.);
        for (_, voice) in &mut self.voices {
            voice.mix_into(out);
        }
    }
    fn voice(&mut self, id: u64) -> Option<&mut VoiceState> {
        self.voices
            .iter_mut()
            .find(|(v, _)| *v == id)
            .map(|(_, v)| v)
    }
}

/// The game's sound mixer; clones share it.
#[derive(Clone, Default, bevy::prelude::Resource)]
pub struct Mixer(Arc<Mutex<Shared>>);

/// How a new voice starts.
pub struct Start {
    pub looped: bool,
    pub gain: f32,
    pub speed: f32,
    pub paused: bool,
}

impl Mixer {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    pub fn play(&self, pcm: Pcm, start: Start) -> Voice {
        let mut shared = self.lock();
        let id = shared.next_id;
        shared.next_id += 1;
        let state = VoiceState {
            pcm,
            position: 0.,
            speed: start.speed,
            gain: start.gain,
            target: start.gain,
            paused: start.paused,
            looped: start.looped,
            finished: false,
        };
        shared.voices.push((id, state));
        Voice {
            id,
            mixer: self.clone(),
        }
    }
    /// Voices held (playing, paused or finished but not yet released).
    pub fn voices(&self) -> usize {
        self.lock().voices.len()
    }
}

/// One playing sound; dropping it (its entity despawning) stops it.
#[derive(Component)]
pub struct Voice {
    id: u64,
    mixer: Mixer,
}

impl Voice {
    fn with<T>(&self, f: impl FnOnce(&mut VoiceState) -> T) -> Option<T> {
        self.mixer.lock().voice(self.id).map(f)
    }
    pub fn set_gain(&self, gain: f32) {
        self.with(|v| v.target = gain);
    }
    pub fn gain(&self) -> f32 {
        self.with(|v| v.target).unwrap_or(0.)
    }
    pub fn set_speed(&self, speed: f32) {
        self.with(|v| v.speed = speed);
    }
    pub fn set_paused(&self, paused: bool) {
        self.with(|v| v.paused = paused);
    }
    pub fn is_paused(&self) -> bool {
        self.with(|v| v.paused).unwrap_or(false)
    }
    /// Source time played.
    pub fn position(&self) -> Duration {
        Duration::from_secs_f64(self.with(|v| v.position).unwrap_or(0.) / f64::from(BACKEND_RATE))
    }
    /// A one-shot that played to its end.
    pub fn finished(&self) -> bool {
        self.with(|v| v.finished).unwrap_or(true)
    }
}

impl Drop for Voice {
    fn drop(&mut self) {
        self.mixer.lock().voices.retain(|(id, _)| *id != self.id);
    }
}

/// The mixer as a playable asset: the host plays it once and it never ends.
#[derive(Asset, TypePath)]
pub struct MixerStream(pub Mixer);

impl Decodable for MixerStream {
    type Decoder = MixerSource;
    fn decoder(&self) -> MixerSource {
        MixerSource {
            mixer: self.0.clone(),
            block: vec![0.; BLOCK * CHANNELS],
            next: BLOCK * CHANNELS,
        }
    }
}

/// The mixed output, handed out sample by sample from blocks mixed under one lock.
pub struct MixerSource {
    mixer: Mixer,
    block: Vec<f32>,
    next: usize,
}

impl Iterator for MixerSource {
    type Item = Sample;
    fn next(&mut self) -> Option<Sample> {
        if self.next == self.block.len() {
            self.mixer.lock().mix(&mut self.block);
            self.next = 0;
        }
        self.next += 1;
        Some(self.block[self.next - 1])
    }
}

impl Source for MixerSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        NonZero::new(CHANNELS as u16).expect("two channels")
    }
    fn sample_rate(&self) -> SampleRate {
        NonZero::new(BACKEND_RATE).expect("backend rate")
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests;
