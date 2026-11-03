//! Bounded VSIF v2 scene cache and compiled BVCD v4 choreography reader.
//! Installed files are read in place; none of their contents are bundled here.
use crate::{bytes, f32le, i16le, u16le, u32le, vpk};
use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeMap,
    io::{Cursor, Write},
};

const MAX_CACHE: usize = 64 * 1024 * 1024;
const MAX_SCENE: usize = 16 * 1024 * 1024;
const MAX_EVENTS: usize = 65536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum EventType {
    Unspecified,
    Section,
    Expression,
    LookAt,
    MoveTo,
    Speak,
    Gesture,
    Sequence,
    Face,
    FireTrigger,
    FlexAnimation,
    SubScene,
    Loop,
    Interrupt,
    StopPoint,
    PermitResponses,
    Generic,
}
impl EventType {
    fn parse(value: u8) -> Result<Self> {
        use EventType::*;
        Ok(match value {
            0 => Unspecified,
            1 => Section,
            2 => Expression,
            3 => LookAt,
            4 => MoveTo,
            5 => Speak,
            6 => Gesture,
            7 => Sequence,
            8 => Face,
            9 => FireTrigger,
            10 => FlexAnimation,
            11 => SubScene,
            12 => Loop,
            13 => Interrupt,
            14 => StopPoint,
            15 => PermitResponses,
            16 => Generic,
            _ => bail!("unsupported choreography event type {value}"),
        })
    }
}

