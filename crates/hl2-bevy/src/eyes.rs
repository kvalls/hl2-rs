//! Owned iris projection with authored scene interests and a visible-interest fallback.
use crate::{FlyCamera, assets::LoadedMap, bevy_to_source, rendering::SourceEntity};
use bevy::{
    asset::uuid_handle, mesh::VertexAttributeValues, prelude::*, render::storage::ShaderBuffer,
};
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
    /// Each eye mesh's slot in [`EYE_ROWS`], and the rows of every slot (two each), as last
    /// computed and as last uploaded.
    slots: std::collections::HashMap<AssetId<Mesh>, u32>,
    /// Slots of eye meshes that are gone, to give out again.
    free: Vec<u32>,
    rows: Vec<Vec4>,
    uploaded: Vec<Vec4>,
}
impl Eyes {
    pub fn new(loaded: &LoadedMap) -> Self {
        let mut result = Self::default();
        for i in &loaded.world.model_instances {
            let Some(id) = i.entity else { continue };
            if !hl2_simulation::attention::is_flex_actor(loaded.world.entities[id].class()) {
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
    /// An eye mesh's slot, given on first sight and kept for the map.
    /// An eye mesh's slot, given on first sight and kept while the mesh exists; None when all
    /// [`EYE_SLOTS`] are taken.
    fn slot(&mut self, mesh: AssetId<Mesh>) -> Option<u32> {
        if let Some(slot) = self.slots.get(&mesh) {
            return Some(*slot);
        }
        let slot = self.free.pop().or_else(|| {
            let next = self.slots.len() as u32;
            (next < EYE_SLOTS as u32).then_some(next)
        })?;
        self.slots.insert(mesh, slot);
        Some(slot)
    }
    /// Frees the slots of eye meshes that no longer exist.
    fn release_gone(&mut self, exists: impl Fn(AssetId<Mesh>) -> bool) {
        let free = &mut self.free;
        self.slots.retain(|mesh, slot| {
            exists(*mesh) || {
                free.push(*slot);
                false
            }
        });
    }
    fn set_rows(&mut self, slot: u32, rows: [Vec4; 2]) {
        let at = slot as usize * 2;
        if self.rows.is_empty() {
            self.rows.resize(2 * EYE_SLOTS, Vec4::ZERO);
        }
        self.rows[at..at + 2].copy_from_slice(&rows);
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
    &'a ViewVisibility,
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

/// An eyeball surface's draw: its model, eyeball and scale.
#[derive(Component)]
pub struct EyeMesh {
    pub key: String,
    pub eye: source_assets::eyes::Eyeball,
    pub scale: f32,
}
/// Every eye mesh's iris projection: two world-space rows per slot (the mesh's second UV
/// holds its slot), which every source material reads; one upload a frame instead of every
/// eye mesh's texture coordinates.
pub const EYE_ROWS: Handle<ShaderBuffer> = uuid_handle!("6f1b5c2e-9d3a-4e8b-a7c1-2f4d8e6b9a10");
/// Eye meshes the buffer holds. Its size never changes: a resized buffer is a new GPU buffer the
/// materials' bind groups would not see.
const EYE_SLOTS: usize = 1024;
/// Creates the projection buffer before any material reads it.
pub fn install_rows(mut buffers: ResMut<Assets<ShaderBuffer>>) {
    let rows = vec![bevy::math::Vec4::ZERO; 2 * EYE_SLOTS];
    let mut buffer = ShaderBuffer::default();
    buffer.set_data(rows);
    let _ = buffers.insert(EYE_ROWS.id(), buffer);
}

pub fn present(
    mut eyes: ResMut<Eyes>,
    game: Res<crate::gameplay::Gameplay>,
    sim: Res<crate::simulation::Simulation>,
    camera: Query<&Transform, With<FlyCamera>>,
    draws: Query<(&SourceEntity, &EyeMesh, &Mesh3d, &ViewVisibility)>,
    (mut meshes, mut buffers): (ResMut<Assets<Mesh>>, ResMut<Assets<ShaderBuffer>>),
    performance: Option<Res<crate::performance::Performance>>,
) {
    let _timing = crate::performance::scope(performance.as_deref(), "eyes");
    let Ok(camera) = camera.single() else { return };
    eyes.release_gone(|mesh| meshes.contains(mesh));
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
    for (owner, mesh_eye, handle, visibility) in &draws {
        // Like retail, only an eye being drawn gets its iris projection (it was culled last frame
        // otherwise); gaze targets stay per actor.
        if !visibility.get() {
            continue;
        }
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
                        .is_some_and(|e| hl2_simulation::attention::is_flex_actor(e.class()))
                }) {
                    eyes.tracking_npc += 1;
                }
            }
        }
        eyes.projections.push(serde_json::json!({"entity":owner.0,"surface":mesh_eye.eye.surface,"rows":projection.iter().map(|r|r.to_array()).collect::<Vec<_>>(),
            "target":selected.as_ref().map(|s|&s.label),"world_target":selected.as_ref().map(|s|s.point.to_array()),"scripted_interest":selected.as_ref().and_then(|s|s.interest.as_ref())}));
        // The shader projects each fragment's world position through this eye's rows; the
        // mesh's second UV names its slot, written once.
        let Some(slot) = eyes.slot(handle.0.id()) else {
            continue;
        };
        // The pose the draws are placed at (`placement::place_entities`).
        let placed = sim.physics.entity_pose(owner.0);
        let placed = placed.unwrap_or((state.origin, state.rotation));
        let rows = world_rows(projection, placed, mesh_eye.scale);
        eyes.set_rows(slot, rows);
        if let Some(mesh) = meshes.get(&handle.0) {
            let (count, tag) = (mesh.count_vertices(), [slot as f32, 0.]);
            let tagged = matches!(mesh.attribute(Mesh::ATTRIBUTE_UV_1),
                Some(VertexAttributeValues::Float32x2(uvs)) if uvs.len() == count && uvs.iter().all(|uv| *uv == tag));
            if !tagged && let Some(mut mesh) = meshes.get_mut(&handle.0) {
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![tag; count]);
            }
            eyes.draws += 1;
        }
    }
    eprintln!("EYEPROBE draws {} slots {}", eyes.draws, eyes.slots.len());
    if eyes.rows != eyes.uploaded
        && let Some(mut buffer) = buffers.get_mut(&EYE_ROWS)
    {
        buffer.set_data(eyes.rows.clone());
        eyes.uploaded = eyes.rows.clone();
    }
}

