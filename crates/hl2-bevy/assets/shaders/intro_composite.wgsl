// The Half-Life 2 intro composite (SDK materialsystem/stdshaders/IntroScreenSpaceEffect_ps2x.fxc,
// drawn by CViewRender::ViewDrawScene_Intro): up to two passes, each one blend mode over the
// camera entity's view ("scene") and the ordinary view ("gman"), added over black with
// SRC_ALPHA, ONE. The engine reads and writes its 8-bit frame buffers without sRGB conversion,
// so the modes work on gamma-space values; ours are linear and are converted around them.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var view_texture: texture_2d<f32>;
@group(0) @binding(1) var scene_texture: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;
struct Intro {
    // (mode, alpha) of the first and second pass.
    passes: vec4<f32>,
    // x: 1 when the camera entity's view exists (black otherwise).
    flags: vec4<f32>,
}
@group(0) @binding(3) var<uniform> intro: Intro;

fn to_gamma(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(max(c, vec3(0.0)), vec3(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((max(c, vec3(0.0)) + 0.055) / 1.055, vec3(2.4));
    return select(high, low, c <= vec3(0.04045));
}

fn average(c: vec3<f32>) -> f32 {
    return dot(c, vec3(1.0 / 3.0));
}

fn rgb_to_hsv(rgb: vec3<f32>) -> vec3<f32> {
    let low = min(min(rgb.r, rgb.g), rgb.b);
    let high = max(max(rgb.r, rgb.g), rgb.b);
    let delta = high - low;
    if delta == 0.0 {
        return vec3(-1.0, 0.0, high);
    }
    var h: f32;
    if rgb.r == high {
        h = (rgb.g - rgb.b) / delta;
    } else if rgb.g == high {
        h = 2.0 + (rgb.b - rgb.r) / delta;
    } else {
        h = 4.0 + (rgb.r - rgb.g) / delta;
    }
    h = h * 60.0;
    if h < 0.0 {
        h = h + 360.0;
    }
    return vec3(h, delta / high, high);
}

fn hsv_to_rgb(hsv: vec3<f32>) -> vec3<f32> {
    let s = hsv.y;
    let v = hsv.z;
    if s == 0.0 {
        return vec3(v);
    }
    let h = hsv.x / 60.0;
    let i = floor(h);
    let f = h - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    if h < 1.0 {
        return vec3(v, t, p);
    } else if h < 2.0 {
        return vec3(q, v, p);
    } else if h < 3.0 {
        return vec3(p, v, t);
    } else if h < 4.0 {
        return vec3(p, q, v);
    } else if h < 5.0 {
        return vec3(t, p, v);
    }
    return vec3(v, p, q);
}

fn blend(mode: i32, scene: vec3<f32>, gman: vec3<f32>) -> vec3<f32> {
    let gman_lum = average(gman);
    switch mode {
        case 0: {
            // Negative greyscale of the scene times gman.
            return (vec3(1.0) - vec3(average(scene))) * gman;
        }
        case 1: {
            if gman_lum < 0.3 {
                return vec3(1.0) - gman;
            }
            return (vec3(1.0) - gman) * vec3(average(scene));
        }
        case 2: {
            return min(vec3(gman_lum), scene);
        }
        case 3: {
            var b: f32;
            if gman_lum < 0.4 {
                b = gman_lum / 0.4;
            } else if gman_lum > 0.7 {
                b = 1.0 - (gman_lum - 0.7) / 0.3;
            } else {
                b = 1.0;
            }
            b = clamp(b, 0.0, 1.0);
            return vec3(gman_lum) * (1.0 - b) + scene * b;
        }
        case 5: {
            if scene.r > 0.0 {
                return scene;
            }
            var hsv = rgb_to_hsv(gman);
            let b = hsv.z - 0.5;
            hsv.z = hsv.z * (1.0 + b);
            hsv.y = hsv.y * (1.0 - b);
            return hsv_to_rgb(hsv);
        }
        case 6: {
            return scene + gman;
        }
        case 7: {
            return scene;
        }
        case 9: {
            let layer = clamp(scene * scene * 2.0, vec3(0.0), vec3(1.0));
            return layer + gman * clamp(1.0 - clamp(average(scene), 0.0, 1.0) * 2.0, 0.0, 1.0);
        }
        default: {
            // Mode 8 (gman alone); mode 4 is not used by the installed intro and draws as 8.
            return gman;
        }
    }
}

@fragment
fn intro_composite(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let gman = clamp(to_gamma(textureSample(view_texture, linear_sampler, in.uv).rgb), vec3(0.0), vec3(1.0));
    var scene = vec3(0.0);
    if intro.flags.x > 0.5 {
        scene = clamp(to_gamma(textureSample(scene_texture, linear_sampler, in.uv).rgb), vec3(0.0), vec3(1.0));
    }
    var out = vec3(0.0);
    if intro.passes.y > 0.0 {
        out = out + clamp(blend(i32(intro.passes.x), scene, gman), vec3(0.0), vec3(1.0)) * intro.passes.y;
    }
    out = clamp(out, vec3(0.0), vec3(1.0));
    if intro.passes.w > 0.0 {
        out = out + clamp(blend(i32(intro.passes.z), scene, gman), vec3(0.0), vec3(1.0)) * intro.passes.w;
    }
    return vec4(to_linear(clamp(out, vec3(0.0), vec3(1.0))), 1.0);
}
