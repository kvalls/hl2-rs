//! Class-gated NPC audio cue resolution and runtime merging for late-loaded NPCs.
use crate::assets::work::Cancellation;
use crate::audio::reuse::AudioReuse;
use hl2_simulation::sounds::{AudioSummary, Library};
use source_assets::{sentence::Sentence, vpk::Vfs};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Returns the static soundscript cues required by an NPC `class`, or empty if none apply.
pub(crate) fn class_sound_cues(class: &str) -> Vec<&'static str> {
    match class.to_ascii_lowercase().as_str() {
        "npc_zombie" | "npc_zombie_torso" | "npc_poisonzombie" => {
            hl2_simulation::ai::zombie::SOUNDS.to_vec()
        }
        "npc_fastzombie" | "npc_fastzombie_torso" => hl2_simulation::ai::zombie::SOUNDS
            .iter()
            .chain(hl2_simulation::ai::zombie::fast::SOUNDS.iter())
            .copied()
            .collect(),
        "npc_headcrab" | "npc_headcrab_fast" | "npc_headcrab_black" | "npc_headcrab_poison" => {
            hl2_simulation::ai::headcrab::SOUNDS.to_vec()
        }
        "npc_barnacle" => hl2_simulation::ai::barnacle::SOUNDS.to_vec(),
        "npc_crow" | "npc_pigeon" | "npc_seagull" => hl2_simulation::ai::bird::SOUNDS.to_vec(),
        "npc_manhack" => hl2_simulation::ai::manhack::SOUNDS.to_vec(),
        "npc_cscanner" | "npc_clawscanner" => hl2_simulation::ai::scanner::SOUNDS.to_vec(),
        _ => Vec::new(),
    }
}

/// Decoded audio waves, metadata, sentences and errors produced off-thread for late NPCs.
#[derive(Default)]
pub(crate) struct PreparedLateAudio {
    pub(crate) waves: Vec<(String, Arc<[u8]>)>,
    pub(crate) decoded: Vec<(String, AudioSummary)>,
    pub(crate) sentences: Vec<(String, Sentence, f32)>,
    pub(crate) errors: Vec<(String, String)>,
    pub(crate) cues: usize,
}

fn already_loaded(loaded_waves: &BTreeSet<String>, path: &str, distance: bool) -> bool {
    loaded_waves.contains(path)
        || (distance && loaded_waves.contains(&super::distance_variant_key(path, 0.)))
}

/// Resolves `cues` against `vfs` and decodes any wave paths absent from `loaded_waves`.
pub(crate) fn prepare_npc_audio(
    vfs: &Vfs,
    reuse: &AudioReuse,
    loaded_waves: &BTreeSet<String>,
    cues: &BTreeSet<String>,
) -> PreparedLateAudio {
    if cues.is_empty() {
        return PreparedLateAudio::default();
    }
    let library = Library::new(vfs);
    let mut out = PreparedLateAudio {
        cues: cues.len(),
        ..PreparedLateAudio::default()
    };
    let mut paths = BTreeSet::new();
    for cue in cues {
        match library.all_alternatives(cue) {
            Ok(variants) => {
                for path in variants {
                    let distance = library.distance_variant(&path);
                    if !already_loaded(loaded_waves, &path, distance) {
                        paths.insert(path);
                    }
                }
            }
            Err(error) => out.errors.push((cue.clone(), format!("{error:#}"))),
        }
    }
    let cancel = Cancellation::default();
    let mut bytes = 0usize;
    for path in paths {
        let distance = library.distance_variant(&path);
        match reuse.wave(vfs, &path, false, distance, &cancel) {
            Ok(decoded) => {
                let new_waves: Vec<_> = decoded
                    .waves
                    .iter()
                    .filter(|(key, _)| !loaded_waves.contains(key))
                    .collect();
                if new_waves.is_empty() {
                    continue;
                }
                let size: usize = new_waves.iter().map(|(_, data)| data.len()).sum();
                if size > super::MAX_PRELOADED_BYTES.saturating_sub(bytes) {
                    out.errors
                        .push((path, "owned audio preload exceeds 512 MiB budget".into()));
                    continue;
                }
                if let Some(sentence) = &decoded.sentence {
                    out.sentences.push((
                        path.clone(),
                        sentence.clone(),
                        decoded.summary.seconds() as f32,
                    ));
                }
                out.decoded.push((path, decoded.summary.clone()));
                for (key, data) in new_waves {
                    bytes += data.len();
                    out.waves.push((key.clone(), data.clone()));
                }
            }
            Err(error) => out.errors.push((path, format!("{error:#}"))),
        }
    }
    out
}

impl super::Audio {
    /// Returns the set of wave keys currently resident in `Audio`.
    pub(crate) fn loaded_wave_paths(&self) -> BTreeSet<String> {
        self.waves.keys().cloned().collect()
    }

