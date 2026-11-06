//! Muzzle flashes (AE_MUZZLEFLASH / AE_NPC_MUZZLEFLASH -> C_BaseAnimating::
//! DispatchMuzzleEffect -> CTempEnts::MuzzleFlash_*) and bullet tracers (FireBullets
//! m_iTracerFreq -> UTIL_Tracer "Tracer" -> FX_PlayerTracer / FX_Tracer), SDK 2013
//! client c_te_legacytempents.cpp, fx.cpp, fx_tracer.cpp, clientsideeffects_test.cpp.
//! Particles are spawned in their attachment's frame at the event and keep that frame
//! (the SDK's local-space emitters follow the attachment for their 0.0001-0.2 s life);
//! muzzle dynamic lights (muzzleflash_light elights) and tracer whiz sounds are not
//! modeled.
use crate::projectile_visuals::Quad;
use glam::{Mat4, Vec3};

/// The weapon-type token of a muzzle event's options ("PISTOL muzzle").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MuzzleKind {
    Combine,
    Smg1,
    Pistol,
    Shotgun,
    Revolver,
    Rpg,
    /// MUZZLEFLASH_AR2 (type 0) of the obsolete CL_EVENT_MUZZLEFLASH0-3 events: the RPG
    /// viewmodel's fire event (v_rpg.mdl "fire", event 5001, no options).
    Ar2,
}
/// DispatchMuzzleEffect: "<type> <attachment>".
pub fn parse(options: &str) -> Option<(MuzzleKind, String)> {
    let mut tokens = options.split_whitespace();
    let kind = match tokens.next()?.to_ascii_uppercase().as_str() {
        "COMBINE" => MuzzleKind::Combine,
        "SMG1" => MuzzleKind::Smg1,
        "PISTOL" => MuzzleKind::Pistol,
        "SHOTGUN" => MuzzleKind::Shotgun,
        "357" => MuzzleKind::Revolver,
        "RPG" => MuzzleKind::Rpg,
        _ => return None,
    };
    Some((kind, tokens.next()?.to_owned()))
}

/// The obsolete CL_EVENT_MUZZLEFLASH0-3 (5001-5004) first-person events
/// (C_BaseAnimating::FireObsoleteEvent -> CTempEnts::MuzzleFlash(pos, angles, atoi(options),
/// ...)) as (kind, 0-based attachment index). The type switch follows retail client.dll
/// (FUN_10249460, build 19307283): 0 AR2 player, 1 shotgun, 2 SMG1, 3/4 pistol,
/// 5 combine; SDK 2013 has no AR2 case.
pub fn parse_obsolete(event: i32, options: &str) -> Option<(MuzzleKind, usize)> {
    if !(5001..=5004).contains(&event) {
        return None;
    }
    let digits: String = options
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let kind = match digits.parse::<i32>().unwrap_or(0) {
        0 => MuzzleKind::Ar2,
        1 => MuzzleKind::Shotgun,
        2 => MuzzleKind::Smg1,
        3 | 4 => MuzzleKind::Pistol,
        5 => MuzzleKind::Combine,
        _ => return None,
    };
    Some((kind, (event - 5001) as usize))
}

/// One SimpleParticle: position/velocity in the emitter frame (x forward).
#[derive(Clone, Debug, PartialEq)]
pub struct Particle {
    pub material: &'static str,
    pub offset: Vec3,
    pub velocity: Vec3,
    pub life: f32,
    pub size: (f32, f32),
    pub color: [u8; 3],
    pub alpha: (f32, f32),
    pub roll: f32,
    pub roll_delta: f32,
}
const PLAYER_FLASH: [&str; 4] = [
    "effects/muzzleflash1_noz",
    "effects/muzzleflash2_noz",
    "effects/muzzleflash3_noz",
    "effects/muzzleflash4_noz",
];
const NPC_FLASH: [&str; 4] = [
    "effects/muzzleflash1",
    "effects/muzzleflash2",
    "effects/muzzleflash3",
    "effects/muzzleflash4",
];
const COMBINE_PLAYER: [&str; 2] = ["effects/combinemuzzle1_noz", "effects/combinemuzzle2_noz"];
const COMBINE_NPC: [&str; 2] = ["effects/combinemuzzle1", "effects/combinemuzzle2"];
const DUST: &str = "particle/particle_smokegrenade";
pub const TRACER_MATERIAL: &str = "effects/spark";
/// The AR2Tracer line material (FX_PlayerAR2Tracer / FX_AR2Tracer, fx_hl2_tracers.cpp).
pub const AR2_TRACER_MATERIAL: &str = "effects/gunshiptracer";
/// AR2ImpactCallback's FX_AddQuad material.
pub const AR2_IMPACT_MATERIAL: &str = "effects/combinemuzzle2_nocull";
/// C_WeaponStunStick::DrawModel halo material.
pub const STUNSTICK_GLOW: &str = "effects/stunstick";
/// Every sprite material these effects draw (preloaded with the projectile sprites).
pub const MATERIALS: &[&str] = &[
    STUNSTICK_GLOW,
    "effects/muzzleflash1_noz",
    "effects/muzzleflash2_noz",
    "effects/muzzleflash3_noz",
    "effects/muzzleflash4_noz",
    "effects/muzzleflash1",
    "effects/muzzleflash2",
    "effects/muzzleflash3",
    "effects/muzzleflash4",
    "effects/combinemuzzle1_noz",
    "effects/combinemuzzle2_noz",
    "effects/combinemuzzle1",
    "effects/combinemuzzle2",
    AR2_CORE,
    AR2_TRACER_MATERIAL,
    AR2_IMPACT_MATERIAL,
];
/// m_pSpriteMuzzleFlash[0] (CTempEnts::LevelInit).
const AR2_CORE: &str = "sprites/ar2_muzzle1";