/// World-space iris rows from `rows`, which take a skinned model-space point (Source units,
/// before scale): a world (Bevy) position maps there through the actor's placement, as the
/// renderer places its draws (origin + rotation x point x scale, in Bevy axes).
fn world_rows(
    rows: [glam::Vec4; 2],
    (origin, rotation): (glam::Vec3, glam::Quat),
    scale: f32,
) -> [Vec4; 2] {
    // Bevy (x, y, z) is Source (x, -z, y).
    let to_source =
        glam::Mat4::from_cols(glam::Vec4::X, glam::Vec4::Z, -glam::Vec4::Y, glam::Vec4::W);
    let to_model = glam::Mat4::from_scale(glam::Vec3::splat(1. / scale))
        * glam::Mat4::from_quat(rotation.inverse())
        * glam::Mat4::from_translation(-origin)
        * to_source;
    rows.map(|row| Vec4::from_array((to_model.transpose() * row).to_array()))
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
    // Scanner beams and lit point_spotlights: near when any of the beam can be seen.
    let lamps = game
        .scene
        .spotlights
        .beams(&game.world, &sim.physics)
        .copied();
    let beams = game.ai.scanner_beams(&game.scene).chain(lamps);
    let beams: std::collections::BTreeMap<_, _> = beams.map(|b| (b.entity, b)).collect();
    for (id, entity) in game.world.entities.iter().enumerate() {
        let tongue = game.scene.tongues.get(&id);
        let beam = beams.get(&id);
        let tracked = tongue.is_some() || beam.is_some();
        if !tracked && !hl2_simulation::attention::has_animated_face(entity.class()) {
            continue;
        }
        let Some(state) = game.scene.states.get(id).filter(|s| !s.killed) else {
            continue;
        };
        // A barnacle is near when the player can see any of its tongue, a scanner any of its beam.
        let (low, high) = match (tongue, beam) {
            (Some(tongue), _) => {
                let side = glam::Vec3::new(16., 16., 0.);
                (
                    tongue.nodes[tongue.nodes.len() - 1] - side,
                    state.origin + side,
                )
            }
            (None, Some(beam)) => (beam.start.min(beam.end), beam.start.max(beam.end)),
            (None, None) => (state.origin, state.origin + glam::Vec3::Z * 64.),
        };
        for &eye in &eyes {
            match sky.bounds_in_pvs(eye, low, high) {
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
    fn world_rows_project_the_rendered_position_like_the_model_point() {
        let rows = [
            glam::Vec4::new(0.3, -0.2, 0.9, 0.5),
            glam::Vec4::new(-0.7, 0.4, 0.1, -0.25),
        ];
        for (origin, yaw, pitch, scale) in [
            (glam::Vec3::ZERO, 0., 0., 1.),
            (glam::Vec3::new(-2456., -2556., 64.), 1.1, 0., 1.),
            (glam::Vec3::new(812., 4096., -300.), -2.5, 0.4, 1.25),
        ] {
            let rotation = glam::Quat::from_rotation_z(yaw) * glam::Quat::from_rotation_y(pitch);
            let world = world_rows(rows, (origin, rotation), scale);
            for model in [
                glam::Vec3::new(1.5, -0.5, 66.),
                glam::Vec3::new(-3., 2., 60.),
            ] {
                // Where the renderer draws the skinned model point, in Bevy axes.
                let source = origin + rotation * (model * scale);
                let at = crate::source_to_bevy(Vec3::from_array(source.to_array())).extend(1.);
                for (world, model_row) in world.iter().zip(rows) {
                    let expected = model_row.dot(model.extend(1.));
                    assert!(
                        (world.dot(at) - expected).abs() < 2e-3,
                        "{origin} {expected}"
                    );
                }
            }
        }
    }
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