    /// Merges late-decoded NPC waves, metadata, sentences and errors into `Audio` and `lipsync`.
    pub(crate) fn merge_late(
        &mut self,
        late: PreparedLateAudio,
        lipsync: &mut hl2_simulation::lipsync::LipSync,
    ) {
        for (path, bytes) in late.waves {
            if self.waves.contains_key(&path) {
                continue;
            }
            let byte_len = bytes.len();
            if byte_len > super::MAX_PRELOADED_BYTES.saturating_sub(self.bytes) {
                self.library
                    .errors
                    .entry(path)
                    .or_insert_with(|| "owned audio preload exceeds 512 MiB budget".into());
                continue;
            }
            match super::mixer::Pcm::parse(bytes) {
                Some(pcm) => {
                    self.bytes += byte_len;
                    self.waves.insert(path, pcm);
                }
                None => {
                    self.library
                        .errors
                        .entry(path)
                        .or_insert_with(|| "not 16-bit backend PCM".into());
                }
            }
        }
        for (path, summary) in late.decoded {
            self.library.decoded.entry(path).or_insert(summary);
        }
        for (wave, sentence, seconds) in late.sentences {
            lipsync.add_sentence(&wave, sentence, seconds);
        }
        for (key, err) in late.errors {
            if !self.waves.contains_key(&key) && !self.library.decoded.contains_key(&key) {
                self.library.errors.entry(key).or_insert(err);
            }
        }
        self.cues += late.cues;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::{App, Assets, Commands, ResMut};
    use source_assets::sentence::Phoneme;

    fn test_audio() -> super::super::Audio {
        let mut app = App::new();
        app.init_resource::<Assets<super::super::mixer::MixerStream>>();
        app.world_mut()
            .run_system_once(
                |mut commands: Commands,
                 mut streams: ResMut<Assets<super::super::mixer::MixerStream>>| {
                    super::super::install(
                        &mut commands,
                        &mut streams,
                        super::super::PreparedAudio::silent(),
                        false,
                    );
                },
            )
            .unwrap();
        app.world_mut()
            .remove_resource::<super::super::Audio>()
            .unwrap()
    }

    fn test_summary(frames: u32) -> AudioSummary {
        AudioSummary {
            codec: "wav".into(),
            channels: 1,
            sample_rate: super::super::BACKEND_RATE,
            frames,
            seconds: f64::from(frames) / f64::from(super::super::BACKEND_RATE),
        }
    }

    #[test]
    fn class_sound_cues_gates_expected_npc_classes() {
        assert_eq!(
            class_sound_cues("npc_zombie"),
            hl2_simulation::ai::zombie::SOUNDS
        );
        assert_eq!(
            class_sound_cues("npc_zombie_torso"),
            hl2_simulation::ai::zombie::SOUNDS
        );
        assert_eq!(
            class_sound_cues("npc_poisonzombie"),
            hl2_simulation::ai::zombie::SOUNDS
        );
        let fast_expected: Vec<&str> = hl2_simulation::ai::zombie::SOUNDS
            .iter()
            .chain(hl2_simulation::ai::zombie::fast::SOUNDS.iter())
            .copied()
            .collect();
        assert_eq!(class_sound_cues("NPC_FastZombie"), fast_expected);
        assert_eq!(class_sound_cues("npc_fastzombie_torso"), fast_expected);
        for class in [
            "npc_headcrab",
            "npc_headcrab_fast",
            "npc_headcrab_black",
            "npc_headcrab_poison",
        ] {
            assert_eq!(
                class_sound_cues(class),
                hl2_simulation::ai::headcrab::SOUNDS
            );
        }
        assert_eq!(
            class_sound_cues("npc_barnacle"),
            hl2_simulation::ai::barnacle::SOUNDS
        );
        for class in ["npc_crow", "npc_pigeon", "npc_seagull"] {
            assert_eq!(class_sound_cues(class), hl2_simulation::ai::bird::SOUNDS);
        }
        assert_eq!(
            class_sound_cues("npc_manhack"),
            hl2_simulation::ai::manhack::SOUNDS
        );
        for class in ["npc_cscanner", "npc_clawscanner"] {
            assert_eq!(class_sound_cues(class), hl2_simulation::ai::scanner::SOUNDS);
        }
        assert!(class_sound_cues("npc_citizen").is_empty());
        assert!(class_sound_cues("prop_physics").is_empty());
        assert!(class_sound_cues("").is_empty());
    }

    #[test]
    fn merge_late_inserts_new_waves_skips_duplicates_and_updates_lipsync() {
        let mut audio = test_audio();
        let mut lipsync = hl2_simulation::lipsync::LipSync::default();
        let dup_wav: Arc<[u8]> = super::super::wav_bytes(&[0.1; 32], 1).unwrap().into();
        let new_wav: Arc<[u8]> = super::super::wav_bytes(&[0.25; 64], 1).unwrap().into();
        let dup_len = dup_wav.len();
        let new_len = new_wav.len();

        audio.merge_late(
            PreparedLateAudio {
                waves: vec![("late/dup.wav".into(), dup_wav.clone())],
                decoded: vec![("late/dup.wav".into(), test_summary(32))],
                cues: 1,
                ..Default::default()
            },
            &mut lipsync,
        );
        assert_eq!(
            audio.loaded_wave_paths(),
            BTreeSet::from(["late/dup.wav".to_owned()])
        );
        assert_eq!(audio.bytes, dup_len);
        assert_eq!(audio.cues, 1);

        let sentence = Sentence {
            phonemes: vec![Phoneme {
                code: 42,
                start: 0.0,
                end: 0.1,
            }],
            emphasis: Vec::new(),
        };
        audio.merge_late(
            PreparedLateAudio {
                waves: vec![
                    ("late/dup.wav".into(), new_wav.clone()),
                    ("late/new.wav".into(), new_wav),
                    ("late/bad.wav".into(), Arc::from(&b"not wav"[..])),
                ],
                decoded: vec![
                    ("late/dup.wav".into(), test_summary(99)),
                    ("late/new.wav".into(), test_summary(64)),
                ],
                sentences: vec![("late/new.wav".into(), sentence, 0.5)],
                errors: vec![
                    ("late/new.wav".into(), "ignored error".into()),
                    ("late/missing.wav".into(), "absent".into()),
                ],
                cues: 3,
            },
            &mut lipsync,
        );

        assert_eq!(
            audio.loaded_wave_paths(),
            BTreeSet::from(["late/dup.wav".to_owned(), "late/new.wav".to_owned()])
        );
        assert_eq!(audio.bytes, dup_len + new_len);
        assert_eq!(audio.cues, 4);
        assert_eq!(audio.library.decoded["late/dup.wav"].frames, 32);
        assert_eq!(audio.library.decoded["late/new.wav"].frames, 64);
        assert!(lipsync.has_sentence("late/new.wav"));
        assert!(!audio.library.errors.contains_key("late/new.wav"));
        assert!(audio.library.errors.contains_key("late/bad.wav"));
        assert_eq!(
            audio
                .library
                .errors
                .get("late/missing.wav")
                .map(String::as_str),
            Some("absent")
        );
    }

    #[test]
    fn prepare_npc_audio_skips_loaded_waves_and_decodes_new_cues() {
        assert!(
            prepare_npc_audio(
                &Vfs::default(),
                &AudioReuse::default(),
                &BTreeSet::new(),
                &BTreeSet::new(),
            )
            .waves
            .is_empty()
        );

        let root = std::env::temp_dir().join(format!(
            "hl2-audio-late-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("hl2/sound/npc/test")).unwrap();
        std::fs::create_dir_all(root.join("hl2/scripts")).unwrap();
        let wav_a = super::super::wav_bytes(&[0.2; 64], 1).unwrap();
        let wav_b = super::super::wav_bytes(&[0.4; 64], 1).unwrap();
        std::fs::write(root.join("hl2/sound/npc/test/a.wav"), &wav_a).unwrap();
        std::fs::write(root.join("hl2/sound/npc/test/b.wav"), &wav_b).unwrap();
        std::fs::write(
            root.join("hl2/scripts/game_sounds_manifest.txt"),
            "game_sounds_manifest { precache_file scripts/test.txt }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("hl2/scripts/test.txt"),
            "\"Test.Cue\" { \"wave\" \"npc/test/b.wav\" }\n",
        )
        .unwrap();
        let vfs = Vfs::mount_game(&root, "hl2").unwrap();
        let loaded = BTreeSet::from(["npc/test/a.wav".to_owned()]);
        let cues = BTreeSet::from([
            "npc/test/a.wav".to_owned(),
            "Test.Cue".to_owned(),
            "Missing.Cue".to_owned(),
        ]);

        let prepared = prepare_npc_audio(&vfs, &AudioReuse::default(), &loaded, &cues);
        assert_eq!(prepared.cues, 3);
        assert_eq!(prepared.waves.len(), 1);
        assert_eq!(prepared.waves[0].0, "npc/test/b.wav");
        assert_eq!(prepared.decoded.len(), 1);
        assert_eq!(prepared.decoded[0].0, "npc/test/b.wav");
        assert_eq!(prepared.errors.len(), 1);
        assert_eq!(prepared.errors[0].0, "Missing.Cue");

        std::fs::remove_dir_all(root).unwrap();
    }
}
