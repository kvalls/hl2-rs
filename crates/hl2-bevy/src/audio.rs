//! Preload owned cue alternatives before App::run; systems use only memory/assets.
use anyhow::Result;
use bevy::{audio::AudioSinkPlayback, prelude::*};
use hl2_simulation::sounds::{AmbientControl, Library, SoundRequest};
use hl2_simulation::soundscapes::{Command as SoundscapeCommand, Definitions, Playback};
use source_assets::vpk::Vfs;
#[cfg(test)]
use std::collections::BTreeSet;
use std::collections::{BTreeMap, HashMap};

const MAX_PRELOADED_BYTES: usize = 512 * 1024 * 1024;
const MAX_PLAYERS: usize = 256;

mod cues;
mod wav;
use wav::{
    BACKEND_RATE, backend_wav, distance_variant_key, distance_variant_mixes, resample, wav_bytes,
};
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
    fn load(
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
/// `--mute`: silence the output for this run only (unattended replays). Sounds still start and
/// are reported; the Audio settings and the saved volume are left as they are.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct OutputMute(pub bool);
/// Whether every channel is silent: a forced `--mute`, or `snd_mute_losefocus` while the
/// window is not focused.
pub fn output_muted(forced: bool, mute_losefocus: bool, unfocused: bool) -> bool {
    forced || (mute_losefocus && unfocused)
}

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
    waves: Vec<(String, std::sync::Arc<[u8]>)>,
    /// Every ambient_generic with a message, by entity index.
    ambient: HashMap<usize, Ambient>,
    soundscapes: Definitions,
    bytes: usize,
    cues: usize,
    /// Speech phoneme data (wave path, sentence, seconds) for lip sync.
    pub sentences: Vec<(String, source_assets::sentence::Sentence, f32)>,
    ready: bool,
    preparation_ms: f64,
    vox: Vox,
}
pub(crate) mod deferred;
#[cfg(test)]
mod deferred_tests;
mod preparation;
pub(crate) mod reuse;

