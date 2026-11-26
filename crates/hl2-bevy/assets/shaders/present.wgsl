// Presents the game frame in the window with the monitor gamma ramp (mat_monitorgamma): the
// display-encoded value v becomes v^(gamma / 2.2), so 2.2 is unchanged, lower is lighter.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct PresentParams {
    // x: ramp exponent (gamma / 2.2).
    params: vec4<f32>,
};
@group(0) @binding(0) var frame_texture: texture_2d<f32>;
@group(0) @binding(1) var frame_sampler: sampler;
@group(0) @binding(2) var<uniform> present: PresentParams;

fn encode(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3(0.0031308));
}
fn decode(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + vec3(0.055)) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    // The frame is sRGB: sampling yields linear values, and the window attachment encodes
    // them again. The ramp applies to the display-encoded values in between.
    let frame = textureSample(frame_texture, frame_sampler, in.uv);
    if abs(present.params.x - 1.0) < 1e-4 { return frame; }
    let ramped = pow(clamp(encode(frame.rgb), vec3(0.0), vec3(1.0)), vec3(present.params.x));
    return vec4(decode(ramped), frame.a);
}
