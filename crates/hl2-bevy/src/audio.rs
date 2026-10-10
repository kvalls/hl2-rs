//! Preload owned cue alternatives before App::run; systems use only memory/assets.
use anyhow::{Context, Result, bail};
use bevy::{audio::AudioSinkPlayback, prelude::*};
use hl2_simulation::sounds::{AmbientControl, Library, SoundRequest};
use hl2_simulation::soundscapes::{Command as SoundscapeCommand, Definitions, Playback};
use source_assets::vpk::Vfs;
use std::collections::{BTreeMap, BTreeSet, HashMap};

const MAX_PRELOADED_BYTES: usize = 512 * 1024 * 1024;
const MAX_PLAYERS: usize = 256;
/// rodio 0.22 keeps each player alive with mono 48 kHz silence spans of 512 samples. A
/// sound appended in another format has its first 512 samples resampled as if they were in
/// that filler format (the weapon-selection tick's attack played 2.2x fast and lost ~12 ms).
/// Sounds are therefore converted once at load to this rate; stereo one-shots also start
/// with one filler-length span of silence so the misread span is silent.
const BACKEND_RATE: u32 = 48_000;
const BACKEND_FILLER_SAMPLES: usize = 512;

/// Resample interleaved samples with Catmull-Rom interpolation (each channel separately).
fn resample(samples: &[f32], channels: usize, from: u32, to: u32) -> Vec<f32> {
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
fn backend_wav(data: Vec<u8>, looped: bool) -> Result<Vec<u8>> {
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
fn wav_bytes(pcm: &[f32], channels: u16) -> Result<Vec<u8>> {
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
/// VOX sentences the game can request (the HEV suit's), with their words' samples
/// preloaded as mono backend-rate PCM. Playback concatenates the words.
#[derive(Default)]
struct Vox {
    sentences: BTreeMap<String, Vec<source_assets::vox::Word>>,
    groups: BTreeMap<String, Vec<String>>,
    words: HashMap<String, std::sync::Arc<[f32]>>,
    errors: BTreeMap<String, String>,
    played: Vec<String>,
    rng: u64,
}
impl Vox {
    fn load(vfs: &Vfs) -> Self {
        let mut vox = Self {
            rng: 0x6a09_e667_f3bc_c909,
            ..Self::default()
        };
        let text = match vfs.read("scripts/sentences.txt") {
            Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
            other => {
                vox.errors
                    .insert("scripts/sentences.txt".into(), format!("{:?}", other.err()));
                return vox;
            }
        };
        for (name, line) in source_assets::vox::sentences(&text) {
            // Only the suit's sentences are requested by the game so far.
            if !name.starts_with("HEV_") {
                continue;
            }
            let words = source_assets::vox::words(&line);
            for word in &words {
                if vox.words.contains_key(&word.wave) || vox.errors.contains_key(&word.wave) {
                    continue;
                }
                let decoded = (|| -> Result<std::sync::Arc<[f32]>> {
                    let encoded = vfs
                        .read(&format!("sound/{}", word.wave))?
                        .context("owned sentence word absent")?;
                    let (data, _) = hl2_simulation::sounds::decode(&word.wave, encoded)?;
                    let (channels, rate, samples) = hl2_simulation::sounds::wav_samples(&data)?;
                    let channels = usize::from(channels.max(1));
                    let mono: Vec<f32> = samples
                        .chunks(channels)
                        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                        .collect();
                    Ok(resample(&mono, 1, rate, BACKEND_RATE).into())
                })();
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
        vox
    }
    fn random(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }
    /// `!NAME` (a sentence) or `#GROUP` (a random member) to mono PCM. The channel
    /// pitch follows UTIL_EmitSoundSuit (100, or 98-104 half of the time); a word's own
    /// pitch replaces it (inferred: the retail mixer's use of word pitch is unread).
    fn render(&mut self, request: &str) -> Option<(String, Vec<f32>)> {
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
    fn render_at(&mut self, name: &str, channel_pitch: i32) -> Option<Vec<f32>> {
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
/// `--mute-ambient`: skip map-start ambient_generic loops so test recordings isolate cues.
pub static MUTE_AMBIENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Clone)]
struct Ambient {
    request: SoundRequest,
    /// SDK m_fLooping (spawnflags 32 clear).
    looped: bool,
    /// Active from spawn (looping and not start silent).
    at_spawn: bool,
    volume: f32,
    pitch: f32,
    /// Source origin and soundlevel; soundlevel 0 plays everywhere.
    spatial: (glam::Vec3, f32),
}
pub struct PreparedAudio {
    library: Library,
    waves: Vec<(String, Vec<u8>)>,
    /// Every ambient_generic with a message, by entity index.
    ambient: HashMap<usize, Ambient>,
    soundscapes: Definitions,
    bytes: usize,
    cues: usize,
    /// Speech phoneme data (wave path, sentence, seconds) for lip sync.
    pub sentences: Vec<(String, source_assets::sentence::Sentence, f32)>,
    vox: Vox,
}
impl PreparedAudio {
    pub fn load(vfs: &Vfs, game: &crate::gameplay::Gameplay) -> Self {
        let mut library = Library::new(vfs);
        let mut cues = BTreeSet::new();
        let mut ambient = HashMap::new();
        let origin_of = |entity: &modkit_core::Entity| {
            entity
                .get("origin")
                .and_then(modkit_core::parse_vec3)
                .map_or(glam::Vec3::ZERO, |v| glam::Vec3::from_array(v.to_array()))
        };
        for (id, entity) in game.world.entities.iter().enumerate() {
            if entity.class() != "ambient_generic" {
                continue;
            }
            let Some(name) = entity.get("message").filter(|name| !name.is_empty()) else {
                continue;
            };
            cues.insert(name.to_owned());
            let flags = entity
                .get("spawnflags")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            let number = |key: &str| entity.get(key).and_then(|s| s.trim().parse::<f32>().ok());
            // volrun = clamp(health * 10, 0, 100); m_iHealth defaults to 0 (silent).
            let volume = (number("health").unwrap_or(0.) * 10.).clamp(0., 100.) / 100.;
            if volume <= 0. {
                continue;
            }
            let pitch = number("pitch")
                .filter(|p| *p > 0.)
                .unwrap_or(100.)
                .min(255.);
            let radius = number("radius").unwrap_or(0.);
            // A raw wave's soundlevel comes from its radius (40 dB at 36 units); a sound
            // script keeps its own soundlevel (SDK EmitAmbientSound).
            let soundlevel = if name.to_ascii_lowercase().contains(".wav")
                || name.to_ascii_lowercase().contains(".mp3")
            {
                if radius > 0. && flags & 1 == 0 {
                    (40. + 20. * (radius / 36.).log10()).trunc()
                } else {
                    0.
                }
            } else {
                library.params(name).soundlevel
            };
            let source = entity
                .get("SourceEntityName")
                .filter(|n| !n.is_empty())
                .and_then(|n| {
                    game.world
                        .entities
                        .iter()
                        .find(|e| e.get("targetname") == Some(n))
                })
                .unwrap_or(entity);
            ambient.insert(
                id,
                Ambient {
                    request: name.into(),
                    looped: flags & 32 == 0,
                    // SDK CAmbientGeneric: only looping sounds that do not start silent play at
                    // spawn; others wait for PlaySound.
                    at_spawn: flags & (16 | 32) == 0,
                    volume,
                    pitch,
                    spatial: (origin_of(source), soundlevel),
                },
            );
        }
        cues.extend(
            game.scene
                .required_sound_requests(&game.world)
                .into_iter()
                .map(|r| r.name),
        );
        cues.extend(
            game.impacts
                .required_sound_requests()
                .into_iter()
                .map(|r| r.name),
        );
        for weapon in game.weapons.values() {
            cues.extend(weapon.sounds.values().cloned());
        }
        for rig in game.world.rigs.values() {
            for clip in rig.clips.values() {
                cues.extend(
                    clip.events
                        .iter()
                        .filter(|e| {
                            (e.id == 5004 || e.name == "AE_CL_PLAYSOUND") && !e.options.is_empty()
                        })
                        .map(|e| e.options.clone()),
                );
            }
        }
        cues.extend(
            [
                "Player.WeaponSelectionMoveSlot",
                "Player.WeaponSelectionClose",
                "Player.WeaponSelected",
                "Player.DenyWeaponSelection",
                "HUDQuickInfo.LowAmmo",
                "HUDQuickInfo.LowHealth",
                "NPC_CombineBall.Launch",
                "NPC_CombineBall.KillImpact",
                "NPC_CombineBall.Impact",
                "NPC_CombineBall.WhizFlyby",
                "NPC_CombineBall.Explosion",
                "BaseExplosionEffect.Sound",
                "HealthKit.Touch",
                "HealthVial.Touch",
                "ItemBattery.Touch",
                "BaseCombatCharacter.AmmoPickup",
                "Player.FallDamage",
                "Player.FallGib",
                "Player.Death",
                "Grenade.Blip",
                "Grenade.ImpactHard",
                "HL2Player.BurnPain",
                "Player.PlasmaDamage",
                "Player.SonicDamage",
                "Flesh.BulletImpact",
            ]
            .into_iter()
            .map(str::to_owned),
        );
        let mut paths = BTreeSet::new();
        let mut looped: BTreeSet<String> = ambient
            .values()
            .filter(|a| a.looped)
            .filter_map(|a| library.all_alternatives(&a.request.name).ok())
            .flatten()
            .collect();
        // Every wave the map's soundscapes can play; their loops use the looping conversion.
        let soundscapes = Definitions::load(vfs, &game.world.name);
        let names: Vec<&str> = game
            .world
            .entities
            .iter()
            .filter(|e| e.class().starts_with("env_soundscape"))
            .filter_map(|e| e.get("soundscape"))
            .collect();
        let (loops, randoms) = soundscapes.waves_by_kind(names);
        looped.extend(loops.iter().cloned());
        paths.extend(loops);
        paths.extend(randoms);
        for (index, error) in soundscapes.errors.iter().enumerate() {
            library
                .errors
                .insert(format!("soundscapes {index}"), error.clone());
        }
        for cue in &cues {
            match library.all_alternatives(cue) {
                Ok(variants) => paths.extend(variants),
                Err(error) => {
                    library.errors.insert(cue.clone(), format!("{error:#}"));
                }
            }
        }
        let mut waves = vec![];
        let mut bytes = 0usize;
        let mut sentences = Vec::new();
        for path in paths {
            let decoded = (|| -> Result<_> {
                let encoded = vfs
                    .read(&format!("sound/{path}"))?
                    .context("owned sound absent")?;
                let sentence = match source_assets::sentence::vdat_chunk(&encoded) {
                    Ok(Some(chunk)) => Some(source_assets::sentence::Sentence::parse(chunk)?),
                    _ => None,
                };
                let (data, summary) = hl2_simulation::sounds::decode(&path, encoded)?;
                let data = backend_wav(data, looped.contains(&path))?;
                if data.len() > MAX_PRELOADED_BYTES.saturating_sub(bytes) {
                    bail!("owned audio preload exceeds 512 MiB budget");
                }
                Ok((data, summary, sentence))
            })();
            match decoded {
                Ok((data, summary, sentence)) => {
                    if let Some(sentence) = sentence {
                        sentences.push((path.clone(), sentence, summary.seconds() as f32));
                    }
                    bytes += data.len();
                    library.decoded.insert(path.clone(), summary);
                    waves.push((path, data));
                }
                Err(error) => {
                    library.errors.insert(path, format!("{error:#}"));
                }
            }
        }
        Self {
            library,
            waves,
            ambient,
            soundscapes,
            bytes,
            cues: cues.len(),
            sentences,
            vox: Vox::load(vfs),
        }
    }
}

#[derive(Resource)]
pub struct Audio {
    library: Library,
    handles: HashMap<String, Handle<AudioSource>>,
    bytes: usize,
    cues: usize,
    requested: u64,
    started: u64,
    failed: u64,
    capacity_rejections: u64,
    frame: u64,
    pending_sinks: usize,
    active_sinks: usize,
    paused_sinks: usize,
    paused: bool,
    pause_changes: u64,
    resume_changes: u64,
    /// Per positioned player this frame: (wave, distance, applied volume before global).
    spatial: Vec<(String, f32, f32)>,
    ambient: HashMap<usize, Ambient>,
    soundscapes: Definitions,
    soundscape: Playback,
    /// Current volume of each soundscape loop handle.
    soundscape_volumes: HashMap<u64, f32>,
    soundscape_time: Option<f64>,
    soundscape_plays: u64,
    vox: Vox,
}
/// `--audio-trace PATH`: one JSON line per sound request, sink start and sink removal,
/// with the sink's last observed playback position, for diagnosing cut-off sounds.
#[derive(Resource)]
pub struct AudioTrace {
    file: std::io::BufWriter<std::fs::File>,
    start: std::time::Instant,
    live: HashMap<Entity, (String, f64, f32)>,
}
impl AudioTrace {
    pub fn create(path: &std::path::Path) -> Result<Self> {
        Ok(Self {
            file: std::io::BufWriter::new(std::fs::File::create(path)?),
            start: std::time::Instant::now(),
            live: HashMap::new(),
        })
    }
    fn write(&mut self, mut line: serde_json::Value) {
        use std::io::Write;
        line["t"] = serde_json::json!(self.start.elapsed().as_secs_f64());
        // Tracing must never stop the game; a failed write only loses diagnostics.
        let _ = writeln!(self.file, "{line}").and_then(|()| self.file.flush());
    }
}
#[derive(Component)]
pub struct SoundPlayer {
    path: String,
    queued_frame: u64,
    observed: bool,
    /// Volume before distance gain and the global volume.
    volume: f32,
    /// Source origin and soundlevel for distance gain (SDK GetDistGainFromSoundLevel).
    spatial: Option<(glam::Vec3, f32)>,
    /// The ambient_generic entity this sound belongs to.
    ambient: Option<usize>,
    /// The soundscape loop handle this sound belongs to.
    soundscape: Option<u64>,
}
impl Audio {
    fn emit(
        &mut self,
        commands: &mut Commands,
        request: &SoundRequest,
        looped: bool,
        (volume, pitch, spatial): (f32, f32, Option<(glam::Vec3, f32)>),
        (paused, ambient, soundscape): (bool, Option<usize>, Option<u64>),
    ) -> Option<String> {
        let path = match self.library.resolve(request) {
            Ok(path) => path,
            Err(error) => {
                self.library
                    .errors
                    .insert(request.name.clone(), format!("{error:#}"));
                self.failed += 1;
                return None;
            }
        };
        let Some(handle) = self.handles.get(&path) else {
            self.library.errors.entry(path).or_insert_with(|| {
                "cue variant was not preloaded; no file reads are allowed in playback systems"
                    .into()
            });
            self.failed += 1;
            return None;
        };
        let settings = if looped {
            PlaybackSettings::LOOP
        } else {
            PlaybackSettings::DESPAWN
        };
        commands.spawn((
            crate::campaign::MapOwned,
            SoundPlayer {
                path: path.clone(),
                queued_frame: self.frame,
                observed: false,
                volume,
                spatial,
                ambient,
                soundscape,
            },
            AudioPlayer::new(handle.clone()),
            PlaybackSettings {
                // Spatial sounds start silent; `queue` applies their distance gain.
                volume: bevy::audio::Volume::Linear(if spatial.is_some() {
                    0.
                } else {
                    volume.clamp(0., 1.)
                }),
                speed: pitch / 100.,
                paused,
                ..settings
            },
        ));
        self.requested += 1;
        Some(path)
    }
    /// A VOX sentence request ("!NAME" or "#GROUP"), rendered from preloaded words.
    fn emit_sentence(
        &mut self,
        commands: &mut Commands,
        sources: &mut Assets<AudioSource>,
        request: &SoundRequest,
        paused: bool,
    ) -> Option<String> {
        let Some((path, pcm)) = self.vox.render(&request.name) else {
            self.library
                .errors
                .insert(request.name.clone(), "unknown VOX sentence".into());
            self.failed += 1;
            return None;
        };
        let wav = match wav_bytes(&pcm, 1) {
            Ok(wav) => wav,
            Err(error) => {
                self.library.errors.insert(path, format!("{error:#}"));
                self.failed += 1;
                return None;
            }
        };
        let volume = request.volume.unwrap_or(1.);
        commands.spawn((
            crate::campaign::MapOwned,
            SoundPlayer {
                path: path.clone(),
                queued_frame: self.frame,
                observed: false,
                volume,
                spatial: None,
                ambient: None,
                soundscape: None,
            },
            AudioPlayer::new(sources.add(AudioSource { bytes: wav.into() })),
            PlaybackSettings {
                volume: bevy::audio::Volume::Linear(volume.clamp(0., 1.)),
                paused,
                ..PlaybackSettings::DESPAWN
            },
        ));
        self.requested += 1;
        Some(path)
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"preloaded_waves":self.handles.len(),"preloaded_bytes":self.bytes,"referenced_cues":self.cues,
            "requested":self.requested,"started_sinks":self.started,"failed_requests":self.failed,"capacity_rejections":self.capacity_rejections,
            "pending_sinks":self.pending_sinks,"active_sinks":self.active_sinks,"paused_sinks":self.paused_sinks,
            "pause_changes":self.pause_changes,"resume_changes":self.resume_changes,"spatial":self.spatial,
            "variants":self.library.variants_played,"decoded":self.library.decoded,"errors":self.library.errors,
            "soundscape":{"definitions":self.soundscapes.len(),"active":self.soundscape.active(),"started":self.soundscape.started,
                "loops":self.soundscape.loops(),"random_plays":self.soundscape_plays,"unknown":self.soundscape.unknown},
            "vox":{"sentences":self.vox.sentences.len(),"words":self.vox.words.len(),"played":self.vox.played,"errors":self.vox.errors},
            "mixing":"ambient_generic loops and positional soundscape sounds use Source distance gain; stereo panning and DSP are not implemented"})
    }
}
pub fn install(
    commands: &mut Commands,
    sources: &mut Assets<AudioSource>,
    prepared: PreparedAudio,
) {
    let handles = prepared
        .waves
        .into_iter()
        .map(|(path, bytes)| {
            (
                path,
                sources.add(AudioSource {
                    bytes: bytes.into(),
                }),
            )
        })
        .collect();
    let mut audio = Audio {
        library: prepared.library,
        handles,
        bytes: prepared.bytes,
        cues: prepared.cues,
        requested: 0,
        started: 0,
        failed: 0,
        capacity_rejections: 0,
        frame: 0,
        pending_sinks: 0,
        active_sinks: 0,
        paused_sinks: 0,
        paused: false,
        pause_changes: 0,
        resume_changes: 0,
        spatial: Vec::new(),
        ambient: prepared.ambient,
        soundscapes: prepared.soundscapes,
        soundscape: Playback::default(),
        soundscape_volumes: HashMap::new(),
        soundscape_time: None,
        soundscape_plays: 0,
        vox: prepared.vox,
    };
    let mute = MUTE_AMBIENT.load(std::sync::atomic::Ordering::Relaxed);
    let mut spawn: Vec<(usize, Ambient)> = audio
        .ambient
        .iter()
        .filter(|(_, a)| a.at_spawn && !mute)
        .map(|(id, a)| (*id, a.clone()))
        .collect();
    spawn.sort_by_key(|(id, _)| *id);
    for (id, ambient) in spawn {
        if audio.requested >= MAX_PLAYERS as u64 {
            audio.capacity_rejections += 1;
            continue;
        }
        audio.emit(
            commands,
            &ambient.request,
            ambient.looped,
            (ambient.volume, ambient.pitch, Some(ambient.spatial)),
            (false, Some(id), None),
        );
    }
    commands.insert_resource(audio);
}
pub fn queue(
    mut commands: Commands,
    mut audio: ResMut<Audio>,
    mut game: ResMut<crate::gameplay::Gameplay>,
    (simulation, mut trace): (Res<crate::movement::Simulation>, Option<ResMut<AudioTrace>>),
    mut players: Query<(
        Entity,
        &SoundPlayer,
        &mut PlaybackSettings,
        Option<&mut AudioSink>,
    )>,
    global: Res<GlobalVolume>,
    mut sources: ResMut<Assets<AudioSource>>,
) {
    let listener = glam::Vec3::from_array(simulation.eye().to_array());
    let paused = simulation.paused();
    if paused != audio.paused {
        if paused {
            audio.pause_changes += 1;
        } else {
            audio.resume_changes += 1;
        }
        audio.paused = paused;
    }
    let mut count = 0;
    audio.spatial.clear();
    let mut owned = Vec::new();
    for (entity, player, mut settings, sink) in &mut players {
        owned.push((entity, player.ambient, player.soundscape));
        count += 1;
        settings.paused = paused;
        let mut sink = sink;
        // Soundscape loops fade; their current volume replaces the spawn volume.
        let base = player
            .soundscape
            .and_then(|h| audio.soundscape_volumes.get(&h).copied())
            .unwrap_or(player.volume);
        if let (None, Some(_), Some(sink)) =
            (player.spatial, player.soundscape, sink.as_deref_mut())
        {
            sink.set_volume(bevy::audio::Volume::Linear(
                base.clamp(0., 1.) * global.volume.to_linear(),
            ));
        }
        if let (Some((origin, soundlevel)), Some(sink)) = (player.spatial, sink.as_deref_mut()) {
            let gain = hl2_simulation::sounds::dist_gain(soundlevel, listener.distance(origin));
            let volume = (base * gain).clamp(0., 1.);
            sink.set_volume(bevy::audio::Volume::Linear(
                volume * global.volume.to_linear(),
            ));
            let distance = listener.distance(origin);
            audio.spatial.push((player.path.clone(), distance, volume));
        }
        if let Some(sink) = sink {
            if paused && !sink.is_paused() {
                sink.pause();
            } else if !paused && sink.is_paused() {
                sink.play();
            }
        }
    }
    for request in &game.sound_requests {
        if let Some(trace) = trace.as_deref_mut() {
            trace.write(serde_json::json!({"event":"request","cue":request.name,"frame":audio.frame,"scene_time":game.scene.time,"paused":paused,
                "ambient":request.ambient.map(|c| format!("{c:?}"))}));
        }
    }
    for request in std::mem::take(&mut game.sound_requests) {
        // ambient_generic inputs replace or stop that entity's own sound (SDK SendSound).
        if let Some(control) = request.ambient {
            let (AmbientControl::Play(id) | AmbientControl::Stop(id)) = control;
            for (entity, player, _) in &owned {
                if player == &Some(id) {
                    commands.entity(*entity).despawn();
                }
            }
            if let (AmbientControl::Play(_), Some(ambient)) =
                (control, audio.ambient.get(&id).cloned())
                && ambient.volume > 0.
                && !MUTE_AMBIENT.load(std::sync::atomic::Ordering::Relaxed)
            {
                audio.emit(
                    &mut commands,
                    &ambient.request,
                    ambient.looped,
                    (ambient.volume, ambient.pitch, Some(ambient.spatial)),
                    (paused, Some(id), None),
                );
            }
            continue;
        }
        if count < MAX_PLAYERS && (request.name.starts_with('!') || request.name.starts_with('#')) {
            if audio
                .emit_sentence(&mut commands, &mut sources, &request, paused)
                .is_some()
            {
                count += 1;
            } else {
                game.unplayed_sounds += 1;
            }
            continue;
        }
        if count >= MAX_PLAYERS {
            audio.capacity_rejections += 1;
            game.unplayed_sounds += 1;
        } else if let Some(path) = {
            // EmitSound: script volume/pitch drawn per emission unless the caller set the
            // volume; positioned requests use the script soundlevel for distance gain.
            let (volume, pitch, soundlevel) = audio.library.draw_params(&request.name);
            let spatial = request.origin.map(|o| (o, soundlevel));
            audio.emit(
                &mut commands,
                &request,
                false,
                (request.volume.unwrap_or(volume), pitch, spatial),
                (paused, None, None),
            )
        } {
            count += 1;
            // Actor speech drives lip sync from the chosen wave's phonemes.
            if let Some(actor) = request.actor.as_ref().and_then(|a| a.entity) {
                let time = game.scene.time;
                game.scene.lipsync.start(actor, &path, time);
            }
        } else {
            game.unplayed_sounds += 1;
        }
    }
    // Client soundscape frame: scene time drives fades and random sounds, so pause freezes
    // them (dt 0). The server-side selection ran in the fixed tick.
    let now = game.scene.time;
    let dt = audio
        .soundscape_time
        .map_or(0., |t| (now - t).max(0.) as f32);
    audio.soundscape_time = Some(now);
    let forward = glam::Vec3::from_array(
        crate::source_direction(simulation.yaw, simulation.pitch).to_array(),
    );
    let right = glam::Vec3::new(simulation.yaw.sin(), -simulation.yaw.cos(), 0.);
    let params = game.scene.soundscape.params.clone();
    let audio = &mut *audio;
    let operations = audio.soundscape.update(
        &audio.soundscapes,
        params.as_ref(),
        now,
        dt,
        (listener, forward, right),
    );
    let mute = MUTE_AMBIENT.load(std::sync::atomic::Ordering::Relaxed);
    for operation in operations {
        if let Some(trace) = trace.as_deref_mut() {
            trace.write(
                serde_json::json!({"event":"soundscape","operation":format!("{operation:?}"),
                "frame":audio.frame,"scene_time":now}),
            );
        }
        match operation {
            SoundscapeCommand::StartLoop {
                handle,
                wave,
                volume,
                pitch,
                spatial,
            } => {
                audio.soundscape_volumes.insert(handle, volume);
                if !mute {
                    audio.emit(
                        &mut commands,
                        &wave.into(),
                        true,
                        (volume, pitch, spatial),
                        (paused, None, Some(handle)),
                    );
                }
            }
            SoundscapeCommand::LoopVolume { handle, volume } => {
                audio.soundscape_volumes.insert(handle, volume);
            }
            SoundscapeCommand::StopLoop { handle } => {
                audio.soundscape_volumes.remove(&handle);
                for (entity, _, owner) in &owned {
                    if owner == &Some(handle) {
                        commands.entity(*entity).despawn();
                    }
                }
            }
            SoundscapeCommand::Play {
                wave,
                volume,
                pitch,
                spatial,
            } => {
                audio.soundscape_plays += 1;
                if mute {
                    continue;
                }
                if count >= MAX_PLAYERS {
                    audio.capacity_rejections += 1;
                } else if audio
                    .emit(
                        &mut commands,
                        &wave.into(),
                        false,
                        (volume, pitch.max(1.), spatial),
                        (paused, None, None),
                    )
                    .is_some()
                {
                    count += 1;
                }
            }
        }
    }
}
pub fn observe(
    mut commands: Commands,
    mut audio: ResMut<Audio>,
    mut players: Query<(Entity, &mut SoundPlayer, Option<&AudioSink>)>,
    (mut trace, mut removed): (Option<ResMut<AudioTrace>>, RemovedComponents<SoundPlayer>),
) {
    if let Some(trace) = trace.as_deref_mut() {
        for entity in removed.read() {
            if let Some((path, seconds, position)) = trace.live.remove(&entity) {
                trace.write(serde_json::json!({"event":"removed","path":path,"frame":audio.frame,
                    "last_position":position,"duration":seconds,"remaining":seconds - f64::from(position)}));
            }
        }
        for (entity, player, sink) in &players {
            let Some(sink) = sink else { continue };
            let position = sink.position().as_secs_f32();
            if let Some(live) = trace.live.get_mut(&entity) {
                live.2 = position;
            } else {
                let seconds = audio
                    .library
                    .decoded
                    .get(&player.path)
                    .map_or(0., |summary| summary.seconds());
                trace
                    .live
                    .insert(entity, (player.path.clone(), seconds, position));
                trace.write(serde_json::json!({"event":"started","path":player.path,"frame":audio.frame,
                    "queued_frame":player.queued_frame,"position":position,"duration":seconds,"volume":sink.volume().to_linear(),"paused":sink.is_paused()}));
            }
        }
    }
    audio.frame += 1;
    audio.pending_sinks = 0;
    audio.active_sinks = 0;
    audio.paused_sinks = 0;
    for (entity, mut player, sink) in &mut players {
        if let Some(sink) = sink {
            audio.active_sinks += 1;
            audio.paused_sinks += usize::from(sink.is_paused());
            if !player.observed {
                player.observed = true;
                audio.started += 1;
                audio.library.played += 1;
                *audio
                    .library
                    .variants_played
                    .entry(player.path.clone())
                    .or_default() += 1;
            }
        } else {
            audio.pending_sinks += 1;
            if audio.frame.saturating_sub(player.queued_frame) > 120 {
                audio.library.errors.insert(player.path.clone(), "playback sink did not start within 120 frames; audio device/backend may be unavailable".into());
                audio.failed += 1;
                commands.entity(entity).despawn();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::audio::{Decodable, Source};

    #[test]
    fn backend_resampling_preserves_tone_and_pads_stereo_one_shots() {
        // A 1.8 kHz tone (the selection tick's bursts) at 22050 Hz keeps its level and pitch.
        let tone: Vec<f32> = (0..22050)
            .map(|i| (i as f32 * std::f32::consts::TAU * 1800. / 22050.).sin() * 0.5)
            .collect();
        let out = resample(&tone, 1, 22050, BACKEND_RATE);
        assert_eq!(out.len(), 48000);
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        assert!((rms(&out) / rms(&tone) - 1.).abs() < 0.01);
        let crossings = out.windows(2).filter(|w| w[0] <= 0. && w[1] > 0.).count();
        assert!((1799..=1801).contains(&crossings));
        assert_eq!(resample(&tone, 1, 22050, 22050), tone);
        // Stereo one-shots start with one filler span of silence; loops and mono do not.
        let wav = |channels: u16, frames: usize| {
            let data = [0x00u8, 0x40].repeat(frames * usize::from(channels));
            let mut wav = b"RIFF".to_vec();
            wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
            wav.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0");
            wav.extend_from_slice(&channels.to_le_bytes());
            wav.extend_from_slice(&44100u32.to_le_bytes());
            wav.extend_from_slice(&(44100 * 2 * u32::from(channels)).to_le_bytes());
            wav.extend_from_slice(&(2 * channels).to_le_bytes());
            wav.extend_from_slice(b"\x10\0data");
            wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
            wav.extend_from_slice(&data);
            wav
        };
        let samples = |bytes: Vec<u8>| hl2_simulation::sounds::wav_samples(&bytes).unwrap();
        let (channels, rate, one_shot) = samples(backend_wav(wav(2, 441), false).unwrap());
        assert_eq!((channels, rate), (2, BACKEND_RATE));
        assert_eq!(one_shot.len(), BACKEND_FILLER_SAMPLES + 2 * 480);
        assert!(one_shot[..BACKEND_FILLER_SAMPLES].iter().all(|&v| v == 0.));
        assert!((one_shot[BACKEND_FILLER_SAMPLES] - 0.5).abs() < 1e-3);
        assert_eq!(
            samples(backend_wav(wav(2, 441), true).unwrap()).2.len(),
            2 * 480
        );
        assert_eq!(
            samples(backend_wav(wav(1, 441), false).unwrap()).2.len(),
            480
        );
    }
    /// Plain 8-bit unsigned or 16-bit signed PCM samples of a RIFF WAVE, as f32.
    fn pcm_samples(bytes: &[u8]) -> Option<Vec<f32>> {
        let (mut offset, mut bits) = (12, None);
        while offset + 8 <= bytes.len() {
            let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().ok()?) as usize;
            let body = bytes.get(offset + 8..offset + 8 + size)?;
            match &bytes[offset..offset + 4] {
                b"fmt " if u16::from_le_bytes([body[0], body[1]]) == 1 => {
                    bits = Some(u16::from_le_bytes([body[14], body[15]]))
                }
                b"data" => {
                    return match bits? {
                        8 => Some(body.iter().map(|&b| (f32::from(b) - 128.) / 128.).collect()),
                        16 => Some(
                            body.as_chunks::<2>()
                                .0
                                .iter()
                                .map(|c| f32::from(i16::from_le_bytes(*c)) / 32768.)
                                .collect(),
                        ),
                        _ => None,
                    };
                }
                _ => {}
            }
            offset += 8 + size + (size & 1);
        }
        None
    }
    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn installed_hev_sentences_match_their_authored_lengths() {
        let root = source_assets::install::discover().unwrap();
        let vfs = Vfs::mount(&root).unwrap();
        let mut vox = Vox::load(&vfs);
        let text = String::from_utf8(vfs.read("scripts/sentences.txt").unwrap().unwrap()).unwrap();
        let mut checked = 0;
        for (name, line) in source_assets::vox::sentences(&text) {
            let Some(length) = line
                .split("Len ")
                .nth(1)
                .and_then(|v| v.split_whitespace().next())
                .and_then(|v| v.trim_end_matches('}').parse::<f32>().ok())
            else {
                continue;
            };
            let Some(words) = vox.sentences.get(&name) else {
                continue;
            };
            if words.iter().any(|w| !vox.words.contains_key(&w.wave)) {
                continue;
            }
            let seconds = vox.render_at(&name, 100).unwrap().len() as f32 / BACKEND_RATE as f32;
            // HEV_MED0's authored Len (1.41, the same as HEV_MED2) is stale; its words
            // render 5.93 s. Every other rendered length is within rounding.
            if name != "HEV_MED0" {
                assert!(
                    (seconds - length).abs() < 0.006,
                    "{name}: {seconds} vs Len {length}"
                );
            }
            checked += 1;
        }
        assert!(checked >= 40, "{checked}");
    }
    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn installed_weapon_selection_backend_preserves_full_duration() {
        let root = source_assets::install::discover().unwrap();
        let vfs = Vfs::mount(&root).unwrap();
        let library = Library::new(&vfs);
        let mut cues: BTreeSet<String> = [
            "Player.WeaponSelectionMoveSlot",
            "Player.WeaponSelectionClose",
            "Player.WeaponSelected",
            "Player.DenyWeaponSelection",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let wanted = ["draw", "drawempty", "ir_draw"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        for weapon in hl2_simulation::gameplay::definitions(&vfs)
            .unwrap()
            .values()
        {
            let rig = source_assets::animation::load(&vfs, &weapon.viewmodel, &wanted).unwrap();
            for (name, clip) in &rig.clips {
                for event in &clip.events {
                    if (event.id == 5004 || event.name == "AE_CL_PLAYSOUND")
                        && !event.options.is_empty()
                    {
                        println!(
                            "{} {name}: sound {} at cycle {}",
                            weapon.viewmodel, event.options, event.cycle
                        );
                        cues.insert(event.options.clone());
                    }
                }
            }
        }
        for cue in cues {
            for path in library.all_alternatives(&cue).unwrap() {
                let encoded = vfs.read(&format!("sound/{path}")).unwrap().unwrap();
                let (bytes, summary) = hl2_simulation::sounds::decode(&path, encoded).unwrap();
                let summary = serde_json::to_value(summary).unwrap();
                let expected =
                    summary["frames"].as_u64().unwrap() * summary["channels"].as_u64().unwrap();
                let source = AudioSource {
                    bytes: bytes.into(),
                };
                let decoder = source.decoder();
                let duration = decoder.total_duration().unwrap().as_secs_f64();
                let decoded: Vec<f32> = source.decoder().collect();
                let samples = decoded.len() as u64;
                // The backend's samples must equal the file's, from the very first one.
                let expected_samples =
                    pcm_samples(&source.bytes).unwrap_or_else(|| decoded.clone());
                let first = decoded
                    .iter()
                    .zip(&expected_samples)
                    .position(|(a, b)| (a - b).abs() > 1. / 64.);
                println!(
                    "{cue} {path}: first differing sample {first:?}; first 8 backend {:?} file {:?}",
                    &decoded[..8],
                    &expected_samples[..8]
                );
                println!("{cue} {path}: {samples}/{expected} samples, {duration:.6} seconds");
                assert_eq!(samples, expected, "backend truncated {path}");
                assert!(
                    (duration - summary["seconds"].as_f64().unwrap()).abs() < 1e-6,
                    "backend changed duration of {path}"
                );
            }
        }
    }
}
