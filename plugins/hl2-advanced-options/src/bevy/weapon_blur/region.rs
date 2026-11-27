//! Conservative moving-mesh raster coverage for motion reductions and object gather.
use super::WeaponBlur;
use bevy::{
    camera::{
        primitives::Aabb,
        visibility::{VisibilitySystems, VisibleEntities},
    },
    ecs::schedule::IntoScheduleConfigs,
    math::Affine3A,
    mesh::{Mesh3d, skinning::SkinnedMesh},
    prelude::{
        App, Camera, Commands, Component, DetectChanges, Entity, GlobalTransform, Local, Mat4,
        PostUpdate, Query, Ref, Res, UVec2, Vec2, Vec3, Vec3A, Vec4, With, Without,
    },
    render::extract_component::{ExtractComponent, ExtractComponentPlugin},
};
use std::{any::TypeId, collections::HashMap};

const FULL: Vec4 = Vec4::new(0., 0., 1., 1.);
const EMPTY: Vec4 = Vec4::new(1., 1., 0., 0.);
/// Consecutive frames a mesh stays active in `MotionRegion` so 1-frame velocity history settles.
const QUIET_FRAMES: u64 = 2;
/// Minimum screen-space motion (in physical pixels) before a mesh enters `MotionRegion`.
/// The shader early-outs below 1.0 pixel (`smoothstep(1.0, 3.0, pixel_motion) == 0.0`).
const MIN_PIXEL_MOTION: f32 = 0.8;

#[derive(Component, Clone, Copy, PartialEq, ExtractComponent)]
#[extract_app(bevy::render::RenderApp)]
pub(super) struct MotionRegion(Vec4);

fn div_ceil(size: UVec2, factor: u32) -> UVec2 {
    UVec2::new(size.x.div_ceil(factor), size.y.div_ceil(factor))
}

impl MotionRegion {
    pub(super) fn is_empty(&self) -> bool {
        self.0.z <= self.0.x || self.0.w <= self.0.y
    }

    pub(super) fn scissor(&self, size: UVec2) -> [u32; 4] {
        let r = self.0;
        if r.z <= r.x || r.w <= r.y || size.x == 0 || size.y == 0 {
            return [0; 4];
        }
        let low = UVec2::new(
            ((r.x * size.x as f32).floor() as u32).saturating_sub(2) / 4,
            ((r.y * size.y as f32).floor() as u32).saturating_sub(2) / 4,
        );
        let high = UVec2::new(
            ((r.z * size.x as f32).ceil() as u32)
                .saturating_add(2)
                .min(size.x)
                .div_ceil(4),
            ((r.w * size.y as f32).ceil() as u32)
                .saturating_add(2)
                .min(size.y)
                .div_ceil(4),
        );
        [low.x, low.y, high.x - low.x, high.y - low.y]
    }

    pub(super) fn dilation_radius(size: UVec2, max_blur: f32) -> u32 {
        ((max_blur.max(0.) * size.max_element() as f32 * 0.5 / 32.).ceil() as u32) + 2
    }

    pub(super) fn tile_scissor(&self, size: UVec2, pad_tiles: u32) -> [u32; 4] {
        let s4 = self.scissor(size);
        if s4[2] == 0 || s4[3] == 0 {
            return [0; 4];
        }
        let tiles = div_ceil(size, 32);
        let pad = UVec2::splat(pad_tiles);
        let low = UVec2::new(s4[0] / 8, s4[1] / 8).saturating_sub(pad);
        let high =
            (UVec2::new((s4[0] + s4[2]).div_ceil(8), (s4[1] + s4[3]).div_ceil(8)) + pad).min(tiles);
        [low.x, low.y, high.x - low.x, high.y - low.y]
    }

