//! Flashlight depth-map settings; only the High/on light enables the shadow pass.
use bevy::{color::Color, light::SpotLight};
use hl2_simulation::flashlight::Light;

pub(super) const RESOLUTION: usize = 1024;

pub(super) fn enabled(light: Option<Light>, high: bool) -> bool {
    high && light.is_some_and(|light| light.attenuation[1] > 0.)
}

pub(super) fn spot(light: Light, shadows: bool) -> SpotLight {
    let angle = light.fov.to_radians() * 0.5;
    SpotLight {
        color: Color::WHITE,
        intensity: 1.,
        range: light.far,
        radius: light.attenuation[1],
        shadow_maps_enabled: shadows,
        // The Source shader applies its bias in reverse depth; no second world-space bias.
        shadow_depth_bias: 0.,
        shadow_normal_bias: 0.,
        shadow_map_near_z: light.near,
        inner_angle: angle,
        outer_angle: angle,
        ..SpotLight::default()
    }
}

#[cfg(test)]
mod tests {
    use super::spot;
    use bevy::math::{Mat4, Vec4};
    use hl2_simulation::flashlight::Light;

    #[test]
    fn depth_projection_keeps_the_cookie_extent_when_the_beam_flickers_or_pulls_back() {
        for fov in [45_f32, 29.] {
            for near in [4_f32, 44.] {
                let light = Light {
                    origin: glam::Vec3::ZERO,
                    direction: glam::Vec3::X,
                    right: glam::Vec3::NEG_Y,
                    up: glam::Vec3::Z,
                    fov,
                    near,
                    far: 750.,
                    attenuation: [0., 100., 0.],
                };
                for shadows in [false, true] {
                    let spot = spot(light, shadows);
                    let projection = Mat4::perspective_infinite_reverse_rh(
                        2. * spot.outer_angle,
                        1.,
                        spot.shadow_map_near_z,
                    );
                    let extent = 128. * (light.fov.to_radians() * 0.5).tan();
                    let clip = projection * Vec4::new(extent, extent, -128., 1.);
                    let ndc = clip / clip.w;
                    assert!((ndc.x - 1.).abs() < 1e-5 && (ndc.y - 1.).abs() < 1e-5);
                    assert!((ndc.z - near / 128.).abs() < 1e-5);
                    assert_eq!(spot.shadow_maps_enabled, shadows);
                    assert_eq!(spot.radius, light.attenuation[1]);
                }
            }
        }
    }
    #[test]
    fn dark_battery_flicker_skips_the_depth_pass_without_changing_the_beam() {
        let view = (
            glam::Vec3::ZERO,
            glam::Vec3::X,
            glam::Vec3::NEG_Y,
            glam::Vec3::Z,
        );
        let mut placer = hl2_simulation::flashlight::Placer::default();
        let dark = placer.place(view, &|_, _| 1., (1., 0.));
        assert_eq!(dark.attenuation[1], 0.);
        assert!(!super::enabled(Some(dark), true));
        let lit = placer.place(view, &|_, _| 1., (100., 1.));
        assert!(super::enabled(Some(lit), true));
        assert!(!super::enabled(Some(lit), false));
        assert!(!super::enabled(None, true));
    }

    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn installed_station_has_cutouts_with_transparent_and_opaque_texels() {
        let root = source_assets::install::discover().unwrap();
        let loaded = crate::assets::load(&root, "hl2", "d1_trainstation_02").unwrap();
        let eye = glam::Vec3::new(-4304., -224., 64.);
        let mut cutouts = Vec::new();
        for surface in &loaded.world.surfaces {
            let Some(material) = loaded.materials.get(&surface.material) else {
                continue;
            };
            let Some(cutoff) = material.alpha_cutoff else {
                continue;
            };
            let Some(base) = &material.base else {
                continue;
            };
            let alpha = || {
                base.rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|p| f32::from(p[3]) / 255.)
            };
            if !alpha().any(|a| a < cutoff) || !alpha().any(|a| a >= cutoff) {
                continue;
            }
            let (mut lo, mut hi) = (
                glam::Vec3::splat(f32::INFINITY),
                glam::Vec3::splat(f32::NEG_INFINITY),
            );
            for vertex in &surface.vertices {
                lo = lo.min(vertex.position);
                hi = hi.max(vertex.position);
            }
            if !lo.is_finite() || !hi.is_finite() {
                continue;
            }
            let distance = (eye - eye.clamp(lo, hi)).length();
            cutouts.push((distance, &surface.material, lo, hi));
        }
        let mut nearest = Vec::new();
        for surface in loaded
            .world
            .surfaces
            .iter()
            .filter(|s| s.material.to_ascii_lowercase().contains("fence_alpha"))
        {
            for triangle in surface.indices.as_chunks::<3>().0 {
                let points: [glam::Vec3; 3] =
                    std::array::from_fn(|i| surface.vertices[triangle[i] as usize].position);
                let centre = (points[0] + points[1] + points[2]) / 3.;
                nearest.push(((centre - eye).length(), &surface.material, points));
            }
        }
        nearest.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (distance, material, points) in nearest.iter().take(6) {
            eprintln!("nearest fence triangle {material}: {distance:.1}, {points:?}");
        }
        cutouts.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert!(
            !cutouts.is_empty(),
            "a real cutout is available for the packaged depth test"
        );
        for (distance, material, lo, hi) in cutouts.iter().take(12) {
            eprintln!("cutout {material}: distance {distance:.1}, bounds {lo:?} .. {hi:?}");
        }
    }
}