#[derive(Resource)]
pub struct Audio {
    ready: bool,
    preparation_ms: f64,
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
    /// Audio page music volume (`snd_musicvolume`) applied to music channels.
    music_volume: f32,
    /// `--mute`, or `snd_mute_losefocus` with the window unfocused ([`output_muted`]).
    muted: bool,
    /// The loudest linear volume given to any sink (0 for a silent `--mute` run).
    peak_output: f32,
    /// CSoundPatch envelopes by (emitter, cue).
    patches: HashMap<(u64, String), hl2_simulation::sounds::PatchEnvelope>,
    /// Captions of sounds started outside `queue` (ambient_generic spawns).
    captions: Vec<hl2_ui::closecaption::Request>,
    /// The listener (player eye) of the latest frame.
    listener: glam::Vec3,
}
/// `soundpatch_captionlength`: looping sound patches caption for this long.
const SOUND_PATCH_CAPTION_LENGTH: f32 = 2.;
impl Audio {
    pub(crate) fn is_ready(&self) -> bool {
        self.ready
    }
    /// The CloseCaption message of an emitted sound (CSoundEmitterSystem::EmitCloseCaption):
    /// its sound script name for `duration` (the chosen wave's length unless given), unless the
    /// sound plays at an `origin` beyond the caption radius of the `player`. Raw waves,
    /// sentences and menu sounds have none. Unpositioned sounds without a speaker are the
    /// local player's own (weapons, footsteps), which selects `<playerclr>` colors.
    fn caption(
        &self,
        request: &SoundRequest,
        path: &str,
        duration: Option<f32>,
        (origin, player): (Option<glam::Vec3>, glam::Vec3),
    ) -> Option<hl2_ui::closecaption::Request> {
        if !request.caption
            || request.ui
            || request.stop
            || hl2_simulation::sounds::raw_wave(&request.name)
            || request.name.starts_with(['!', '#'])
            || hl2_simulation::captions::sound_caption_attenuated(player, origin)
        {
            return None;
        }
        let duration = duration.unwrap_or_else(|| {
            self.library
                .decoded
                .get(path)
                .map_or(0., |summary| summary.seconds() as f32)
        });
        Some(hl2_ui::closecaption::Request {
            token: request.name.to_ascii_lowercase(),
            duration: hl2_ui::closecaption::message_duration(duration),
            from_player: request.actor.is_none()
                && request.origin.is_none()
                && request.ambient.is_none(),
            gender: self.library.caption_gender(request.actor.as_ref()),
        })
    }
    /// A scene's CloseCaption message, with the speaking actor's gender.
    fn scene_caption(
        &self,
        caption: hl2_simulation::captions::CaptionRequest,
    ) -> hl2_ui::closecaption::Request {
        hl2_ui::closecaption::Request {
            token: caption.token,
            duration: hl2_ui::closecaption::message_duration(caption.duration),
            from_player: caption.from_player,
            gender: self.library.caption_gender(caption.actor.as_ref()),
        }
    }
    /// Settings factor for one channel, before the master volume: music channels (dry-mix `#`
    /// waves at soundlevel 0) follow the music volume; everything is silent while muted.
    fn mix(&self, music: bool) -> f32 {
        if self.muted {
            0.
        } else if music {
            self.music_volume
        } else {
            1.
        }
    }
    /// A sink volume, recorded in `peak_output`.
    fn output(&mut self, linear: f32) -> bevy::audio::Volume {
        self.peak_output = self.peak_output.max(linear);
        bevy::audio::Volume::Linear(linear)
    }
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
    /// A music channel, scaled by the music volume (see [`hl2_simulation::sounds::is_music`]).
    music: bool,
    /// UI sounds bypass simulation pausing so menu events play while paused.
    ui: bool,
    /// The emitting entity/handle and cue, for StopSound and sound patches.
    emitter: Option<(u64, String)>,
}
impl Audio {
    /// The volume a new player starts with: spatial sounds already carry their distance
    /// gain, because Source mixes a channel's first samples at its spatialized volume
    /// (a one-frame silent start swallowed the 25 ms far layer of NPC gunfire); `queue`
    /// keeps it current afterwards.
    fn start_volume(&self, volume: f32, spatial: Option<(glam::Vec3, f32)>) -> f32 {
        let gain = spatial.map_or(1., |(origin, soundlevel)| {
            hl2_simulation::sounds::dist_gain(soundlevel, self.listener.distance(origin))
        });
        (volume * gain).clamp(0., 1.)
    }
    fn emit(
        &mut self,
        commands: &mut Commands,
        request: &SoundRequest,
        looped: bool,
        (volume, pitch, spatial): (f32, f32, Option<(glam::Vec3, f32)>),
        (paused, ambient, soundscape): (bool, Option<usize>, Option<u64>),
    ) -> Option<String> {
        let wave = match self.library.resolve_wave(request) {
            Ok(wave) => wave,
            Err(error) => {
                warn!("sound request failed: {error:#}");
                self.library
                    .errors
                    .insert(request.name.clone(), format!("{error:#}"));
                self.failed += 1;
                return None;
            }
        };
        let path = wave.path;
        // A distance-variant wave plays the premix for the listener's distance at the start
        // (unpositioned requests are at the listener: the near channel).
        let variant = self.library.distance_variant(&path).then(|| {
            let distance = spatial.map_or(0., |(origin, _)| self.listener.distance(origin));
            distance_variant_key(&path, distance)
        });
        let variant = variant.filter(|key| self.handles.contains_key(key));
        let Some(handle) = variant
            .as_ref()
            .and_then(|key| self.handles.get(key))
            .or_else(|| self.handles.get(&path))
        else {
            self.library.errors.entry(path).or_insert_with(|| {
                "cue variant was not preloaded; no file reads are allowed in playback systems"
                    .into()
            });
            self.failed += 1;
            return None;
        };
        // Positioned channels use their own soundlevel; others the script's.
        let soundlevel =
            spatial.map_or_else(|| self.library.params(&request.name).soundlevel, |s| s.1);
        let music = hl2_simulation::sounds::is_music(wave.dry_mix, soundlevel);
        let settings = if looped {
            PlaybackSettings::LOOP
        } else {
            PlaybackSettings::DESPAWN
        };
        commands.spawn((
            crate::campaign::MapOwned,
            SoundPlayer {
                // The premix key for distance variants (traces show the mix chosen).
                path: variant.unwrap_or_else(|| path.clone()),
                queued_frame: self.frame,
                observed: false,
                volume,
                spatial,
                ambient,
                soundscape,
                music,
                ui: request.ui,
                emitter: request.emitter.map(|e| (e, request.name.clone())),
            },
            AudioPlayer::new(handle.clone()),
            PlaybackSettings {
                volume: self.output(self.start_volume(volume, spatial) * self.mix(music)),
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
        // NPC sentences (CAI_Sentence) use the class's SentenceParameters script entry
        // for volume and soundlevel and play at the speaker; the suit's are unspatialized.
        let parameters = hl2_simulation::ai::runtime::sentence_parameters(&request.name)
            .map(|cue| self.library.draw_params(cue));
        let volume = request.volume.or(parameters.map(|p| p.0)).unwrap_or(1.);
        let spatial = request
            .origin
            .zip(parameters)
            .map(|(origin, (_, _, soundlevel))| (origin, soundlevel));
        commands.spawn((
            crate::campaign::MapOwned,
            SoundPlayer {
                path: path.clone(),
                queued_frame: self.frame,
                observed: false,
                volume,
                spatial,
                ambient: None,
                soundscape: None,
                music: false,
                ui: request.ui,
                emitter: None,
            },
            AudioPlayer::new(sources.add(AudioSource { bytes: wav.into() })),
            PlaybackSettings {
                volume: self.output(self.start_volume(volume, spatial) * self.mix(false)),
                paused,
                ..PlaybackSettings::DESPAWN
            },
        ));
        self.requested += 1;
        Some(path)
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"ready":self.ready,"preparation_ms":self.preparation_ms,"preloaded_waves":self.handles.len(),"preloaded_bytes":self.bytes,"referenced_cues":self.cues,
            "requested":self.requested,"started_sinks":self.started,"failed_requests":self.failed,"capacity_rejections":self.capacity_rejections,
            "pending_sinks":self.pending_sinks,"active_sinks":self.active_sinks,"paused_sinks":self.paused_sinks,
            "pause_changes":self.pause_changes,"resume_changes":self.resume_changes,"spatial":self.spatial,
            "muted":self.muted,"peak_output":self.peak_output,
            "variants":self.library.variants_played,"decoded":self.library.decoded,"errors":self.library.errors,
            "soundscape":{"definitions":self.soundscapes.len(),"active":self.soundscape.active(),"started":self.soundscape.started,
                "loops":self.soundscape.loops(),"random_plays":self.soundscape_plays,"unknown":self.soundscape.unknown},
            "vox":{"sentences":self.vox.sentences.len(),"words":self.vox.words.len(),"played":self.vox.played,"errors":self.vox.errors},
            "mixing":"ambient_generic loops and positional soundscape sounds use Source distance gain; distance-variant (^) waves play the near/far premix for the start distance; stereo panning and DSP are not implemented"})
    }
}
/// `mute` (`--mute`) silences the map-start sounds spawned here; the mixer keeps it after.
pub fn install(
    commands: &mut Commands,
    sources: &mut Assets<AudioSource>,
    prepared: PreparedAudio,
    mute: bool,
) {
    let handles = prepared
        .waves
        .into_iter()
        .map(|(path, bytes)| (path, sources.add(AudioSource { bytes })))
        .collect();
    let mut audio = Audio {
        ready: prepared.ready,
        preparation_ms: prepared.preparation_ms,
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
        music_volume: 1.,
        muted: mute,
        peak_output: 0.,
        patches: HashMap::new(),
        captions: Vec::new(),
        listener: glam::Vec3::ZERO,
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
        if let Some(path) = audio.emit(
            commands,
            &ambient.request,
            ambient.looped,
            (ambient.volume, ambient.pitch, Some(ambient.spatial)),
            (false, Some(id), None),
        ) && let Some(caption) =
            audio.caption(&ambient.request, &path, None, (None, glam::Vec3::ZERO))
        {
            audio.captions.push(caption);
        }
    }
    commands.insert_resource(audio);
}
/// Master volume, Audio settings, the primary window (focus) and `--mute` used to mix every
/// channel.
type MixInputs<'w, 's> = (
    Res<'w, GlobalVolume>,
    Option<Res<'w, crate::ui_host::Console>>,
    Query<'w, 's, &'static Window, With<bevy::window::PrimaryWindow>>,
    Option<Res<'w, OutputMute>>,
);
struct PlayerUpdateCtx<'a, 'w, 's> {
    scene_time: f64,
    listener: glam::Vec3,
    mix_scale: f32,
    paused: bool,
    commands: &'a mut Commands<'w, 's>,
}

