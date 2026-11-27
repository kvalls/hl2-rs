//! Entities and the view drawn blended across one tick, like retail single player: entities
//! simulated every tick (every move type but `MOVETYPE_STEP`, so physics props and pushers such
//! as doors and trains) and the predicted local player are drawn one tick back, between their
//! last two tick values (Source SDK `game/client/c_baseentity.cpp` `GetInterpolationAmount`,
//! `game/server/baseentity.cpp` `SetMoveType`). View angles stay per frame. NPCs, which retail
//! draws `cl_interp` back, keep their tick poses.
use bevy::prelude::{
    App, DetectChanges, Fixed, FixedUpdate, IntoScheduleConfigs, Plugin, Query, Res, ResMut,
    Resource, RunFixedMainLoop, Time, Vec3, Virtual,
};
use std::collections::VecDeque;

use crate::{gameplay::Gameplay, rendering::SourceEntity, simulation::Simulation};

/// One tick's move over this distance (units) is a teleport and is not blended: an
/// approximation of retail's no-interpolation flag.
const SNAP: f32 = 128.;
/// The steady lag a carried prop is drawn with behind the view (seconds): one tick, as retail
/// draws it one tick back. See [`TickBlend::held`].
const HELD_LAG: f64 = 0.015;
/// A correction larger than this (radians) is a view reset or teleport, not a turn: drawn as is.
const HELD_SNAP: f32 = 0.35;
/// Frames of view angles kept to look one tick back (8 covers 15 ms down to about 500 FPS).
const VIEW_HISTORY: usize = 8;

type Pose = (glam::Vec3, glam::Quat);

/// The blend as a presentation system takes it (absent in headless setups without the plugin).
pub(crate) type Blend<'w> = Option<Res<'w, TickBlend>>;

/// The values at the start of the current tick and how far the frame is into it.
#[derive(Resource, Default)]
pub struct TickBlend {
    /// Each drawn entity root's pose by entity id, reused across ticks.
    previous: Vec<Option<Pose>>,
    previous_view: Option<Vec3>,
    /// The view angles (yaw, pitch) the previous and the current tick aimed with.
    tick_views: [Option<(f32, f32)>; 2],
    /// Recent frames' virtual time and view angles, oldest first.
    frames: VecDeque<(f64, f32, f32)>,
    /// `Time<Fixed>::overstep_fraction`: 0 at the previous tick, toward 1 at the current one.
    alpha: f32,
}

impl TickBlend {
    /// Entity `id` as drawn this frame, given its pose at the current tick.
    pub fn pose(&self, id: usize, current: Pose) -> Pose {
        match self.previous.get(id).copied().flatten() {
            Some(previous) => blend_pose(previous, current, self.alpha),
            None => current,
        }
    }
    /// The view origin as drawn this frame, given its value at the current tick.
    pub fn view(&self, current: Vec3) -> Vec3 {
        match self.previous_view {
            Some(previous) if previous != current && previous.distance(current) <= SNAP => {
                previous.lerp(current, self.alpha)
            }
            _ => current,
        }
    }
    /// The carried prop as drawn this frame, given its blended pose and the drawn eye.
    ///
    /// Each tick aims the hold at the view angles of whichever frame runs the tick, so at a frame
    /// rate above the tick rate those samples land unevenly late and the blended prop trails the
    /// per-frame view by a lag that pulses (13.3 Hz at 120 FPS). The prop is turned about the eye
    /// from the blended aim of its two ticks to the view of [`HELD_LAG`] ago: a steady one-tick
    /// trail, like retail's, without the pulse. Drawing only; the body stays where it is.
    pub(crate) fn held(&self, blended: Pose, eye: Vec3) -> Pose {
        let eye = glam::Vec3::from_array(eye.to_array());
        let ([Some(from), Some(to)], Some(lagged)) = (self.tick_views, self.lagged_view()) else {
            return blended;
        };
        let aimed = view_rotation(
            lerp_angle(from.0, to.0, self.alpha),
            lerp_angle(from.1, to.1, self.alpha),
        );
        let correction = view_rotation(lagged.0, lagged.1) * aimed.inverse();
        if correction.angle_between(glam::Quat::IDENTITY) > HELD_SNAP {
            return blended;
        }
        (eye + correction * (blended.0 - eye), correction * blended.1)
    }
    /// The view angles [`HELD_LAG`] before the latest frame, interpolated between frames.
    fn lagged_view(&self) -> Option<(f32, f32)> {
        let &(now, ..) = self.frames.back()?;
        let at = now - HELD_LAG;
        let mut later = *self.frames.back()?;
        for &earlier in self.frames.iter().rev().skip(1) {
            if earlier.0 <= at {
                let span = later.0 - earlier.0;
                let t = if span > 0. {
                    ((at - earlier.0) / span) as f32
                } else {
                    1.
                };
                return Some((
                    lerp_angle(earlier.1, later.1, t),
                    lerp_angle(earlier.2, later.2, t),
                ));
            }
            later = earlier;
        }
        // Fewer frames than one tick of history (just loaded or resumed): the oldest one.
        Some((later.1, later.2))
    }
    /// Keeps the latest frame's view angles for [`TickBlend::held`].
    fn push_frame(&mut self, time: f64, (yaw, pitch): (f32, f32)) {
        if self.frames.back().is_some_and(|&(last, ..)| time <= last) {
            self.frames.pop_back();
        }
        if self.frames.len() == VIEW_HISTORY {
            self.frames.pop_front();
        }
        self.frames.push_back((time, yaw, pitch));
    }
}

