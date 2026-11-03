//! Model ambient/local light presentation and the viewmodel's draw-to-world lighting basis.
use super::{ModelLighting, ModelLit, SourceMaterial};
use crate::source_to_bevy;
use bevy::{
    asset::Assets,
    camera::visibility::InheritedVisibility,
    ecs::system::{Query, Res, ResMut},
    math::{Mat4, Vec3},
    pbr::MeshMaterial3d,
    transform::components::Transform,
};
use std::collections::BTreeMap;

/// Recompute each lit model's light state when its illumination origin moves
/// (retail light cache at origin + rotation * illumposition * scale), and every frame
/// while an entity light (muzzle elight) reaches it (ComputeDynamicLighting).
pub fn present_lighting(
    game: Res<crate::gameplay::Gameplay>,
    sim: Res<crate::simulation::Simulation>,
    effects: Option<Res<crate::effects::Effects>>,
    flashlight: Res<crate::flashlight::HostFlashlight>,
    mut draws: Query<(
        &mut ModelLit,
        &MeshMaterial3d<SourceMaterial>,
        &InheritedVisibility,
    )>,
    mut materials: ResMut<Assets<SourceMaterial>>,
    performance: Option<Res<crate::performance::Performance>>,
) {
    let _timing = crate::performance::scope(performance.as_deref(), "model lighting");
    let Some(data) = game.world.lighting.as_deref() else {
        return;
    };
    let elights = effects
        .as_deref()
        .map_or_else(Vec::new, |e| e.elights(data.dynamic_minlight));
    let mut states =
        BTreeMap::<Option<usize>, (glam::Vec3, modkit_core::lighting::LightState)>::new();
    let (yaw, pitch) = sim.view_angles();
    let view = viewmodel_basis(sim.view_origin(), yaw, pitch);
    let beam = crate::flashlight::ViewmodelLight::from_light(flashlight.light());
    for (mut lit, material, visible) in &mut draws {
        if !visible.get() && lit.last.is_some() {
            continue;
        }
        let illumination = game
            .world
            .illumination
            .get(&lit.key)
            .copied()
            .unwrap_or_default();
        let origin = match lit.entity {
            Some(id) => {
                let Some(state) = game.scene.states.get(id) else {
                    continue;
                };
                let (origin, rotation) = sim
                    .physics
                    .entity_pose(id)
                    .unwrap_or((state.origin, state.rotation));
                origin + rotation * (illumination.position * lit.scale)
            }
            None => glam::Vec3::from_array(sim.eye().to_array()),
        };
        let lit_dynamic = elights
            .iter()
            .any(|l| modkit_core::lighting::dynamic_reaches(origin, l));
        if lit.entity.is_some()
            && !lit_dynamic
            && !lit.dynamic
            && lit.last.is_some_and(|(last, hl)| {
                last.distance_squared(origin) < 1. && hl == lit.half_lambert
            })
        {
            continue;
        }
        lit.dynamic = lit_dynamic;
        let (_, state) = states.entry(lit.entity).or_insert_with(|| {
            let mut state = data.state_at(origin);
            if illumination.ambient_boost() {
                data.boost(&mut state, origin);
            }
            for light in &elights {
                data.add_dynamic(&mut state, origin, light);
            }
            (origin, state)
        });
        let mut uniform = ModelLighting::from_state(state, lit.half_lambert);
        if lit.entity.is_none() {
            uniform.basis = view;
            uniform.params.w = 2.;
        }
        if let Some(mut material) = materials.get_mut(&material.0) {
            if material.lighting != uniform {
                material.lighting = uniform;
            }
            if lit.entity.is_none() && material.viewmodel_flashlight != beam {
                material.viewmodel_flashlight = beam;
            }
        }
        lit.last = Some((origin, lit.half_lambert));
    }
}
/// The fixed +X viewmodel draw space mapped onto the player's world view.
fn viewmodel_basis(origin: Vec3, yaw: f32, pitch: f32) -> Mat4 {
    Transform::from_translation(source_to_bevy(origin))
        .looking_to(source_to_bevy(crate::source_direction(yaw, pitch)), Vec3::Y)
        .to_matrix()
        * Transform::IDENTITY
            .looking_to(Vec3::X, Vec3::Y)
            .to_matrix()
            .inverse()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weapon_draw_positions_and_normals_follow_the_world_view_without_framing_warp() {
        for (origin, yaw, pitch) in [
            (Vec3::ZERO, 0., 0.),
            (Vec3::new(-4040., -2460., 136.), 1.2, -0.3),
        ] {
            let basis = viewmodel_basis(origin, yaw, pitch);
            let forward = source_to_bevy(crate::source_direction(yaw, pitch));
            let point = basis.transform_point3(Vec3::X * 30.);
            assert!(point.distance(source_to_bevy(origin) + forward * 30.) < 0.001);
            let normal = basis.transform_vector3(Vec3::NEG_X);
            assert!(normal.distance(-forward) < 1e-5);
            assert!((normal.length() - 1.).abs() < 1e-5);
        }
    }
}
