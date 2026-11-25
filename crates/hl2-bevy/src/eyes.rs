//! Owned iris projection with authored scene interests and a visible-interest fallback.
use crate::rendering::actor_poses::{ActorPoses, entity_stamp};
use crate::{FlyCamera, assets::LoadedMap, bevy_to_source, rendering::SourceEntity};
use bevy::{
    asset::uuid_handle, mesh::VertexAttributeValues, prelude::*, render::storage::ShaderBuffer,
};
use std::{collections::BTreeMap, sync::Arc};
#[derive(Clone)]
struct Actor {
    key: String,
    gaze: source_assets::eyes::Gaze,
    scale: f32,
}
#[derive(Clone, Copy)]
struct OriginRecord {
    entity: usize,
    origin: [f32; 3],
    forward: [f32; 3],
    attachment: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum TargetLabel {
    Player,
    View,
    Entity(usize),
}
impl TargetLabel {
    fn to_report(self) -> String {
        match self {
            Self::Player => "player".to_owned(),
            Self::View => "view".to_owned(),
            Self::Entity(id) => format!("#{id}"),
        }
    }
}
#[derive(Clone)]
struct ProjectionRecord {
    entity: usize,
    surface: usize,
    rows: [[f32; 4]; 2],
    target: Option<TargetLabel>,
    world_target: Option<[f32; 3]>,
    scripted_interest: Option<hl2_simulation::attention::Interest>,
}
const GAZE_TAU: f32 = 0.045;

#[derive(Clone, Copy)]
struct RenderedGaze {
    dir: glam::Vec3,
    dist: f32,
    time: f64,
}

#[derive(Resource, Default)]
pub struct Eyes {
    actors: BTreeMap<usize, Actor>,
    pose_samples: usize,
    attachment_actors: usize,
    origins: Vec<OriginRecord>,
    draws: usize,
    tracking_player: usize,
    tracking_npc: usize,
    tracking_entity: usize,
    tracking_scene: usize,
    projections: Vec<ProjectionRecord>,
    /// SDK CBaseFlex m_viewtarget: the last eye target persists per actor while valid.
    view_targets: BTreeMap<usize, glam::Vec3>,
    /// Clamped and smoothly slewed rendered eye gaze per actor.
    rendered_gaze: BTreeMap<usize, RenderedGaze>,
    /// Bone-local eye forward/up per (entity, eyeball surface) for FACS eyelids; flexes
    /// consume the previous presentation's basis.
    pub lid_bases: BTreeMap<(usize, usize), (glam::Vec3, glam::Vec3)>,
    /// Each eye mesh's slot in [`EYE_ROWS`], and the rows of every slot (two each).
    slots: std::collections::HashMap<AssetId<Mesh>, u32>,
    /// Slots of eye meshes that are gone, to give out again.
    free: Vec<u32>,
    rows: Vec<Vec4>,
    dirty: bool,
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
        let target = &mut self.rows[at..at + 2];
        if target != rows {
            target.copy_from_slice(&rows);
            self.dirty = true;
        }
    }
    /// A runtime spawn of a template's model (DESIGN 14r part A) tracks like it.
    pub fn copy_actor(&mut self, template: usize, entity: usize) {
        if let Some(actor) = self.actors.get(&template).cloned() {
            self.actors.insert(entity, actor);
        }
    }
    /// Register a late-precached actor's gaze attachment metadata.
    pub fn insert_actor(
        &mut self,
        id: usize,
        key: String,
        gaze: source_assets::eyes::Gaze,
        scale: f32,
    ) {
        self.actors.insert(id, Actor { key, gaze, scale });
    }
    pub fn report(&self) -> serde_json::Value {
        let origins: Vec<_> = self
            .origins
            .iter()
            .map(|o| {
                serde_json::json!({
                    "entity": o.entity,
                    "origin": o.origin,
                    "forward": o.forward,
                    "attachment": o.attachment,
                })
            })
            .collect();
        let projections: Vec<_> = self
            .projections
            .iter()
            .map(|p| {
                serde_json::json!({
                    "entity": p.entity,
                    "surface": p.surface,
                    "rows": p.rows,
                    "target": p.target.map(TargetLabel::to_report),
                    "world_target": p.world_target,
                    "scripted_interest": p.scripted_interest,
                })
            })
            .collect();
        serde_json::json!({"draws":self.draws,"pose_samples":self.pose_samples,"attachment_actors":self.attachment_actors,"origins":origins,"tracking_player":self.tracking_player,"tracking_npc":self.tracking_npc,"tracking_entity":self.tracking_entity,"tracking_scene":self.tracking_scene,"projections":projections,
            "policy":"newest valid authored LOOKAT interest, shared per actor; nearest-visible fallback outside scripted interests; animated owned eyes attachments supply actor origin/forward; native random/head/facial/PVS latch behavior remains unfinished",
            "shader":"owned Eyes iris alpha over sclera with retail planar basis; glints and Source model lighting remain unfinished; FACS eyelids follow the previous frame's eye basis"})
    }
}
fn bone_pose(
    game: &crate::gameplay::Gameplay,
    key: &str,
    id: usize,
    bone: usize,
    poses: &BTreeMap<usize, Arc<Vec<glam::Mat4>>>,
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

/// The eyes' presentation assets and the actor skeletons it shares with the renderer.
type EyeAssets<'w> = (
    ResMut<'w, Assets<Mesh>>,
    ResMut<'w, Assets<ShaderBuffer>>,
    ResMut<'w, ActorPoses>,
);

