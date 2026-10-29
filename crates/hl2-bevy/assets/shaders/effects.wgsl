#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::view
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var effect_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var effect_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> parameters: vec4<f32>;
fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    return select(pow((color + vec3(0.055)) / 1.055, vec3(2.4)), color / 12.92, color <= vec3(0.04045));
}
@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    var color = textureSample(effect_texture, effect_sampler, mesh.uv);
    color.a = select(color.a, 1.0, parameters.y > 0.5);
    // DecalModulate (w = 2): alpha test GREATER 0, mod2x of the frame (2 x texture, blended
    // DST_COLOR x SRC) with TONEMAP_SCALE_NONE. The lit frame is the multiplicand, so neither
    // the vertex light (studio decals carry one for lit decals) nor exposure applies.
    if parameters.w > 1.5 {
        if color.a < parameters.x { discard; }
        return vec4(srgb_to_linear(color.rgb * parameters.z), 1.0);
    }
    // Lit decals (w = 1, DecalBaseTimesLightmapAlphaBlend): texture x the linear light
    // of the surface beneath (vertex colour) x exposure, as the world is lit.
    if parameters.w > 0.5 {
        if color.a < parameters.x { discard; }
        return vec4(srgb_to_linear(color.rgb) * mesh.color.rgb * view.exposure, color.a);
    }
    color *= mesh.color;
    if color.a < parameters.x { discard; }
    // Source sprite shaders end in FinalOutput TONEMAP_SCALE_LINEAR (camera exposure).
    return vec4(srgb_to_linear(color.rgb * parameters.z) * view.exposure, color.a);
}
