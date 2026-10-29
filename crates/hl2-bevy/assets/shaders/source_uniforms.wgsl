// Shared single-buffer layout; field order matches SourceMaterial uniform(0).
struct ModelLighting {
    params: vec4<f32>,
    basis: mat4x4<f32>,
    ambient: array<vec4<f32>, 6>,
    position: array<vec4<f32>, 4>,
    color: array<vec4<f32>, 4>,
    direction: array<vec4<f32>, 4>,
    attenuation: array<vec4<f32>, 4>,
    spot: array<vec4<f32>, 4>,
}
struct ViewmodelLight {
    world_to_beam: mat4x4<f32>,
    origin_linear: vec4<f32>,
    limits: vec4<f32>,
}
struct SourceUniforms {
    tint: vec4<f32>,
    parameters: vec4<f32>,
    secondary_uv: mat3x3<f32>,
    lighting: ModelLighting,
    envmap_tint: vec4<f32>,
    envmap_contrast: vec4<f32>,
    envmap_saturation: vec4<f32>,
    self_illum: vec4<f32>,
    viewmodel_flashlight: ViewmodelLight,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> material: SourceUniforms;
