//! Host adapters for the AI core: `Motor` over the npc.rs ground controller and the
//! scene's actor animation, and `SightWorld` over the Rapier world queries.
use super::schedule::{Goal, Motor, MoveStatus};
use super::senses::SightWorld;
use crate::{
    entities::Scene,
    npc::{ActorPose, Controller, GoalKey, GoalState, MoveRequest, MoveStyle, AI_SCENE},
    npc_probe::ActorHull,
    physics::Physics,
};
use glam::Vec3;
use modkit_core::World;

/// CAI_Motor over the shared controller for one NPC during one think.
pub struct SceneMotor<'a> {
    pub entity: usize,
    pub world: &'a World,
    pub scene: &'a mut Scene,
    pub controller: &'a mut Controller,
    pub physics: &'a Physics,
    pub transients: &'a [ActorHull],
    /// The NPC's current AI navigation goal.
    pub goal: &'a mut Option<GoalKey>,
    /// Goal serials (GoalKey::event) for AI_SCENE keys.
    pub serial: &'a mut usize,
    /// Seconds since the NPC's last think (UpdateYaw clamps it to 0.2).
    pub dt: f32,
    /// MaxYawSpeed for the current activity (the motor turns at 10x it per second).
    pub yaw_speed: f32,
    /// CNPC_MetroPolice::NPC_TranslateActivity: ACT_IDLE becomes ACT_IDLE_ANGRY while
    /// the weapon is drawn and the cop is in combat or has the baton on.
    pub idle_angry: bool,
    /// The active weapon's class (Weapon_TranslateActivity).
    pub weapon: Option<String>,
    /// CNPC_Combine::NPC_TranslateActivity: ACT_RANGE_ATTACK2 is the grenade throw
    /// (ACT_COMBINE_THROW_GRENADE; combine_soldier has no ACT_RANGE_ATTACK2 sequence).
    pub combine: bool,
    /// CZombie::NPC_TranslateActivity: ACT_RUN becomes ACT_WALK (routes walk).
    pub walk_only: bool,
    /// CNPC_BaseZombie::NPC_TranslateActivity while IsOnFire.
    pub on_fire: bool,
    /// Weapon_SetActivity requests of this think: (weapon activity, NPC sequence
    /// duration), applied by the runtime to the weapon's world model.
    pub weapon_activity: Option<(String, f32)>,
    /// Diagnostics: refused move requests by reason.
    pub blocked_moves: &'a mut std::collections::BTreeMap<String, u32>,
}
pub fn yaw_of(scene: &Scene, entity: usize) -> f32 {
    let forward = scene.states[entity].rotation * Vec3::X;
    forward.y.atan2(forward.x).to_degrees()
}
pub fn angle_diff(a: f32, b: f32) -> f32 {
    (a - b + 180.).rem_euclid(360.) - 180.
}
/// AI_ClampYaw: move `current` toward `ideal` by at most `speed` x `dt` degrees.
pub fn clamp_yaw(speed: f32, current: f32, ideal: f32, dt: f32) -> f32 {
    let delta = angle_diff(ideal, current);
    let step = speed * dt;
    if delta.abs() <= step {
        ideal
    } else {
        current + step * delta.signum()
    }
}
impl SceneMotor<'_> {
    fn release_goal(&mut self) {
        if let Some(key) = self.goal.take() {
            self.controller.cancel(key);
            self.controller.forget(key);
            self.scene.ai_forget_move(key);
        }
    }
}
impl SceneMotor<'_> {
    /// CAI_BaseNPC::NPC_TranslateActivity -> Weapon_TranslateActivity: the active
    /// weapon's acttable, kept only when the model has the result.
    fn translate_activity(&self, activity: &str) -> String {
        let weapon = self.weapon.as_deref().unwrap_or("");
        let activity = if self.idle_angry && activity == "ACT_IDLE" {
            "ACT_IDLE_ANGRY"
        } else if self.combine && activity == "ACT_RANGE_ATTACK2" {
            "ACT_COMBINE_THROW_GRENADE"
        } else if self.walk_only && activity == "ACT_RUN" {
            "ACT_WALK"
        } else {
            activity
        };
        let on_fire = match activity {
            "ACT_WALK" | "ACT_RUN_ON_FIRE" if self.on_fire => Some("ACT_WALK_ON_FIRE"),
            "ACT_IDLE" if self.on_fire => Some("ACT_IDLE_ON_FIRE"),
            _ => None,
        };
        let activity = on_fire
            .filter(|a| self.scene.actor_has_activity(self.world, self.entity, a))
            .unwrap_or(activity);
        let translated = crate::npc_weapons::translate(weapon, activity)
            .filter(|t| self.scene.actor_has_activity(self.world, self.entity, t));
        translated.unwrap_or(activity).to_owned()
    }
    /// Weapon_SetActivity(weapon activity, SequenceDuration) after a sequence change.
    fn weapon_follows(&mut self, activity: String) {
        if self.weapon.is_some() {
            let duration = self.scene.ai_sequence_duration(self.world, self.entity);
            self.weapon_activity = Some((activity, duration));
        }
    }
}
impl Motor for SceneMotor<'_> {
    fn set_activity(&mut self, activity: &str) {
        let activity = self.translate_activity(activity);
        if self
            .scene
            .ai_set_activity(self.world, self.entity, &activity)
        {
            self.weapon_follows(activity);
        }
    }
    fn reselect_activity(&mut self, activity: &str) {
        let activity = self.translate_activity(activity);
        if self
            .scene
            .ai_reselect_activity(self.world, self.entity, &activity)
        {
            self.weapon_follows(activity);
        }
    }
    fn activity_finished(&self) -> bool {
        self.scene.ai_activity_finished(self.entity)
    }
    fn stop_moving(&mut self) -> bool {
        self.release_goal();
        true
    }
    /// CAI_Motor::UpdateYaw toward the goal; FacingIdeal (within 0.006 degrees).
    fn face(&mut self, goal: Goal) -> bool {
        let state = &self.scene.states[self.entity];
        let ideal = match goal {
            Goal::Yaw(yaw) => yaw,
            Goal::Position(p) => {
                let d = p - state.origin;
                if d.truncate().length_squared() < 1e-6 {
                    return true;
                }
                d.y.atan2(d.x).to_degrees()
            }
        };
        let current = yaw_of(self.scene, self.entity);
        let next = clamp_yaw(self.yaw_speed * 10., current, ideal, self.dt.min(0.2));
        if next != current {
            self.scene.ai_set_yaw(self.entity, next);
        }
        angle_diff(ideal, next).abs() <= 0.006
    }
    fn path_to(&mut self, goal: Vec3, run: bool) -> bool {
        let run = run && !self.walk_only;
        // Goal entities often float above the floor (the d1_canals_01 rally point is 32
        // units up); ground routes end on the floor below (CAI_MoveProbe::FloorPoint over
        // the step height and a little more), else at the goal as given.
        let hull = crate::npc_probe::Hull {
            mins: HUMAN_MINS,
            maxs: HUMAN_MAXS,
        };
        let goal = crate::npc_makers::floor_point(self.physics, goal, hull, self.entity, 0., -72.)
            .unwrap_or(goal);
        self.release_goal();
        *self.serial += 1;
        let key = GoalKey {
            scene: AI_SCENE,
            event: *self.serial,
            actor: self.entity,
        };
        let state = &self.scene.states[self.entity];
        let pose = ActorPose {
            entity: self.entity,
            feet: state.origin,
            yaw_degrees: yaw_of(self.scene, self.entity),
            scripted_by: None,
        };
        self.scene.ai_register_move(key);
        let status = self.controller.request(
            key,
            MoveRequest {
                target_entity: self.entity,
                target_feet: goal,
                style: if run { MoveStyle::Run } else { MoveStyle::Walk },
                event_distance: 0.,
                force_short: false,
            },
            pose,
            self.physics,
            self.transients,
        );
        *self.goal = Some(key);
        if let GoalState::Blocked(reason) = &status {
            let reason = format!("{reason:?}");
            let reason: String = reason.chars().take(160).collect();
            *self.blocked_moves.entry(reason).or_default() += 1;
        }
        !matches!(status, GoalState::Blocked(_) | GoalState::Canceled)
    }
    /// FindCoverNode with CAI_BaseNPC::IsCoverPosition (the threat's eye cannot see the
    /// node's eye point: feet + the standing eye offset, MASK_BLOCKLOS ignoring this NPC).
    fn find_cover(&mut self, threat: Vec3, threat_eye: Vec3, min: f32, max: f32) -> Option<Vec3> {
        let sight = PhysicsSight {
            physics: self.physics,
            world: self.world,
        };
        let me = self.entity;
        let origin = self.scene.states[me].origin;
        self.controller
            .find_cover_node(origin, threat, min, max, |feet| {
                !sight.visible(threat_eye, feet + Vec3::Z * EYE_HEIGHT, &[me])
            })
    }
    /// FindLosNode with TestShootPosition (WeaponLOSCondition from the node's
    /// Weapon_ShootPosition, m_HackedGunPos 55 above the feet, to the threat's eye).
    fn find_los(&mut self, threat: Vec3, threat_eye: Vec3, min: f32, max: f32) -> Option<Vec3> {
        let sight = PhysicsSight {
            physics: self.physics,
            world: self.world,
        };
        let me = self.entity;
        let origin = self.scene.states[me].origin;
        self.controller
            .find_los_node(origin, threat, min, max, |feet| {
                sight.visible(feet + Vec3::Z * GUN_HEIGHT, threat_eye, &[me])
            })
    }
    fn movement(&self) -> MoveStatus {
        let Some(key) = *self.goal else {
            return MoveStatus::Idle;
        };
        match self.controller.status(key) {
            Some(GoalState::Arrived) => MoveStatus::Arrived,
            Some(GoalState::Moving) => MoveStatus::Moving,
            Some(_) => MoveStatus::Failed,
            None => MoveStatus::Idle,
        }
    }
}

