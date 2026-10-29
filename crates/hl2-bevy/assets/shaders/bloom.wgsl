// Source 8-bit bloom (SDK viewpostprocess.cpp Generate8BitBloomTexture): gamma-space shape
// and quarter-size downsample, 13-tap Gaussian blur in X then Y (Y times the bloom amount),
// and an additive composite onto the gamma frame (Engine_Post).
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var bloom: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;
// texel: texel size of the sampled texture (xy) and blur direction (zw); scale in params.x.
struct Pass {
    texel_direction: vec4<f32>,
    params: vec4<f32>,
}
@group(0) @binding(3) var<uniform> pass_data: Pass;

fn encode(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3(0.0031308));
}
fn decode(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + vec3(0.055)) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
}

// One bilinear tap of the SDK downsample: the mean of a 2x2 gamma block, then Shape
// (r_bloomtint 0.3/0.59/0.11, exponent 2.2).
fn shaped_tap(origin: vec2<i32>, size: vec2<i32>) -> vec3<f32> {
    var sum = vec3(0.0);
    for (var y = 0; y < 2; y++) {
        for (var x = 0; x < 2; x++) {
            let p = clamp(origin + vec2(x, y), vec2(0), size - vec2(1));
            sum += encode(textureLoad(source, p, 0).rgb);
        }
    }
    let pixel = sum * 0.25;
    let lum = dot(pixel, vec3(0.3, 0.59, 0.11));
    return pow(pixel, vec3(2.2)) * lum;
}

@fragment
fn shape(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(source));
    let base = vec2<i32>(floor(in.position.xy)) * 4;
    let c = shaped_tap(base, size) + shaped_tap(base + vec2(2, 0), size)
        + shaped_tap(base + vec2(0, 2), size) + shaped_tap(base + vec2(2, 2), size);
    return vec4(c * 0.25, 1.0);
}

@fragment
fn blur(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let step = pass_data.texel_direction.xy * pass_data.texel_direction.zw;
    let offsets = array<f32, 6>(1.3366, 3.4295, 5.4264, 7.4359, 9.4436, 11.4401);
    let weights = array<f32, 6>(0.2185, 0.0821, 0.0461, 0.0262, 0.0162, 0.0102);
    var color = textureSample(source, linear_sampler, in.uv).rgb * 0.2013;
    for (var i = 0; i < 6; i++) {
        let d = step * offsets[i];
        color += (textureSample(source, linear_sampler, in.uv + d).rgb
            + textureSample(source, linear_sampler, in.uv - d).rgb) * weights[i];
    }
    return vec4(color * pass_data.params.x, 1.0);
}

@fragment
fn composite(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let frame = textureLoad(source, vec2<i32>(floor(in.position.xy)), 0);
    let glow = textureSample(bloom, linear_sampler, in.uv).rgb;
    let gamma = min(encode(frame.rgb) + glow * pass_data.params.x, vec3(1.0));
    return vec4(decode(gamma), frame.a);
}

// Pre-bloom frame for the auto-exposure histogram (the SDK measures before bloom), stored
// gamma encoded for 8-bit precision.
@fragment
fn presample(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return vec4(encode(textureSample(source, linear_sampler, in.uv).rgb), 1.0);
}

#ifdef COLOR_CORRECTION
// Color correction (Engine_Post PerformColorCorrection): the gamma-space frame, after the bloom
// add, goes through up to four 32x32x32 lookup volumes weighted with the default weight.
struct Correction {
    weights: vec4<f32>,
    default_weight: vec4<f32>,
}
@group(1) @binding(0) var lookup0: texture_3d<f32>;
@group(1) @binding(1) var lookup1: texture_3d<f32>;
@group(1) @binding(2) var lookup2: texture_3d<f32>;
@group(1) @binding(3) var lookup3: texture_3d<f32>;
@group(1) @binding(4) var<uniform> correction: Correction;

fn look_up(volume: texture_3d<f32>, at: vec3<f32>) -> vec3<f32> {
    return textureSampleLevel(volume, linear_sampler, at, 0.0).rgb;
}

fn correct(color: vec3<f32>) -> vec3<f32> {
    // Texel centres: 0 reads 0.5/32, 1 reads 31.5/32.
    let at = color * (31.0 / 32.0) + vec3(0.5 / 32.0);
    var result = color * correction.default_weight.x;
    result += look_up(lookup0, at) * correction.weights.x;
#if LOOKUPS > 1
    result += look_up(lookup1, at) * correction.weights.y;
#endif
#if LOOKUPS > 2
    result += look_up(lookup2, at) * correction.weights.z;
#endif
#if LOOKUPS > 3
    result += look_up(lookup3, at) * correction.weights.w;
#endif
    return result;
}

@fragment
fn composite_cc(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let frame = textureLoad(source, vec2<i32>(floor(in.position.xy)), 0);
    var gamma = encode(frame.rgb);
#ifdef BLOOM
    gamma += textureSample(bloom, linear_sampler, in.uv).rgb * pass_data.params.x;
#endif
    return vec4(decode(correct(clamp(gamma, vec3(0.0), vec3(1.0)))), frame.a);
}
#endif