fn update_player(
    entity: Entity,
    player: &SoundPlayer,
    mut sink: Option<&mut AudioSink>,
    audio: &mut Audio,
    ctx: &mut PlayerUpdateCtx<'_, '_, '_>,
) {
    if let Some(envelope) = player
        .emitter
        .as_ref()
        .and_then(|key| audio.patches.get_mut(key))
    {
        let (volume, pitch) = envelope.sample(ctx.scene_time);
        if envelope.stopped {
            if let Some(key) = &player.emitter {
                audio.patches.remove(key);
            }
            ctx.commands.entity(entity).despawn();
            return;
        }
        if let Some(sink) = sink.as_deref_mut() {
            let gain = player.spatial.map_or(1., |(origin, soundlevel)| {
                hl2_simulation::sounds::dist_gain(soundlevel, ctx.listener.distance(origin))
            });
            let linear = (volume * gain).clamp(0., 1.) * audio.mix(player.music) * ctx.mix_scale;
            sink.set_volume(audio.output(linear));
            sink.set_speed(pitch.max(1.) / 100.);
        }
    }
    let base = player
        .soundscape
        .and_then(|h| audio.soundscape_volumes.get(&h).copied())
        .unwrap_or(player.volume);
    let mix = audio.mix(player.music) * ctx.mix_scale;
    if let (None, Some(sink)) = (player.spatial, sink.as_deref_mut()) {
        sink.set_volume(audio.output(base.clamp(0., 1.) * mix));
    }
    if let (Some((origin, soundlevel)), Some(sink)) = (player.spatial, sink.as_deref_mut()) {
        let gain = hl2_simulation::sounds::dist_gain(soundlevel, ctx.listener.distance(origin));
        let volume = (base * gain).clamp(0., 1.);
        sink.set_volume(audio.output(volume * mix));
        let distance = ctx.listener.distance(origin);
        audio.spatial.push((player.path.clone(), distance, volume));
    }
    if let Some(sink) = sink {
        if !player.ui && ctx.paused && !sink.is_paused() {
            sink.pause();
        } else if !player.ui && !ctx.paused && sink.is_paused() {
            sink.play();
        }
    }
}

