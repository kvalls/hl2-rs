//! Owned iris projection with authored scene interests and a visible-interest fallback.
use crate::{
    FlyCamera,
    assets::LoadedMap,
    bevy_to_source,
    rendering::{EyeMesh, SourceEntity},
};
use bevy::{mesh::VertexAttributeValues, prelude::*};
use std::collections::BTreeMap;
#[derive(Clone)]
struct Actor {
    key: String,
    gaze: source_assets::eyes::Gaze,
    scale: f32,
}
#[derive(Resource, Default)]
pub struct Eyes {
    actors: BTreeMap<usize, Actor>,
    pose_samples: usize,
    attachment_actors: usize,
    origins: Vec<serde_json::Value>,
    draws: usize,
    tracking_player: usize,
    tracking_npc: usize,
    tracking_entity: usize,
    tracking_scene: usize,
    projections: Vec<serde_json::Value>,
    /// SDK CBaseFlex m_viewtarget: the last eye target persists per actor while valid.
    view_targets: BTreeMap<usize, glam::Vec3>,
    /// Bone-local eye forward/up per (entity, eyeball surface) for FACS eyelids; flexes
    /// consume the previous presentation's basis.
    pub lid_bases: BTreeMap<(usize, usize), (glam::Vec3, glam::Vec3)>,
}
impl Eyes {
    pub fn new(loaded: &LoadedMap) -> Self {
        let mut result = Self::default();
        for i in &loaded.world.model_instances {
            let Some(id) = i.entity else { continue };
            if !loaded.world.entities[id].class().starts_with("npc_") {
                continue;
            }
            if let Some(gaze) = loaded.gaze.get(&i.asset_key()) {
                result.actors.insert(
                    id,
                    Actor {
                        key: i.asset_key(),
                        gaze: gaze.clone(),
                        scale: i.scale,
                    },
                );
            }
        }
        result
    }
    /// A runtime spawn of a template's model (DESIGN 14r part A) tracks like it.
    pub fn copy_actor(&mut self, template: usize, entity: usize) {
        if let Some(actor) = self.actors.get(&template).cloned() {
            self.actors.insert(entity, actor);
        }
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"draws":self.draws,"pose_samples":self.pose_samples,"attachment_actors":self.attachment_actors,"origins":self.origins,"tracking_player":self.tracking_player,"tracking_npc":self.tracking_npc,"tracking_entity":self.tracking_entity,"tracking_scene":self.tracking_scene,"projections":self.projections,
            "policy":"newest valid authored LOOKAT interest, shared per actor; nearest-visible fallback outside scripted interests; animated owned eyes attachments supply actor origin/forward; native random/head/facial/PVS latch behavior remains unfinished",
            "shader":"owned Eyes iris alpha over sclera with retail planar basis; glints and Source model lighting remain unfinished; FACS eyelids follow the previous frame's eye basis"})
    }
}
fn bone_pose(
    game: &crate::gameplay::Gameplay,
    key: &str,
    id: usize,
    bone: usize,
    poses: &BTreeMap<usize, Vec<glam::Mat4>>,
) -> Option<glam::Mat4> {
    let rig = game.world.rigs.get(key)?;
    Some(*poses.get(&id)?.get(bone)? * rig.bones.get(bone)?.inverse_bind.inverse())
}
type EyeDrawItem<'a> = (
    &'a SourceEntity,
    &'a EyeMesh,
    &'a Mesh3d,
    Option<&'a bevy::mesh::skinning::SkinnedMesh>,
);

fn collect_actor_poses<'a>(
    eyes: &'a Eyes,
    draws: impl IntoIterator<Item = EyeDrawItem<'a>>,
    game: &crate::gameplay::Gameplay,
) -> BTreeMap<usize, Vec<glam::Mat4>> {
    let mut pose_keys: BTreeMap<usize, &str> = eyes
        .actors
        .iter()
        .map(|(&id, a)| (id, a.key.as_str()))
        .collect();
    for (owner, eye, _, _) in draws {
        pose_keys.entry(owner.0).or_insert(&eye.key);
    }
    pose_keys
        .into_iter()
        .filter_map(|(id, key)| {
            let state = game.scene.states.get(id)?;
            if (state.killed || !state.visible) && !game.scene.ragdoll_matrices.contains_key(&id) {
                return None;
            }
            let rig = game.world.rigs.get(key)?;
            Some((id, game.scene.actor_matrices(rig, id)))
        })
        .collect()
}

