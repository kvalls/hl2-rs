//! Source model lighting uniforms: ambient cube and up to four local lights per entity.
use crate::source_to_bevy;
use bevy::{
    math::{Mat4, Vec3, Vec4},
    render::render_resource::ShaderType,
};

/// Source model lighting for one entity: ambient cube plus up to four local
/// lights, positions/directions in Bevy space and colors linear.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct ModelLighting {
    /// x = lit model, y = local light count, z = half-Lambert, w = 1 when the flashlight does not light
    /// it (unlit, camera, additive and sky materials); 2 for the viewmodel beam path.
    pub params: Vec4,
    /// Draw space to Bevy world space (Mat4 default is identity), except for the
    /// view model, which is drawn by a fixed camera at the origin.
    pub basis: Mat4,
    /// Source cube faces +X, -X, +Y, -Y, +Z, -Z.
    pub ambient: [Vec4; 6],
    /// w = 0 point, 1 spot, 2 directional.
    pub position: [Vec4; 4],
    pub color: [Vec4; 4],
    pub direction: [Vec4; 4],
    /// Constant, linear, quadratic.
    pub attenuation: [Vec4; 4],
    /// Exponent, outer cosine, 1 / (inner - outer).
    pub spot: [Vec4; 4],
}
impl ModelLighting {
    /// Shared CPU light state converted for the shader.
    pub fn from_state(state: &modkit_core::lighting::LightState, half_lambert: bool) -> Self {
        use modkit_core::lighting::ShaderLightKind;
        let mut out = Self {
            params: Vec4::new(
                1.,
                state.lights.len().min(4) as f32,
                f32::from(half_lambert),
                0.,
            ),
            ..Default::default()
        };
        for (face, color) in out.ambient.iter_mut().zip(&state.ambient) {
            *face = Vec3::from_array(color.to_array()).extend(0.);
        }
        for (i, light) in state.lights.iter().take(4).enumerate() {
            let kind = match light.kind {
                ShaderLightKind::Point => 0.,
                ShaderLightKind::Spot => 1.,
                ShaderLightKind::Directional => 2.,
            };
            out.position[i] =
                source_to_bevy(Vec3::from_array(light.position.to_array())).extend(kind);
            out.color[i] = Vec3::from_array(light.color.to_array()).extend(0.);
            out.direction[i] =
                source_to_bevy(Vec3::from_array(light.direction.to_array())).extend(0.);
            out.attenuation[i] = Vec3::from_array(light.attenuation).extend(0.);
            let scale = if light.inner > light.outer {
                1. / (light.inner - light.outer)
            } else {
                1.
            };
            out.spot[i] = Vec4::new(light.exponent, light.outer, scale, 0.);
        }
        out
    }
    /// Unlit-data fallback: the former constant 180/255 gamma gray.
    pub(crate) fn fallback(half_lambert: bool) -> Self {
        let gray = (180f32 / 255.).powf(2.2);
        Self {
            params: Vec4::new(1., 0., f32::from(half_lambert), 0.),
            ambient: [Vec4::new(gray, gray, gray, 0.); 6],
            ..Default::default()
        }
    }
}