    pub(super) fn gather_scissor(&self, size: UVec2, max_blur: f32) -> [u32; 4] {
        let ts = self.tile_scissor(size, Self::dilation_radius(size, max_blur));
        if ts[2] == 0 || ts[3] == 0 {
            return [0; 4];
        }
        let tiles = div_ceil(size, 32).max(UVec2::ONE).as_vec2();
        let qsize = div_ceil(size, 4);
        let low_tile = UVec2::new(ts[0], ts[1]).as_vec2();
        let high_tile = UVec2::new(ts[0] + ts[2], ts[1] + ts[3]).as_vec2();
        let low = ((low_tile / tiles) * qsize.as_vec2())
            .floor()
            .as_uvec2()
            .saturating_sub(UVec2::ONE);
        let high =
            (((high_tile / tiles) * qsize.as_vec2()).ceil().as_uvec2() + UVec2::ONE).min(qsize);
        [low.x, low.y, high.x - low.x, high.y - low.y]
    }

    pub(super) fn pass_scissor(&self, pass: usize, size: UVec2, max_blur: f32) -> [u32; 4] {
        match pass {
            0 => self.scissor(size),
            1 => self.tile_scissor(size, 0),
            _ => self.tile_scissor(size, Self::dilation_radius(size, max_blur)),
        }
    }
}

fn project(clip_from_local: Mat4, aabb: &Aabb) -> Vec4 {
    if !aabb.center.is_finite()
        || !aabb.half_extents.is_finite()
        || aabb.half_extents.min_element() < 0.
    {
        return FULL;
    }
    let mut low = Vec2::splat(f32::INFINITY);
    let mut high = Vec2::splat(f32::NEG_INFINITY);
    let mut front = 0;
    for x in [-1., 1.] {
        for y in [-1., 1.] {
            for z in [-1., 1.] {
                let local = aabb.center + aabb.half_extents * Vec3A::new(x, y, z);
                let clip = clip_from_local * bevy::math::Vec3::from(local).extend(1.);
                if !clip.is_finite() {
                    return FULL;
                }
                if clip.w <= 1e-6 {
                    continue;
                }
                front += 1;
                let uv = Vec2::new(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
                if !uv.is_finite() {
                    return FULL;
                }
                low = low.min(uv);
                high = high.max(uv);
            }
        }
    }
    if front == 0 {
        return Vec4::ZERO;
    }
    if front != 8 {
        return FULL;
    }
    let low = low.clamp(Vec2::ZERO, Vec2::ONE);
    let high = high.clamp(Vec2::ZERO, Vec2::ONE);
    Vec4::new(low.x, low.y, high.x, high.y)
}

type Cameras<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Camera,
        &'static GlobalTransform,
        &'static VisibleEntities,
        Ref<'static, WeaponBlur>,
        Option<&'static mut MotionRegion>,
    ),
>;
type Meshes<'w, 's> = Query<
    'w,
    's,
    (
        Ref<'static, Mesh3d>,
        Ref<'static, GlobalTransform>,
        Option<&'static Aabb>,
        Option<Ref<'static, SkinnedMesh>>,
    ),
>;

#[derive(Default)]
struct RegionState {
    frame: u64,
    last_moved_mesh: HashMap<Entity, u64>,
    prev_transforms: HashMap<Entity, Affine3A>,
    prev_joints: HashMap<Entity, Affine3A>,
    pending_joints: HashMap<Entity, Affine3A>,
    frame_mesh_motion: HashMap<Entity, f32>,
    prev_raw: HashMap<Entity, Vec4>,
}

fn affine_point_motion(curr: Affine3A, prev: Affine3A, center: Vec3, half: Vec3) -> f32 {
    let dc = (curr.transform_point3(center) - prev.transform_point3(center)).length();
    let dx = (curr.matrix3.x_axis - prev.matrix3.x_axis).length() * half.x;
    let dy = (curr.matrix3.y_axis - prev.matrix3.y_axis).length() * half.y;
    let dz = (curr.matrix3.z_axis - prev.matrix3.z_axis).length() * half.z;
    dc + dx + dy + dz
}

type MeshItem<'a> = (
    Ref<'a, Mesh3d>,
    Ref<'a, GlobalTransform>,
    Option<&'a Aabb>,
    Option<Ref<'a, SkinnedMesh>>,
);