pub fn present(
    mut eyes: ResMut<Eyes>,
    game: Res<crate::gameplay::Gameplay>,
    sim: Res<crate::simulation::Simulation>,
    camera: Query<&Transform, With<FlyCamera>>,
    draws: Query<(
        &SourceEntity,
        &EyeMesh,
        &Mesh3d,
        Option<&bevy::mesh::skinning::SkinnedMesh>,
    )>,
    mut meshes: ResMut<Assets<Mesh>>,
    performance: Option<Res<crate::performance::Performance>>,
) {
    let _timing = crate::performance::scope(performance.as_deref(), "eyes");
    let Ok(camera) = camera.single() else { return };
    // Evaluate each visible actor once per presentation; target and iris consumers
    // share these skin transforms rather than sampling the full rig for every eye.
    let poses = collect_actor_poses(&eyes, draws, &game);
    let mut candidates = vec![(
        None,
        glam::Vec3::from_array(bevy_to_source(camera.translation).to_array()),
        glam::Vec3::ZERO,
    )];
    let mut origins = Vec::new();
    let mut attachment_actors = 0;
    for (&id, actor) in &eyes.actors {
        let Some(state) = game.scene.states.get(id) else {
            continue;
        };
        if (!state.visible || state.killed) && !game.scene.ragdoll_matrices.contains_key(&id) {
            continue;
        }
        let attached = actor.gaze.attachment.as_ref().and_then(|attachment| {
            bone_pose(&game, &actor.key, id, attachment.bone, &poses)
                .map(|bone| (bone * attachment.local, attachment.world_align))
        });
        let (origin, forward) = if let Some((attachment, world_align)) = attached {
            attachment_actors += 1;
            (
                state.origin
                    + state.rotation
                        * (attachment.transform_point3(glam::Vec3::ZERO) * actor.scale),
                if world_align {
                    glam::Vec3::X
                } else {
                    state.rotation * attachment.transform_vector3(glam::Vec3::X).normalize()
                },
            )
        } else {
            // Base NPC eye offset is already in world axes; PVS-dependent latching
            // and derived retail eye-position overrides are not modeled here.
            (
                state.origin + actor.gaze.view_offset * actor.scale,
                state.rotation * glam::Vec3::X,
            )
        };
        candidates.push((Some(id), origin, forward));
        origins.push(serde_json::json!({"entity":id,"origin":origin.to_array(),"forward":forward.to_array(),"attachment":attached.is_some()}));
    }
    eyes.pose_samples = poses.len();
    eyes.attachment_actors = attachment_actors;
    eyes.origins = origins;
    eyes.projections.clear();
    eyes.lid_bases.clear();
    eyes.draws = 0;
    eyes.tracking_player = 0;
    eyes.tracking_npc = 0;
    eyes.tracking_entity = 0;
    eyes.tracking_scene = 0;
    #[derive(Clone)]
    struct Selected {
        point: glam::Vec3,
        label: String,
        interest: Option<hl2_simulation::attention::Interest>,
    }
    let mut actor_targets = BTreeMap::<usize, Option<Selected>>::new();
    for (owner, mesh_eye, handle, skinned) in &draws {
        let Some(state) = game.scene.states.get(owner.0) else {
            continue;
        };
        if (!state.visible || state.killed) && !game.scene.ragdoll_matrices.contains_key(&owner.0) {
            continue;
        }
        let Some(bone) = bone_pose(&game, &mesh_eye.key, owner.0, mesh_eye.eye.bone, &poses) else {
            continue;
        };
        let origin = bone.transform_point3(mesh_eye.eye.origin);
        let head_forward = -bone.transform_vector3(mesh_eye.eye.forward).normalize();
        let world_origin = state.origin + state.rotation * (origin * mesh_eye.scale);
        // SetViewtarget is per actor; both eyes consume the same world target. Outside the
        // player's PVS an animated-face actor keeps its last view target.
        let frozen = !game.scene.look_targets.in_pvs(owner.0)
            && game
                .scene
                .entity(&game.world, owner.0)
                .is_some_and(|e| hl2_simulation::attention::has_animated_face(e.class()));
        let latched = frozen
            .then(|| eyes.view_targets.get(&owner.0).copied())
            .flatten();
        let selected = actor_targets.entry(owner.0).or_insert_with(|| {
            if let Some(point) = latched {
                return Some(Selected {
                    point,
                    label: "view".into(),
                    interest: None,
                });
            }
            use hl2_simulation::attention::Target;
            let actor_eye = candidates.iter().find(|(id, _, _)| *id == Some(owner.0));
            let (actor_eye, forward) = actor_eye.map_or(
                (world_origin, state.rotation * head_forward),
                |(_, p, f)| (*p, *f),
            );
            let position = |target| match target {
                Target::Player => Some(candidates[0].1),
                Target::Position(p) => Some(p),
                Target::Entity(id) => game.scene.states.get(id).filter(|s| !s.killed).map(|s| {
                    candidates
                        .iter()
                        .find(|(candidate, _, _)| *candidate == Some(id))
                        .map_or(s.origin, |(_, p, _)| *p)
                }),
            };
            // Explicit (scene/AI) and random (MaintainLookTargets glance) interests.
            if let Some((interest, point)) = game
                .scene
                .look_targets
                .eye_target(owner.0, actor_eye, forward, position)
            {
                let label = match interest.target {
                    Target::Player => "player".into(),
                    Target::Entity(id) => format!("#{id}"),
                    Target::Position(_) => "view".into(),
                };
                return Some(Selected {
                    point,
                    label,
                    // Random glances carry a ramp; only explicit interests count as scene.
                    interest: (interest.ramp == 0.).then(|| interest.clone()),
                });
            }
            // Actors with an animated face rely on their look queues (no target: the
            // latched/random view point below); others (cyclers) keep the nearest-visible
            // fallback.
            if game
                .scene
                .entity(&game.world, owner.0)
                .is_some_and(|e| hl2_simulation::attention::has_animated_face(e.class()))
            {
                return None;
            }
            candidates
                .iter()
                .filter_map(|&(id, p, _)| {
                    if id == Some(owner.0) {
                        return None;
                    }
                    let distance = p.distance(actor_eye);
                    if !(1. ..=1024.).contains(&distance)
                        || (p - actor_eye).normalize().dot(forward) <= 0.259
                    {
                        return None;
                    }
                    let excluded = [owner.0, id.unwrap_or(usize::MAX)];
                    if sim
                        .physics
                        .projectile_ray(actor_eye, p, &excluded)
                        .is_some()
                    {
                        return None;
                    }
                    Some((id, p, distance))
                })
                .min_by(|a, b| a.2.total_cmp(&b.2))
                .map(|(id, point, _)| Selected {
                    point,
                    label: id.map_or("player".into(), |i| format!("#{i}")),
                    interest: None,
                })
        });
        if selected.is_none() {
            // CAI_BaseActor::MaintainLookTargets: without a target, keep the latched view
            // target while ValidEyeTarget holds, else look 128 units ahead with jitter.
            let (eye, forward) = candidates
                .iter()
                .find(|(id, _, _)| *id == Some(owner.0))
                .map_or(
                    (world_origin, state.rotation * head_forward),
                    |(_, p, f)| (*p, *f),
                );
            let point = eyes
                .view_targets
                .get(&owner.0)
                .copied()
                .filter(|&p| valid_eye_target(eye, forward, p))
                .unwrap_or_else(|| random_view_target(eye, forward, owner.0, game.scene.time));
            *selected = Some(Selected {
                point,
                label: "view".into(),
                interest: None,
            });
        }
        if let Some(selected) = selected.as_ref() {
            eyes.view_targets.insert(owner.0, selected.point);
        }
        let local_target = selected
            .as_ref()
            .map(|s| state.rotation.inverse() * (s.point - state.origin) / mesh_eye.scale);
        let projection = source_assets::eyes::projection(&mesh_eye.eye, bone, local_target);
        eyes.lid_bases.insert(
            (owner.0, mesh_eye.eye.surface),
            source_assets::eyes::local_basis(
                bone,
                source_assets::eyes::basis(&mesh_eye.eye, bone, local_target),
            ),
        );
        if let Some(selected) = selected {
            if selected.interest.is_some() {
                eyes.tracking_scene += 1;
            }
            if selected.label == "player" {
                eyes.tracking_player += 1;
            } else if selected.label != "view" {
                eyes.tracking_entity += 1;
                let id = selected
                    .label
                    .strip_prefix('#')
                    .and_then(|s| s.parse::<usize>().ok());
                if id.is_some_and(|id| {
                    game.scene
                        .entity(&game.world, id)
                        .is_some_and(|e| e.class().starts_with("npc_"))
                }) {
                    eyes.tracking_npc += 1;
                }
            }
        }
        eyes.projections.push(serde_json::json!({"entity":owner.0,"surface":mesh_eye.eye.surface,"rows":projection.iter().map(|r|r.to_array()).collect::<Vec<_>>(),
            "target":selected.as_ref().map(|s|&s.label),"world_target":selected.as_ref().map(|s|s.point.to_array()),"scripted_interest":selected.as_ref().and_then(|s|s.interest.as_ref())}));
        if let Some(mesh) = meshes.get(&handle.0) {
            let Some(VertexAttributeValues::Float32x3(positions)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                continue;
            };
            let uvs = positions
                .iter()
                .enumerate()
                .map(|(vertex, p)| {
                    let point =
                        glam::Vec3::from_array(bevy_to_source(Vec3::from_array(*p)).to_array())
                            / mesh_eye.scale;
                    let point = if skinned.is_some() {
                        crate::gpu_skinning::skin_vertex(mesh, vertex, point, &poses[&owner.0])
                    } else {
                        point
                    };
                    [
                        projection[0].dot(point.extend(1.)),
                        projection[1].dot(point.extend(1.)),
                    ]
                })
                .collect::<Vec<_>>();
            if !matches!(mesh.attribute(Mesh::ATTRIBUTE_UV_1),Some(VertexAttributeValues::Float32x2(old)) if *old == uvs)
                && let Some(mut mesh) = meshes.get_mut(&handle.0)
            {
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uvs);
            }
            eyes.draws += 1;
        }
    }
}