/// Each eyed actor's skeleton, from the poses shared with the renderer (composed again only
/// when the actor's pose changed); an actor whose clip its rig lacks is composed directly.
fn collect_actor_poses<'a>(
    eyes: &'a Eyes,
    draws: impl IntoIterator<Item = EyeDrawItem<'a>>,
    game: &crate::gameplay::Gameplay,
    shared: &mut ActorPoses,
) -> BTreeMap<usize, Arc<Vec<glam::Mat4>>> {
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
            Some((
                id,
                match entity_stamp(game, rig, id) {
                    Some(stamp) => shared.pose(game, (key, rig), id, &stamp),
                    None => Arc::new(game.scene.actor_matrices(rig, id)),
                },
            ))
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

fn collect_candidates(
    eyes: &mut Eyes,
    game: &crate::gameplay::Gameplay,
    camera: Vec3,
    poses: &BTreeMap<usize, Arc<Vec<glam::Mat4>>>,
) -> Vec<(Option<usize>, glam::Vec3, glam::Vec3)> {
    let mut candidates = vec![(
        None,
        glam::Vec3::from_array(bevy_to_source(camera).to_array()),
        glam::Vec3::ZERO,
    )];
    let mut origins = std::mem::take(&mut eyes.origins);
    origins.clear();
    let mut attachment_actors = 0;
    for (&id, actor) in &eyes.actors {
        let Some(state) = game.scene.states.get(id) else {
            continue;
        };
        if (!state.visible || state.killed) && !game.scene.ragdoll_matrices.contains_key(&id) {
            continue;
        }
        let attached = actor.gaze.attachment.as_ref().and_then(|attachment| {
            bone_pose(game, &actor.key, id, attachment.bone, poses)
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
        origins.push(OriginRecord {
            entity: id,
            origin: origin.to_array(),
            forward: forward.to_array(),
            attachment: attached.is_some(),
        });
    }
    eyes.pose_samples = poses.len();
    eyes.attachment_actors = attachment_actors;
    eyes.origins = origins;
    candidates
}

#[derive(Clone)]
struct Selected {
    rendered: glam::Vec3,
    label: TargetLabel,
    interest: Option<hl2_simulation::attention::Interest>,
}

struct ActorEyeCtx {
    actor: usize,
    eye: glam::Vec3,
    forward: glam::Vec3,
}

fn slerp_unit(from: glam::Vec3, to: glam::Vec3, t: f32) -> glam::Vec3 {
    let dot = from.dot(to).clamp(-1., 1.);
    if dot > 0.9995 {
        return from.lerp(to, t).normalize_or(to);
    }
    if dot < -0.9995 {
        let perp = if from.x.abs() < 0.9 {
            glam::Vec3::X
        } else {
            glam::Vec3::Y
        };
        let axis = from.cross(perp).normalize_or(glam::Vec3::Z);
        return (glam::Quat::from_axis_angle(axis, std::f32::consts::PI * t) * from)
            .normalize_or(to);
    }
    let theta = dot.acos();
    let sin_t = theta.sin();
    ((from * ((1. - t) * theta).sin() + to * (t * theta).sin()) / sin_t).normalize_or(to)
}

fn smooth_gaze(
    rendered: &mut BTreeMap<usize, RenderedGaze>,
    ctx: &ActorEyeCtx,
    target: glam::Vec3,
    max_deflection: f32,
    time: f64,
) -> glam::Vec3 {
    let (actor, eye, forward) = (ctx.actor, ctx.eye, ctx.forward);
    let desired = source_assets::eyes::clamp_view_target(eye, forward, target, max_deflection);
    let desired_vec = desired - eye;
    let desired_dist = desired_vec.length().max(6.);
    let desired_dir = (desired_vec / desired_dist).normalize_or(forward);
    if let Some(prev) = rendered.get_mut(&actor)
        && time >= prev.time
        && time - prev.time <= 0.5
    {
        if time == prev.time {
            return source_assets::eyes::clamp_view_target(
                eye,
                forward,
                eye + prev.dir * prev.dist,
                max_deflection,
            );
        }
        let alpha = 1. - (-((time - prev.time) as f32) / GAZE_TAU).exp();
        let dir = slerp_unit(prev.dir, desired_dir, alpha);
        let dist = prev.dist + (desired_dist - prev.dist) * alpha;
        let point =
            source_assets::eyes::clamp_view_target(eye, forward, eye + dir * dist, max_deflection);
        let clamped_vec = point - eye;
        let clamped_dist = clamped_vec.length().max(6.);
        let clamped_dir = (clamped_vec / clamped_dist).normalize_or(forward);
        *prev = RenderedGaze {
            dir: clamped_dir,
            dist: clamped_dist,
            time,
        };
        return point;
    }
    rendered.insert(
        actor,
        RenderedGaze {
            dir: desired_dir,
            dist: desired_dist,
            time,
        },
    );
    desired
}

fn select_raw_target(
    eyes: &Eyes,
    game: &crate::gameplay::Gameplay,
    sim: &crate::simulation::Simulation,
    ctx: &ActorEyeCtx,
    candidates: &[(Option<usize>, glam::Vec3, glam::Vec3)],
) -> (
    glam::Vec3,
    TargetLabel,
    Option<hl2_simulation::attention::Interest>,
) {
    use hl2_simulation::attention::Target;
    let has_face = game
        .scene
        .entity(&game.world, ctx.actor)
        .is_some_and(|e| hl2_simulation::attention::has_animated_face(e.class()));
    if !game.scene.look_targets.in_pvs(ctx.actor)
        && has_face
        && let Some(&point) = eyes.view_targets.get(&ctx.actor)
    {
        return (point, TargetLabel::View, None);
    }
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
    if let Some((interest, point)) =
        game.scene
            .look_targets
            .eye_target(ctx.actor, ctx.eye, ctx.forward, position)
    {
        let label = match interest.target {
            Target::Player => TargetLabel::Player,
            Target::Entity(id) => TargetLabel::Entity(id),
            Target::Position(_) => TargetLabel::View,
        };
        return (
            point,
            label,
            (interest.ramp == 0.).then(|| interest.clone()),
        );
    }
    if !has_face
        && let Some((id, point, _)) = candidates
            .iter()
            .filter_map(|&(id, p, _)| {
                if id == Some(ctx.actor) {
                    return None;
                }
                let distance = p.distance(ctx.eye);
                if !(1. ..=1024.).contains(&distance)
                    || (p - ctx.eye).normalize().dot(ctx.forward) <= 0.259
                {
                    return None;
                }
                let excluded = [ctx.actor, id.unwrap_or(usize::MAX)];
                sim.physics
                    .projectile_ray(ctx.eye, p, &excluded)
                    .is_none()
                    .then_some((id, p, distance))
            })
            .min_by(|a, b| a.2.total_cmp(&b.2))
    {
        let label = id.map_or(TargetLabel::Player, TargetLabel::Entity);
        return (point, label, None);
    }
    let point = eyes
        .view_targets
        .get(&ctx.actor)
        .copied()
        .filter(|&p| valid_eye_target(ctx.eye, ctx.forward, p))
        .unwrap_or_else(|| random_view_target(ctx.eye, ctx.forward, ctx.actor, game.scene.time));
    (point, TargetLabel::View, None)
}

fn select_actor_target(
    eyes: &mut Eyes,
    game: &crate::gameplay::Gameplay,
    sim: &crate::simulation::Simulation,
    ctx: &ActorEyeCtx,
    candidates: &[(Option<usize>, glam::Vec3, glam::Vec3)],
) -> Selected {
    let (point, label, interest) = select_raw_target(eyes, game, sim, ctx, candidates);
    eyes.view_targets.insert(ctx.actor, point);
    let max_deflection = eyes
        .actors
        .get(&ctx.actor)
        .map_or(source_assets::eyes::DEFAULT_MAX_EYE_DEFLECTION, |a| {
            a.gaze.max_eye_deflection
        });
    let rendered = smooth_gaze(
        &mut eyes.rendered_gaze,
        ctx,
        point,
        max_deflection,
        game.scene.time,
    );
    Selected {
        rendered,
        label,
        interest,
    }
}

pub fn present(
    mut eyes: ResMut<Eyes>,
    game: Res<crate::gameplay::Gameplay>,
    sim: Res<crate::simulation::Simulation>,
    camera: Query<&Transform, With<FlyCamera>>,
    draws: Query<(&SourceEntity, &EyeMesh, &Mesh3d, &ViewVisibility)>,
    (mut meshes, mut buffers, mut shared_poses): EyeAssets,
    performance: Option<Res<crate::performance::Performance>>,
) {
    let _timing = crate::performance::scope(performance.as_deref(), "eyes");
    let Ok(camera) = camera.single() else { return };
    eyes.release_gone(|mesh| meshes.contains(mesh));
    // Evaluate each visible actor once per presentation; target and iris consumers
    // share these skin transforms rather than sampling the full rig for every eye.
    let poses = collect_actor_poses(&eyes, draws, &game, &mut shared_poses);
    let candidates = collect_candidates(&mut eyes, &game, camera.translation, &poses);
    eyes.projections.clear();
    eyes.lid_bases.clear();
    eyes.draws = 0;
    eyes.tracking_player = 0;
    eyes.tracking_npc = 0;
    eyes.tracking_entity = 0;
    eyes.tracking_scene = 0;
    let mut actor_targets = BTreeMap::<usize, Selected>::new();
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
        let selected = actor_targets.entry(owner.0).or_insert_with(|| {
            let origin = bone.transform_point3(mesh_eye.eye.origin);
            let head_forward = -bone.transform_vector3(mesh_eye.eye.forward).normalize();
            let world_origin = state.origin + state.rotation * (origin * mesh_eye.scale);
            let (actor_eye, forward) = candidates
                .iter()
                .find(|(id, _, _)| *id == Some(owner.0))
                .map_or(
                    (world_origin, state.rotation * head_forward),
                    |(_, p, f)| (*p, *f),
                );
            let ctx = ActorEyeCtx {
                actor: owner.0,
                eye: actor_eye,
                forward,
            };
            select_actor_target(&mut eyes, &game, &sim, &ctx, &candidates)
        });
        let local_target =
            Some(state.rotation.inverse() * (selected.rendered - state.origin) / mesh_eye.scale);
        let projection = source_assets::eyes::projection(&mesh_eye.eye, bone, local_target);
        eyes.lid_bases.insert(
            (owner.0, mesh_eye.eye.surface),
            source_assets::eyes::local_basis(
                bone,
                source_assets::eyes::basis(&mesh_eye.eye, bone, local_target),
            ),
        );
        if selected.interest.is_some() {
            eyes.tracking_scene += 1;
        }
        match selected.label {
            TargetLabel::Player => eyes.tracking_player += 1,
            TargetLabel::View => {}
            TargetLabel::Entity(id) => {
                eyes.tracking_entity += 1;
                if game
                    .scene
                    .entity(&game.world, id)
                    .is_some_and(|e| hl2_simulation::attention::is_flex_actor(e.class()))
                {
                    eyes.tracking_npc += 1;
                }
            }
        }
        eyes.projections.push(ProjectionRecord {
            entity: owner.0,
            surface: mesh_eye.eye.surface,
            rows: [projection[0].to_array(), projection[1].to_array()],
            target: Some(selected.label),
            world_target: Some(selected.rendered.to_array()),
            scripted_interest: selected.interest.clone(),
        });
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
    if eyes.dirty
        && let Some(mut buffer) = buffers.get_mut(&EYE_ROWS)
    {
        crate::material_slots::write_rows(&mut buffer, &eyes.rows);
        eyes.dirty = false;
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
    mut visibility: ResMut<crate::visibility::SourceVisibility>,
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
    let Some(rows) = visibility.actor_rows(eyes) else {
        game.scene.look_targets.clear_pvs(now.max(1e-6));
        return;
    };
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
        let tracked = tongue.is_some() || beam.is_some() || game.ai.actors.contains_key(&id);
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
        if visibility.actor_visible(&rows, low, high) {
            visible.insert(id);
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

    #[test]
    fn smooth_gaze_snaps_on_first_sight_clamps_cone_and_slews_without_teleports() {
        let eye = glam::Vec3::new(0., 0., 64.);
        let forward = glam::Vec3::X;
        let ctx = ActorEyeCtx {
            actor: 7,
            eye,
            forward,
        };
        let mut rendered = BTreeMap::new();
        let left = eye
            + glam::Vec3::new(
                100. * 25f32.to_radians().cos(),
                100. * 25f32.to_radians().sin(),
                0.,
            );
        let right = eye
            + glam::Vec3::new(
                100. * 25f32.to_radians().cos(),
                -100. * 25f32.to_radians().sin(),
                0.,
            );

        // First sight snaps immediately to the target with zero warm-up lag.
        let p0 = smooth_gaze(&mut rendered, &ctx, left, 0.866, 1.0);
        assert!((p0 - left).length() < 1e-4, "{p0}");

        // Switching across to +25 deg rotates smoothly without a 1-frame teleport, and freezes when paused.
        let mut prev_y = (p0 - eye).normalize().y;
        let p_paused = smooth_gaze(&mut rendered, &ctx, right, 0.866, 1.0);
        assert!((p_paused - p0).length() < 1e-4, "{p_paused}");

        let mut p_last = p0;
        for step in 1..=14 {
            let t = 1.0 + f64::from(step) * 0.015;
            p_last = smooth_gaze(&mut rendered, &ctx, right, 0.866, t);
            let dir = (p_last - eye).normalize();
            assert!(
                dir.y < prev_y,
                "expected monotonic slew at step {step}: {dir:?}"
            );
            assert!(
                dir.dot(forward) >= 0.865,
                "slew violated cone at step {step}"
            );
            prev_y = dir.y;
        }
        let right_dir = (right - eye).normalize();
        assert!(
            (p_last - eye)
                .normalize()
                .angle_between(right_dir)
                .to_degrees()
                < 1.0
        );

        // Extreme 70 deg target stays clamped to max_eye_deflection both mid-saccade and steady-state.
        let extreme = eye + glam::Vec3::new(34.2, -94.0, 0.);
        let p_mid = smooth_gaze(&mut rendered, &ctx, extreme, 0.866, 1.25);
        assert!((p_mid - eye).normalize().dot(forward) >= 0.865);
        let p_end = smooth_gaze(&mut rendered, &ctx, extreme, 0.866, 2.0);
        assert!(((p_end - eye).normalize().dot(forward) - 0.866).abs() < 1e-3);
    }
}
