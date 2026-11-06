//! A scanner's eye and what its flash does (DESIGN 14o session 47). The numbers are the SDK's
//! npc_scanner and npc_basescanner defines; retail is not compared. `scanner` runs the steps,
//! `runtime::scanners` traces the line to the player, the host draws the sprite and the light.

use crate::ai::conditions::{
    Cond, COND_CAN_MELEE_ATTACK1, COND_NOT_FACING_ATTACK, COND_TOO_FAR_TO_ATTACK,
};
use crate::muzzle_effects::FlashLight;
use crate::player_damage::{ScreenFade, FFADE_IN};
use glam::Vec3;
use serde::Serialize;

/// The sprite on the model's "light" attachment.
pub const EYE_SPRITE: &str = "sprites/blueflare1";
pub const EYE_ATTACHMENT: &str = "light";
/// m_flDistTooFar: past this no attack is looked at at all.
pub const ATTACKS_CHECKED_WITHIN: f32 = 1024.;
/// SCANNER_ATTACK_RANGE, across the ground.
const ATTACK_RANGE: f32 = 350.;
/// How squarely it has to face what it attacks, across the ground.
const ATTACK_FACING: f32 = 0.7;
/// SCANNER_ATTACK_MIN_DELAY and SCANNER_ATTACK_MAX_DELAY.
pub const ATTACK_DELAY: (f64, f64) = (8., 12.);
/// How long the eye warns in red, and how soon after the flash it blinds.
pub const WARNING: f64 = 1.;
pub const BLIND_AFTER: f64 = 0.05;
/// A viewer looking less directly at it than this is spared.
const BLIND_FROM: f32 = 0.5;
/// SCANNER_FLASH_MAX_VALUE, and the share of it left when the game renders with HDR. The owned
/// game runs with HDR and nothing here can read the setting, so that share always applies.
const BLIND_ALPHA: f32 = 240.;
const HDR_SHARE: f32 = 0.9;
/// How long the white takes to clear and how long it stays first, seconds.
const BLIND_FADE: f32 = 3.;
const BLIND_HOLD: f32 = 0.5;

/// What lights the eye: its color and the sprite's scale.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Eye {
    pub color: [u8; 3],
    pub scale: f32,
}
impl Eye {
    pub const WARNING: Self = Self {
        color: [255, 0, 0],
        scale: 0.5,
    };
    pub const PHOTO: Self = Self {
        color: [255; 3],
        scale: 1.4,
    };
    pub const ATTACK: Self = Self {
        color: [255; 3],
        scale: 1.8,
    };
}

/// A lit eye where it is this frame, for the host.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EyeFlare {
    pub entity: usize,
    pub at: Vec3,
    /// The way the body faces: the eye is on its front.
    pub forward: Vec3,
    pub eye: Eye,
}

/// MeleeAttack1Conditions for a scanner whose attack is due: `to_enemy` runs from it to its enemy
/// and `forward` is the way the body faces.
pub fn attack_condition(to_enemy: Vec3, forward: Vec3) -> Cond {
    let across = to_enemy.truncate();
    let facing = forward.truncate().normalize_or_zero();
    if across.length() > ATTACK_RANGE {
        COND_TOO_FAR_TO_ATTACK
    } else if across.normalize_or_zero().dot(facing) < ATTACK_FACING {
        COND_NOT_FACING_ATTACK
    } else {
        COND_CAN_MELEE_ATTACK1
    }
}

/// The white a flash from `at` leaves on a viewer: the more directly they look at it, the brighter.
pub fn blinding(at: Vec3, eye: Vec3, view: Vec3) -> Option<ScreenFade> {
    let looking = (at - eye).normalize_or_zero().dot(view);
    if looking <= BLIND_FROM {
        return None;
    }
    let alpha = ((BLIND_ALPHA * looking) as u8) as f32 * HDR_SHARE;
    let color = [255, 255, 255, alpha as u8];
    Some(ScreenFade::new(color, BLIND_FADE, BLIND_HOLD, FFADE_IN))
}

/// The light of an attack's flash, at what it aims at. A dynamic light in the SDK: here it lights
/// models only, since nothing lights world surfaces yet.
pub fn light(origin: Vec3) -> FlashLight {
    FlashLight {
        origin,
        color: [200, 200, 255],
        radius: 300.,
        decay: 50.,
        lifetime: 0.2,
        exponent: 0,
    }
}