fn mesh_world_displacement(
    mesh_entity: Entity,
    (mesh, mesh_transform, aabb, skin): &MeshItem<'_>,
    joints: &Query<Ref<GlobalTransform>>,
    state: &mut RegionState,
) -> f32 {
    if let Some(&cached) = state.frame_mesh_motion.get(&mesh_entity) {
        return cached;
    }
    let center = aabb.map_or(Vec3::ZERO, |a| Vec3::from(a.center));
    let half = aabb.map_or(Vec3::splat(32.), |a| {
        Vec3::from(a.half_extents).max(Vec3::ONE)
    });
    let curr_tf = mesh_transform.affine();
    let prev_tf = state.prev_transforms.insert(mesh_entity, curr_tf);
    let mut motion = if mesh.is_changed() {
        f32::INFINITY
    } else if mesh_transform.is_changed() {
        prev_tf.map_or(0., |prev| affine_point_motion(curr_tf, prev, center, half))
    } else {
        0.
    };
    if let Some(skin) = skin {
        let force_check = skin.is_changed() || mesh_transform.is_changed();
        for &joint_entity in &skin.joints {
            let Ok(joint_tf) = joints.get(joint_entity) else {
                continue;
            };
            if !force_check
                && !joint_tf.is_changed()
                && state.prev_joints.contains_key(&joint_entity)
            {
                continue;
            }
            let curr_joint = joint_tf.affine();
            // Defer prev_joints updates until frame end so shared skeletons move all submeshes.
            state.pending_joints.insert(joint_entity, curr_joint);
            let Some(&prev_joint) = state.prev_joints.get(&joint_entity) else {
                continue;
            };
            if prev_joint == curr_joint {
                continue;
            }
            if curr_joint.matrix3.determinant().abs() < 1e-12 {
                if prev_joint.matrix3.determinant().abs() >= 1e-12 {
                    motion = f32::INFINITY;
                }
                continue;
            }
            // Inverse-bind cancels when mapping animated mesh-local Aabb back through the joint delta.
            let prev_world = prev_joint * curr_joint.inverse() * curr_tf;
            motion = motion.max(affine_point_motion(curr_tf, prev_world, center, half));
        }
    }
    state.frame_mesh_motion.insert(mesh_entity, motion);
    motion
}

fn union_rect(a: Vec4, b: Vec4) -> Vec4 {
    let a_valid = a.z > a.x && a.w > a.y;
    let b_valid = b.z > b.x && b.w > b.y;
    match (a_valid, b_valid) {
        (true, true) => Vec4::new(a.x.min(b.x), a.y.min(b.y), a.z.max(b.z), a.w.max(b.w)),
        (true, false) => a,
        (false, true) => b,
        (false, false) => EMPTY,
    }
}