/// A [0, 1) source.
pub trait Random {
    fn unit(&mut self) -> f32;
    fn float(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }
    fn int(&mut self, low: i32, high: i32) -> i32 {
        (low + ((high - low + 1) as f32 * self.unit()) as i32).min(high)
    }
}
impl<F: FnMut() -> f32> Random for F {
    fn unit(&mut self) -> f32 {
        self().clamp(0., 0.999_999)
    }
}

fn flash(
    material: &'static str,
    x: f32,
    life: f32,
    size: f32,
    color: [u8; 3],
    end_alpha: f32,
    roll: f32,
) -> Particle {
    Particle {
        material,
        offset: Vec3::X * x,
        velocity: Vec3::ZERO,
        life,
        size: (size, size),
        color,
        alpha: (255., end_alpha),
        roll: roll.to_radians(),
        roll_delta: 0.,
    }
}
/// MuzzleFlash_*_Player / _NPC particles for one event.
pub fn particles(kind: MuzzleKind, first_person: bool, r: &mut impl Random) -> Vec<Particle> {
    let mut out = Vec::new();
    let warm = |r: &mut dyn FnMut() -> i32| [255, 255, (200 + r()) as u8];
    match (kind, first_person) {
        // MuzzleFlash_Pistol_Player / _357_Player: a dust puff, then five flashes
        // (pistol life 0.01, scale 1-1.25, step 4; 357 0.01, 1.25-1.5, step 8).
        (MuzzleKind::Pistol | MuzzleKind::Revolver, true) => {
            let pistol = kind == MuzzleKind::Pistol;
            if !pistol || r.int(0, 3) != 0 {
                let color = r.int(200, 255) as u8;
                let start = r.int(2, 4) as f32;
                let (life, speed) = if pistol {
                    (r.float(0.25, 0.5), r.float(48., 64.))
                } else {
                    (r.float(0.5, 1.), r.float(8., 64.))
                };
                let alpha = r.int(64, 128) as f32;
                let up = r.float(4., 16.);
                let roll = r.int(0, 360) as f32;
                let roll_delta = if pistol {
                    r.float(-0.1, 0.1)
                } else {
                    r.float(-0.5, 0.5)
                };
                out.push(Particle {
                    material: DUST,
                    offset: Vec3::X * 8.,
                    // forward * speed + world up (kept in the frame: up is +Z there).
                    velocity: Vec3::new(speed, 0., up),
                    life,
                    size: (start, start * if pistol { 4. } else { 8. }),
                    color: [color; 3],
                    alpha: (alpha, 0.),
                    roll: roll.to_radians(),
                    roll_delta,
                });
            }
            let (scale, step) = if pistol {
                (r.float(1., 1.25), 4.)
            } else {
                (r.float(1.25, 1.5), 8.)
            };
            for i in 1..6 {
                let material = PLAYER_FLASH[r.int(0, 3) as usize];
                let color = warm(&mut || r.int(0, 55));
                let size = r.float(6., 8.) * (8 - i) as f32 / 6. * scale;
                let roll = r.int(0, 360) as f32;
                out.push(flash(
                    material,
                    i as f32 * step * scale,
                    0.01,
                    size,
                    color,
                    255.,
                    roll,
                ));
            }
        }
        // MuzzleFlash_SMG1_Player (0.025 s) / _Shotgun_Player (one frame): five flashes.
        (MuzzleKind::Smg1 | MuzzleKind::Shotgun, true) => {
            let scale = r.float(1.25, 1.5);
            let life = if kind == MuzzleKind::Smg1 {
                0.025
            } else {
                0.0001
            };
            for i in 1..6 {
                let material = PLAYER_FLASH[r.int(0, 3) as usize];
                let color = warm(&mut || r.int(0, 55));
                let size = r.float(6., 8.) * (8 - i) as f32 / 6. * scale;
                let roll = r.int(0, 360) as f32;
                out.push(flash(
                    material,
                    i as f32 * 8. * scale,
                    life,
                    size,
                    color,
                    255.,
                    roll,
                ));
            }
        }
        // MuzzleFlash_Combine_Player: five flashes and a dimmer core at the muzzle.
        (MuzzleKind::Combine, true) => {
            let scale = r.float(2., 2.25);
            for i in 1..6 {
                let material = COMBINE_PLAYER[r.int(0, 1) as usize];
                let color = warm(&mut || r.int(0, 55));
                let size = r.float(6., 8.) * (12 - i) as f32 / 12. * scale;
                let roll = r.int(0, 360) as f32;
                out.push(flash(
                    material,
                    i as f32 * 8. * scale,
                    0.025,
                    size,
                    color,
                    255.,
                    roll,
                ));
            }
            let material = COMBINE_PLAYER[r.int(0, 1) as usize];
            let alpha = r.int(64, 128) as f32;
            let size = r.float(10., 16.);
            let roll = r.int(0, 360) as f32;
            let mut core = flash(material, 0., 0.025, size, [255; 3], 32., roll);
            core.alpha = (alpha, 32.);
            out.push(core);
        }
        // FX_MuzzleEffectAttached: pistol 0.5 / SMG1 1.0 (one frame), shotgun 0.75 (0.1 s).
        (
            MuzzleKind::Pistol | MuzzleKind::Smg1 | MuzzleKind::Shotgun | MuzzleKind::Revolver,
            false,
        ) => {
            let (scale, one_frame) = match kind {
                MuzzleKind::Pistol | MuzzleKind::Revolver => (0.5, true),
                MuzzleKind::Smg1 => (1., true),
                _ => (0.75, false),
            };
            let flash_scale = r.float(scale - 0.25, scale + 0.25).clamp(0.5, 8.);
            for i in 1..9 {
                let material = NPC_FLASH[r.int(0, 3) as usize];
                let size = r.float(6., 9.) * (12 - i) as f32 / 9. * flash_scale;
                let roll = r.int(0, 360) as f32;
                let life = if one_frame { 0.0001 } else { 0.1 };
                out.push(flash(
                    material,
                    i as f32 * 2. * scale,
                    life,
                    size,
                    [255; 3],
                    128.,
                    roll,
                ));
            }
        }
        // MuzzleFlash_Combine_NPC front burst (the side burst and embers are not modeled).
        (MuzzleKind::Combine, false) => {
            let scale = r.float(1., 1.5);
            let burst = r.float(50., 150.);
            for i in 1..6 {
                let material = COMBINE_NPC[r.int(0, 1) as usize];
                let size = r.float(6., 8.) * (6. * 1.25 - i as f32) / 6. * scale;
                let roll = r.int(0, 360) as f32;
                let mut p = flash(
                    material,
                    i as f32 * 2. * scale,
                    0.1,
                    size,
                    [255; 3],
                    0.,
                    roll,
                );
                p.velocity = Vec3::X * burst;
                out.push(p);
            }
        }
        // Retail MuzzleFlash_AR2_Player (client.dll FUN_10249b90; declared but not
        // defined in SDK 2013): temp-entity sprites (kRenderTransAdd, FTENT_PERSIST, die
        // 0.025 s): eight m_pSpriteAR2Flash[0-3] (sprites/ar2_muzzle1b-4b = effects/
        // muzzleflash1-4, 256 px) at i * 8 * s along the barrel, s = U(1.25, 1.5), scale
        // U(6, 8) * (12 - i) / 9 * s / 32, color (164, 164, 164-228), random roll; then
        // m_pSpriteMuzzleFlash[0] (sprites/ar2_muzzle1) at the muzzle, white, scale
        // U(0.05, 0.1). Sizes here are half widths (128 px x scale).
        (MuzzleKind::Ar2, true) => {
            let s = r.float(1.25, 1.5).max(0.5);
            for i in 1..9 {
                let material = NPC_FLASH[r.int(0, 3) as usize];
                let blue = (164 + r.int(0, 64)) as u8;
                let scale = r.float(6., 8.) * (12 - i) as f32 / 9. * s / 32.;
                let roll = r.int(0, 360) as f32;
                out.push(flash(
                    material,
                    i as f32 * 8. * s,
                    0.025,
                    128. * scale,
                    [164, 164, blue],
                    255.,
                    roll,
                ));
            }
            let scale = r.float(0.05, 0.1);
            let roll = r.int(0, 360) as f32;
            out.push(flash(
                AR2_CORE,
                0.,
                0.025,
                128. * scale,
                [255; 3],
                255.,
                roll,
            ));
        }
        // MuzzleFlash_RPG_NPC: FX_MuzzleEffectAttached(1.5) (0.1 s); the player's RPG
        // flash comes from its viewmodel's obsolete event (Ar2 above).
        (MuzzleKind::Rpg, false) => {
            let flash_scale = r.float(1.25, 1.75).clamp(0.5, 8.);
            for i in 1..9 {
                let material = NPC_FLASH[r.int(0, 3) as usize];
                let size = r.float(6., 9.) * (12 - i) as f32 / 9. * flash_scale;
                let roll = r.int(0, 360) as f32;
                out.push(flash(
                    material,
                    i as f32 * 3.,
                    0.1,
                    size,
                    [255; 3],
                    128.,
                    roll,
                ));
            }
        }
        (MuzzleKind::Rpg | MuzzleKind::Ar2, _) => {}
    }
    out
}