/// FVisible with MASK_BLOCKLOS: world and solid entities (brushes, doors, props) block
/// sight; NPC bodies (CONTENTS_MONSTER) do not, so they are skipped.
pub struct PhysicsSight<'a> {
    pub physics: &'a Physics,
    pub world: &'a World,
}
impl SightWorld for PhysicsSight<'_> {
    fn visible(&self, from: Vec3, to: Vec3, ignore: &[usize]) -> bool {
        let mut excluded: Vec<usize> = ignore.to_vec();
        for _ in 0..8 {
            let Some(hit) = self.physics.projectile_ray(from, to, &excluded) else {
                return true;
            };
            let npc = self
                .world
                .entities
                .get(hit.entity)
                .is_some_and(|e| e.class().starts_with("npc_"));
            if !npc {
                return false;
            }
            excluded.push(hit.entity);
        }
        false
    }
}

/// CC_NPC_Create placement over the physics world (HULL_HUMAN for every class: the
/// per-class hulls are not modeled; UTIL_DropToFloor traces 256 units down).
pub struct PhysicsPlacement<'a> {
    pub physics: &'a Physics,
}
/// The humanoids' standing eye offset and m_HackedGunPos height (units above the feet).
pub const EYE_HEIGHT: f32 = 64.;
pub const GUN_HEIGHT: f32 = 55.;
const HUMAN_MINS: Vec3 = Vec3::new(-13., -13., 0.);
const HUMAN_MAXS: Vec3 = Vec3::new(13., 13., 72.);
impl super::spawn::PlacementWorld for PhysicsPlacement<'_> {
    fn trace_line(&self, start: Vec3, end: Vec3) -> Option<Vec3> {
        let delta = end - start;
        self.physics
            .impact_ray(start, delta.normalize_or_zero(), delta.length())
            .map(|hit| hit.position)
    }
    fn drop_to_floor(&self, _class: &str, origin: Vec3) -> Option<Vec3> {
        use modkit_core::movement::CollisionWorld;
        let trace =
            self.physics
                .trace_hull(origin, origin - Vec3::Z * 256., HUMAN_MINS, HUMAN_MAXS);
        (!trace.start_solid && trace.fraction < 1.)
            .then(|| origin - Vec3::Z * 256. * trace.fraction)
    }
    fn hull_fits(&self, _class: &str, origin: Vec3) -> bool {
        use modkit_core::movement::CollisionWorld;
        // UTIL_DropToFloor's trace ends DIST_EPSILON off the floor; ours ends on the
        // contact, so the fit test (AI_TraceHull origin -> origin + 1) starts slightly up
        // or the floor itself would count as start-solid.
        let start = origin + Vec3::Z * 0.25;
        let trace = self
            .physics
            .trace_hull(start, start + Vec3::Z, HUMAN_MINS, HUMAN_MAXS);
        !trace.start_solid && trace.fraction >= 1.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wall(min: Vec3, max: Vec3) -> modkit_core::Brush {
        modkit_core::Brush {
            contents: 1,
            planes: [
                (Vec3::X, max.x),
                (-Vec3::X, -min.x),
                (Vec3::Y, max.y),
                (-Vec3::Y, -min.y),
                (Vec3::Z, max.z),
                (-Vec3::Z, -min.z),
            ]
            .into_iter()
            .map(|(normal, distance)| modkit_core::Plane { normal, distance })
            .collect(),
        }
    }
    #[test]
    fn sight_is_blocked_by_world_brushes_but_not_by_npc_bodies() {
        use rapier3d::prelude::*;
        // Synthetic: a wall at x 100..110 and an NPC collider (entity 0) at x 50.
        let world = World {
            brushes: vec![wall(Vec3::new(100., -50., -50.), Vec3::new(110., 50., 50.))],
            entities: vec![modkit_core::Entity {
                properties: vec![("classname".into(), "npc_citizen".into())],
            }],
            ..Default::default()
        };
        let mut physics = Physics::new(&world);
        physics.colliders.insert(
            ColliderBuilder::cuboid(16. / 39.37, 16. / 39.37, 36. / 39.37)
                .translation(vector![50. / 39.37, 0., 0.])
                .user_data(1),
        );
        physics.refresh_queries();
        let sight = PhysicsSight {
            physics: &physics,
            world: &world,
        };
        assert!(
            sight.visible(Vec3::ZERO, Vec3::X * 90., &[]),
            "NPC body ignored"
        );
        assert!(
            !sight.visible(Vec3::ZERO, Vec3::X * 200., &[]),
            "wall blocks"
        );
    }
    #[test]
    fn placement_drops_to_the_floor_and_rejects_a_wall() {
        use crate::ai::spawn::{place, SpawnRequest};
        // Synthetic: a floor (top z 0) and a wall at x 200..220.
        let world = World {
            brushes: vec![
                wall(Vec3::new(-500., -500., -16.), Vec3::new(500., 500., 0.)),
                wall(Vec3::new(200., -500., 0.), Vec3::new(220., 500., 200.)),
            ],
            ..Default::default()
        };
        let physics = Physics::new(&world);
        let request = SpawnRequest {
            classname: "npc_metropolice".into(),
            name: None,
            equipment: String::new(),
            aimed: true,
            ..Default::default()
        };
        let placement = PhysicsPlacement { physics: &physics };
        let eye = Vec3::new(0., 0., 64.);
        let floor = place(&request, eye, Vec3::new(1., 0., -1.), &placement).unwrap();
        assert!(
            (floor.origin - Vec3::new(64., 0., 0.)).length() < 1.,
            "{floor:?}"
        );
        assert!(floor.yaw.abs() < 1e-3);
        // Aiming straight at the wall: the hull overlaps it ("Bad Position").
        assert!(place(&request, eye, Vec3::X, &placement).is_err());
    }
    #[test]
    fn yaw_clamps_at_the_motor_rate_and_wraps() {
        // MaxYawSpeed 45 -> 450 deg/s, 0.1 s think: 45 degrees per think.
        assert_eq!(clamp_yaw(450., 0., 90., 0.1), 45.);
        assert_eq!(clamp_yaw(450., 60., 90., 0.1), 90.);
        assert_eq!(clamp_yaw(450., 170., -170., 0.1), -170.);
        assert!((clamp_yaw(450., -170., 100., 0.1) - (-215.)).abs() < 1e-4);
    }
}