/// The rotation that turns Source +X into the view direction (`coords::source_direction`).
fn view_rotation(yaw: f32, pitch: f32) -> glam::Quat {
    glam::Quat::from_rotation_z(yaw) * glam::Quat::from_rotation_y(-pitch)
}

/// `from` toward `to` by `t` along the shorter way round.
fn lerp_angle(from: f32, to: f32, t: f32) -> f32 {
    let delta =
        (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    from + delta * t
}

/// The entity the player carries (physcannon or +use), if any.
fn held_entity(game: &Gameplay) -> Option<usize> {
    game.inventory
        .physcannon
        .held
        .or(game.inventory.carry.as_ref().map(|carry| carry.entity))
}

/// `previous` toward `current` by `alpha`; exactly `current` when nothing moved or it jumped.
fn blend_pose(previous: Pose, current: Pose, alpha: f32) -> Pose {
    if previous == current || previous.0.distance(current.0) > SNAP {
        return current;
    }
    (
        previous.0.lerp(current.0, alpha),
        previous.1.slerp(current.1, alpha),
    )
}

/// An entity's pose at the current tick: its physics body, else its scene state.
fn tick_pose(game: &Gameplay, sim: &Simulation, id: usize) -> Option<Pose> {
    let state = game.scene.states.get(id)?;
    Some(
        sim.physics
            .entity_pose(id)
            .unwrap_or((state.origin, state.rotation)),
    )
}

/// Entity `id`'s pose as drawn this frame (blended when `blend` is present).
pub(crate) fn drawn_pose(
    game: &Gameplay,
    sim: &Simulation,
    blend: Option<&TickBlend>,
    id: usize,
) -> Option<Pose> {
    let current = tick_pose(game, sim, id)?;
    let Some(blend) = blend else {
        return Some(current);
    };
    let drawn = blend.pose(id, current);
    Some(if held_entity(game) == Some(id) {
        blend.held(drawn, blend.view(sim.view_origin()))
    } else {
        drawn
    })
}

/// Records the view origin and every drawn entity root's pose before each tick, stepping or
/// not, so a paused scene presents exactly its current values.
fn record(
    mut blend: ResMut<TickBlend>,
    sim: Res<Simulation>,
    game: Option<Res<Gameplay>>,
    roots: Query<&SourceEntity>,
    performance: Option<Res<crate::performance::Performance>>,
) {
    let _timing = crate::performance::scope(performance.as_deref(), "tick_blend");
    let blend = &mut *blend;
    blend.previous_view = Some(sim.view_origin());
    // The tick that follows aims with the angles controls set this frame.
    blend.tick_views = [blend.tick_views[1], Some(sim.view_angles())];
    blend.previous.clear();
    let Some(game) = game else {
        return;
    };
    blend.previous.resize(game.scene.states.len(), None);
    for root in &roots {
        // NPCs (MOVETYPE_STEP) are drawn cl_interp back in retail, not a tick: out of this blend.
        if let Some(slot) = blend.previous.get_mut(root.0)
            && slot.is_none()
            && !game.ai.actors.contains_key(&root.0)
        {
            *slot = tick_pose(&game, &sim, root.0);
        }
    }
}

/// How far this frame is into the tick; a newly installed map drops the old map's values.
fn advance(
    mut blend: ResMut<TickBlend>,
    (fixed, time): (Res<Time<Fixed>>, Res<Time<Virtual>>),
    sim: Res<Simulation>,
    game: Option<Res<Gameplay>>,
) {
    if game.is_some_and(|g| g.is_added()) {
        blend.previous.clear();
        blend.previous_view = None;
        blend.tick_views = [None; 2];
        blend.frames.clear();
    }
    blend.alpha = fixed.overstep_fraction().clamp(0., 1.);
    blend.push_frame(time.elapsed_secs_f64(), sim.view_angles());
}

pub struct TickBlendPlugin;
impl Plugin for TickBlendPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TickBlend>()
            .add_systems(FixedUpdate, record.before(crate::io_trace::flush))
            .add_systems(
                RunFixedMainLoop,
                advance
                    .in_set(bevy::app::RunFixedMainLoopSystems::AfterFixedMainLoop)
                    .before(crate::camera::present_player_view),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, yaw: f32) -> Pose {
        (glam::Vec3::new(x, 0., 0.), glam::Quat::from_rotation_z(yaw))
    }

    #[test]
    fn a_pose_moves_from_the_previous_tick_to_the_current_one() {
        let (from, to) = (at(0., 0.), at(10., 1.));
        assert_eq!(blend_pose(from, to, 0.).0, from.0);
        assert!(blend_pose(from, to, 0.).1.angle_between(from.1) < 1e-5);
        assert_eq!(blend_pose(from, to, 1.).0, to.0);
        assert!(blend_pose(from, to, 1.).1.angle_between(to.1) < 1e-5);
        let half = blend_pose(from, to, 0.5);
        assert_eq!(half.0.x, 5.);
        assert!((half.1.angle_between(from.1) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn unmoved_missing_and_teleported_values_are_exactly_current() {
        let still = at(3., 0.25);
        let drawn = blend_pose(still, still, 0.37);
        assert_eq!(drawn.0.to_array(), still.0.to_array());
        assert_eq!(drawn.1.to_array(), still.1.to_array());
        assert_eq!(blend_pose(at(0., 0.), at(500., 0.), 0.5).0.x, 500.);
        let mut blend = TickBlend {
            alpha: 0.5,
            ..Default::default()
        };
        assert_eq!(blend.pose(4, still).0, still.0);
        assert_eq!(blend.view(Vec3::X), Vec3::X);
        blend.previous = vec![None, Some(at(0., 0.))];
        blend.previous_view = Some(Vec3::ZERO);
        assert_eq!(blend.pose(1, at(2., 0.)).0.x, 1.);
        assert_eq!(blend.view(Vec3::new(4., 0., 0.)).x, 2.);
        assert_eq!(blend.view(Vec3::new(400., 0., 0.)).x, 400.);
    }

    #[test]
    fn view_rotation_points_along_the_source_view_direction() {
        for (yaw, pitch) in [(0., 0.), (1.2, 0.3), (-2.5, -0.7), (3.1, 1.2)] {
            let direction = view_rotation(yaw, pitch) * glam::Vec3::X;
            let expected = crate::coords::source_direction(yaw, pitch);
            assert!((direction - glam::Vec3::from_array(expected.to_array())).length() < 1e-5);
        }
        let near_wrap = lerp_angle(3.1, -3.1, 0.5);
        assert!(
            (near_wrap.abs() - std::f32::consts::PI).abs() < 1e-4,
            "{near_wrap}"
        );
    }

    /// A steady turn at 120 FPS with 15 ms ticks, each tick aiming with the yaw of the frame
    /// that runs it, as the game does: the drawn prop's lag behind the view pulses without the
    /// correction and is a steady one tick with it.
    #[test]
    fn a_carried_prop_trails_a_steady_turn_by_one_tick_without_pulsing() {
        const FRAME: f64 = 1. / 120.;
        const TICK: f64 = 0.015;
        let omega = std::f64::consts::PI; // 180 degrees per second
        let frame_of_tick = |k: i64| ((k as f64 * TICK) / FRAME - 1e-9).ceil();
        let sample = |k: i64| (omega * frame_of_tick(k) * FRAME) as f32;
        let prop = |yaw: f32| {
            let direction = view_rotation(yaw, 0.) * glam::Vec3::X;
            (direction * 55., view_rotation(yaw, 0.))
        };
        let mut blend = TickBlend::default();
        let (mut raw, mut corrected) = (Vec::new(), Vec::new());
        for f in 0..120 {
            let now = f as f64 * FRAME;
            let tick = (now / TICK + 1e-9).floor() as i64;
            blend.push_frame(now, ((omega * now) as f32, 0.));
            blend.tick_views = [Some((sample(tick - 1), 0.)), Some((sample(tick), 0.))];
            blend.alpha = ((now - tick as f64 * TICK) / TICK) as f32;
            let drawn = blend_pose(prop(sample(tick - 1)), prop(sample(tick)), blend.alpha);
            if f < 12 {
                continue; // fill the view history first
            }
            let lag = |pose: Pose| omega as f32 * now as f32 - pose.0.y.atan2(pose.0.x);
            raw.push(lag(drawn));
            corrected.push(lag(blend.held(drawn, Vec3::ZERO)));
        }
        let spread = |lags: &[f32]| {
            let (lo, hi) = lags
                .iter()
                .fold((f32::MAX, f32::MIN), |(lo, hi), &l| (lo.min(l), hi.max(l)));
            hi - lo
        };
        let one_tick = (omega * TICK) as f32;
        assert!(
            spread(&raw) > 0.3 * one_tick,
            "uncorrected pulse {}",
            spread(&raw)
        );
        assert!(
            spread(&corrected) < 0.01 * one_tick,
            "corrected pulse {}",
            spread(&corrected)
        );
        assert!(
            (corrected[0] - one_tick).abs() < 0.01 * one_tick,
            "steady lag {}",
            corrected[0]
        );
    }
}