#[derive(Clone, Debug)]
pub struct Sample {
    pub time: f32,
    pub value: f32,
}
#[derive(Clone, Debug)]
pub struct Tag {
    pub name: String,
    pub value: f32,
}
#[derive(Clone, Debug)]
pub struct FlexSample {
    pub sample: Sample,
    pub curve_type: u16,
}
#[derive(Clone, Debug)]
pub struct FlexTrack {
    pub controller: String,
    pub flags: u8,
    pub min: f32,
    pub max: f32,
    pub samples: Vec<FlexSample>,
    pub combo_samples: Vec<FlexSample>,
}
impl FlexTrack {
    pub fn is_active(&self) -> bool {
        self.flags & 1 != 0
    }
    pub fn is_combo(&self) -> bool {
        self.flags & 2 != 0
    }
    /// GetZeroValue: balance tracks rest at 0.5; value tracks at the default edge zero
    /// (binary scenes carry no edge overrides).
    fn zero(&self, balance: bool) -> f32 {
        if balance {
            0.5
        } else if self.min != self.max {
            (0. - self.min) / (self.max - self.min)
        } else {
            0.
        }
    }
    fn samples(&self, balance: bool) -> &[FlexSample] {
        if balance {
            &self.combo_samples
        } else {
            &self.samples
        }
    }
    /// GetBoundedSample: virtual samples at 0 and at the event duration.
    fn bounded(&self, i: isize, balance: bool, duration: f32) -> (f32, f32, u16) {
        let samples = self.samples(balance);
        if i < 0 {
            (0., self.zero(balance), 0)
        } else if i as usize >= samples.len() {
            (duration, self.zero(balance), 0)
        } else {
            let s = &samples[i as usize];
            (s.sample.time, s.sample.value, s.curve_type)
        }
    }
    /// CFlexAnimationTrack::GetFracIntensity for an event-relative time.
    fn fraction(&self, time: f32, balance: bool, duration: f32) -> f32 {
        let count = self.samples(balance).len() as isize;
        if count < 1 {
            return self.zero(balance);
        }
        let mut j = (count / 2).max(1);
        let mut i = j;
        let (mut start, mut end) = (
            self.bounded(i, balance, duration),
            self.bounded(i + 1, balance, duration),
        );
        while i > -2 && i < count + 1 {
            start = self.bounded(i, balance, duration);
            end = self.bounded(i + 1, balance, duration);
            j = (j / 2).max(1);
            if time < start.0 {
                i -= j;
            } else if time > end.0 {
                i += j;
            } else {
                if time == end.0 {
                    i += 1;
                    start = self.bounded(i, balance, duration);
                    end = self.bounded(i + 1, balance, duration);
                }
                break;
            }
        }
        let pre = self.bounded((i - 1).max(-1), balance, duration);
        let next = self.bounded((i + 2).min(count), balance, duration);
        let dt = end.0 - start.0;
        let f = if dt > 0. {
            ((time - start.0) / dt).clamp(0., 1.)
        } else {
            0.
        };
        let v = |s: (f32, f32, u16)| glam::Vec2::new(s.0, s.1);
        let early = (start.2 >> 8) & 0xff;
        let late = end.2 & 0xff;
        let y = if early == HOLD {
            start.1
        } else if late == HOLD {
            end.1
        } else {
            let a = curve(late, v(pre), v(start), v(end), v(next), f);
            if early == late {
                a
            } else {
                let b = curve(early, v(pre), v(start), v(end), v(next), f);
                b + (a - b) * f
            }
        };
        y.clamp(0., 1.)
    }
    fn internal(&self, time: f32, start: f32, end: Option<f32>, balance: bool) -> f32 {
        let fraction = match end {
            Some(end) if time >= start && time <= end => {
                self.fraction(time - start, balance, end - start)
            }
            _ => self.zero(balance),
        };
        if !balance && self.min != self.max {
            fraction * (self.max - self.min) + self.min
        } else {
            fraction
        }
    }
    /// CFlexAnimationTrack::GetIntensity at scene time; `side` 0 maps to the `right_`
    /// controller and 1 to `left_` for combo (stereo) tracks.
    pub fn intensity(&self, time: f32, start: f32, end: Option<f32>, side: usize) -> f32 {
        let magnitude = self.internal(time, start, end, false);
        let mut scale = 1.;
        if self.is_combo() {
            let balance = self.internal(time, start, end, true);
            if side == 0 && balance > 0.5 {
                scale = (1. - balance) / 0.5;
            } else if side == 1 && balance < 0.5 {
                scale = balance / 0.5;
            }
        }
        magnitude * scale
    }
}
const HOLD: u16 = 15;
/// Interpolator_CurveInterpolate y value for the supported interpolators; unsupported
/// kinds (B-spline, Kochanek-Bartels, simple cubic, exponential decay) use the default.
fn curve(
    kind: u16,
    pre: glam::Vec2,
    start: glam::Vec2,
    end: glam::Vec2,
    next: glam::Vec2,
    f: f32,
) -> f32 {
    use std::f32::consts::PI;
    match kind {
        2 => start.y + (end.y - start.y) * (PI * f * 0.5).sin(),
        3 => start.y + (end.y - start.y) * (1. - (PI * f * 0.5 + 0.5 * PI).sin()),
        4 => {
            let s = f * f * (3. - 2. * f);
            start.y + (end.y - start.y) * s
        }
        6 => start.y + (end.y - start.y) * f,
        _ => catmull_rom_normalize_x(pre, start, end, next, f).y,
    }
}
#[derive(Clone, Debug)]
pub struct Speech {
    pub caption_type: i8,
    pub caption_token: String,
    pub flags: u8,
}
#[derive(Clone, Debug)]
pub struct Event {
    pub kind: EventType,
    pub name: String,
    pub start: f32,
    pub end: Option<f32>,
    pub parameters: [String; 3],
    pub ramp: Vec<Sample>,
    /// Resume condition, lock body facing, fixed length, active, short move, play over script.
    pub flags: u8,
    pub distance: f32,
    pub relative_tags: Vec<Tag>,
    pub timing_tags: Vec<Tag>,
    pub absolute_tags: [Vec<Tag>; 2],
    pub gesture_duration: Option<f32>,
    pub relative_reference: Option<(String, String)>,
    pub flex_tracks: Vec<FlexTrack>,
    pub loop_count: Option<i8>,
    pub speech: Option<Speech>,
    pub actor: Option<usize>,
    pub channel: Option<usize>,
    /// Activity of the containing actor and channel; the event itself also needs bit 3.
    pub enabled: bool,
}
impl Event {
    pub fn active(&self) -> bool {
        self.enabled && self.flags & 8 != 0
    }
    pub fn resume_condition(&self) -> bool {
        self.flags & 1 != 0
    }
    pub fn lock_body_facing(&self) -> bool {
        self.flags & 2 != 0
    }
    /// Events whose start puts an NPC actor in SCHED_SCENE_GENERIC
    /// (CBaseFlex::EnterSceneSequence: sequences, moves, faces, look-ats that do not lock
    /// body facing, and the generic AI events); speech, expressions, flex animation and
    /// triggers leave its AI running. Gestures are counted too (conservative: ours play
    /// them on the scene's animation path).
    pub fn takes_npc_ai(&self) -> bool {
        match self.kind {
            EventType::Sequence
            | EventType::MoveTo
            | EventType::Face
            | EventType::Gesture
            | EventType::Generic => true,
            EventType::LookAt => !self.lock_body_facing(),
            _ => false,
        }
    }
    /// Compiled BVCD ramps restore the default normalized-X Catmull-Rom curve.
    /// Text VCD edge/curve overrides are not represented by this binary format.
    pub fn intensity(&self, scene: &ChoreoScene, time: f32) -> f32 {
        self.end.map_or(0., |end| {
            compiled_ramp(&self.ramp, end - self.start, time - self.start)
                * compiled_ramp(&scene.ramp, scene.stop_time(), time)
        })
    }
}
/// Evaluate a compiled scene/event ramp with implicit zero-valued boundary samples.
/// Neighbor time intervals are normalized before Catmull-Rom; output is clamped.
pub fn compiled_ramp(samples: &[Sample], duration: f32, time: f32) -> f32 {
    if !time.is_finite() || !duration.is_finite() {
        return 0.;
    }
    if samples.is_empty() {
        return 1.;
    }
    // A real sample at t=0 supersedes the implicit zero boundary. The native
    // span search starts from a real sample; avoid a zero-width virtual segment.
    if samples[0].time == time {
        return samples[0].value.clamp(0., 1.);
    }
    let right = samples.partition_point(|s| s.time < time);
    let bounded = |index: isize| {
        if index < 0 {
            (0., 0.)
        } else {
            samples
                .get(index as usize)
                .map_or((duration, 0.), |s| (s.time, s.value))
        }
    };
    let (start_t, start) = bounded(right as isize - 1);
    let (end_t, end) = bounded(right as isize);
    let (mut pre_t, mut pre) = bounded(right as isize - 2);
    let (mut next_t, mut next) = bounded(right as isize + 1);
    if right < 2 {
        pre_t = start_t;
    }
    if right + 1 >= samples.len() {
        next_t = end_t;
    }
    let dt = end_t - start_t;
    if dt != 0. {
        if pre_t != start_t {
            pre = start + (pre - start) * dt / (start_t - pre_t);
        }
        if next_t != end_t {
            next = end + (next - end) * dt / (next_t - end_t);
        }
    }
    let t = if dt > 0. {
        ((time - start_t) / dt).clamp(0., 1.)
    } else {
        0.
    };
    let y = start
        + 0.5 * (end - pre) * t
        + 0.5 * (2. * pre - 5. * start + 4. * end - next) * t * t
        + 0.5 * (-pre + 3. * start - 3. * end + next) * t * t * t;
    y.clamp(0., 1.)
}
/// First tag index with this name (case-insensitive), as CChoreoEvent::FindAbsoluteTag.
pub fn find_tag(tags: &[Tag], name: &str) -> Option<usize> {
    tags.iter().position(|t| t.name.eq_ignore_ascii_case(name))
}
/// SDK CChoreoEvent::GetOriginalPercentageFromPlaybackPercentage for gesture events.
/// `playback`/`original` are the absolute tag lists; `linear[i]` marks playback tag i.
pub fn gesture_original_percentage(
    playback: &[Tag],
    original: &[Tag],
    linear: &[bool],
    t: f32,
) -> f32 {
    let count = playback.len() as isize;
    if count as usize != original.len() || count == 0 {
        return t;
    }
    if t <= 0. {
        return 0.;
    }
    let bounded = |tags: &[Tag], i: isize| {
        if i < 0 {
            0.
        } else if i >= count {
            1.
        } else {
            tags[i as usize].value
        }
    };
    let (mut s, mut n) = (0., 0.);
    let mut i = -1;
    while i < count {
        s = bounded(playback, i);
        n = bounded(playback, i + 1);
        if t >= s && t <= n {
            break;
        }
        i += 1;
    }
    let prev = (i - 1).max(-2);
    let start = i.max(-1);
    let end = (i + 1).min(count);
    let next = (i + 2).min(count + 1);
    let is_tag = |index: isize| index >= 0 && index < count;
    let is_linear = |index: isize| is_tag(index) && linear.get(index as usize) == Some(&true);
    if is_tag(start) && is_tag(end) && is_linear(start) && is_linear(end) {
        let (ps, pe) = (playback[start as usize].value, playback[end as usize].value);
        let f = (t - ps) / (pe - ps);
        return (1. - f) * original[start as usize].value + f * original[end as usize].value;
    }
    let point = |index: isize| glam::Vec2::new(bounded(playback, index), bounded(original, index));
    let (mut pre, p_start, p_end, mut post) = (point(prev), point(start), point(end), point(next));
    if is_linear(start) {
        pre = p_start - (p_end - p_start);
    }
    if is_linear(end) {
        post = p_end + (p_end - p_start);
    }
    let dt = n - s;
    let f = if dt > 0. { (t - s) / dt } else { 0. }.clamp(0., 1.);
    catmull_rom_normalize_x(pre, p_start, p_end, post, f).y
}
/// mathlib Catmull_Rom_Spline_NormalizeX: neighbor points rescaled to the segment's x span.
pub fn catmull_rom_normalize_x(
    p1: glam::Vec2,
    p2: glam::Vec2,
    p3: glam::Vec2,
    p4: glam::Vec2,
    t: f32,
) -> glam::Vec2 {
    let dt = p3.x - p2.x;
    let (mut p1n, mut p4n) = (p1, p4);
    if dt != 0. {
        if p1.x != p2.x {
            p1n = p2.lerp(p1, dt / (p2.x - p1.x));
        }
        if p4.x != p3.x {
            p4n = p3.lerp(p4, dt / (p4.x - p3.x));
        }
    }
    let (t2, t3) = (t * t, t * t * t);
    0.5 * ((-p1n + 3. * p2 - 3. * p3 + p4n) * t3
        + (2. * p1n - 5. * p2 + 4. * p3 - p4n) * t2
        + (-p1n + p3) * t)
        + p2
}
#[derive(Clone, Debug)]
pub struct Channel {
    pub name: String,
    pub active: bool,
    pub events: Vec<usize>,
}
#[derive(Clone, Debug)]
pub struct Actor {
    pub name: String,
    pub active: bool,
    pub channels: Vec<Channel>,
}
#[derive(Clone, Debug)]
pub struct ChoreoScene {
    pub text_crc32: u32,
    pub actors: Vec<Actor>,
    /// Original serialized order, including events from inactive actors/channels.
    pub events: Vec<Event>,
    pub ramp: Vec<Sample>,
    pub ignore_phonemes: bool,
}
impl ChoreoScene {
    pub fn parse(data: &[u8], strings: &[String]) -> Result<Self> {
        if data.len() > MAX_SCENE {
            bail!("compiled scene exceeds 16 MiB");
        }
        let mut r = Reader {
            data,
            at: 0,
            strings,
            copied_strings: 0,
        };
        if r.take(4)? != b"bvcd" || r.byte()? != 4 {
            bail!("expected compiled BVCD version 4");
        }
        let text_crc32 = r.uint()?;
        let count = r.byte()? as usize;
        let mut events = Vec::new();
        for _ in 0..count {
            events.push(r.event(None, None)?);
        }
        let count = r.byte()? as usize;
        let mut actors = Vec::new();
        for actor in 0..count {
            let name = r.string()?;
            let channel_count = r.byte()? as usize;
            let mut channels = Vec::new();
            for channel in 0..channel_count {
                let name = r.string()?;
                let event_count = r.byte()? as usize;
                let first = events.len();
                if first + event_count > MAX_EVENTS {
                    bail!("scene exceeds event limit");
                }
                for _ in 0..event_count {
                    events.push(r.event(Some(actor), Some(channel))?);
                }
                let active = r.boolean()?;
                for e in &mut events[first..] {
                    e.enabled = active;
                }
                channels.push(Channel {
                    name,
                    active,
                    events: (first..events.len()).collect(),
                });
            }
            let active = r.boolean()?;
            if !active {
                for c in &channels {
                    for id in &c.events {
                        events[*id].enabled = false;
                    }
                }
            }
            actors.push(Actor {
                name,
                active,
                channels,
            });
        }
        let ramp = r.curve()?;
        let ignore_phonemes = r.boolean()?;
        if r.at != data.len() {
            bail!("compiled scene has {} trailing bytes", data.len() - r.at);
        }
        Ok(Self {
            text_crc32,
            actors,
            events,
            ramp,
            ignore_phonemes,
        })
    }
    /// Source's stop-time calculation includes all events, even inactive ones.
    pub fn stop_time(&self) -> f32 {
        self.events
            .iter()
            .map(|e| e.end.unwrap_or(e.start))
            .fold(0., f32::max)
    }
}

