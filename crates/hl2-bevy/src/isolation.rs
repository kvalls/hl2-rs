//! Startup subsystem isolation flags and Bevy run conditions for performance testing.
use anyhow::{Context, Result};
use bevy::prelude::{Res, Resource};

/// Subsystem isolation flags parsed from CLI arguments.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Isolation {
    pub no_physics_step: bool,
    pub no_animation: bool,
    pub no_flexes: bool,
    pub no_eyes: bool,
    pub no_model_lighting: bool,
    pub no_lightstyles: bool,
    pub no_hud: bool,
    pub no_audio: bool,
    pub no_effects: bool,
    pub force_lod: Option<usize>,
}

impl Isolation {
    /// Parses one CLI flag into this isolation config, returning `Ok(true)` if recognized.
    pub fn parse_flag(
        &mut self,
        arg: &str,
        args: &mut dyn Iterator<Item = String>,
    ) -> Result<bool> {
        match arg {
            "--no-physics-step" => self.no_physics_step = true,
            "--no-animation" => self.no_animation = true,
            "--no-flexes" => self.no_flexes = true,
            "--no-eyes" => self.no_eyes = true,
            "--no-model-lighting" => self.no_model_lighting = true,
            "--no-lightstyles" => self.no_lightstyles = true,
            "--no-hud" => self.no_hud = true,
            "--no-audio" => self.no_audio = true,
            "--no-effects" => self.no_effects = true,
            "--force-lod" => {
                let raw = args
                    .next()
                    .with_context(|| format!("missing value after {arg}"))?;
                let level = raw
                    .parse::<usize>()
                    .with_context(|| format!("invalid value {raw:?} for {arg}"))?;
                self.force_lod = Some(level);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}

pub fn run_animation(isolation: Option<Res<Isolation>>) -> bool {
    isolation.is_none_or(|i| !i.no_animation)
}

pub fn run_eyes(isolation: Option<Res<Isolation>>) -> bool {
    isolation.is_none_or(|i| !i.no_eyes)
}

pub fn run_model_lighting(isolation: Option<Res<Isolation>>) -> bool {
    isolation.is_none_or(|i| !i.no_model_lighting)
}

pub fn run_lightstyles(isolation: Option<Res<Isolation>>) -> bool {
    isolation.is_none_or(|i| !i.no_lightstyles)
}

pub fn run_hud(isolation: Option<Res<Isolation>>) -> bool {
    isolation.is_none_or(|i| !i.no_hud)
}

pub fn run_audio(isolation: Option<Res<Isolation>>) -> bool {
    isolation.is_none_or(|i| !i.no_audio)
}

pub fn run_effects(isolation: Option<Res<Isolation>>) -> bool {
    isolation.is_none_or(|i| !i.no_effects)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::{system::RunSystemOnce, world::World as BevyWorld};

    #[test]
    fn parse_flag_handles_all_boolean_flags() {
        type FlagCheck = fn(&Isolation) -> bool;
        let cases: [(&str, FlagCheck); 9] = [
            ("--no-physics-step", |i| i.no_physics_step),
            ("--no-animation", |i| i.no_animation),
            ("--no-flexes", |i| i.no_flexes),
            ("--no-eyes", |i| i.no_eyes),
            ("--no-model-lighting", |i| i.no_model_lighting),
            ("--no-lightstyles", |i| i.no_lightstyles),
            ("--no-hud", |i| i.no_hud),
            ("--no-audio", |i| i.no_audio),
            ("--no-effects", |i| i.no_effects),
        ];
        for (flag, check) in cases {
            let mut isolation = Isolation::default();
            let mut rest = std::iter::empty::<String>();
            assert!(isolation.parse_flag(flag, &mut rest).unwrap(), "{flag}");
            assert!(check(&isolation), "{flag}");
        }
    }

    #[test]
    fn parse_flag_handles_force_lod_and_unknown_flags() {
        let mut isolation = Isolation::default();
        let mut args = vec!["2".to_owned(), "--trailing".to_owned()].into_iter();
        assert!(isolation.parse_flag("--force-lod", &mut args).unwrap());
        assert_eq!(isolation.force_lod, Some(2));
        assert_eq!(args.next().as_deref(), Some("--trailing"));

        let mut empty = std::iter::empty::<String>();
        assert!(
            isolation
                .parse_flag("--force-lod", &mut empty)
                .unwrap_err()
                .to_string()
                .contains("--force-lod")
        );

        let mut invalid = vec!["bad".to_owned()].into_iter();
        assert!(
            isolation
                .parse_flag("--force-lod", &mut invalid)
                .unwrap_err()
                .to_string()
                .contains("--force-lod")
        );

        let mut untouched = Isolation::default();
        let mut extra = vec!["value".to_owned()].into_iter();
        assert!(!untouched.parse_flag("--unknown-flag", &mut extra).unwrap());
        assert_eq!(untouched, Isolation::default());
        assert_eq!(extra.next().as_deref(), Some("value"));
    }

    #[test]
    fn run_predicates_default_true_and_gate_when_disabled() {
        let mut world = BevyWorld::new();
        assert!(world.run_system_once(run_animation).unwrap());
        assert!(world.run_system_once(run_eyes).unwrap());
        assert!(world.run_system_once(run_model_lighting).unwrap());
        assert!(world.run_system_once(run_lightstyles).unwrap());
        assert!(world.run_system_once(run_hud).unwrap());
        assert!(world.run_system_once(run_audio).unwrap());
        assert!(world.run_system_once(run_effects).unwrap());

        world.insert_resource(Isolation::default());
        assert!(world.run_system_once(run_animation).unwrap());
        assert!(world.run_system_once(run_eyes).unwrap());
        assert!(world.run_system_once(run_model_lighting).unwrap());
        assert!(world.run_system_once(run_lightstyles).unwrap());
        assert!(world.run_system_once(run_hud).unwrap());
        assert!(world.run_system_once(run_audio).unwrap());
        assert!(world.run_system_once(run_effects).unwrap());

        world.insert_resource(Isolation {
            no_animation: true,
            no_eyes: true,
            no_model_lighting: true,
            no_lightstyles: true,
            no_hud: true,
            no_audio: true,
            no_effects: true,
            ..Default::default()
        });
        assert!(!world.run_system_once(run_animation).unwrap());
        assert!(!world.run_system_once(run_eyes).unwrap());
        assert!(!world.run_system_once(run_model_lighting).unwrap());
        assert!(!world.run_system_once(run_lightstyles).unwrap());
        assert!(!world.run_system_once(run_hud).unwrap());
        assert!(!world.run_system_once(run_audio).unwrap());
        assert!(!world.run_system_once(run_effects).unwrap());
    }
}
