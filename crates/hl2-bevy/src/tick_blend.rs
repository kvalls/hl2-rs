//! Entities and the view drawn blended across one tick, like retail single player: entities
//! simulated every tick (every move type but `MOVETYPE_STEP`, so physics props and pushers such
//! as doors and trains) and the predicted local player are drawn one tick back, between their
//! last two tick values (Source SDK `game/client/c_baseentity.cpp` `GetInterpolationAmount`,
//! `game/server/baseentity.cpp` `SetMoveType`). View angles stay per frame. NPCs, which retail
//! draws `cl_interp` back, keep their tick poses.
use bevy::prelude::{
    App, DetectChanges, Fixed, FixedUpdate, IntoScheduleConfigs, Plugin, Query, Res, ResMut,
    Resource, RunFixedMainLoop, Time, Vec3,
};

use crate::{gameplay::Gameplay, rendering::SourceEntity, simulation::Simulation};

/// One tick's move over this distance (units) is a teleport and is not blended: an
/// approximation of retail's no-interpolation flag.
const SNAP: f32 = 128.;

type Pose = (glam::Vec3, glam::Quat);

/// The blend as a presentation system takes it (absent in headless setups without the plugin).
pub(crate) type Blend<'w> = Option<Res<'w, TickBlend>>;

/// The values at the start of the current tick and how far the frame is into it.
#[derive(Resource, Default)]
pub struct TickBlend {
    /// Each drawn entity root's pose by entity id, reused across ticks.
    previous: Vec<Option<Pose>>,
    previous_view: Option<Vec3>,
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
    Some(blend.map_or(current, |b| b.pose(id, current)))
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
fn advance(mut blend: ResMut<TickBlend>, fixed: Res<Time<Fixed>>, game: Option<Res<Gameplay>>) {
    if game.is_some_and(|g| g.is_added()) {
        blend.previous.clear();
        blend.previous_view = None;
    }
    blend.alpha = fixed.overstep_fraction().clamp(0., 1.);
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
}
