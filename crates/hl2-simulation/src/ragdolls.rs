//! Client-side NPC death ragdolls (C_BaseAnimating::BecomeRagdollOnClient ->
//! CreateClientRagdoll -> ragdoll_shared.cpp RagdollCreate): the model's .phy ragdoll as
//! jointed physics bodies placed at the death pose, nudged by the death force, then
//! driving the skinned mesh. Bones without a solid keep their death-pose transform
//! relative to their parent. DESIGN 14n session 30 (11) steps (b)-(c).
use crate::entities::Scene;
use crate::physics::Physics;
use glam::{Mat4, Quat, Vec3};
use modkit_core::{RagdollModel, World};
use rapier3d::prelude::RigidBodyHandle;
use std::collections::BTreeMap;
use std::sync::Arc;

/// g_ragdoll_maxcount (client): older ragdolls are removed beyond this count.
pub const MAX_RAGDOLLS: usize = 8;
/// C_ClientRagdoll::ImpactTrace impulse scales (retail client.dll FUN_100a5510: 4000 for
/// shots, 500 for blasts; the 2013 SDK source has 4000 for both).
const SHOT_IMPULSE: f32 = 4000.;
const BLAST_IMPULSE: f32 = 500.;
/// g_ragdoll_fadespeed: alpha per second while a retired ragdoll fades out.
const FADE_SPEED: f32 = 600.;

/// Client-side effects that strike existing ragdolls.
#[derive(Clone, Copy, Debug)]
pub enum Impact {
    /// A bullet or club trace from `start` to its impact (FX_AffectRagdolls, from the
    /// impact effect or RagdollImpact for a miss).
    Shot { start: Vec3, end: Vec3 },
    /// An explosion temp entity (C_TEExplosion::AffectRagdolls).
    Blast {
        origin: Vec3,
        magnitude: f32,
        radius: f32,
    },
    /// A crossbow bolt's BoltImpact at a wall point, flying along `direction`. The
    /// bolt's kill reaches the entity damage list a tick later than its impact, so a bolt
    /// that finds no ragdoll retries once (`retried`).
    Bolt {
        origin: Vec3,
        direction: Vec3,
        retried: bool,
    },
}

pub struct Ragdoll {
    model: Arc<RagdollModel>,
    bodies: Vec<RigidBodyHandle>,
    /// Every studio bone's parent and death-pose transform relative to its parent (model
    /// space for root bones).
    parents: Vec<Option<usize>>,
    locals: Vec<Mat4>,
    inverse_binds: Vec<Mat4>,
    scale: f32,
    serial: u64,
    /// World anchors of crossbow pins (StickRagdollNow ball-sockets).
    pins: Vec<RigidBodyHandle>,
    /// Render alpha once retired (C_ClientRagdoll::SUB_Remove starts FadeOut).
    fade: Option<f32>,
}

#[derive(Default)]
pub struct Ragdolls {
    pub active: BTreeMap<usize, Ragdoll>,
    serial: u64,
    pub spawned: u32,
    /// Deaths without a ragdoll (no .phy ragdoll, rig or instance).
    pub skipped: u32,
    pub removed: u32,
    /// Shots and blasts that pushed a ragdoll, and crossbow pins.
    pub shot_pushes: u32,
    /// Shots tested against ragdolls, and the last one that missed (diagnostics).
    pub shots_seen: u32,
    pub last_missed_shot: Option<(Vec3, Vec3)>,
    pub blast_pushes: u32,
    pub pins: u32,
    /// Ragdolls that started with a death pose's animation velocity.
    pub death_poses: u32,
    /// Every NPC's origin at the previous tick (its root velocity at death).
    last_origins: BTreeMap<usize, Vec3>,
}

