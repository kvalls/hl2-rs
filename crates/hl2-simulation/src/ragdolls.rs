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
/// CRagdoll::CheckSettleStationaryRagdoll: RAGDOLL_SLEEP_TOLERANCE (units per axis per
/// update) and ragdoll_sleepaftertime (seconds).
const SLEEP_TOLERANCE: f32 = 1.;
const SLEEP_AFTER_TIME: f64 = 5.;

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
    /// Scene time of RagdollCreate (WasRagdollCreatedOnCurrentTick).
    born: f64,
    /// World anchors of crossbow pins (StickRagdollNow ball-sockets).
    pins: Vec<RigidBodyHandle>,
    /// Render alpha once retired (C_ClientRagdoll::SUB_Remove starts FadeOut).
    fade: Option<f32>,
    /// C_ClientRagdoll::HandleAnimatedFriction.
    friction: AnimatedFriction,
    /// CRagdoll::CheckSettleStationaryRagdoll.
    settle: Settle,
}

/// CRagdoll's m_vecLastOrigin and m_flLastOriginChangeTime.
#[derive(Clone, Copy, Debug)]
struct Settle {
    origin: Option<Vec3>,
    changed: f64,
}

impl Settle {
    /// RagdollCreate: "It's moving now..."
    fn new(now: f64) -> Self {
        Self {
            origin: None,
            changed: now,
        }
    }
    /// One CheckSettleStationaryRagdoll (VPhysicsUpdate, once per update while a solid is
    /// awake) with the root solid's position: true once the root has moved at most
    /// SLEEP_TOLERANCE on every axis per update for ragdoll_sleepaftertime.
    fn check(&mut self, origin: Vec3, now: f64) -> bool {
        let moved = self
            .origin
            .is_none_or(|last| (origin - last).abs().max_element() > SLEEP_TOLERANCE);
        self.origin = Some(origin);
        if moved {
            self.changed = now;
            return false;
        }
        now - self.changed >= SLEEP_AFTER_TIME
    }
}

/// m_iFrictionAnimState (RAGDOLL_FRICTION_NONE, _IN, _HOLD, _OUT, _OFF).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum FrictionState {
    #[default]
    None,
    In,
    Hold,
    Out,
    Off,
}

/// C_ClientRagdoll's animated joint friction: every client think while it runs sets
/// SetAngularMotor(0, m_iCurrentFriction) on all ragdoll constraints.
#[derive(Clone, Copy, Debug, Default)]
struct AnimatedFriction {
    state: FrictionState,
    /// m_flFrictionTime and m_flFrictionModTime.
    until: f64,
    span: f32,
    /// m_iCurrentFriction (an int, 0 until the first think).
    current: i32,
}

impl AnimatedFriction {
    /// One HandleAnimatedFriction: the friction to apply to the joints, None once off.
    /// `params` is the model's animatedfriction (min, max, time in, time out, hold).
    fn think(&mut self, params: Option<[f32; 5]>, now: f64) -> Option<i32> {
        if self.state == FrictionState::Off {
            return None;
        }
        let [min, max, time_in, time_out, hold] = params.unwrap_or_default();
        // iMin/iMaxAnimatedFriction are atoi'd.
        let (min, max) = (min.trunc(), max.trunc());
        let remap = |value: f32, a: f32, b: f32| -> i32 {
            // RemapValClamped, truncated into m_iCurrentFriction.
            if a == b {
                return if value >= b { max as i32 } else { min as i32 };
            }
            (min + (max - min) * ((value - a) / (b - a)).clamp(0., 1.)) as i32
        };
        match self.state {
            FrictionState::None => {
                if min != 0. || max != 0. {
                    self.state = FrictionState::In;
                    self.span = time_in;
                    self.until = now + f64::from(time_in);
                    self.current = min as i32;
                } else {
                    self.state = FrictionState::Off;
                }
            }
            FrictionState::In => {
                let left = (self.until - now) as f32;
                self.current = remap(left, self.span, 0.);
                if left <= 0. {
                    self.span = hold;
                    self.until = now + f64::from(hold);
                    self.state = FrictionState::Hold;
                }
            }
            FrictionState::Hold => {
                if self.until < now {
                    self.span = time_out;
                    self.until = now + f64::from(time_out);
                    self.state = FrictionState::Out;
                }
            }
            FrictionState::Out => {
                let left = (self.until - now) as f32;
                self.current = remap(left, 0., self.span);
                if left <= 0. {
                    self.state = FrictionState::Off;
                }
            }
            FrictionState::Off => {}
        }
        Some(self.current)
    }
}