/// SDK CAI_BaseActor::ValidEyeTarget: at least 1 unit away and within 75 degrees of the head.
fn valid_eye_target(eye: glam::Vec3, forward: glam::Vec3, target: glam::Vec3) -> bool {
    let delta = target - eye;
    delta.length() >= 1. && delta.normalize().dot(forward) > 0.259
}

/// SDK random view: eye + head * 128 + right * U(-32, 32) + up * U(-16, 16), with
/// VectorVectors axes. The jitter is a deterministic hash of the actor and scene clock.
fn random_view_target(eye: glam::Vec3, forward: glam::Vec3, actor: usize, time: f64) -> glam::Vec3 {
    let forward = forward.normalize_or(glam::Vec3::X);
    let right = if forward.x.abs() < 1e-6 && forward.y.abs() < 1e-6 {
        glam::Vec3::NEG_Y
    } else {
        forward.cross(glam::Vec3::Z).normalize()
    };
    let up = right.cross(forward);
    let mut seed = (actor as u64 ^ (time * 1000.) as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let mut unit = || {
        seed ^= seed >> 29;
        seed = seed.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        (seed >> 40) as f32 / (1u64 << 24) as f32 * 2. - 1.
    };
    eye + forward * 128. + right * (unit() * 32.) + up * (unit() * 16.)
}

/// COND_IN_PVS for animated-face NPCs (CAI_BaseActor::MaintainLookTargets skips the head
/// and eye updates when the player cannot see the actor): the BSP PVS of the player's eye
/// or of the active monitor's point_camera (CPointCamera::SetupVisibility adds its view
/// to the client's visibility; gating on the player alone froze Kleiner on Barney's
/// monitor with his eyes rolled up), refreshed every 0.1 s of scene time. A PVS error
/// leaves the set unknown, which fails open.
pub fn attention_pvs(
    mut game: ResMut<crate::gameplay::Gameplay>,
    sim: Res<crate::simulation::Simulation>,
    sky: Res<crate::sky::Sky>,
    monitors: Res<crate::monitors::Monitors>,
) {
    let now = game.scene.time;
    if now - game.scene.look_targets.pvs_time < 0.1 && game.scene.look_targets.pvs_time > 0. {
        return;
    }
    let mut eyes = vec![glam::Vec3::from_array(sim.eye().to_array())];
    if let Some(camera) = monitors.selected().and_then(|c| game.scene.states.get(c)) {
        eyes.push(camera.origin);
    }
    let mut visible = std::collections::BTreeSet::new();
    for (id, entity) in game.world.entities.iter().enumerate() {
        if !hl2_simulation::attention::has_animated_face(entity.class()) {
            continue;
        }
        let Some(state) = game.scene.states.get(id).filter(|s| !s.killed) else {
            continue;
        };
        let head = state.origin + glam::Vec3::Z * 64.;
        for &eye in &eyes {
            match sky.bounds_in_pvs(eye, state.origin, head) {
                Ok(true) => {
                    visible.insert(id);
                    break;
                }
                Ok(false) => {}
                Err(_) => return,
            }
        }
    }
    game.scene.look_targets.set_pvs(visible, now.max(1e-6));
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn view_target_is_ahead_within_sdk_jitter_and_valid() {
        let eye = glam::Vec3::new(10., 20., 30.);
        for actor in 0..32 {
            let p = random_view_target(eye, glam::Vec3::X, actor, actor as f64 * 0.37);
            let d = p - eye;
            assert!((d.x - 128.).abs() < 1e-4 && d.y.abs() <= 32. && d.z.abs() <= 16.);
            assert!(valid_eye_target(eye, glam::Vec3::X, p));
        }
        assert!(!valid_eye_target(
            eye,
            glam::Vec3::X,
            eye - glam::Vec3::X * 50.
        ));
        assert!(!valid_eye_target(
            eye,
            glam::Vec3::X,
            eye + glam::Vec3::X * 0.5
        ));
    }
}
