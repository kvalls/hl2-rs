//! What the simulation asks the effects system to show.
use glam::Vec3;
use hl2_simulation::muzzle_effects::{FlashLight, MuzzleKind};

/// A muzzle flash or tracer for the effects system (muzzle_effects.rs).
#[derive(Clone, Debug)]
pub enum ShotEffect {
    NpcFlash(MuzzleKind, glam::Mat4),
    /// A muzzle elight (operator id, origin, MuzzleFlash_Combine_NPC blue).
    MuzzleLight(usize, glam::Vec3, bool),
    /// Start, end, AR2Tracer.
    NpcTracer(Vec3, Vec3, bool),
    /// Start (ComputeTracerStartPosition), end, the view's right axis, AR2Tracer.
    PlayerTracer(Vec3, Vec3, Vec3, bool),
    /// AR2Impact origin and surface normal.
    Ar2Impact(Vec3, Vec3),
    /// A light that goes off once.
    Light(FlashLight),
}
