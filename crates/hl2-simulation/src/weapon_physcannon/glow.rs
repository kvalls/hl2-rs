//! The gravity gun's sprites and beams on the viewmodel (Source SDK hl2/weapon_physcannon.cpp
//! StartEffects, DoEffectClosed, DoEffectReady, DoEffectHolding, DoEffectIdle, DoEffectLaunch).
//! Each effect asks the center and blast sprites for a scale and a brightness over a time, and
//! CSprite ramps to them: the glow grows into place instead of appearing.

use super::PhyscannonState;
use crate::gameplay::Inventory;
use crate::viewmodel_effects::{Anchor, ViewmodelBeam, ViewmodelEffects, ViewmodelSprite};
use crate::weapon_crossbow::SpriteRamp;
use serde::Serialize;

const ORANGE: [u8; 3] = [255, 128, 0];
const WHITE: [u8; 3] = [255, 255, 255];
/// StartEffects attachment names.
const GLOW_ATTACHMENTS: [&str; 6] = ["fork1b", "fork1m", "fork1t", "fork2b", "fork2m", "fork2t"];
const BEAM_ATTACHMENTS: [&str; 4] = ["fork1t", "fork2t", "fork1t", "fork2t"];
const CORE: &str = "sprites/orangecore1";
const BLAST: &str = "sprites/orangecore2";

/// m_EffectState, without the launch, which is drawn from its own time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum Effect {
    #[default]
    Closed,
    Ready,
    Holding,
}

/// One CSprite: TurnOn or TurnOff, SetScale and SetBrightness with their times.
#[derive(Clone, Copy, Debug, Default, Serialize)]
struct Sprite {
    on: bool,
    scale: SpriteRamp,
    brightness: SpriteRamp,
}
impl Sprite {
    fn ask(&mut self, on: bool, scale: (f32, f32), brightness: (f32, f32), now: f64) {
        self.on = on;
        self.scale.set(scale.0, scale.1, now);
        self.brightness.set(brightness.0, brightness.1, now);
    }
    /// Its scale and brightness now, when it is on and lit.
    fn drawn(&self, now: f64) -> Option<(f32, f32)> {
        let brightness = self.brightness.value(now);
        (self.on && brightness > 0.).then(|| (self.scale.value(now), brightness))
    }
}

/// m_hCenterSprite and m_hBlastSprite.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Glow {
    started: bool,
    pub effect: Effect,
    core: Sprite,
    blast: Sprite,
}
impl Glow {
    /// DoEffect: StartEffects makes the sprites at 0.1 and 255 the first time, then the effect
    /// asks for its own values. Asking again for the same value restarts nothing.
    pub fn show(&mut self, effect: Effect, now: f64) {
        if !std::mem::replace(&mut self.started, true) {
            self.core.ask(true, (0.1, 0.), (255., 0.), now);
            self.blast.ask(false, (0.1, 0.), (255., 0.), now);
        }
        self.effect = effect;
        match effect {
            Effect::Closed => {
                self.core.ask(false, (0., 0.1), (0., 0.1), now);
                self.blast.ask(true, (1., 0.), (0., 0.), now);
            }
            Effect::Ready => {
                self.core.ask(true, (0.15, 0.2), (128., 0.2), now);
                self.blast.ask(true, (0.1, 0.2), (255., 0.1), now);
            }
            Effect::Holding => {
                self.core.ask(true, (0.2, 0.2), (255., 0.1), now);
                self.blast.ask(false, (0.1, 0.), (0., 0.), now);
            }
        }
    }
}

impl PhyscannonState {
    /// The effect the gun shows now: holding with something held or attack2 down, else ready.
    /// ItemPostFrame asks for EFFECT_READY every frame while attack2 is up and nothing is held
    /// (retail's too), open prongs or not, so CloseElements' EFFECT_CLOSED never outlasts its
    /// frame: the idle core stays lit at 0.15 and 128, and a press grows it from there.
    pub(crate) fn show_effect(&mut self, now: f64) {
        let effect = if self.held.is_some() || self.holding_effect {
            Effect::Holding
        } else {
            Effect::Ready
        };
        self.glow.show(effect, now);
    }
}

fn center(material: &'static str, (scale, brightness): (f32, f32)) -> ViewmodelSprite {
    ViewmodelSprite {
        anchor: Anchor::Index(1),
        material,
        scale: (scale, scale),
        brightness: (brightness, brightness),
        color: WHITE,
    }
}