/// A spawned emitter: its frame (attachment frame, in world space or the viewmodel's
/// model space) and particles.
#[derive(Clone, Debug)]
pub struct Emitter {
    pub frame: Mat4,
    pub born: f64,
    pub particles: Vec<Particle>,
    pub first_person: bool,
}
/// A bullet tracer line. Player tracers (FX_PlayerTracer) are static segments for
/// 0.01 s; others (FX_Tracer) are FX_AddDiscreetLine lines moving at `speed`.
#[derive(Clone, Debug, PartialEq)]
pub struct Tracer {
    pub start: Vec3,
    pub direction: Vec3,
    pub born: f64,
    pub speed: f32,
    pub length: f32,
    pub clip: f32,
    pub width: f32,
    pub life: f32,
    pub material: &'static str,
}
/// TRACER_SPEED.
const TRACER_SPEED: f32 = 5000.;
/// FX_PlayerTracer from TracerCallback's start (data.m_vStart + right * 4, z - 0.5): none
/// under 256 units; a segment from 8 + U(-24, 64) along the shot, U(0.1, 0.6) of its
/// length, width U(0.5, 0.75), 0.01 s.
pub fn player_tracer(
    start: Vec3,
    end: Vec3,
    right: Vec3,
    now: f64,
    r: &mut impl Random,
) -> Option<Tracer> {
    let start = start + right * 4. - Vec3::Z * 0.5;
    let delta = end - start;
    let length = delta.length();
    if length < 256. {
        return None;
    }
    let direction = delta / length;
    let from = start + direction * (8. + r.float(-24., 64.));
    let segment = length * r.float(0.1, 0.6);
    Some(Tracer {
        start: from,
        direction,
        born: now,
        speed: 0.,
        length: segment,
        clip: 0.,
        width: r.float(0.5, 0.75),
        life: 0.01,
        material: TRACER_MATERIAL,
    })
}
/// FX_PlayerAR2Tracer (AR2TracerCallback for the local player, same start offset as
/// FX_PlayerTracer): none under 128 units; a static line from U(0, 8) along the shot,
/// MIN(length, U(256, 1024)) long, half-width U(6, 12), 0.01 s, effects/gunshiptracer.
pub fn player_ar2_tracer(
    start: Vec3,
    end: Vec3,
    right: Vec3,
    now: f64,
    r: &mut impl Random,
) -> Option<Tracer> {
    let start = start + right * 4. - Vec3::Z * 0.5;
    let delta = end - start;
    let length = delta.length();
    if length < 128. {
        return None;
    }
    let direction = delta / length;
    let from = start + direction * r.float(0., 8.);
    let segment = length.min(r.float(256., 1024.));
    Some(Tracer {
        start: from,
        direction,
        born: now,
        speed: 0.,
        length: segment,
        clip: 0.,
        width: r.float(6., 12.),
        life: 0.01,
        material: AR2_TRACER_MATERIAL,
    })
}
/// FX_AR2Tracer (UTIL_Tracer passes no velocity: the 8000 default): none under 128
/// units; a U(128, 256) unit line moving at 8000, clipped to the end, half-width
/// U(0.5, 1.5), alive until its tail passes the end.
pub fn npc_ar2_tracer(start: Vec3, end: Vec3, now: f64, r: &mut impl Random) -> Option<Tracer> {
    const AR2_TRACER_SPEED: f32 = 8000.;
    let delta = end - start;
    let distance = delta.length();
    if distance < 128. {
        return None;
    }
    let length = r.float(128., 256.);
    Some(Tracer {
        start,
        direction: delta / distance,
        born: now,
        speed: AR2_TRACER_SPEED,
        length,
        clip: distance,
        width: r.float(0.5, 1.5),
        life: (distance + length) / AR2_TRACER_SPEED,
        material: AR2_TRACER_MATERIAL,
    })
}
/// An AR2Impact quad (CWeaponAR2::DoImpactEffect -> AR2ImpactCallback -> FX_AddQuad):
/// on the surface 1 unit out along its normal, size U(24, 32) -> 0 with bias 0.75, alpha
/// 1 -> 0 with bias 0.4, random yaw, white, 0.25 s, FXQUAD_BIAS_SCALE | FXQUAD_BIAS_ALPHA.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceQuad {
    pub origin: Vec3,
    pub normal: Vec3,
    pub born: f64,
    pub start_size: f32,
    pub yaw_degrees: f32,
}
const AR2_IMPACT_LIFE: f32 = 0.25;
/// mathlib Bias: x^(ln b / ln 0.5).
fn bias(x: f32, amount: f32) -> f32 {
    x.powf(amount.ln() / 0.5f32.ln())
}
impl SurfaceQuad {
    pub fn ar2_impact(origin: Vec3, normal: Vec3, now: f64, r: &mut impl Random) -> Self {
        Self {
            origin,
            normal: normal.normalize_or(Vec3::Z),
            born: now,
            start_size: r.float(24., 32.),
            yaw_degrees: r.int(0, 360) as f32,
        }
    }
    /// CFXQuad::Draw at `now`: (size, alpha), None once its life is over.
    pub fn sample(&self, now: f64) -> Option<(f32, f32)> {
        let age = (now - self.born) as f32;
        if !(0. ..=AR2_IMPACT_LIFE).contains(&age) {
            return None;
        }
        let t = age / AR2_IMPACT_LIFE;
        let size = self.start_size * (1. - bias(t, 0.75));
        let alpha = (1. - bias(t, 0.4)).clamp(0., 1.);
        (alpha > 0.).then_some((size, alpha))
    }
    /// The quad's corners (VectorVectors of the normal rotated by the yaw), additive.
    pub fn quad(&self, now: f64) -> Option<Quad> {
        let (size, alpha) = self.sample(now)?;
        let (right, up) = crate::npc_weapons::vector_vectors(self.normal);
        let yaw = self.yaw_degrees.to_radians();
        let quarter = (self.yaw_degrees + 90.).to_radians();
        let r = (right * yaw.cos() - up * yaw.sin()) * (size * 0.5);
        let u = (right * quarter.cos() - up * quarter.sin()) * (size * 0.5);
        let o = self.origin;
        let c = (255. * alpha).round() as u8;
        Some(Quad {
            material: AR2_IMPACT_MATERIAL.into(),
            positions: [o + r - u, o - r - u, o - r + u, o + r + u],
            color: [c, c, c, 255],
            uv: [[1., 1.], [0., 1.], [0., 0.], [1., 0.]],
        })
    }
}
/// FX_Tracer: none under 256 units; a 64-128 unit line moving at TRACER_SPEED, clipped
/// to the end, width U(0.75, 0.9), alive until its tail passes the end.
pub fn npc_tracer(start: Vec3, end: Vec3, now: f64, r: &mut impl Random) -> Option<Tracer> {
    let delta = end - start;
    let distance = delta.length();
    if distance < 256. {
        return None;
    }
    let length = r.float(64., 128.);
    Some(Tracer {
        start,
        direction: delta / distance,
        born: now,
        speed: TRACER_SPEED,
        length,
        clip: distance,
        width: r.float(0.75, 0.9),
        life: (distance + length) / TRACER_SPEED,
        material: TRACER_MATERIAL,
    })
}
impl Tracer {
    /// The visible segment at `now` (CFXDiscreetLine::Draw / CFXStaticLine).
    pub fn segment(&self, now: f64) -> Option<(Vec3, Vec3)> {
        let age = (now - self.born) as f32;
        if age < 0. || age > self.life.max(0.016) {
            return None;
        }
        if self.speed == 0. {
            return Some((self.start, self.start + self.direction * self.length));
        }
        let head = (self.speed * age).min(self.clip);
        let tail = (self.speed * age - self.length).clamp(0., self.clip);
        (head > tail).then(|| {
            (
                self.start + self.direction * tail,
                self.start + self.direction * head,
            )
        })
    }
}