fn camera_region(
    entity: Entity,
    (camera, transform, visible, blur): (&Camera, &GlobalTransform, &VisibleEntities, &WeaponBlur),
    meshes: &Meshes,
    joints: &Query<Ref<GlobalTransform>>,
    state: &mut RegionState,
) -> MotionRegion {
    let frame = state.frame;
    if !camera.is_active {
        state.prev_raw.insert(entity, EMPTY);
        return MotionRegion(EMPTY);
    }
    if blur.max == 0. {
        for &mesh_entity in visible.get(TypeId::of::<Mesh3d>()) {
            if let Ok(item) = meshes.get(mesh_entity) {
                mesh_world_displacement(mesh_entity, &item, joints, state);
            }
        }
        state.prev_raw.insert(entity, EMPTY);
        return MotionRegion(EMPTY);
    }
    let clip = camera.clip_from_view();
    let clip_from_world = clip * transform.to_matrix().inverse();
    let viewport = camera
        .physical_viewport_size()
        .unwrap_or(UVec2::new(3456, 2168));
    let focal_px = (clip.x_axis.x.abs() * viewport.x as f32)
        .max(clip.y_axis.y.abs() * viewport.y as f32)
        * 0.5;
    let mut low = Vec2::ONE;
    let mut high = Vec2::ZERO;
    for &mesh_entity in visible.get(TypeId::of::<Mesh3d>()) {
        let Ok(item) = meshes.get(mesh_entity) else {
            continue;
        };
        let world_motion = mesh_world_displacement(mesh_entity, &item, joints, state);
        let (_, mesh_transform, aabb, _) = &item;
        let center = aabb.map_or(Vec3::ZERO, |a| Vec3::from(a.center));
        let half_radius = aabb.map_or(32., |a| Vec3::from(a.half_extents).length());
        let world_center = mesh_transform.affine().transform_point3(center);
        let dist = ((world_center - transform.translation()).length() - half_radius).max(8.);
        let pixel_motion = (world_motion * focal_px / dist) * blur.scale;
        if pixel_motion >= MIN_PIXEL_MOTION {
            state.last_moved_mesh.insert(mesh_entity, frame);
        }
        let mesh_active = state
            .last_moved_mesh
            .get(&mesh_entity)
            .is_some_and(|&f| frame.saturating_sub(f) < QUIET_FRAMES);
        if !mesh_active {
            continue;
        }
        let r = aabb.map_or(FULL, |aabb| {
            project(clip_from_world * mesh_transform.to_matrix(), aabb)
        });
        if r.z > r.x && r.w > r.y {
            low = low.min(r.truncate().truncate());
            high = high.max(Vec2::new(r.z, r.w));
        }
    }
    let raw = Vec4::new(low.x, low.y, high.x, high.y);
    let prev = state.prev_raw.insert(entity, raw).unwrap_or(EMPTY);
    let combined = if raw.z > raw.x && raw.w > raw.y {
        union_rect(raw, prev)
    } else {
        EMPTY
    };
    MotionRegion(combined)
}

fn collect(
    mut commands: Commands,
    mut cameras: Cameras,
    meshes: Meshes,
    joints: Query<Ref<GlobalTransform>>,
    stale: Query<Entity, (With<MotionRegion>, Without<WeaponBlur>)>,
    performance: Option<Res<crate::bevy::diagnostics::Diagnostics>>,
    mut state: Local<RegionState>,
) {
    for entity in &stale {
        commands.entity(entity).remove::<MotionRegion>();
    }
    if cameras.is_empty() {
        return;
    }
    let _timing = performance.as_deref().map(|p| p.scope("motion_region"));
    state.frame = state.frame.wrapping_add(1);
    state.frame_mesh_motion.clear();
    state.pending_joints.clear();
    for (entity, camera, transform, visible, blur, previous) in &mut cameras {
        let current = camera_region(
            entity,
            (camera, transform, visible, &blur),
            &meshes,
            &joints,
            &mut state,
        );
        if let Some(mut previous) = previous {
            if *previous != current {
                *previous = current;
            }
        } else {
            commands.entity(entity).insert(current);
        }
    }
    let state = &mut *state;
    state.prev_joints.extend(state.pending_joints.drain());
}