impl Inventory {
    /// What the gravity gun draws on its viewmodel at `time`.
    pub(crate) fn physcannon_effects(&self, time: f64, out: &mut ViewmodelEffects) {
        let glow = &self.physcannon.glow;
        // DoEffectIdle flickers the glow sprites every frame (16-24, 0.3-0.35).
        for name in GLOW_ATTACHMENTS {
            out.sprites.push(ViewmodelSprite {
                anchor: Anchor::Name(name),
                material: "sprites/glow04_noz",
                scale: (0.3, 0.35),
                brightness: (16., 24.),
                color: ORANGE,
            });
        }
        if glow.effect == Effect::Holding {
            // DoEffectHolding: end caps on (flicker 200-255, 0.1-0.15) and the four fork beams
            // to attachment 1.
            for name in ["fork1t", "fork2t"] {
                out.sprites.push(ViewmodelSprite {
                    anchor: Anchor::Name(name),
                    material: "sprites/orangeflare1",
                    scale: (0.1, 0.15),
                    brightness: (200., 255.),
                    color: WHITE,
                });
            }
            for name in BEAM_ATTACHMENTS {
                out.beams.push(ViewmodelBeam {
                    start: Anchor::Name(name),
                    end: Anchor::Index(1),
                    material: "sprites/orangelight1",
                    width: 0.,
                    end_width: (2., 4.),
                    brightness: 128.,
                    color: WHITE,
                    noise: (8., 16.),
                    shade_in: false,
                });
            }
        }
        out.sprites
            .extend(glow.core.drawn(time).map(|now| center(CORE, now)));
        out.sprites
            .extend(glow.blast.drawn(time).map(|now| center(BLAST, now)));
        // DoEffectLaunch: the blast sprite on, scale to 2.0 and brightness to 0 over 0.1 s
        // (from the ready values).
        if let Some((at, _)) = self.physcannon.last_launch {
            let t = ((time - at) / 0.1) as f32;
            if (0. ..1.).contains(&t) {
                out.sprites
                    .push(center(BLAST, (0.1 + (2. - 0.1) * t, 255. * (1. - t))));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_at(glow: &Glow, now: f64) -> (f32, f32) {
        glow.core.drawn(now).expect("core on")
    }

    #[test]
    fn the_core_grows_into_place_over_a_fifth_of_a_second() {
        let mut glow = Glow::default();
        glow.show(Effect::Closed, 0.);
        assert!(glow.core.drawn(0.).is_none());
        glow.show(Effect::Holding, 1.);
        // From the closed 0 to 0.2 over 0.2 s, and to full brightness in half that.
        let (scale, brightness) = core_at(&glow, 1.05);
        assert!((scale - 0.05).abs() < 1e-4, "{scale}");
        assert!((brightness - 127.5).abs() < 0.1, "{brightness}");
        let (scale, brightness) = core_at(&glow, 1.1);
        assert!((scale - 0.1).abs() < 1e-4 && brightness == 255.);
        assert_eq!(core_at(&glow, 1.3), (0.2, 255.));
    }

    #[test]
    fn idle_shows_the_ready_core_and_a_press_grows_it_from_there() {
        let mut state = PhyscannonState::default();
        // Prongs closed, attack2 up: EFFECT_READY after StartEffects' 0.1 and 255.
        state.show_effect(0.);
        assert_eq!(core_at(&state.glow, 0.5), (0.15, 128.));
        state.holding_effect = true;
        state.show_effect(1.);
        let (scale, brightness) = core_at(&state.glow, 1.1);
        assert!((scale - 0.175).abs() < 1e-4, "{scale}");
        assert_eq!(brightness, 255.);
    }

    #[test]
    fn asking_again_every_frame_does_not_restart_the_ramp() {
        let mut glow = Glow::default();
        glow.show(Effect::Closed, 0.);
        for frame in 0..10 {
            glow.show(Effect::Holding, 1. + frame as f64 * 0.015);
        }
        let (scale, _) = core_at(&glow, 1.15);
        assert!((scale - 0.15).abs() < 1e-4, "{scale}");
    }

    #[test]
    fn opening_shrinks_the_blast_in_from_full_size_and_holding_puts_it_out() {
        let mut glow = Glow::default();
        glow.show(Effect::Closed, 0.);
        // Closed: on at scale 1 with no brightness, so nothing is drawn.
        assert!(glow.blast.drawn(0.5).is_none());
        glow.show(Effect::Ready, 1.);
        let (scale, brightness) = glow.blast.drawn(1.1).expect("blast on");
        assert!((scale - 0.55).abs() < 1e-4 && brightness == 255.);
        assert_eq!(glow.blast.drawn(1.3), Some((0.1, 255.)));
        // Ready's core: 0.15 at 128.
        assert_eq!(core_at(&glow, 1.3), (0.15, 128.));
        glow.show(Effect::Holding, 2.);
        assert!(glow.blast.drawn(2.).is_none());
    }
}