/// One entity light (dlight_t through IVEfx::CL_AllocElight): it lights models only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Elight {
    pub key: i64,
    pub origin: Vec3,
    pub radius: f32,
    pub decay: f32,
    pub die: f64,
    pub color: [u8; 3],
    pub exponent: i8,
}
impl Elight {
    pub fn dynamic_light(&self) -> modkit_core::lighting::DynamicLight {
        modkit_core::lighting::DynamicLight {
            origin: self.origin,
            radius: self.radius,
            color: self.color,
            exponent: self.exponent,
            minlight: 0.,
        }
    }
}
/// The engine's 64 elight slots (retail engine.dll 0x1048b3f0, reviewed privately).
pub const MAX_ELIGHTS: usize = 64;
/// LIGHT_INDEX_MUZZLEFLASH (dlight.h); keys add the entity index.
pub const LIGHT_INDEX_MUZZLEFLASH: i64 = 0x100000;
/// A light that goes off once (te->DynamicLight): its life in seconds, its radius lost per second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlashLight {
    pub origin: Vec3,
    pub color: [u8; 3],
    pub radius: f32,
    pub decay: f32,
    pub lifetime: f64,
    pub exponent: i8,
}
#[derive(Clone, Debug)]
pub struct Elights {
    pub slots: [Option<Elight>; MAX_ELIGHTS],
    pub allocated: u64,
}
impl Default for Elights {
    fn default() -> Self {
        Self {
            slots: [None; MAX_ELIGHTS],
            allocated: 0,
        }
    }
}
impl Elights {
    /// CL_AllocElight (retail 0x100de1a0): the slot with the same nonzero key, else the
    /// first dead one (die < now), else slot 0; cleared and keyed.
    pub fn alloc(&mut self, key: i64, now: f64) -> &mut Elight {
        let index = (key != 0)
            .then(|| {
                self.slots
                    .iter()
                    .position(|s| s.is_some_and(|l| l.key == key))
            })
            .flatten()
            .or_else(|| {
                self.slots
                    .iter()
                    .position(|s| s.is_none_or(|l| l.die < now))
            })
            .unwrap_or(0);
        self.allocated += 1;
        self.slots[index].insert(Elight {
            key,
            origin: Vec3::ZERO,
            radius: 0.,
            decay: 0.,
            die: 0.,
            color: [0; 3],
            exponent: 0,
        })
    }
    /// A light of no entity, in the first free slot.
    pub fn flash(&mut self, flash: FlashLight, now: f64) {
        let light = self.alloc(0, now);
        light.origin = flash.origin;
        light.color = flash.color;
        light.radius = flash.radius;
        light.decay = flash.decay;
        light.die = now + flash.lifetime;
        light.exponent = flash.exponent;
    }
    /// C_BaseAnimating::ProcessMuzzleFlashEvent (attachment 1 of the flashing model):
    /// (255, 192, 64) exponent 5, radius RandomInt(32, 64) shrinking to 0 in 0.05 s.
    /// CTempEnts::MuzzleFlash_Combine_NPC: (64, 128, 255), radius RandomInt(32, 128).
    pub fn muzzle_flash(
        &mut self,
        entity: usize,
        origin: Vec3,
        combine: bool,
        now: f64,
        r: &mut impl Random,
    ) {
        let radius = if combine {
            r.int(32, 128)
        } else {
            r.int(32, 64)
        } as f32;
        let light = self.alloc(LIGHT_INDEX_MUZZLEFLASH + entity as i64, now);
        light.origin = origin;
        light.radius = radius;
        light.decay = radius / 0.05;
        light.die = now + 0.05;
        light.color = if combine {
            [64, 128, 255]
        } else {
            [255, 192, 64]
        };
        light.exponent = 5;
    }
    /// CL_DecayLights for elights (retail 0x100de690), once per rendered frame:
    /// radius -= frametime * decay, cleared after `die` or at 0.
    pub fn decay(&mut self, now: f64, frametime: f32) {
        if frametime <= 0. {
            return;
        }
        for slot in &mut self.slots {
            if let Some(light) = slot {
                if now > light.die {
                    *slot = None;
                    continue;
                }
                light.radius -= frametime * light.decay;
                if light.radius <= 0. {
                    *slot = None;
                }
            }
        }
    }
    /// Lights with a radius left, as the model-lighting pass reads them.
    pub fn active(&self) -> impl Iterator<Item = &Elight> {
        self.slots.iter().flatten().filter(|l| l.radius > 0.)
    }
}