pub(super) fn install(app: &mut App) {
    app.add_plugins(ExtractComponentPlugin::<MotionRegion>::default())
        .add_systems(
            PostUpdate,
            collect.after(VisibilitySystems::CheckVisibility),
        );
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::{Quat, Vec3};

    #[test]
    fn projection_fails_open_at_the_camera_and_skips_boxes_behind_it() {
        let projection = bevy::math::proj::perspective_infinite_reverse(1., 1.5, 0.1);
        let bounds = Aabb {
            center: Vec3A::ZERO,
            half_extents: Vec3A::ONE,
        };
        assert_eq!(project(projection, &bounds), FULL);
        assert_eq!(
            project(projection * Mat4::from_translation(Vec3::Z * 4.), &bounds),
            Vec4::ZERO
        );
        let invalid = Aabb {
            center: Vec3A::splat(f32::NAN),
            ..bounds
        };
        assert_eq!(project(projection, &invalid), FULL);
    }

    #[test]
    fn rounded_scissor_contains_projected_interior_points_on_odd_targets() {
        let size = UVec2::new(1031, 773);
        let bounds = Aabb {
            center: Vec3A::ZERO,
            half_extents: Vec3A::ONE,
        };
        for angle in [-0.7, 0., 0.9] {
            let matrix = bevy::math::proj::perspective_infinite_reverse(1., 1.5, 0.1)
                * Mat4::from_rotation_translation(
                    Quat::from_rotation_y(angle),
                    Vec3::new(1., 0.4, -6.),
                );
            let rect = MotionRegion(project(matrix, &bounds)).scissor(size);
            for x in [-1., -0.5, 0., 0.5, 1.] {
                for y in [-1., 0., 1.] {
                    for z in [-1., 0., 1.] {
                        let p = matrix * Vec4::new(x, y, z, 1.);
                        let uv = Vec2::new(p.x / p.w * 0.5 + 0.5, 0.5 - p.y / p.w * 0.5);
                        if uv.cmpge(Vec2::ZERO).all() && uv.cmple(Vec2::ONE).all() {
                            let pixel = (uv * size.as_vec2()).as_uvec2().min(size - UVec2::ONE) / 4;
                            assert!(pixel.x >= rect[0] && pixel.x < rect[0] + rect[2]);
                            assert!(pixel.y >= rect[1] && pixel.y < rect[1] + rect[3]);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn empty_and_full_extents_preserve_clear_and_partial_tiles() {
        assert_eq!(
            MotionRegion(Vec4::ZERO).scissor(UVec2::new(1031, 773)),
            [0; 4]
        );
        assert_eq!(
            MotionRegion(FULL).scissor(UVec2::new(1031, 773)),
            [0, 0, 258, 194]
        );
        assert_eq!(MotionRegion(FULL).scissor(UVec2::ZERO), [0; 4]);
    }

    #[test]
    fn tile_and_gather_scissors_bound_reduced_and_dilated_extents() {
        let empty = MotionRegion(EMPTY);
        assert!(empty.is_empty());
        assert_eq!(empty.tile_scissor(UVec2::new(3456, 2168), 3), [0; 4]);
        assert_eq!(empty.gather_scissor(UVec2::new(3456, 2168), 0.08), [0; 4]);

        let region = MotionRegion(Vec4::new(0.25, 0.25, 0.5, 0.5));
        assert!(!region.is_empty());
        for size in [UVec2::new(1031, 773), UVec2::new(3456, 2168)] {
            let s4 = region.scissor(size);
            let t0 = region.tile_scissor(size, 0);
            let td = region.tile_scissor(size, MotionRegion::dilation_radius(size, 0.08));
            let g4 = region.gather_scissor(size, 0.08);
            assert!(t0[2] > 0 && t0[3] > 0);
            assert!(td[0] <= t0[0] && td[1] <= t0[1]);
            assert!(td[0] + td[2] >= t0[0] + t0[2] && td[1] + td[3] >= t0[1] + t0[3]);
            assert!(g4[0] <= s4[0] && g4[1] <= s4[1]);
            assert!(g4[0] + g4[2] >= s4[0] + s4[2] && g4[1] + g4[3] >= s4[1] + s4[3]);
            assert!(g4[0] + g4[2] <= size.x.div_ceil(4));
            assert!(g4[1] + g4[3] <= size.y.div_ceil(4));
        }
    }

    fn setup_shared_skeleton_app(blur_max: f32) -> (App, Entity, Entity, Vec4, Vec4) {
        use bevy::{
            camera::RenderTargetInfo,
            prelude::{Transform, TransformPlugin, TransformSystems},
        };
        let mut app = App::new();
        app.add_plugins(TransformPlugin)
            .add_systems(PostUpdate, collect.after(TransformSystems::Propagate));

        let joint = app.world_mut().spawn(Transform::IDENTITY).id();
        let bounds = Aabb {
            center: Vec3A::ZERO,
            half_extents: Vec3A::splat(0.5),
        };
        let pos_left = Transform::from_xyz(-2.0, 0.0, -8.0);
        let pos_right = Transform::from_xyz(2.0, 0.0, -8.0);
        let mesh_left = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                pos_left,
                bounds,
                SkinnedMesh {
                    inverse_bindposes: Default::default(),
                    joints: vec![joint],
                },
            ))
            .id();
        let mesh_right = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                pos_right,
                bounds,
                SkinnedMesh {
                    inverse_bindposes: Default::default(),
                    joints: vec![joint],
                },
            ))
            .id();

        let mut camera = Camera::default();
        let proj = bevy::math::proj::perspective_infinite_reverse(1.0, 1.5, 0.1);
        camera.computed.clip_from_view = proj;
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: UVec2::new(1000, 1000),
            scale_factor: 1.0,
        });
        let mut visible = VisibleEntities::default();
        visible
            .entities
            .insert(TypeId::of::<Mesh3d>(), vec![mesh_left, mesh_right]);
        let cam_entity = app
            .world_mut()
            .spawn((
                camera,
                Transform::IDENTITY,
                visible,
                WeaponBlur {
                    max: blur_max,
                    scale: 1.0,
                    world: false,
                    camera_moving: false,
                },
            ))
            .id();
        let rect_left = project(proj * pos_left.to_matrix(), &bounds);
        let rect_right = project(proj * pos_right.to_matrix(), &bounds);
        (app, cam_entity, joint, rect_left, rect_right)
    }

    #[test]
    fn shared_skeleton_motion_covers_all_moving_submeshes_and_clears_when_still() {
        use bevy::prelude::Transform;
        let (mut app, cam, joint, rect_left, rect_right) = setup_shared_skeleton_app(0.12);
        // Spawned Mesh3d starts marked changed; let QUIET_FRAMES settle before testing joint motion.
        for _ in 0..=QUIET_FRAMES {
            app.update();
        }
        assert!(app.world().get::<MotionRegion>(cam).unwrap().is_empty());

        app.world_mut()
            .get_mut::<Transform>(joint)
            .unwrap()
            .translation
            .x = 0.5;
        app.update();
        let region = app.world().get::<MotionRegion>(cam).unwrap().0;
        let both = union_rect(rect_left, rect_right);
        assert!(
            region.x <= both.x + 1e-4
                && region.y <= both.y + 1e-4
                && region.z >= both.z - 1e-4
                && region.w >= both.w - 1e-4,
            "expected shared skeleton to cover both submeshes {both:?}, got {region:?}"
        );

        for _ in 0..QUIET_FRAMES {
            app.update();
        }
        assert!(app.world().get::<MotionRegion>(cam).unwrap().is_empty());
    }

    #[test]
    fn warmup_frames_keep_region_empty_and_seed_shared_joint_history() {
        use bevy::prelude::Transform;
        let (mut app, cam, joint, rect_left, rect_right) = setup_shared_skeleton_app(0.0);
        app.update();
        assert!(app.world().get::<MotionRegion>(cam).unwrap().is_empty());

        app.world_mut()
            .get_mut::<Transform>(joint)
            .unwrap()
            .translation
            .x = 0.5;
        app.world_mut().get_mut::<WeaponBlur>(cam).unwrap().max = 0.12;
        app.update();
        let region = app.world().get::<MotionRegion>(cam).unwrap().0;
        let both = union_rect(rect_left, rect_right);
        assert!(
            region.x <= both.x + 1e-4 && region.z >= both.z - 1e-4,
            "expected warmup frame history to cover both submeshes {both:?}, got {region:?}"
        );
    }

    #[test]
    fn singular_zero_scale_joint_does_not_nan_or_panic() {
        use bevy::prelude::Transform;
        let (mut app, cam, joint, _, _) = setup_shared_skeleton_app(0.12);
        app.update();
        app.world_mut().get_mut::<Transform>(joint).unwrap().scale = Vec3::ZERO;
        app.update();
        let region = app.world().get::<MotionRegion>(cam).unwrap().0;
        assert!(region.is_finite());
    }
}
