//! Conservative weapon raster coverage for the first reduction; later dilation stays full.
use super::WeaponBlur;
use bevy::{
    camera::{
        primitives::Aabb,
        visibility::{VisibilitySystems, VisibleEntities},
    },
    ecs::schedule::IntoScheduleConfigs,
    prelude::{
        App, Camera, Commands, Component, Entity, GlobalTransform, Mat4, Mesh3d, PostUpdate, Query,
        Res, UVec2, Vec2, Vec3A, Vec4, With, Without,
    },
    render::extract_component::{ExtractComponent, ExtractComponentPlugin},
};
use std::any::TypeId;

const FULL: Vec4 = Vec4::new(0., 0., 1., 1.);
#[derive(Component, Clone, Copy, PartialEq, ExtractComponent)]
pub(super) struct MotionRegion(Vec4);

impl MotionRegion {
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
        &'static WeaponBlur,
        Option<&'static mut MotionRegion>,
    ),
    With<crate::camera::ViewmodelCamera>,
>;
fn collect(
    mut commands: Commands,
    mut cameras: Cameras,
    meshes: Query<(&GlobalTransform, Option<&Aabb>), With<Mesh3d>>,
    stale: Query<Entity, (With<MotionRegion>, Without<WeaponBlur>)>,
    performance: Option<Res<crate::performance::Performance>>,
) {
    for entity in &stale {
        commands.entity(entity).remove::<MotionRegion>();
    }
    if cameras.is_empty() {
        return;
    }
    let _timing = performance
        .as_deref()
        .map(|p| crate::performance::scope(Some(p), "motion_region"));
    for (entity, camera, transform, visible, blur, previous) in &mut cameras {
        let clip_from_world = camera.clip_from_view() * transform.to_matrix().inverse();
        let mut low = Vec2::ONE;
        let mut high = Vec2::ZERO;
        if camera.is_active && blur.max > 0. {
            for mesh in visible.get(TypeId::of::<Mesh3d>()) {
                let Ok((transform, aabb)) = meshes.get(*mesh) else {
                    continue;
                };
                let r = aabb.map_or(FULL, |aabb| {
                    project(clip_from_world * transform.to_matrix(), aabb)
                });
                if r.z > r.x && r.w > r.y {
                    low = low.min(r.truncate().truncate());
                    high = high.max(Vec2::new(r.z, r.w));
                }
            }
        }
        let current = MotionRegion(Vec4::new(low.x, low.y, high.x, high.y));
        if let Some(mut previous) = previous {
            if *previous != current {
                *previous = current;
            }
        } else {
            commands.entity(entity).insert(current);
        }
    }
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
        let projection = Mat4::perspective_infinite_reverse_rh(1., 1.5, 0.1);
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
            let matrix = Mat4::perspective_infinite_reverse_rh(1., 1.5, 0.1)
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
}