fn apply_output_settings(
    audio: &mut Audio,
    ui: Option<&crate::ui_host::Console>,
    forced: bool,
    unfocused: bool,
) {
    let (music_vol, mute_losefocus, video_playing) = ui.map_or((0., false, false), |u| {
        (
            u.source.config.audio.music_volume.clamp(0., 1.),
            u.source.config.audio.mute_losefocus,
            u.source.is_video_playing(),
        )
    });
    audio.music_volume = music_vol;
    audio.muted = output_muted(forced, mute_losefocus, unfocused) || video_playing;
}

fn update_pause_state(audio: &mut Audio, paused: bool) {
    if paused != audio.paused {
        if paused {
            audio.pause_changes += 1;
        } else {
            audio.resume_changes += 1;
        }
        audio.paused = paused;
    }
}

pub fn queue(
    mut commands: Commands,
    mut audio: ResMut<Audio>,
    mut game: ResMut<crate::gameplay::Gameplay>,
    (simulation, mut trace, mut pending): (
        Res<crate::simulation::Simulation>,
        Option<ResMut<AudioTrace>>,
        Option<ResMut<deferred::PendingAudio>>,
    ),
    mut players: Query<(
        Entity,
        &SoundPlayer,
        &mut PlaybackSettings,
        Option<&mut AudioSink>,
    )>,
    mut sources: ResMut<Assets<AudioSource>>,
    (global, ui, windows, forced_mute): MixInputs,
) {
    if let Some(pending) = pending.as_deref_mut().filter(|p| p.waiting()) {
        pending.defer(&mut game);
        return;
    }
    apply_output_settings(
        &mut audio,
        ui.as_deref(),
        forced_mute.is_some_and(|m| m.0),
        windows.single().is_ok_and(|w| !w.focused),
    );
    let listener = glam::Vec3::from_array(simulation.view_origin().to_array());
    audio.listener = listener;
    // Caption radius checks measure from the player (its eye stands in for its origin).
    let player = glam::Vec3::from_array(simulation.eye().to_array());
    let paused = !simulation.world_running();
    update_pause_state(&mut audio, paused);
    let mut count = 0;
    audio.spatial.clear();
    let mut owned = Vec::new();
    let mut emitted: Vec<(Entity, (u64, String))> = Vec::new();
    let mut ctx = PlayerUpdateCtx {
        scene_time: game.scene.time,
        listener,
        mix_scale: global.volume.to_linear(),
        paused,
        commands: &mut commands,
    };
    for (entity, player, mut settings, mut sink) in &mut players {
        owned.push((entity, player.ambient, player.soundscape));
        if let Some(key) = &player.emitter {
            emitted.push((entity, key.clone()));
        }
        count += 1;
        if !player.ui {
            settings.paused = paused;
        }
        update_player(entity, player, sink.as_deref_mut(), &mut audio, &mut ctx);
    }
    for request in &game.sound_requests {
        if let Some(trace) = trace.as_deref_mut() {
            trace.write(serde_json::json!({"event":"request","cue":request.name,"frame":audio.frame,"scene_time":game.scene.time,"paused":paused,
                "ambient":request.ambient.map(|c| format!("{c:?}")),"stop":request.stop,
                "actor_model":request.actor.as_ref().map(|a| a.model.as_str()),"entity":request.actor.as_ref().and_then(|a| a.entity)}));
        }
    }
    // CloseCaption messages: spawn-time ambient sounds, then scene speech.
    let mut captions = std::mem::take(&mut audio.captions);
    for caption in std::mem::take(&mut game.captions) {
        captions.push(audio.scene_caption(caption));
    }
    for request in std::mem::take(&mut game.sound_requests) {
        // CBaseEntity::StopSound(name) on the emitter, and CSoundEnvelopeController ops.
        if let Some(emitter) = request.emitter {
            let key = (emitter, request.name.clone());
            let despawn = |commands: &mut Commands| {
                for (entity, owner) in &emitted {
                    if owner == &key {
                        commands.entity(*entity).despawn();
                    }
                }
            };
            if request.stop {
                despawn(&mut commands);
                audio.patches.remove(&key);
                continue;
            }
            if let Some(op) = request.patch {
                let time = game.scene.time;
                match op {
                    hl2_simulation::sounds::PatchOp::Play { volume, pitch } => {
                        despawn(&mut commands);
                        audio.patches.insert(
                            key,
                            hl2_simulation::sounds::PatchEnvelope::new(volume, pitch, time),
                        );
                        let soundlevel = audio.library.draw_params(&request.name).2;
                        let spatial = request.origin.map(|o| (o, soundlevel));
                        // The patch loops (SoundCreate on a looping wave; cue points
                        // are not read, so the wave is assumed to loop).
                        if count < MAX_PLAYERS
                            && let Some(path) = audio.emit(
                                &mut commands,
                                &request,
                                true,
                                (volume, pitch.max(1.), spatial),
                                (paused, None, None),
                            )
                        {
                            count += 1;
                            // CSoundPatch::StartSound captions the script name.
                            captions.extend(audio.caption(
                                &request,
                                &path,
                                Some(SOUND_PATCH_CAPTION_LENGTH),
                                (request.origin, player),
                            ));
                        }
                    }
                    hl2_simulation::sounds::PatchOp::Destroy => {
                        despawn(&mut commands);
                        audio.patches.remove(&key);
                    }
                    op => {
                        if let Some(envelope) = audio.patches.get_mut(&key) {
                            envelope.apply(op, time);
                        }
                    }
                }
                continue;
            }
        }
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
                && let Some(path) = audio.emit(
                    &mut commands,
                    &ambient.request,
                    ambient.looped,
                    (ambient.volume, ambient.pitch, Some(ambient.spatial)),
                    (paused, Some(id), None),
                )
            {
                // EmitAmbientSound captions the start of the sound, at any distance.
                captions.extend(audio.caption(&ambient.request, &path, None, (None, player)));
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
                (if request.ui { false } else { paused }, None, None),
            )
        } {
            count += 1;
            captions.extend(audio.caption(&request, &path, None, (request.origin, player)));
            // Actor speech drives lip sync from the chosen wave's phonemes.
            if let Some(actor) = request.actor.as_ref().and_then(|a| a.entity) {
                let time = game.scene.time;
                game.scene.lipsync.start(actor, &path, time);
            }
        } else {
            game.unplayed_sounds += 1;
        }
    }
    game.caption_messages.extend(captions);
    // Client soundscape frame: scene time drives fades and random sounds, so pause freezes
    // them (dt 0). The server-side selection ran in the fixed tick.
    let now = game.scene.time;
    let dt = audio
        .soundscape_time
        .map_or(0., |t| (now - t).max(0.) as f32);
    audio.soundscape_time = Some(now);
    let (yaw, pitch) = simulation.view_angles();
    let forward = glam::Vec3::from_array(crate::source_direction(yaw, pitch).to_array());
    let right = glam::Vec3::new(yaw.sin(), -yaw.cos(), 0.);
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
    use super::wav::BACKEND_FILLER_SAMPLES;
    use super::*;
    use bevy::audio::{Decodable, Source};

    #[test]
    fn forced_mute_silences_regardless_of_focus_and_settings() {
        for mute_losefocus in [false, true] {
            for unfocused in [false, true] {
                assert!(output_muted(true, mute_losefocus, unfocused));
            }
        }
        assert!(output_muted(false, true, true));
        assert!(!output_muted(false, true, false));
        assert!(!output_muted(false, false, true));
    }

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
        let mut vox = Vox::load(&vfs, &Default::default(), &Default::default()).unwrap();
        let text = String::from_utf8(vfs.read("scripts/sentences.txt").unwrap().unwrap()).unwrap();
        let mut checked = 0;
        for (name, line) in source_assets::vox::sentences(&text) {
            // METROPOLICE_* sentences all carry a placeholder {Len 1.5} (caption hint).
            if !name.starts_with("HEV_") {
                continue;
            }
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
