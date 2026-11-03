//! Immutable audio planning followed by expensive sample preparation off-thread.
mod decoding;
use super::cues;
use super::{Ambient, PreparedAudio, Vox};
use anyhow::Result;
use hl2_simulation::{sounds::Library, soundscapes::Definitions};
use source_assets::vpk::Vfs;
use std::collections::{BTreeSet, HashMap};

pub(crate) struct AudioPlan {
    library: Library,
    cues: usize,
    ambient: HashMap<usize, Ambient>,
    paths: BTreeSet<String>,
    looped: BTreeSet<String>,
    soundscapes: Definitions,
}
impl PreparedAudio {
    pub(crate) fn plan(vfs: &Vfs, game: &crate::gameplay::Gameplay) -> AudioPlan {
        let mut library = Library::new(vfs);
        let mut cues = BTreeSet::new();
        let ambient = ambient(game, &mut library, &mut cues);
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
        // PhysBreakSound: every surfaceprop's break cue (a prop can break on any map).
        cues.extend(
            game.world
                .surface_materials
                .values()
                .filter_map(|m| m.break_sound.clone()),
        );
        for weapon in game.weapons.values() {
            cues.extend(weapon.sounds.values().cloned());
        }
        // Player-ally speech (talker response rules: scene speak sounds, die sounds).
        cues.extend(game.ai.speech_cues());
        // Studio client sound events: viewmodels and NPC sequences (5004 sounds and
        // CL_EVENT_FOOTSTEP names).
        for rig in game.world.rigs.values() {
            cues.extend(hl2_simulation::sounds::animation_sound_names(rig));
        }
        cues.extend(
            cues::FIXED
                .iter()
                .copied()
                .chain(hl2_simulation::ai::zombie::SOUNDS.iter().copied())
                .chain(hl2_simulation::fire::SOUNDS.iter().copied())
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
        AudioPlan {
            library,
            cues: cues.len(),
            ambient,
            paths,
            looped,
            soundscapes,
        }
    }
    /// No sample, VOX, scene or soundscape decoding for the startup placeholder.
    pub fn silent() -> Self {
        Self {
            library: Library::default(),
            waves: Vec::new(),
            ambient: HashMap::new(),
            soundscapes: Definitions::default(),
            bytes: 0,
            cues: 0,
            sentences: Vec::new(),
            ready: false,
            preparation_ms: 0.,
            vox: Vox::default(),
        }
    }
}
impl AudioPlan {
    pub(crate) fn decode(
        self,
        vfs: &Vfs,
        cancel: &crate::assets::work::Cancellation,
        reuse: &super::reuse::AudioReuse,
    ) -> Result<PreparedAudio> {
        let started = std::time::Instant::now();
        let Self {
            mut library,
            cues,
            ambient,
            paths,
            looped,
            soundscapes,
        } = self;
        let specs: Vec<_> = paths
            .into_iter()
            .map(|path| {
                let looped = looped.contains(&path);
                let distance = library.distance_variant(&path);
                (path, looped, distance)
            })
            .collect();
        let decoding::DecodedAudio {
            waves,
            sentences,
            bytes,
            vox,
        } = decoding::load(vfs, &specs, &mut library, reuse, cancel)?;
        Ok(PreparedAudio {
            ready: true,
            preparation_ms: started.elapsed().as_secs_f64() * 1000.,
            library,
            waves,
            ambient,
            soundscapes,
            bytes,
            cues,
            sentences,
            vox,
        })
    }
}

fn ambient(
    game: &crate::gameplay::Gameplay,
    library: &mut Library,
    cues: &mut BTreeSet<String>,
) -> HashMap<usize, Ambient> {
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
    ambient
}
