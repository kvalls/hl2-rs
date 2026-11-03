// A projected shadow on the world (Source SDK shadow_ps2x.fxc): five coverage taps one texel
// apart averaged, less the vertex fade, blend the surface toward the shadow colour; the pipeline
// multiplies the framebuffer by the result.
#import bevy_pbr::forward_io::VertexOutput

struct ShadowParameters {
    color: vec4<f32>,
    jitter: vec4<f32>,
    channel: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var shadow_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var shadow_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> parameters: ShadowParameters;

fn coverage_at(uv: vec2<f32>) -> f32 {
    return dot(textureSample(shadow_texture, shadow_sampler, uv), parameters.channel);
}

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let a = parameters.jitter.xy;
    let b = vec2(a.x, -a.y);
    let uv = mesh.uv;
    var coverage = (coverage_at(uv) + coverage_at(uv + a) + coverage_at(uv - a) + coverage_at(uv + b)
        + coverage_at(uv - b)) * 0.2;
#ifdef VERTEX_COLORS
    coverage = saturate(coverage - mesh.color.a);
#endif
    return vec4(vec3(1.0) + coverage * (parameters.color.rgb - vec3(1.0)), 1.0);
}