#[derive(Default)]
pub struct Ragdolls {
    pub active: BTreeMap<usize, Ragdoll>,
    /// Map prop_ragdolls (CRagdollProp: server ragdolls, outside the client LRU,
    /// FX_AffectRagdolls, settling and fading), created once per map.
    pub props: BTreeMap<usize, Ragdoll>,
    /// prop_ragdoll entities not created yet (point_template children wait for ForceSpawn).
    props_waiting: Option<Vec<usize>>,
    /// prop_ragdolls without a rig, .phy ragdoll or matching solids: entity and reason.
    pub props_skipped: Vec<(usize, &'static str)>,
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
    /// Shots FX_AffectRagdolls refused because more than one ragdoll was simulating.
    pub shots_busy: u32,
    pub blast_pushes: u32,
    pub pins: u32,
    /// Ragdolls that started with a death pose's animation velocity.
    pub death_poses: u32,
    /// PhysForceRagdollToSleep calls (CheckSettleStationaryRagdoll).
    pub forced_sleeps: u32,
    /// The last death force's solid and the solid nearest its point (diagnostics).
    pub last_force_body: Option<[usize; 2]>,
    /// Crossbow BoltImpacts that found no ragdoll solid after their retry, and the last
    /// one's ray (diagnostics).
    pub bolt_misses: u32,
    pub last_missed_bolt: Option<(Vec3, Vec3)>,
    pub missed_bolt_bodies: Vec<Vec3>,
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
    /// matrices, the death force (scene.npc_death_forces) on the hit hitbox's physics bone.
    pub fn spawn(
        &mut self,
        world: &World,
        scene: &mut Scene,
        physics: &mut Physics,
        id: usize,
        dt: f32,
    ) {
        self.spawn_as(world, scene, physics, id, id, dt);
    }
    /// `spawn` for NPC `source` drawn by entity `id` (a model with the same skeleton: the
    /// headless zombie corpse): the pose, motion and death force come from `source`.
    pub fn spawn_as(
        &mut self,
        world: &World,
        scene: &mut Scene,
        physics: &mut Physics,
        source: usize,
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
        let source_rig = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(source))
            .and_then(|i| world.rigs.get(&i.asset_key()))
            .unwrap_or(rig);
        // actor_matrices are skinning matrices (pose x inverse bind): the bones' model
        // space poses are those times the bind pose.
        let skin = scene.actor_matrices(source_rig, source);
        if skin.len() != rig.bones.len() {
            self.skipped += 1;
            return;
        }
        let matrices: Vec<Mat4> = skin
            .iter()
            .zip(&rig.bones)
            .map(|(m, b)| *m * b.inverse_bind.inverse())
            .collect();
        let state = &scene.states[source];
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
            .get(&source)
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
        let death = scene.npc_death_forces.remove(&source);
        let hit_bone = scene.npc_force_bones.remove(&source);
        if let Some((force, position)) = death {
            // m_nForceBone = the trace's physicsbone: the solid that simulates the hit
            // hitbox's bone (its own, else its nearest ancestor's). Without a hitbox, the
            // solid nearest the force point, else the root.
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
            let parents: Vec<_> = rig.bones.iter().map(|b| b.parent).collect();
            let solids: Vec<_> = model.bodies.iter().map(|b| b.bone).collect();
            let force_body = hit_bone
                .and_then(|bone| physics_bone(&parents, &solids, bone))
                .unwrap_or(nearest);
            self.last_force_body = Some([force_body, nearest]);
            physics.ragdoll_nudge(&bodies, force_body, force);
        }
        // CRagdoll::Init: RagdollApplyAnimationAsVelocity from the bones 0.1 s before death
        // (taken as the current pose; the root's motion is the spawn velocity) to the death
        // pose (C_AI_BaseNPC::GetRagdollInitBoneArrays).
        let hitgroup = scene.npc_hit_groups.remove(&source).unwrap_or(0);
        let force = death.map_or(Vec3::ZERO, |(force, _)| force);
        let roll = crate::fire::roll(source, scene.time, 21);
        if let Some(pose) =
            death_pose_matrices(rig, scene.states[source].rotation, force, hitgroup, roll)
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
        if source != id {
            // The drawing entity takes the corpse's frame; it is done like a corpse.
            let (origin, rotation) = (scene.states[source].origin, scene.states[source].rotation);
            let state = &mut scene.states[id];
            state.origin = origin;
            state.rotation = rotation;
            state.killed = true;
            state.visible = false;
        }
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
                born: scene.time,
                pins: Vec::new(),
                fade: None,
                friction: AnimatedFriction::default(),
                settle: Settle::new(scene.time),
            },
        );
        self.spawned += 1;
    }
    /// CRagGib::Spawn -> BecomeRagdollOnClient(force) for a runtime raggib entity (the
    /// zombie's legs after BecomeTorso): a client ragdoll (LRU, fade) posed by the model's
    /// sequence 0 at cycle 0 at the entity's origin and angles, at rest, with the force on
    /// the root solid (m_nForceBone 0). False without a rig or .phy ragdoll.
    pub fn spawn_gib(
        &mut self,
        world: &World,
        scene: &mut Scene,
        physics: &mut Physics,
        id: usize,
        force: Vec3,
    ) -> bool {
        let Some(instance) = world.model_instances.iter().find(|i| i.entity == Some(id)) else {
            return false;
        };
        let key = instance.asset_key();
        let (Some(rig), Some(model)) = (world.rigs.get(&key), world.model_ragdolls.get(&key))
        else {
            self.skipped += 1;
            return false;
        };
        let sequence = rig
            .sequences
            .iter()
            .min_by_key(|s| s.order)
            .map_or("", |s| s.name.as_str());
        let skin = rig.matrices(sequence, 0.);
        if skin.len() != rig.bones.len() {
            self.skipped += 1;
            return false;
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
            .filter_map(|b| matrices.get(b.bone))
            .map(|m| {
                let (_, rotation, translation) = (entity * *m).to_scale_rotation_translation();
                (translation, rotation.normalize())
            })
            .collect();
        self.serial += 1;
        let bodies = physics.spawn_ragdoll(world, model, &poses, Vec3::ZERO, self.serial);
        if poses.len() != model.bodies.len() || bodies.len() != model.bodies.len() {
            physics.remove_ragdoll(&bodies);
            self.skipped += 1;
            return false;
        }
        physics.ragdoll_nudge(&bodies, 0, force);
        let parents: Vec<_> = rig.bones.iter().map(|b| b.parent).collect();
        let locals = parents
            .iter()
            .enumerate()
            .map(|(i, parent)| match parent {
                Some(p) => matrices[*p].inverse() * matrices[i],
                None => matrices[i],
            })
            .collect();
        // The server entity is done (like a corpse's): only the client ragdoll draws.
        let state = &mut scene.states[id];
        state.killed = true;
        state.visible = false;
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
                born: scene.time,
                pins: Vec::new(),
                fade: None,
                friction: AnimatedFriction::default(),
                settle: Settle::new(scene.time),
            },
        );
        self.spawned += 1;
        true
    }
    /// CRagdollProp::Spawn for the map's prop_ragdolls once their entities exist (at map
    /// spawn, or at a point_template's ForceSpawn): SetupBones at the entity's angles
    /// (sequence 0 when its clip is loaded, else the bind pose), RagdollCreate, angleOverride,
    /// RagdollActivate (asleep with SF_RAGDOLLPROP_STARTASLEEP), then SetAbsAngles(vec3_angle).
    pub fn spawn_props(&mut self, world: &World, scene: &mut Scene, physics: &mut Physics) {
        let waiting = self.props_waiting.get_or_insert_with(|| {
            world
                .model_instances
                .iter()
                .filter(|i| i.kind == "prop_ragdoll")
                .filter_map(|i| i.entity)
                .collect()
        });
        if waiting.is_empty() {
            return;
        }
        let ready: Vec<usize> = waiting
            .iter()
            .copied()
            .filter(|id| scene.states.get(*id).is_some_and(|s| !s.killed))
            .collect();
        waiting.retain(|id| !ready.contains(id));
        for id in ready {
            let Some(instance) = world.model_instances.iter().find(|i| i.entity == Some(id)) else {
                continue;
            };
            let key = instance.asset_key();
            let (rig, model) = match (world.rigs.get(&key), world.model_ragdolls.get(&key)) {
                (Some(rig), Some(model)) => (rig, model),
                (None, _) => {
                    self.props_skipped.push((id, "no rig"));
                    continue;
                }
                (_, None) => {
                    self.props_skipped.push((id, "no ragdoll"));
                    continue;
                }
            };
            let entity = world.entities.get(id);
            let flags = entity
                .and_then(|e| e.get("spawnflags"))
                .and_then(|f| f.parse::<u32>().ok())
                .unwrap_or(0);
            let sequence = rig
                .sequences
                .iter()
                .min_by_key(|s| s.order)
                .map_or("", |s| s.name.as_str());
            let placement = Mat4::from_scale_rotation_translation(
                Vec3::splat(instance.scale),
                crate::physics::angles(instance.angles),
                instance.origin,
            );
            let mut bones: Vec<Mat4> = rig
                .matrices(sequence, 0.)
                .iter()
                .zip(&rig.bones)
                .map(|(m, b)| placement * *m * b.inverse_bind.inverse())
                .collect();
            // Bones without a solid keep their sequence pose relative to their parents.
            let parents: Vec<_> = rig.bones.iter().map(|b| b.parent).collect();
            let unposed = bones.clone();
            if let Some(text) = entity.and_then(|e| e.get("angleOverride")) {
                angle_override(&mut bones, model, text, instance.origin);
            }
            let poses: Vec<(Vec3, Quat)> = model
                .bodies
                .iter()
                .filter_map(|b| bones.get(b.bone))
                .map(|m| {
                    let (_, rotation, translation) = m.to_scale_rotation_translation();
                    (translation, rotation.normalize())
                })
                .collect();
            self.serial += 1;
            let bodies = physics.spawn_ragdoll(world, model, &poses, Vec3::ZERO, self.serial);
            if poses.len() != model.bodies.len() || bodies.len() != model.bodies.len() {
                physics.remove_ragdoll(&bodies);
                self.props_skipped.push((id, "solids"));
                continue;
            }
            if flags & SF_RAGDOLLPROP_STARTASLEEP != 0 {
                physics.ragdoll_sleep(&bodies);
            }
            // SetAbsAngles(vec3_angle): the entity's model space is unrotated from now on;
            // its origin follows the root solid.
            let state = &mut scene.states[id];
            state.rotation = Quat::IDENTITY;
            state.origin = poses[0].0;
            let unrotated = Mat4::from_scale_rotation_translation(
                Vec3::splat(instance.scale),
                Quat::IDENTITY,
                poses[0].0,
            )
            .inverse();
            let locals = parents
                .iter()
                .enumerate()
                .map(|(i, parent)| match parent {
                    Some(p) => unposed[*p].inverse() * unposed[i],
                    None => unrotated * bones[i],
                })
                .collect();
            self.props.insert(
                id,
                Ragdoll {
                    model: model.clone(),
                    bodies,
                    parents,
                    locals,
                    inverse_binds: rig.bones.iter().map(|b| b.inverse_bind).collect(),
                    scale: instance.scale,
                    serial: self.serial,
                    born: scene.time,
                    pins: Vec::new(),
                    fade: None,
                    friction: AnimatedFriction::default(),
                    settle: Settle::new(scene.time),
                },
            );
        }
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
    /// Diagnostics: per ragdoll entity, its awake solids' activity (Physics::ragdoll_activity).
    pub fn activity(&self, physics: &Physics) -> BTreeMap<usize, Vec<(usize, f32, f32, f32)>> {
        self.active
            .iter()
            .map(|(&id, r)| (id, physics.ragdoll_activity(&r.bodies)))
            .collect()
    }
    /// CRagdollLRURetirement m_iSimulatedRagdollCount: ragdolls whose root solid is awake.
    pub fn simulating(&self, physics: &Physics) -> usize {
        self.active
            .values()
            .filter(|r| {
                physics
                    .ragdoll_body_center(r.bodies[0])
                    .is_some_and(|(_, awake)| awake)
            })
            .count()
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
    pub fn impacts(&mut self, scene: &mut Scene, physics: &mut Physics) {
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
                    if self.simulating(physics) > 1 {
                        self.shots_busy += 1;
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
                    let Some(ragdoll) = self.active.get_mut(&id) else {
                        continue;
                    };
                    if ragdoll.born >= scene.time {
                        continue;
                    }
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
                    ragdoll.settle.changed = scene.time;
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
                    for (&id, ragdoll) in &mut self.active {
                        if ragdoll.born >= scene.time {
                            continue;
                        }
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
                        // ImpactTrace: ResetRagdollSleepAfterTime.
                        ragdoll.settle.changed = scene.time;
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
                        if retried {
                            self.bolt_misses += 1;
                            self.last_missed_bolt = Some((origin, origin - direction * 128.));
                            self.missed_bolt_bodies = self
                                .active
                                .values()
                                .flat_map(|r| r.bodies.iter())
                                .filter_map(|h| physics.ragdoll_body_center(*h))
                                .map(|(center, _)| center)
                                .collect();
                        } else {
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
        // C_ClientRagdoll::ClientThink -> HandleAnimatedFriction.
        for ragdoll in self.active.values_mut() {
            if let Some(friction) = ragdoll
                .friction
                .think(ragdoll.model.animated_friction, scene.time)
            {
                physics.ragdoll_joint_friction(&ragdoll.bodies, friction as f32);
            }
        }
        // CRagdoll::VPhysicsUpdate -> CheckSettleStationaryRagdoll: a ragdoll that stays
        // (nearly) still is forced asleep, so FX_AffectRagdolls' simulating count drops.
        for ragdoll in self.active.values_mut() {
            let awake = ragdoll.bodies.iter().any(|h| {
                physics
                    .ragdoll_body_center(*h)
                    .is_some_and(|(_, awake)| awake)
            });
            let Some(root) = ragdoll.bodies.first().and_then(|h| physics.body_state(*h)) else {
                continue;
            };
            if awake && ragdoll.settle.check(root.position, scene.time) {
                physics.ragdoll_sleep(&ragdoll.bodies);
                self.forced_sleeps += 1;
            }
        }
        for (&id, ragdoll) in self.active.iter().chain(&self.props) {
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
/// SF_RAGDOLLPROP_STARTASLEEP (SDK physics_prop_ragdoll.cpp).
const SF_RAGDOLLPROP_STARTASLEEP: u32 = 0x10000;

/// CRagdollProp::InitRagdoll's angleOverride ("index,pitch yaw roll,index,..."): each
/// listed solid's bone takes those world angles at its parent solid's (possibly already
/// overridden) bone x originParentSpace (the offset in the unposed bones); a root solid
/// stays at the entity origin. Returns the overridden solids.
fn angle_override(
    bones: &mut [Mat4],
    model: &modkit_core::RagdollModel,
    text: &str,
    origin: Vec3,
) -> usize {
    let parent_of = |element: usize| {
        model
            .joints
            .iter()
            .find(|j| j.child == element)
            .map(|j| j.parent)
    };
    let solid_bone = |element: usize| model.bodies.get(element).map(|b| b.bone);
    // originParentSpace: each solid's origin in its parent solid's bone, before overrides.
    let offsets: Vec<Option<Vec3>> = (0..model.bodies.len())
        .map(|element| {
            let parent = solid_bone(parent_of(element)?)?;
            let bone = solid_bone(element)?;
            Some(
                bones[parent]
                    .inverse()
                    .transform_point3(bones[bone].w_axis.truncate()),
            )
        })
        .collect();
    let tokens: Vec<&str> = text.split(',').map(str::trim).collect();
    let mut overridden = 0;
    for pair in tokens.chunks(2) {
        let [index, angles] = pair else { break };
        let Some(element) = index
            .parse::<usize>()
            .ok()
            .filter(|e| *e < model.bodies.len())
        else {
            continue;
        };
        let values: Vec<f32> = angles
            .split_whitespace()
            .filter_map(|v| v.parse().ok())
            .collect();
        let (Some(bone), [pitch, yaw, roll]) = (solid_bone(element), values.as_slice()) else {
            continue;
        };
        let position = match (parent_of(element).and_then(solid_bone), offsets[element]) {
            (Some(parent), Some(offset)) => bones[parent].transform_point3(offset),
            _ => origin,
        };
        bones[bone] = Mat4::from_rotation_translation(
            crate::physics::angles(Vec3::new(*pitch, *yaw, *roll)),
            position,
        );
        overridden += 1;
    }
    overridden
}

/// A studio bone's physicsbone: the index of the ragdoll solid that simulates it (the
/// bone's own solid, else its nearest ancestor's).
fn physics_bone(parents: &[Option<usize>], solids: &[usize], bone: usize) -> Option<usize> {
    std::iter::successors(Some(bone), |b| parents.get(*b).copied().flatten())
        .find_map(|b| solids.iter().position(|&solid| solid == b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angle_override_poses_solids_in_world_angles_from_their_parents() {
        // Two solids: root on bone 0, child on bone 1, 10 units along the root's X. The
        // override turns the root 90 degrees of yaw and gives the child pitch 90: the child
        // follows the turned root (originParentSpace) and takes its own world angles.
        let body = |bone| modkit_core::RagdollBody {
            bone,
            pieces: Vec::new(),
            mass: 1.,
            inertia: 1.,
            damping: 0.,
            rotdamping: 0.,
            surfaceprop: "default".into(),
            compact_inertia: None,
        };
        let model = RagdollModel {
            bodies: vec![body(0), body(1)],
            joints: vec![modkit_core::RagdollJoint {
                parent: 0,
                child: 1,
                limits: [(-30., 30.); 3],
                friction: [0.; 3],
                child_in_parent: (Vec3::X * 10., Quat::IDENTITY),
            }],
            self_collisions: false,
            collision_pairs: Vec::new(),
            animated_friction: None,
        };
        let origin = Vec3::new(100., 0., 50.);
        let mut bones = vec![
            Mat4::from_translation(origin),
            Mat4::from_translation(origin + Vec3::X * 10.),
        ];
        assert_eq!(
            angle_override(&mut bones, &model, "0,0 90 0,1,90 90 0,7,0 0 0", origin),
            2
        );
        let close = |a: Vec3, b: Vec3| a.distance(b) < 1e-3;
        assert!(close(bones[0].w_axis.truncate(), origin));
        assert!(close(bones[0].transform_vector3(Vec3::X), Vec3::Y));
        assert!(close(bones[1].w_axis.truncate(), origin + Vec3::Y * 10.));
        // Pitch 90 points the child's X down (Source pitch is positive down).
        assert!(close(bones[1].transform_vector3(Vec3::X), Vec3::NEG_Z));
    }
    #[test]
    fn the_death_force_goes_to_the_hit_hitboxs_physics_bone() {
        // police.mdl-like chain: pelvis 0 -> spine 1 -> spine2 2 (solid 1) -> clavicle 3 ->
        // upper arm 4 (solid 2); pelvis is solid 0. A chest hitbox on spine2 pushes the
        // 17.7 kg spine solid, not the 2.6 kg arm whose origin can be nearer the chest
        // surface; an unsimulated bone (spine 1) maps to its nearest simulated ancestor.
        let parents = [None, Some(0), Some(1), Some(2), Some(3)];
        let solids = [0, 2, 4];
        assert_eq!(physics_bone(&parents, &solids, 2), Some(1));
        assert_eq!(physics_bone(&parents, &solids, 4), Some(2));
        assert_eq!(physics_bone(&parents, &solids, 3), Some(1));
        assert_eq!(physics_bone(&parents, &solids, 1), Some(0));
        assert_eq!(physics_bone(&parents, &solids, 9), None);
    }
    #[test]
    fn a_still_ragdoll_is_forced_asleep_after_ragdoll_sleepaftertime() {
        // Created at t 10, then creeping 0.1 units per 15 ms update: asleep at t 15.
        let mut settle = Settle::new(10.);
        let mut slept = None;
        for tick in 0..400 {
            let now = 10. + f64::from(tick) * 0.015;
            let origin = Vec3::new(0.1 * tick as f32, 0., 70.);
            if settle.check(origin, now) {
                slept = Some(now);
                break;
            }
        }
        assert!(slept.is_some_and(|t| (t - 15.).abs() < 0.016), "{slept:?}");
        // A step over 1 unit on one axis restarts the five seconds; so does an impact.
        let mut settle = Settle::new(0.);
        assert!(!settle.check(Vec3::ZERO, 0.));
        assert!(!settle.check(Vec3::new(0., 1.5, 0.), 4.));
        assert!(!settle.check(Vec3::new(0., 1.5, 0.), 8.9));
        assert!(settle.check(Vec3::new(0., 1.5, 0.), 9.));
        settle.changed = 9.;
        assert!(!settle.check(Vec3::new(0., 1.5, 0.), 13.9));
    }
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
    fn animated_friction_ramps_in_then_out_like_the_client_ragdoll() {
        // police.phy: animatedfriction min 1, max 1000, in 0.8 s, out 0.5 s, hold 0.
        let police = Some([1., 1000., 0.8, 0.5, 0.]);
        let mut friction = AnimatedFriction::default();
        let mut seen = Vec::new();
        for tick in 0..=100 {
            seen.push(friction.think(police, f64::from(tick) * 0.015));
        }
        // NONE -> IN at the first think (min), the ramp's middle, max at 0.8 s.
        assert_eq!(seen[0], Some(1));
        assert!((480..=520).contains(&seen[27].unwrap()), "{:?}", seen[27]);
        assert_eq!(seen[54], Some(1000));
        // HOLD (0 s), then OUT back down to min by 1.3 s, then off.
        assert!((400..=600).contains(&seen[71].unwrap()), "{:?}", seen[71]);
        assert!(
            seen[90].is_none() && seen[100].is_none(),
            "{:?}",
            &seen[84..92]
        );
        assert_eq!(seen.iter().rev().flatten().next(), Some(&1));
        // Without animatedfriction: one think applies m_iCurrentFriction 0, then off.
        let mut none = AnimatedFriction::default();
        assert_eq!(none.think(None, 0.), Some(0));
        assert_eq!(none.think(None, 0.015), None);
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
                        animated_friction: None,
                    }),
                    bodies: Vec::new(),
                    parents: Vec::new(),
                    locals: Vec::new(),
                    inverse_binds: Vec::new(),
                    scale: 1.,
                    serial: id as u64,
                    born: 0.,
                    pins: Vec::new(),
                    fade: None,
                    friction: AnimatedFriction::default(),
                    settle: Settle::new(0.),
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
