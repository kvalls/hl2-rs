//! The placed world beam for the separate origin-based viewmodel camera.
use bevy::{
    math::{Mat4, Vec3, Vec4},
    render::render_resource::ShaderType,
};
use hl2_simulation::flashlight::Light;

#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub(crate) struct ViewmodelLight {
    /// World position to beam coordinates: xy divided by tan(FOV/2), z forward.
    pub world_to_beam: Mat4,
    /// World origin, linear attenuation.
    pub origin_linear: Vec4,
    /// Near, far, enabled, unused.
    pub limits: Vec4,
}

impl ViewmodelLight {
    pub(crate) fn from_light(light: Option<Light>) -> Self {
        let Some(light) = light.filter(|light| light.attenuation[1] > 0.) else {
            return Self::default();
        };
        let bevy = |v: glam::Vec3| crate::source_to_bevy(Vec3::from_array(v.to_array()));
        let origin = bevy(light.origin);
        let tan = (light.fov.to_radians() * 0.5).tan();
        let rows = [
            bevy(light.right) / tan,
            bevy(light.up) / tan,
            bevy(light.direction),
        ];
        Self {
            world_to_beam: Mat4::from_cols(
                rows[0].extend(-rows[0].dot(origin)),
                rows[1].extend(-rows[1].dot(origin)),
                rows[2].extend(-rows[2].dot(origin)),
                Vec4::W,
            )
            .transpose(),
            origin_linear: origin.extend(light.attenuation[1]),
            limits: Vec4::new(light.near, light.far, 1., 0.),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translated_rotated_beam_keeps_cookie_coordinates_and_power_planes() {
        for (origin, direction) in [
            (glam::Vec3::ZERO, glam::Vec3::X),
            (
                glam::Vec3::new(-4040., -2460., 136.),
                glam::Vec3::new(1., 2., -0.4).normalize(),
            ),
        ] {
            let right = direction.cross(glam::Vec3::Z).normalize();
            let up = right.cross(direction);
            for fov in [45_f32, 29.] {
                let light = Light {
                    origin,
                    direction,
                    right,
                    up,
                    fov,
                    near: 44.,
                    far: 750.,
                    attenuation: [0., 37., 0.],
                };
                let beam = ViewmodelLight::from_light(Some(light));
                let extent = 128. * (fov.to_radians() * 0.5).tan();
                for (x, y) in [(0., 0.), (1., 1.), (-1., 0.5)] {
                    let point = origin + direction * 128. + right * extent * x + up * extent * y;
                    let world = crate::source_to_bevy(Vec3::from_array(point.to_array()));
                    let projected = beam.world_to_beam * world.extend(1.);
                    assert!((projected.x / projected.z - x).abs() < 1e-4);
                    assert!((projected.y / projected.z - y).abs() < 1e-4);
                    assert!((projected.z - 128.).abs() < 0.002);
                }
                assert_eq!(beam.limits, Vec4::new(44., 750., 1., 0.));
                assert_eq!(beam.origin_linear.w, 37.);
                let dark = Light {
                    attenuation: [0.; 3],
                    ..light
                };
                assert_eq!(
                    ViewmodelLight::from_light(Some(dark)),
                    ViewmodelLight::default()
                );
            }
        }
        assert_eq!(ViewmodelLight::from_light(None), ViewmodelLight::default());
    }
    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn installed_obstructions_provide_a_clear_camera_with_weapon_inside_the_beam() {
        let root = source_assets::install::discover().unwrap();
        let loaded = crate::assets::load(&root, "hl2", "d1_trainstation_02").unwrap();
        let physics = hl2_simulation::physics::Physics::new(&loaded.world);
        let eye = glam::Vec3::new(-4304., -224., 64.);
        let trace = |from: glam::Vec3, to: glam::Vec3| {
            let delta = to - from;
            let distance = delta.length();
            if distance < 1e-3 {
                return 1.;
            }
            physics
                .impact_hull(from, delta / distance, distance, glam::Vec3::splat(4.))
                .map_or(1., |hit| (hit.position - from).length() / distance)
        };
        let mut accepted = 0;
        for forward in [
            glam::Vec3::X,
            glam::Vec3::NEG_X,
            glam::Vec3::Y,
            glam::Vec3::NEG_Y,
        ] {
            let Some(hit) = physics.impact_hull(
                eye - glam::Vec3::Z * 20.,
                forward,
                750.,
                glam::Vec3::splat(4.),
            ) else {
                continue;
            };
            for back in [24., 48., 72., 96.] {
                let camera = hit.position - forward * back + glam::Vec3::Z * 20.;
                // A tiny sweep rejects a camera already embedded in a brush or prop.
                if trace(camera, camera + glam::Vec3::Z * 0.1) < 1. {
                    continue;
                }
                let right = forward.cross(glam::Vec3::Z);
                let mut placer = hl2_simulation::flashlight::Placer::default();
                let mut light =
                    placer.place((camera, forward, right, glam::Vec3::Z), &trace, (100., 0.));
                for i in 1..60 {
                    light = placer.place(
                        (camera, forward, right, glam::Vec3::Z),
                        &trace,
                        (100., i as f32 * 0.015),
                    );
                }
                let point = camera + forward * 30. + right * 5. - glam::Vec3::Z * 10.;
                let beam = ViewmodelLight::from_light(Some(light));
                let projected = beam.world_to_beam
                    * crate::source_to_bevy(Vec3::from_array(point.to_array())).extend(1.);
                if projected.z <= light.near
                    || projected.x.abs().max(projected.y.abs()) > projected.z
                {
                    continue;
                }
                accepted += 1;
                eprintln!("clear camera {camera:?}, forward {forward:?}; beam {light:?}");
            }
        }
        assert!(
            accepted > 0,
            "an obstruction can place the weapon inside the real beam"
        );
    }
}