impl Ragdolls {
    /// The root motion part of RagdollApplyAnimationAsVelocity: each NPC's origin change
    /// over the last tick, for the ragdolls of NPCs that die on the next one.
    pub fn track(&mut self, world: &World, scene: &Scene) {
        self.last_origins.clear();
        for (id, entity) in world.entities.iter().enumerate() {
            if entity.class().starts_with("npc_") {
                if let Some(state) = scene.states.get(id).filter(|s| !s.killed) {
                    self.last_origins.insert(id, state.origin);
                }
            }
        }
    }
    /// RagdollCreate for an NPC killed this tick: bodies at their bones' current world
    /// matrices, the death force (scene.npc_death_forces) on the solid nearest the hit.
    pub fn spawn(
        &mut self,
        world: &World,
        scene: &mut Scene,
        physics: &mut Physics,
        id: usize,
        dt: f32,
    ) {
        let Some(instance) = world.model_instances.iter().find(|i| i.entity == Some(id)) else {
            self.skipped += 1;
            return;
        };
        let key = instance.asset_key();
        let (Some(rig), Some(model)) = (world.rigs.get(&key), world.model_ragdolls.get(&key))
        else {
            self.skipped += 1;
            return;
        };
        // actor_matrices are skinning matrices (pose x inverse bind): the bones' model
        // space poses are those times the bind pose.
        let skin = scene.actor_matrices(rig, id);
        if skin.len() != rig.bones.len() {
            self.skipped += 1;
            return;
        }
        let matrices: Vec<Mat4> = skin
            .iter()
            .zip(&rig.bones)
            .map(|(m, b)| *m * b.inverse_bind.inverse())
            .collect();
        let state = &scene.states[id];
        let entity = Mat4::from_scale_rotation_translation(
            Vec3::splat(instance.scale),
            state.rotation,
            state.origin,
        );
        let poses: Vec<(Vec3, Quat)> = model
            .bodies
            .iter()
            .map(|b| {
                let (_, rotation, translation) =
                    (entity * matrices[b.bone]).to_scale_rotation_translation();
                (translation, rotation.normalize())
            })
            .collect();
        // Root velocity over the last tick (a teleport or respawn is not motion).
        let velocity = self
            .last_origins
            .get(&id)
            .map(|last| (state.origin - *last) / dt.max(1e-3))
            .filter(|v| v.length() < 1000.)
            .unwrap_or(Vec3::ZERO);
        self.serial += 1;
        let bodies = physics.spawn_ragdoll(world, model, &poses, velocity, self.serial);
        if bodies.len() != model.bodies.len() {
            physics.remove_ragdoll(&bodies);
            self.skipped += 1;
            return;
        }
        let death = scene.npc_death_forces.remove(&id);
        if let Some((force, position)) = death {
            // m_nForceBone: the solid nearest the force point (the hit hitbox's physics
            // bone), else the root.
            let nearest = position.map_or(0, |position| {
                poses
                    .iter()
                    .enumerate()
                    .min_by(|a, b| {
                        a.1 .0
                            .distance_squared(position)
                            .total_cmp(&b.1 .0.distance_squared(position))
                    })
                    .map_or(0, |(i, _)| i)
            });
            physics.ragdoll_nudge(&bodies, nearest, force);
        }
        // CRagdoll::Init: RagdollApplyAnimationAsVelocity from the bones 0.1 s before death
        // (taken as the current pose; the root's motion is the spawn velocity) to the death
        // pose (C_AI_BaseNPC::GetRagdollInitBoneArrays).
        let hitgroup = scene.npc_hit_groups.remove(&id).unwrap_or(0);
        let force = death.map_or(Vec3::ZERO, |(force, _)| force);
        let roll = crate::fire::roll(id, scene.time, 21);
        if let Some(pose) =
            death_pose_matrices(rig, scene.states[id].rotation, force, hitgroup, roll)
        {
            for (handle, body) in bodies.iter().zip(&model.bodies) {
                let (linear, angular) = bone_velocity(
                    entity * matrices[body.bone],
                    entity * pose[body.bone],
                    DEATH_POSE_DT,
                );
                physics.ragdoll_add_velocity(*handle, linear, angular);
            }
            self.death_poses += 1;
        }
        let parents: Vec<_> = rig.bones.iter().map(|b| b.parent).collect();
        let locals = parents
            .iter()
            .enumerate()
            .map(|(i, parent)| match parent {
                Some(p) => matrices[*p].inverse() * matrices[i],
                None => matrices[i],
            })
            .collect();
        scene.ragdoll_matrices.insert(id, skin);
        self.active.insert(
            id,
            Ragdoll {
                model: model.clone(),
                bodies,
                parents,
                locals,
                inverse_binds: rig.bones.iter().map(|b| b.inverse_bind).collect(),
                scale: instance.scale,
                serial: self.serial,
                pins: Vec::new(),
                fade: None,
            },
        );
        self.spawned += 1;
    }
    /// CRagdollLRURetirement::Update as retail runs it for HL2 (client.dll FUN_101aa3a0
    /// takes the SDK's non-episodic branch while hl2_episodic is 0): beyond
    /// g_ragdoll_maxcount, the oldest ragdoll outside the view (CullBox; the view-cluster
    /// PVS test is not modeled) starts fading, one per update; else the oldest ones fade
    /// until the count fits. Fading ones lose 600 alpha per second and are released at 0.
    pub fn retire(
        &mut self,
        scene: &mut Scene,
        physics: &mut Physics,
        eye: Vec3,
        forward: Vec3,
        dt: f32,
    ) {
        let mut lru: Vec<(u64, usize)> = self
            .active
            .iter()
            .filter(|(_, r)| r.fade.is_none())
            .map(|(&id, r)| (r.serial, id))
            .collect();
        lru.sort_unstable();
        if lru.len() > MAX_RAGDOLLS {
            // A wide frustum test on the root solid (the 4:3 fov 75 view is ~91 degrees
            // across at 16:9), padded by the body's reach.
            let outside = |id: usize| {
                let to = scene.states[id].origin - eye;
                let distance = to.length();
                distance > 48. && to.dot(forward) < distance * (55f32).to_radians().cos() - 48.
            };
            let start = match lru.iter().find(|(_, id)| outside(*id)) {
                Some(&(_, id)) => vec![id],
                None => lru[..lru.len() - MAX_RAGDOLLS]
                    .iter()
                    .map(|(_, id)| *id)
                    .collect(),
            };
            for id in start {
                if let Some(r) = self.active.get_mut(&id) {
                    r.fade = Some(255.);
                }
            }
        }
        let mut done = Vec::new();
        for (&id, r) in &mut self.active {
            if let Some(alpha) = &mut r.fade {
                *alpha = (*alpha - FADE_SPEED * dt).max(0.);
                if *alpha == 0. {
                    done.push(id);
                }
            }
        }
        for id in done {
            self.remove(scene, physics, id);
        }
    }
    /// A fading ragdoll's render alpha (0-1), for the host's blend.
    pub fn fade_alpha(&self, id: usize) -> Option<f32> {
        self.active.get(&id)?.fade.map(|a| a / 255.)
    }
    pub fn fading(&self) -> usize {
        self.active.values().filter(|r| r.fade.is_some()).count()
    }
    pub fn remove(&mut self, scene: &mut Scene, physics: &mut Physics, id: usize) {
        if let Some(r) = self.active.remove(&id) {
            physics.remove_ragdoll(&r.bodies);
            for anchor in r.pins {
                physics.remove_body(anchor);
            }
            scene.ragdoll_matrices.remove(&id);
            self.removed += 1;
        }
    }
    /// This tick's client ragdoll effects (`scene.ragdoll_impacts`), after this tick's
    /// deaths became ragdolls: shots and blasts skip ragdolls created on this tick (the
    /// death force already pushed them), bolts pin them too.
    /// This tick's client ragdoll effects. The killing shot's or blast's ragdoll exists
    /// already: the client builds it in the entity update (BecomeRagdollOnClient) before
    /// the same snapshot's temp entities (Impact, explosion) fire, so they push it.
    pub fn impacts(&mut self, scene: &mut Scene, physics: &mut Physics) {
        physics.refresh_entity_queries();
        let impacts = std::mem::take(&mut scene.ragdoll_impacts);
        let mut retry = Vec::new();
        let owner = |active: &BTreeMap<usize, Ragdoll>, body| {
            active
                .iter()
                .find_map(|(&id, r)| r.bodies.contains(&body).then_some(id))
        };
        for impact in impacts {
            match impact {
                Impact::Shot { start, end } => {
                    // FX_AffectRagdolls: not while more than one ragdoll simulates.
                    let simulating = self
                        .active
                        .values()
                        .filter(|r| {
                            physics
                                .ragdoll_body_center(r.bodies[0])
                                .is_some_and(|(_, awake)| awake)
                        })
                        .count();
                    if simulating > 1 {
                        continue;
                    }
                    self.shots_seen += 1;
                    let Some((body, _, fraction)) = physics.ragdoll_ray(start, end) else {
                        self.last_missed_shot = Some((start, end));
                        continue;
                    };
                    let Some(id) = owner(&self.active, body) else {
                        continue;
                    };
                    let ragdoll = &self.active[&id];
                    // C_ClientRagdoll::ImpactTrace on VPhysicsGetObject (the root solid):
                    // 4000 along the shot at start + fraction x (endpos - start), the
                    // trace fraction applied twice as in the SDK and retail.
                    let dir = (end - start) * fraction;
                    let at = start + dir * fraction;
                    physics.ragdoll_impulse_at(
                        ragdoll.bodies[0],
                        dir.normalize_or_zero() * SHOT_IMPULSE,
                        at,
                    );
                    self.shot_pushes += 1;
                }
                Impact::Blast {
                    origin,
                    magnitude,
                    radius,
                } => {
                    if radius <= 0. || magnitude <= 0. {
                        continue;
                    }
                    for (&id, ragdoll) in &self.active {
                        let position = scene.states[id].origin;
                        let delta = position - origin;
                        let distance = delta.length().max(1e-3);
                        let force = magnitude - magnitude / radius * distance;
                        if force <= 1. {
                            continue;
                        }
                        // UTIL_TraceLine(MASK_SHOT_HULL): anything in between blocks.
                        if physics.ray(origin, delta / distance, distance).is_some() {
                            continue;
                        }
                        // tr.startpos = endpos - dir x force, 32 units lower; ImpactTrace
                        // DMG_BLAST: (endpos - startpos) x 500 at the root's center.
                        let push = delta / distance * force + Vec3::Z * 32.;
                        let Some((center, _)) = physics.ragdoll_body_center(ragdoll.bodies[0])
                        else {
                            continue;
                        };
                        physics.ragdoll_impulse_at(ragdoll.bodies[0], push * BLAST_IMPULSE, center);
                        self.blast_pushes += 1;
                    }
                }
                Impact::Bolt {
                    origin,
                    direction,
                    retried,
                } => {
                    // StickRagdollNow: the first ragdoll solid on the 128 units in front
                    // of the wall point (CRagdollBoltEnumerator), pinned at the wall.
                    let Some((body, point, _)) =
                        physics.ragdoll_ray(origin, origin - direction * 128.)
                    else {
                        if !retried {
                            retry.push(Impact::Bolt {
                                origin,
                                direction,
                                retried: true,
                            });
                        }
                        continue;
                    };
                    let Some(id) = owner(&self.active, body) else {
                        continue;
                    };
                    if let Some(anchor) = physics.pin_ragdoll(body, origin, point) {
                        if let Some(ragdoll) = self.active.get_mut(&id) {
                            ragdoll.pins.push(anchor);
                        }
                        self.pins += 1;
                        scene.sounds.push(crate::sounds::SoundRequest {
                            origin: Some(scene.states[id].origin),
                            .."Weapon_Crossbow.BoltSkewer".into()
                        });
                    }
                }
            }
        }
        scene.ragdoll_impacts.extend(retry);
    }
    /// After the physics step: each ragdoll's bone matrices in its entity's model space,
    /// with the entity origin following the root solid (render placement and bounds).
    /// A ragdoll whose entity came back (respawn, Kill removal) is dropped.
    pub fn update(&mut self, scene: &mut Scene, physics: &mut Physics) {
        let gone: Vec<usize> = self
            .active
            .keys()
            .copied()
            .filter(|id| scene.states.get(*id).is_none_or(|s| !s.killed))
            .collect();
        for id in gone {
            self.remove(scene, physics, id);
        }
        for (&id, ragdoll) in &self.active {
            let Some(poses) = physics.ragdoll_poses(&ragdoll.bodies) else {
                continue;
            };
            let scale = Vec3::splat(ragdoll.scale);
            let mut simulated: Vec<Option<Mat4>> = vec![None; ragdoll.parents.len()];
            for (body, (position, rotation)) in ragdoll.model.bodies.iter().zip(&poses) {
                if let Some(slot) = simulated.get_mut(body.bone) {
                    *slot = Some(Mat4::from_scale_rotation_translation(
                        scale, *rotation, *position,
                    ));
                }
            }
            let state = &mut scene.states[id];
            state.origin = poses[0].0;
            let entity = Mat4::from_scale_rotation_translation(scale, state.rotation, state.origin);
            // Studio bones list parents first.
            let mut world_bones: Vec<Mat4> = Vec::with_capacity(ragdoll.parents.len());
            for (i, parent) in ragdoll.parents.iter().enumerate() {
                let bone = match (simulated[i], parent) {
                    (Some(m), _) => m,
                    (None, Some(p)) if *p < i => world_bones[*p] * ragdoll.locals[i],
                    _ => entity * ragdoll.locals[i],
                };
                world_bones.push(bone);
            }
            let to_model = entity.inverse();
            scene.ragdoll_matrices.insert(
                id,
                world_bones
                    .iter()
                    .zip(&ragdoll.inverse_binds)
                    .map(|(m, inverse_bind)| to_model * *m * *inverse_bind)
                    .collect(),
            );
        }
        scene.ragdoll_frame = scene.ragdoll_frame.wrapping_add(1);
    }
}
/// C_BaseAnimating::BecomeRagdollOnClient's boneDt.
const DEATH_POSE_DT: f32 = 0.1;
/// MAX_DEATHPOSE_FRAMES: the death pose cycle is frame / 6.
const DEATH_POSE_FRAMES: f32 = 6.;
/// SelectDeathPoseActivityAndFrame (shared/death_pose.cpp): GetDeathPoseActivity compares
/// the side the force came from (-force) with the NPC's forward and right, the larger
/// absolute dot product deciding; the frame follows the last hitgroup (generic and the
/// rest keep the activity with the head frame).
pub fn death_pose(rotation: Quat, force: Vec3, hitgroup: i32) -> (&'static str, u32) {
    use crate::skill::*;
    let from = (-force).normalize_or_zero();
    let forward = (rotation * Vec3::X).dot(from);
    let right = (rotation * -Vec3::Y).dot(from);
    let activity = if right.abs() > forward.abs() {
        if right < 0. {
            "ACT_DIE_LEFTSIDE"
        } else {
            "ACT_DIE_RIGHTSIDE"
        }
    } else if forward < 0. {
        "ACT_DIE_BACKSIDE"
    } else {
        "ACT_DIE_FRONTSIDE"
    };
    let frame = match hitgroup {
        HITGROUP_CHEST | HITGROUP_STOMACH => 2,
        HITGROUP_LEFTARM => 3,
        HITGROUP_RIGHTARM => 4,
        HITGROUP_LEFTLEG => 5,
        HITGROUP_RIGHTLEG => 6,
        _ => 1,
    };
    (activity, frame)
}
/// The death pose's model-space bone matrices: SelectWeightedSequence for the activity
/// (`roll` in [0, 1)), sampled at cycle frame / 6. None when the model has no such
/// sequence (ACT_INVALID: no animation velocity).
fn death_pose_matrices(
    rig: &modkit_core::animation::Rig,
    rotation: Quat,
    force: Vec3,
    hitgroup: i32,
    roll: f32,
) -> Option<Vec<Mat4>> {
    let (activity, frame) = death_pose(rotation, force, hitgroup);
    let choices: Vec<_> = rig
        .sequences
        .iter()
        .filter(|s| s.activity.eq_ignore_ascii_case(activity) && s.weight > 0)
        .filter(|s| rig.clips.contains_key(&s.name.to_lowercase()))
        .collect();
    let total: i32 = choices.iter().map(|s| s.weight).sum();
    let mut pick = (roll * total as f32) as i32;
    let sequence = choices.iter().find(|s| {
        pick -= s.weight;
        pick < 0
    })?;
    let clip = rig.clips.get(&sequence.name.to_lowercase())?;
    let poses = clip.sample(frame as f32 / DEATH_POSE_FRAMES)?;
    Some(
        rig.local_matrices(&poses)
            .iter()
            .zip(&rig.bones)
            .map(|(m, b)| *m * b.inverse_bind.inverse())
            .collect(),
    )
}
/// CalcBoneDerivatives: a bone's linear (units/s) and world angular (rad/s) velocity
/// from one world pose to the next over `dt`.
fn bone_velocity(previous: Mat4, current: Mat4, dt: f32) -> (Vec3, Vec3) {
    let (_, from, start) = previous.to_scale_rotation_translation();
    let (_, to, end) = current.to_scale_rotation_translation();
    let (axis, angle) = (to * from.inverse()).normalize().to_axis_angle();
    let angle = if angle > std::f32::consts::PI {
        angle - std::f32::consts::TAU
    } else {
        angle
    };
    ((end - start) / dt, axis * angle / dt)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn death_poses_follow_the_hit_side_and_hitgroup() {
        use crate::skill::*;
        // An NPC facing +X (yaw 0); its right is -Y.
        let yaw0 = Quat::IDENTITY;
        // Shot from the front (force along -X): front side, chest frame.
        assert_eq!(
            death_pose(yaw0, -Vec3::X * 9000., HITGROUP_CHEST),
            ("ACT_DIE_FRONTSIDE", 2)
        );
        assert_eq!(
            death_pose(yaw0, Vec3::X * 9000., HITGROUP_HEAD),
            ("ACT_DIE_BACKSIDE", 1)
        );
        // From its right (force along +Y): right side; legs and arms pick their frames.
        assert_eq!(
            death_pose(yaw0, Vec3::Y, HITGROUP_LEFTLEG),
            ("ACT_DIE_RIGHTSIDE", 5)
        );
        assert_eq!(
            death_pose(yaw0, -Vec3::Y, HITGROUP_RIGHTARM),
            ("ACT_DIE_LEFTSIDE", 4)
        );
        // No force (SetHealth) and a generic hit: front side, head frame.
        assert_eq!(
            death_pose(yaw0, Vec3::ZERO, HITGROUP_GENERIC),
            ("ACT_DIE_FRONTSIDE", 1)
        );
        // Facing +Y (yaw 90), shot from the front (force along -Y).
        let yaw90 = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        assert_eq!(
            death_pose(yaw90, -Vec3::Y, HITGROUP_STOMACH).0,
            "ACT_DIE_FRONTSIDE"
        );
        // CalcBoneDerivatives: 10 units and 90 degrees about Z in 0.1 s.
        let (linear, angular) = bone_velocity(
            Mat4::IDENTITY,
            Mat4::from_rotation_translation(yaw90, Vec3::X * 10.),
            0.1,
        );
        assert!((linear - Vec3::X * 100.).length() < 1e-3, "{linear}");
        assert!((angular - Vec3::Z * std::f32::consts::FRAC_PI_2 * 10.).length() < 1e-3);
    }

    #[test]
    fn the_lru_fades_out_of_view_ragdolls_first_then_the_oldest() {
        // Ten corpses north of a player looking north; the second oldest lies behind him.
        let world = World {
            entities: (0..10)
                .map(|i| modkit_core::Entity {
                    properties: vec![
                        ("classname".into(), "npc_metropolice".into()),
                        ("origin".into(), format!("0 {} 0", 100 + 20 * i)),
                    ],
                })
                .collect(),
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        scene.states[1].origin = Vec3::new(0., -300., 0.);
        let mut physics = Physics::new(&world);
        let mut ragdolls = Ragdolls::default();
        for id in 0..10 {
            ragdolls.active.insert(
                id,
                Ragdoll {
                    model: Arc::new(RagdollModel {
                        bodies: Vec::new(),
                        joints: Vec::new(),
                        self_collisions: false,
                        collision_pairs: Vec::new(),
                    }),
                    bodies: Vec::new(),
                    parents: Vec::new(),
                    locals: Vec::new(),
                    inverse_binds: Vec::new(),
                    scale: 1.,
                    serial: id as u64,
                    pins: Vec::new(),
                    fade: None,
                },
            );
        }
        let (eye, forward) = (Vec3::new(0., 0., 64.), Vec3::Y);
        // One per update while one is out of view: the one behind the player first.
        ragdolls.retire(&mut scene, &mut physics, eye, forward, 0.015);
        assert_eq!(ragdolls.fading(), 1);
        assert!(ragdolls.fade_alpha(1).is_some());
        // Then, all in view, the oldest beyond eight (corpse 0).
        ragdolls.retire(&mut scene, &mut physics, eye, forward, 0.015);
        assert_eq!(ragdolls.fading(), 2);
        assert!(ragdolls.fade_alpha(0).is_some());
        // 600 alpha per second: released after ~0.43 s.
        for _ in 0..30 {
            ragdolls.retire(&mut scene, &mut physics, eye, forward, 0.015);
        }
        assert_eq!(ragdolls.active.len(), 8);
        assert_eq!(ragdolls.fading(), 0);
        assert_eq!(ragdolls.removed, 2);
    }
}