/// Live muzzle emitters and tracers.
#[derive(Clone, Debug, Default)]
pub struct MuzzleEffects {
    pub emitters: Vec<Emitter>,
    pub tracers: Vec<Tracer>,
    pub surface_quads: Vec<SurfaceQuad>,
    pub elights: Elights,
    pub flashes: u64,
    pub tracers_made: u64,
}
impl MuzzleEffects {
    pub fn flash(
        &mut self,
        kind: MuzzleKind,
        first_person: bool,
        frame: Mat4,
        now: f64,
        r: &mut impl Random,
    ) {
        let particles = particles(kind, first_person, r);
        if !particles.is_empty() {
            self.flashes += 1;
            self.emitters.push(Emitter {
                frame,
                born: now,
                particles,
                first_person,
            });
        }
    }
    pub fn tracer(&mut self, tracer: Option<Tracer>) {
        if let Some(t) = tracer {
            self.tracers_made += 1;
            self.tracers.push(t);
        }
    }
    /// Drop finished effects; a particle stays at least one drawn frame.
    pub fn retain(&mut self, now: f64) {
        self.emitters.retain(|e| {
            let age = (now - e.born) as f32;
            e.particles.iter().any(|p| age <= p.life.max(0.016))
        });
        self.tracers
            .retain(|t| ((now - t.born) as f32) <= t.life.max(0.016));
        self.surface_quads.retain(|q| q.sample(now).is_some());
    }
    /// Camera-facing particle quads for the emitters of one space (`first_person`):
    /// `right`/`up` are that space's view axes; `scale` multiplies sizes and offsets.
    pub fn particle_quads(
        &self,
        first_person: bool,
        now: f64,
        right: Vec3,
        up: Vec3,
        scale: f32,
    ) -> Vec<Quad> {
        let mut out = Vec::new();
        for e in self
            .emitters
            .iter()
            .filter(|e| e.first_person == first_person)
        {
            let age = (now - e.born) as f32;
            for p in &e.particles {
                if age > p.life.max(0.016) {
                    continue;
                }
                let t = (age / p.life.max(1e-4)).clamp(0., 1.);
                let local = p.offset + p.velocity * age;
                let center = e.frame.transform_point3(local * scale);
                let size = (p.size.0 + (p.size.1 - p.size.0) * t) * scale;
                let alpha = (p.alpha.0 + (p.alpha.1 - p.alpha.0) * t).clamp(0., 255.) / 255.;
                out.push(billboard(
                    p.material,
                    center,
                    right,
                    up,
                    size,
                    p.roll + p.roll_delta * age,
                    p.color,
                    alpha,
                ));
            }
        }
        out
    }
    /// Tracer line quads facing `eye`.
    pub fn tracer_quads(&self, now: f64, eye: Vec3) -> Vec<Quad> {
        self.tracers
            .iter()
            .filter_map(|t| {
                let (a, b) = t.segment(now)?;
                let cross = (b - a).cross(b - eye).normalize_or_zero() * t.width;
                (cross != Vec3::ZERO).then(|| Quad {
                    material: t.material.into(),
                    positions: [a - cross, a + cross, b + cross, b - cross],
                    color: [255, 255, 255, 255],
                    uv: [[1., 0.], [0., 0.], [0., 1.], [1., 1.]],
                })
            })
            .chain(self.surface_quads.iter().filter_map(|q| q.quad(now)))
            .collect()
    }
}
/// C_WeaponStunStick::DrawModel while active: two DrawHalo sprites 1 unit behind
/// attachment 1 (radius 4-6 at grey 0.1-0.2, then 2-3 at 0.9-1.0), redrawn with new random
/// sizes every frame. The 1-in-6 TE_BEAMTESLA arcs of ClientThink are not modeled.
pub fn stunstick_glow(attachment: Mat4, right: Vec3, up: Vec3, r: &mut impl Random) -> [Quad; 2] {
    let center = attachment.transform_point3(Vec3::NEG_X);
    let grey = |v: f32| [(v * 255.).round() as u8; 3];
    let (outer, outer_size) = (grey(r.float(0.1, 0.2)), r.float(4., 6.));
    let (inner, inner_size) = (grey(r.float(0.9, 1.)), r.float(2., 3.));
    [
        billboard(STUNSTICK_GLOW, center, right, up, outer_size, 0., outer, 1.),
        billboard(STUNSTICK_GLOW, center, right, up, inner_size, 0., inner, 1.),
    ]
}
/// An additive sprite quad (rgb x alpha) or, for the dust puff, a blended one.
#[allow(clippy::too_many_arguments)]
fn billboard(
    material: &str,
    center: Vec3,
    right: Vec3,
    up: Vec3,
    size: f32,
    roll: f32,
    color: [u8; 3],
    alpha: f32,
) -> Quad {
    let (sin, cos) = roll.sin_cos();
    let r = (right * cos + up * sin) * size;
    let u = (right * -sin + up * cos) * size;
    let blended = material == DUST;
    let c = |v: u8| {
        if blended {
            v
        } else {
            (f32::from(v) * alpha).round() as u8
        }
    };
    Quad {
        material: material.into(),
        positions: [
            center - r - u,
            center - r + u,
            center + r + u,
            center + r - u,
        ],
        color: [
            c(color[0]),
            c(color[1]),
            c(color[2]),
            if blended {
                (alpha * 255.).round() as u8
            } else {
                255
            },
        ],
        uv: crate::explosion_particles::UV,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn muzzle_elights_alloc_by_key_and_decay_like_the_engine() {
        let mut lights = Elights::default();
        let mut half = || 0.5;
        lights.muzzle_flash(7, Vec3::X, false, 1., &mut half);
        let first = *lights.active().next().unwrap();
        // RandomInt(32, 64) at 0.5 -> 48; 255/192/64 exponent 5; decay radius / 0.05.
        assert_eq!(first.radius, 48.);
        assert_eq!(first.decay, 960.);
        assert_eq!(first.color, [255, 192, 64]);
        assert_eq!(first.key, LIGHT_INDEX_MUZZLEFLASH + 7);
        // Same key reuses the slot (the Combine NPC flash overrides the weapon's).
        lights.muzzle_flash(7, Vec3::Y, true, 1.01, &mut half);
        assert_eq!(lights.active().count(), 1);
        assert_eq!(lights.active().next().unwrap().color, [64, 128, 255]);
        lights.muzzle_flash(8, Vec3::Y, false, 1.01, &mut half);
        assert_eq!(lights.active().count(), 2);
        // 1/60 s frames: 48 -> 32 -> 16 -> gone (radius 0 at 0.05 s).
        let mut radii = Vec::new();
        for frame in 1..=4 {
            lights.decay(1.01 + frame as f64 / 60., 1. / 60.);
            radii.push(
                lights
                    .slots
                    .iter()
                    .flatten()
                    .find(|l| l.key == LIGHT_INDEX_MUZZLEFLASH + 8)
                    .map(|l| l.radius),
            );
        }
        assert!((radii[0].unwrap() - 32.).abs() < 1e-3);
        assert!((radii[1].unwrap() - 16.).abs() < 1e-3);
        assert!(radii[2].is_none_or(|r| r < 1e-3) && radii[3].is_none());
    }
    #[test]
    fn options_kinds_and_sdk_particle_sets() {
        assert_eq!(
            parse("PISTOL muzzle"),
            Some((MuzzleKind::Pistol, "muzzle".into()))
        );
        assert_eq!(parse("357 muzzle").unwrap().0, MuzzleKind::Revolver);
        assert_eq!(parse("COMBINE MUZZLE").unwrap().0, MuzzleKind::Combine);
        assert!(parse("FOO muzzle").is_none());
        let mut half = || 0.5;
        // NPC pistol: FX_MuzzleEffectAttached(0.5): eight one-frame flashes 1 unit apart.
        let npc = particles(MuzzleKind::Pistol, false, &mut half);
        assert_eq!(npc.len(), 8);
        assert!((npc[1].offset.x - 2.).abs() < 1e-4);
        assert_eq!(npc[0].life, 0.0001);
        // Player SMG1: five 0.025 s flashes; player shotgun: one frame.
        assert_eq!(particles(MuzzleKind::Smg1, true, &mut half).len(), 5);
        assert!(particles(MuzzleKind::Shotgun, true, &mut half)
            .iter()
            .all(|p| p.life == 0.0001));
        // Player 357: always the dust puff plus five flashes.
        let revolver = particles(MuzzleKind::Revolver, true, &mut half);
        assert_eq!(revolver.len(), 6);
        assert_eq!(revolver[0].material, DUST);
        assert!(particles(MuzzleKind::Rpg, true, &mut half).is_empty());
        // v_rpg.mdl's event 5001 with no options: retail type 0, the AR2 player flash:
        // eight barrel sprites and the core, 0.025 s, the first one ~47 units half-width.
        assert_eq!(parse_obsolete(5001, ""), Some((MuzzleKind::Ar2, 0)));
        assert_eq!(parse_obsolete(5002, "4"), Some((MuzzleKind::Pistol, 1)));
        assert_eq!(parse_obsolete(3014, ""), None);
        let rpg = particles(MuzzleKind::Ar2, true, &mut half);
        assert_eq!(rpg.len(), 9);
        assert!(rpg.iter().all(|p| p.life == 0.025));
        assert!((rpg[0].size.0 - 128. * 7. * 11. / 9. * 1.375 / 32.).abs() < 1e-3);
        assert!((rpg[7].offset.x - 8. * 8. * 1.375).abs() < 1e-3);
        assert_eq!(rpg[8].material, AR2_CORE);
        assert_eq!(particles(MuzzleKind::Rpg, false, &mut half).len(), 8);
    }
    #[test]
    fn tracers_skip_short_shots_and_npc_lines_travel_at_tracer_speed() {
        let mut half = || 0.5;
        assert!(npc_tracer(Vec3::ZERO, Vec3::X * 200., 0., &mut half).is_none());
        let t = npc_tracer(Vec3::ZERO, Vec3::X * 1000., 0., &mut half).unwrap();
        assert_eq!(t.length, 96.);
        // After 0.1 s the head is at 500 and the tail 96 behind it.
        let (a, b) = t.segment(0.1).unwrap();
        assert!((b.x - 500.).abs() < 1e-3 && (a.x - 404.).abs() < 1e-3);
        // Clipped at the end; gone once the tail passes it.
        let (_, b) = t.segment(0.2).unwrap();
        assert!((b.x - 1000.).abs() < 1e-3);
        assert!(t.segment(0.3).is_none());
        let p = player_tracer(Vec3::ZERO, Vec3::X * 1000., Vec3::NEG_Y, 0., &mut half).unwrap();
        // Start: right * 4 (y -4), z - 0.5, then 8 + U(-24, 64) = 28 along the shot.
        assert!(
            (p.start.x - 28.).abs() < 0.1 && (p.start.y + 4.).abs() < 0.2,
            "{p:?}"
        );
        assert!(player_tracer(Vec3::ZERO, Vec3::X * 100., Vec3::NEG_Y, 0., &mut half).is_none());
    }
    #[test]
    fn ar2_tracers_and_impact_quads_follow_the_sdk_parameters() {
        let mut half = || 0.5;
        let p = player_ar2_tracer(Vec3::ZERO, Vec3::X * 1000., Vec3::NEG_Y, 0., &mut half).unwrap();
        assert_eq!(p.material, AR2_TRACER_MATERIAL);
        assert!((p.width - 9.).abs() < 1e-4 && (p.length - 640.).abs() < 1e-3);
        assert!(
            player_ar2_tracer(Vec3::ZERO, Vec3::X * 100., Vec3::NEG_Y, 0., &mut half).is_none()
        );
        let n = npc_ar2_tracer(Vec3::ZERO, Vec3::X * 800., 0., &mut half).unwrap();
        assert_eq!(n.speed, 8000.);
        assert!((n.life - (800. + 192.) / 8000.).abs() < 1e-6);
        let q = SurfaceQuad::ar2_impact(Vec3::ZERO, Vec3::Z, 0., &mut half);
        assert_eq!(q.start_size, 28.);
        let (size, alpha) = q.sample(0.).unwrap();
        assert!((size - 28.).abs() < 1e-4 && (alpha - 1.).abs() < 1e-6);
        // Half-way: Bias(0.5, b) = b.
        let (size, alpha) = q.sample(0.125).unwrap();
        assert!((size - 28. * 0.25).abs() < 1e-3 && (alpha - 0.6).abs() < 1e-4);
        assert!(q.sample(0.3).is_none());
        let quad = q.quad(0.).unwrap();
        assert!(quad.positions.iter().all(|p| p.z.abs() < 1e-4));
    }
    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn owned_ar2_tracer_blends_alpha_add_not_opaque() {
        let vfs =
            source_assets::vpk::Vfs::mount(&source_assets::install::discover().unwrap()).unwrap();
        let tracer = crate::projectile_visuals::load_sprite(&vfs, AR2_TRACER_MATERIAL).unwrap();
        assert_eq!(tracer.kind, crate::projectile_visuals::KIND_ALPHA_ADD);
        for name in MATERIALS
            .iter()
            .filter(|name| **name != AR2_TRACER_MATERIAL)
        {
            assert_ne!(
                crate::projectile_visuals::load_sprite(&vfs, name)
                    .unwrap()
                    .kind,
                0,
                "{name}"
            );
        }
    }
}