#[derive(Clone, Debug)]
pub struct Summary {
    pub duration_msec: u32,
    pub sounds: Vec<String>,
}
#[derive(Clone, Debug)]
struct Entry {
    offset: usize,
    length: usize,
    summary: Summary,
}
pub struct Cache {
    data: Vec<u8>,
    strings: Vec<String>,
    entries: BTreeMap<u32, Entry>,
}
impl Cache {
    pub fn parse(data: Vec<u8>) -> Result<Self> {
        if data.len() > MAX_CACHE {
            bail!("scene cache exceeds 64 MiB");
        }
        if bytes(&data, 0, 4)? != b"VSIF" || u32le(&data, 4)? != 2 {
            bail!("expected VSIF scene cache version 2");
        }
        let count = u32le(&data, 8)? as usize;
        let string_count = u32le(&data, 12)? as usize;
        if count > 65536 || string_count > 32768 {
            bail!("scene cache counts exceed limits");
        }
        let directory = u32le(&data, 16)? as usize;
        let string_start = 20 + string_count * 4;
        if directory < string_start {
            bail!("scene directory overlaps string table");
        }
        bytes(&data, 20, string_count * 4)?;
        let mut strings: Vec<String> = Vec::new();
        let mut copied_strings = 0usize;
        for i in 0..string_count {
            let offset = u32le(&data, 20 + i * 4)? as usize;
            if offset < string_start || offset >= directory {
                bail!("scene string offset outside pool");
            }
            let tail = bytes(&data, offset, directory - offset)?;
            let length = tail
                .iter()
                .position(|v| *v == 0)
                .context("unterminated scene string")?;
            if length > 4096 {
                bail!("scene string exceeds length limit");
            }
            copied_strings += length + std::mem::size_of::<String>();
            if copied_strings > MAX_CACHE {
                bail!("decoded scene cache string budget exceeded");
            }
            strings.push(String::from_utf8_lossy(&tail[..length]).into_owned());
        }
        let table = bytes(&data, directory, count * 16)?;
        let after_table = directory + table.len();
        let mut entries = BTreeMap::new();
        let mut previous = None;
        for row in table.as_chunks::<16>().0 {
            let crc = u32le(row, 0)?;
            if previous.is_some_and(|old| crc <= old) {
                bail!("scene directory is not strictly sorted");
            }
            previous = Some(crc);
            let offset = u32le(row, 4)? as usize;
            let length = u32le(row, 8)? as usize;
            let summary_at = u32le(row, 12)? as usize;
            if offset < after_table || summary_at < after_table || length > MAX_SCENE {
                bail!("invalid scene data range");
            }
            bytes(&data, offset, length)?;
            let duration_msec = u32le(&data, summary_at)?;
            let sounds = u32le(&data, summary_at + 4)? as usize;
            if sounds > 65536 {
                bail!("scene sound count exceeds limit");
            }
            let mut sound_names = Vec::new();
            for raw in bytes(&data, summary_at + 8, sounds * 4)?.as_chunks::<4>().0 {
                let id = u32le(raw, 0)? as usize;
                let name = strings.get(id).context("scene summary string index")?;
                copied_strings += name.len() + std::mem::size_of::<String>();
                if copied_strings > MAX_CACHE {
                    bail!("decoded scene cache string budget exceeded");
                }
                sound_names.push(name.clone());
            }
            entries.insert(
                crc,
                Entry {
                    offset,
                    length,
                    summary: Summary {
                        duration_msec,
                        sounds: sound_names,
                    },
                },
            );
        }
        Ok(Self {
            data,
            strings,
            entries,
        })
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn string_count(&self) -> usize {
        self.strings.len()
    }
    pub fn filename_crc(path: &str) -> Result<u32> {
        let path = vpk::normalize(path)?;
        if !path.starts_with("scenes/") || !path.ends_with(".vcd") || path.contains('\0') {
            bail!("invalid scene filename");
        }
        Ok(crc32fast::hash(path.replace('/', "\\").as_bytes()))
    }
    pub fn scene(&self, path: &str) -> Result<Option<ChoreoScene>> {
        self.scene_by_crc(Self::filename_crc(path)?)
    }
    pub fn summary(&self, path: &str) -> Result<Option<&Summary>> {
        Ok(self
            .entries
            .get(&Self::filename_crc(path)?)
            .map(|e| &e.summary))
    }
    /// Allows private audits without requiring names that the cache does not store.
    pub fn checksums(&self) -> impl Iterator<Item = u32> + '_ {
        self.entries.keys().copied()
    }
    pub fn scene_by_crc(&self, crc: u32) -> Result<Option<ChoreoScene>> {
        let Some(entry) = self.entries.get(&crc) else {
            return Ok(None);
        };
        let raw = bytes(&self.data, entry.offset, entry.length)?;
        let data = decompress(raw).with_context(|| format!("scene {crc:08x}"))?;
        ChoreoScene::parse(&data, &self.strings)
            .with_context(|| format!("scene {crc:08x}"))
            .map(Some)
    }
}
fn decompress(raw: &[u8]) -> Result<Vec<u8>> {
    if !raw.starts_with(b"LZMA") {
        return Ok(raw.to_vec());
    }
    let expected = u32le(raw, 4)? as usize;
    let packed = u32le(raw, 8)? as usize;
    if expected > MAX_SCENE || packed != raw.len().saturating_sub(17) {
        bail!("invalid scene LZMA sizes");
    }
    let mut stream = bytes(raw, 12, 5)?.to_vec();
    stream.extend_from_slice(&(expected as u64).to_le_bytes());
    stream.extend_from_slice(bytes(raw, 17, packed)?);
    let mut output = LimitedOutput {
        data: Vec::new(),
        limit: expected,
    };
    lzma_rs::lzma_decompress_with_options(
        &mut Cursor::new(stream),
        &mut output,
        &lzma_rs::decompress::Options {
            memlimit: Some(64 * 1024 * 1024),
            ..Default::default()
        },
    )?;
    if output.data.len() != expected {
        bail!("scene LZMA decoded length mismatch");
    }
    Ok(output.data)
}
struct LimitedOutput {
    data: Vec<u8>,
    limit: usize,
}
impl Write for LimitedOutput {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if data.len() > self.limit.saturating_sub(self.data.len()) {
            return Err(std::io::Error::other("scene output exceeds declared size"));
        }
        self.data.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
    strings: &'a [String],
    copied_strings: usize,
}
impl Reader<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8]> {
        let v = bytes(self.data, self.at, count)?;
        self.at += count;
        Ok(v)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn boolean(&mut self) -> Result<bool> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => bail!("invalid scene boolean"),
        }
    }
    fn short(&mut self) -> Result<i16> {
        i16le(self.take(2)?, 0)
    }
    fn ushort(&mut self) -> Result<u16> {
        u16le(self.take(2)?, 0)
    }
    fn uint(&mut self) -> Result<u32> {
        u32le(self.take(4)?, 0)
    }
    fn float(&mut self) -> Result<f32> {
        f32le(self.take(4)?, 0)
    }
    fn string(&mut self) -> Result<String> {
        let id = usize::try_from(self.short()?).context("negative scene string index")?;
        let value = self
            .strings
            .get(id)
            .context("scene string index outside pool")?;
        if value.len() > 4096 {
            bail!("scene string exceeds length limit");
        }
        self.copied_strings += value.len() + std::mem::size_of::<String>();
        if self.copied_strings > MAX_SCENE {
            bail!("decoded scene string budget exceeded");
        }
        Ok(value.clone())
    }
    fn curve(&mut self) -> Result<Vec<Sample>> {
        let count = self.byte()?;
        (0..count)
            .map(|_| {
                Ok(Sample {
                    time: self.float()?,
                    value: self.byte()? as f32 / 255.,
                })
            })
            .collect()
    }
    fn tags(&mut self, absolute: bool) -> Result<Vec<Tag>> {
        let count = self.byte()?;
        (0..count)
            .map(|_| {
                Ok(Tag {
                    name: self.string()?,
                    value: if absolute {
                        self.ushort()? as f32 / 4096.
                    } else {
                        self.byte()? as f32 / 255.
                    },
                })
            })
            .collect()
    }
    fn flex_samples(&mut self, count: usize) -> Result<Vec<FlexSample>> {
        bytes(self.data, self.at, count * 7)?;
        (0..count)
            .map(|_| {
                Ok(FlexSample {
                    sample: Sample {
                        time: self.float()?,
                        value: self.byte()? as f32 / 255.,
                    },
                    curve_type: self.ushort()?,
                })
            })
            .collect()
    }
    fn event(&mut self, actor: Option<usize>, channel: Option<usize>) -> Result<Event> {
        let kind = EventType::parse(self.byte()?)?;
        let name = self.string()?;
        let start = self.float()?;
        let end = self.float()?;
        // Retail scenes include negative starts and inverted editor ranges.
        // Preserve finite authored times, rather than imposing an editor policy.
        let parameters = [self.string()?, self.string()?, self.string()?];
        let ramp = self.curve()?;
        let flags = self.byte()?;
        let distance = self.float()?;
        let relative_tags = self.tags(false)?;
        let timing_tags = self.tags(false)?;
        let absolute_tags = [self.tags(true)?, self.tags(true)?];
        let gesture_duration = if kind == EventType::Gesture {
            let value = self.float()?;
            if value == -1. {
                None
            } else {
                Some(value)
            }
        } else {
            None
        };
        let relative_reference = if self.boolean()? {
            Some((self.string()?, self.string()?))
        } else {
            None
        };
        let track_count = self.byte()?;
        let mut flex_tracks = Vec::new();
        for _ in 0..track_count {
            let controller = self.string()?;
            let flags = self.byte()?;
            let min = self.float()?;
            let max = self.float()?;
            let count = usize::try_from(self.short()?).context("negative flex sample count")?;
            let samples = self.flex_samples(count)?;
            let combo_samples = if flags & 2 != 0 {
                let count = self.ushort()? as usize;
                self.flex_samples(count)?
            } else {
                Vec::new()
            };
            flex_tracks.push(FlexTrack {
                controller,
                flags,
                min,
                max,
                samples,
                combo_samples,
            });
        }
        let loop_count = if kind == EventType::Loop {
            Some(self.byte()? as i8)
        } else {
            None
        };
        let speech = if kind == EventType::Speak {
            Some(Speech {
                caption_type: self.byte()? as i8,
                caption_token: self.string()?,
                flags: self.byte()?,
            })
        } else {
            None
        };
        Ok(Event {
            kind,
            name,
            start,
            end: if end == -1. { None } else { Some(end) },
            parameters,
            ramp,
            flags,
            distance,
            relative_tags,
            timing_tags,
            absolute_tags,
            gesture_duration,
            relative_reference,
            flex_tracks,
            loop_count,
            speech,
            actor,
            channel,
            enabled: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(out: &mut Vec<u8>, kind: u8) {
        out.push(kind);
        out.extend(0i16.to_le_bytes());
        out.extend(1f32.to_le_bytes());
        out.extend(2f32.to_le_bytes());
        for _ in 0..3 {
            out.extend(0i16.to_le_bytes());
        }
        out.extend([0, 8]);
        out.extend(0f32.to_le_bytes());
        out.extend([0, 0, 0, 0]);
        if kind == 6 {
            out.extend(3f32.to_le_bytes());
        }
        out.extend([0, 0]);
        if kind == 12 {
            out.push(255);
        }
        if kind == 5 {
            out.push(0);
            out.extend(0i16.to_le_bytes());
            out.push(5);
        }
    }
    fn binary() -> Vec<u8> {
        let mut b = b"bvcd\x04".to_vec();
        b.extend(123u32.to_le_bytes());
        b.push(17);
        for kind in 0..17 {
            event(&mut b, kind);
        }
        b.extend([1]);
        b.extend(0i16.to_le_bytes());
        b.push(1);
        b.extend(0i16.to_le_bytes());
        b.push(1);
        event(&mut b, 9);
        b.extend([0, 1, 0, 1]);
        b
    }
    fn cache(blob: &[u8]) -> Vec<u8> {
        let mut b = b"VSIF".to_vec();
        for v in [2u32, 1, 1, 26, 24] {
            b.extend(v.to_le_bytes());
        }
        b.extend(b"s\0");
        for v in [
            Cache::filename_crc("scenes/test.vcd").unwrap(),
            50,
            blob.len() as u32,
            42,
        ] {
            b.extend(v.to_le_bytes());
        }
        b.extend(2000u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend(blob);
        b
    }
    fn tags(values: &[(&str, f32)]) -> Vec<Tag> {
        values
            .iter()
            .map(|(name, value)| Tag {
                name: (*name).into(),
                value: *value,
            })
            .collect()
    }
    #[test]
    fn gesture_retiming_follows_sdk_tag_segments() {
        let playback = tags(&[("apex", 0.25), ("loop", 0.5), ("end", 0.8)]);
        let original = tags(&[("apex", 0.2), ("loop", 0.3), ("end", 0.6)]);
        // Untagged, mismatched and nonpositive inputs.
        assert_eq!(gesture_original_percentage(&[], &[], &[], 0.4), 0.4);
        assert_eq!(
            gesture_original_percentage(&playback, &original[..2], &[], 0.4),
            0.4
        );
        assert_eq!(
            gesture_original_percentage(&playback, &original, &[], -0.1),
            0.
        );
        // Linear loop/end segment interpolates original tags directly.
        let linear = [false, true, true];
        let mid = gesture_original_percentage(&playback, &original, &linear, 0.65);
        assert!((mid - 0.45).abs() < 1e-6);
        // Tags land on their original percentages and the end maps to one.
        for (t, want) in [(0.25, 0.2), (0.5, 0.3), (0.8, 0.6), (1., 1.)] {
            let got = gesture_original_percentage(&playback, &original, &linear, t);
            assert!((got - want).abs() < 1e-5, "{t}: {got}");
        }
        // Identical timelines stay on the identity line between real tags. The SDK's
        // virtual boundary points coincide with 0 and 1, so the edge segments ease.
        for i in 5..=16 {
            let t = i as f32 / 20.;
            let got = gesture_original_percentage(&playback, &playback, &[], t);
            assert!((got - t).abs() < 1e-5, "{t}: {got}");
        }
        let eased = gesture_original_percentage(&playback, &playback, &[], 0.05);
        assert!(eased > 0. && eased < 0.05);
        assert_eq!(find_tag(&playback, "LOOP"), Some(1));
        assert_eq!(find_tag(&playback, "missing"), None);
    }
    #[test]
    fn compiled_ramps_normalize_irregular_times_and_zero_edges() {
        let samples = [(0., 0.2), (0.25, 0.4), (1.25, 0.6), (2., 0.8)]
            .into_iter()
            .map(|(time, value)| Sample { time, value })
            .collect::<Vec<_>>();
        // Cubic interpolation at one quarter of the interval is 0.490625; linear is 0.45.
        assert!((compiled_ramp(&samples, 2., 0.5) - 0.490625).abs() < 1e-6);
        let edges = [
            Sample {
                time: 0.25,
                value: 1.,
            },
            Sample {
                time: 0.75,
                value: 1.,
            },
        ];
        assert_eq!(compiled_ramp(&edges, 1., 0.), 0.);
        assert_eq!(compiled_ramp(&edges, 1., 1.), 0.);
        assert!((compiled_ramp(&edges, 1., 0.0625) - 0.203125).abs() < 1e-6);
        assert_eq!(compiled_ramp(&edges, 1., 0.5), 1.);
        assert_eq!(compiled_ramp(&[], 1., 0.5), 1.);
        assert_eq!(
            compiled_ramp(
                &[
                    Sample {
                        time: 0.,
                        value: 1.
                    },
                    Sample {
                        time: 1.,
                        value: 1.
                    }
                ],
                1.,
                0.
            ),
            1.
        );
    }
    #[test]
    fn event_intensity_multiplies_scene_ramp_and_requires_an_end() {
        let mut scene = ChoreoScene::parse(&binary(), &["s".into()]).unwrap();
        scene.ramp = vec![
            Sample {
                time: 0.,
                value: 0.5,
            },
            Sample {
                time: 2.,
                value: 0.5,
            },
        ];
        scene.events[0].start = 0.;
        scene.events[0].end = Some(2.);
        scene.events[0].ramp = vec![
            Sample {
                time: 0.,
                value: 0.8,
            },
            Sample {
                time: 2.,
                value: 0.8,
            },
        ];
        // Each constant segment overshoots to 1.125 times its control value at its midpoint.
        assert!((scene.events[0].intensity(&scene, 1.) - 0.50625).abs() < 1e-6);
        scene.events[0].end = None;
        assert_eq!(scene.events[0].intensity(&scene, 1.), 0.);
    }
    #[test]
    fn all_event_types_and_inactive_channel_are_preserved() {
        let scene = ChoreoScene::parse(&binary(), &["s".into()]).unwrap();
        assert_eq!(scene.events.len(), 18);
        assert_eq!(scene.events[5].speech.as_ref().unwrap().flags, 5);
        assert_eq!(scene.events[6].gesture_duration, Some(3.));
        assert_eq!(scene.events[12].loop_count, Some(-1));
        assert_eq!(scene.events[17].actor, Some(0));
        assert!(!scene.events[17].active());
        assert_eq!(scene.stop_time(), 2.);
        assert!(scene.ignore_phonemes);
    }
    #[test]
    fn rich_gesture_tags_flex_tracks_and_relative_reference_keep_binary_alignment() {
        let strings: Vec<String> = (0..11).map(|i| format!("field{i}")).collect();
        let mut b = b"bvcd\x04".to_vec();
        b.extend(0u32.to_le_bytes());
        b.extend([1, 6]);
        b.extend(0i16.to_le_bytes());
        b.extend((-0.25f32).to_le_bytes());
        b.extend(2f32.to_le_bytes());
        for id in 1i16..=3 {
            b.extend(id.to_le_bytes());
        }
        b.push(1);
        b.extend(0.5f32.to_le_bytes());
        b.extend([128, 41]);
        b.extend(4.5f32.to_le_bytes());
        for (id, value) in [(4i16, 64u8), (5, 255)] {
            b.push(1);
            b.extend(id.to_le_bytes());
            b.push(value);
        }
        for (id, value) in [(6i16, 6144u16), (7, 2048)] {
            b.push(1);
            b.extend(id.to_le_bytes());
            b.extend(value.to_le_bytes());
        }
        b.extend(3f32.to_le_bytes());
        b.push(1);
        b.extend(8i16.to_le_bytes());
        b.extend(9i16.to_le_bytes());
        b.push(1);
        b.extend(10i16.to_le_bytes());
        b.push(3);
        b.extend((-1f32).to_le_bytes());
        b.extend(1f32.to_le_bytes());
        b.extend(1i16.to_le_bytes());
        b.extend(0.75f32.to_le_bytes());
        b.push(192);
        b.extend(0x1234u16.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(1.25f32.to_le_bytes());
        b.push(128);
        b.extend(0x4321u16.to_le_bytes());
        b.extend([0, 1]);
        b.extend(0.5f32.to_le_bytes());
        b.extend([200, 1]);
        let scene = ChoreoScene::parse(&b, &strings).unwrap();
        let e = &scene.events[0];
        assert_eq!(e.start, -0.25);
        assert_eq!(e.parameters, ["field1", "field2", "field3"]);
        assert!(e.active() && e.resume_condition());
        assert_eq!(e.ramp[0].value, 128. / 255.);
        assert_eq!(e.relative_tags[0].name, "field4");
        assert_eq!(e.relative_tags[0].value, 64. / 255.);
        assert_eq!(e.timing_tags[0].value, 1.);
        assert_eq!(e.absolute_tags[0][0].value, 1.5);
        assert_eq!(e.absolute_tags[1][0].value, 0.5);
        assert_eq!(e.gesture_duration, Some(3.));
        assert_eq!(
            e.relative_reference,
            Some(("field8".into(), "field9".into()))
        );
        let f = &e.flex_tracks[0];
        assert_eq!(f.controller, "field10");
        assert_eq!((f.min, f.max), (-1., 1.));
        assert_eq!(f.samples[0].curve_type, 0x1234);
        assert_eq!(f.combo_samples[0].sample.time, 1.25);
        assert_eq!(f.combo_samples[0].curve_type, 0x4321);
        assert_eq!(scene.ramp[0].value, 200. / 255.);
        assert!(scene.ignore_phonemes);
        for end in 0..b.len() {
            assert!(
                ChoreoScene::parse(&b[..end], &strings).is_err(),
                "rich cut at {end}"
            );
        }
        assert!(ChoreoScene::parse(&b, &vec!["x".repeat(4097); 11]).is_err());
    }
    #[test]
    fn truncated_payloads_unknown_versions_and_strings_are_rejected() {
        let b = binary();
        let strings = ["s".into()];
        for end in 0..b.len() {
            assert!(
                ChoreoScene::parse(&b[..end], &strings).is_err(),
                "cut at {end}"
            );
        }
        let mut bad = b.clone();
        bad.push(0);
        assert!(ChoreoScene::parse(&bad, &strings).is_err());
        let mut bad = b.clone();
        bad[4] = 5;
        assert!(ChoreoScene::parse(&bad, &strings).is_err());
        let mut bad = b.clone();
        bad[10] = 17;
        assert!(ChoreoScene::parse(&bad, &strings).is_err());
        let mut bad = b;
        bad[11..13].copy_from_slice(&(-1i16).to_le_bytes());
        assert!(ChoreoScene::parse(&bad, &strings).is_err());
    }
    #[test]
    fn cache_lookup_normalizes_names_and_lzma_obeys_bounds() {
        let original = binary();
        let c = Cache::parse(cache(&original)).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c.string_count(), 1);
        assert!(c.scene("SCENES\\TEST.VCD").unwrap().is_some());
        assert!(c.scene("scenes/missing.vcd").unwrap().is_none());
        assert!(c.scene("scenes/../test.vcd").is_err());
        let mut packed = Vec::new();
        lzma_rs::lzma_compress(&mut Cursor::new(&original), &mut packed).unwrap();
        let mut valve = b"LZMA".to_vec();
        valve.extend((original.len() as u32).to_le_bytes());
        valve.extend(((packed.len() - 13) as u32).to_le_bytes());
        valve.extend(&packed[..5]);
        valve.extend(&packed[13..]);
        assert_eq!(
            Cache::parse(cache(&valve))
                .unwrap()
                .scene("scenes/test.vcd")
                .unwrap()
                .unwrap()
                .events
                .len(),
            18
        );
        valve[4..8].copy_from_slice(&((MAX_SCENE + 1) as u32).to_le_bytes());
        assert!(decompress(&valve).is_err());
        let mut bad = cache(&original);
        bad[16..20].copy_from_slice(&0u32.to_le_bytes());
        assert!(Cache::parse(bad).is_err());
    }
    #[test]
    fn scene_cache_tolerates_non_utf8_strings() {
        let original = binary();
        let mut b = b"VSIF".to_vec();
        // counts: version 2, entries 1, string_count 1, directory 37, string 0 offset 24
        for v in [2u32, 1, 1, 37, 24] {
            b.extend(v.to_le_bytes());
        }
        // Non-UTF-8 sequence at index 10: 10 ASCII bytes followed by 0xFF, 0xFE, and NUL (13 bytes total)
        b.extend(b"0123456789\xFF\xFE\0");
        // directory table at 37: 16 bytes -> 37 + 16 = 53
        for v in [
            Cache::filename_crc("scenes/test.vcd").unwrap(),
            61,
            original.len() as u32,
            53,
        ] {
            b.extend(v.to_le_bytes());
        }
        // summary at 53: 8 bytes (duration 2000, 0 sounds) -> 53 + 8 = 61
        b.extend(2000u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        // payload at 61
        b.extend(&original);

        let cache = Cache::parse(b).expect("tolerant non-utf8 string parsing");
        assert_eq!(cache.string_count(), 1);
        assert_eq!(cache.strings[0], "0123456789\u{FFFD}\u{FFFD}");
        assert!(cache.scene("scenes/test.vcd").unwrap().is_some());
    }
}
#[cfg(test)]
mod flex_track_tests {
    use super::*;
    fn sample(time: f32, value: f32, curve: u16) -> FlexSample {
        FlexSample {
            sample: Sample { time, value },
            curve_type: curve,
        }
    }
    #[test]
    fn flex_track_intensity_follows_samples_and_balance() {
        let linear = 0x0606;
        let mut track = FlexTrack {
            controller: "jaw_drop".into(),
            flags: 1,
            min: 0.,
            max: 1.,
            samples: vec![sample(0.5, 1., linear), sample(1.5, 1., linear)],
            combo_samples: Vec::new(),
        };
        // Event 10..12 s: outside it the zero value applies; inside, the samples.
        assert_eq!(track.intensity(9., 10., Some(12.), 0), 0.);
        assert!((track.intensity(11., 10., Some(12.), 0) - 1.).abs() < 1e-5);
        assert!((track.intensity(10.25, 10., Some(12.), 0) - 0.5).abs() < 1e-5);
        // A -1..1 controller remaps the 0..1 curve and rests at its zero position.
        track.min = -1.;
        assert!((track.intensity(9., 10., Some(12.), 0)).abs() < 1e-5);
        assert!((track.intensity(11., 10., Some(12.), 0) - 1.).abs() < 1e-5);
        // Combo balance fully toward side 1 silences side 0.
        track.min = 0.;
        track.flags = 3;
        track.combo_samples = vec![sample(0., 1., linear), sample(2., 1., linear)];
        assert!(track.intensity(11., 10., Some(12.), 0).abs() < 1e-5);
        assert!((track.intensity(11., 10., Some(12.), 1) - 1.).abs() < 1e-5);
        // Hold curves keep the start value across the segment.
        let hold = FlexTrack {
            flags: 1,
            combo_samples: Vec::new(),
            samples: vec![sample(0.5, 0.2, 0x0f0f), sample(1.5, 0.8, 0x0f0f)],
            ..track.clone()
        };
        assert!((hold.intensity(11., 10., Some(12.), 0) - 0.2).abs() < 1e-5);
    }
}
