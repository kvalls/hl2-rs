// Source image-space motion blur (materialsystem/stdshaders/motion_blur_ps2x.fxc): the frame is
// averaged along one blur vector per pixel, made of the global (turning) vector, the falling
// vector (a zoom, damped in the middle of the screen) and the roll vector.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
// values: yaw, pitch, falling and roll blur; params.x the longest vector, params.y the samples.
struct Blur {
    values: vec4<f32>,
    params: vec4<f32>,
}
@group(0) @binding(2) var<uniform> blur: Blur;

@fragment
fn motion_blur(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var falling = in.uv * 2.0 - 1.0;
    // cross(float3(falling, 0), float3(0, 0, 1)).xy
    var roll = vec2(falling.y, -falling.x);
    let global = vec2(blur.values.x, -blur.values.y);
    // Keeps the samples on screen by pointing the vector in.
    let falling_intensity = -abs(blur.values.z);
    falling = falling * dot(falling, falling) * falling_intensity;
    roll = roll * blur.values.w;
    var vector = global + falling + roll;
    if length(vector) > blur.params.x {
        vector = normalize(vector) * blur.params.x;
    }
    let count = i32(blur.params.y);
    let step = vector / f32(count - 1);
    var color = vec3(0.0);
    for (var i = 0; i < count; i++) {
        color += textureSampleLevel(source, linear_sampler, in.uv + step * f32(i), 0.0).rgb;
    }
    let alpha = textureSampleLevel(source, linear_sampler, in.uv, 0.0).a;
    return vec4(color / f32(count), alpha);
}
