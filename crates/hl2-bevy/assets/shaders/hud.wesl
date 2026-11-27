import bevy_sprite_render::mesh2d::vertex_output::VertexOutput;
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var hud_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var hud_sampler: sampler;
fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    return select(pow((color + vec3(0.055)) / 1.055, vec3(2.4)), color / 12.92, color <= vec3(0.04045));
}
@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let color = mesh.color * textureSample(hud_texture, hud_sampler, mesh.uv);
    // Resource/font bytes use display RGB. The Bevy output attachment is sRGB.
    return vec4(srgb_to_linear(color.rgb), color.a);
}
