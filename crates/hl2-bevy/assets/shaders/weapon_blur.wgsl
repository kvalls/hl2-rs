// Advanced-only silhouette reconstruction. Reductions choose dominant motion; the gather
// integrates weapon coverage over the preserved world rather than clamping to its outline.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

#ifdef WORLD_IMAGE
@group(0) @binding(0) var full_frame: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;
@group(0) @binding(2) var blurred: texture_2d<f32>;
@fragment
fn world_downsample(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let texel = 1.0/vec2<f32>(textureDimensions(full_frame));
    var sum = vec4(0.0);
    for (var y = -1; y <= 1; y += 2) { for (var x = -1; x <= 1; x += 2) {
        sum += textureSampleLevel(full_frame,image_sampler,in.uv+vec2<f32>(f32(x),f32(y))*texel,0.0);
    }}
    return sum*0.25;
}

#else

struct Data { params: vec4<f32>, reprojection: mat4x4<f32>, flags: vec4<f32> }
fn camera_motion(uv: vec2<f32>, z: f32, data: Data) -> vec2<f32> {
    let clip = vec4(uv * vec2(2.0,-2.0) + vec2(-1.0,1.0),z,1.0);
    let previous = data.reprojection * clip;
    if abs(previous.w) < 1e-8 { return vec2(0.0); }
    // Bound extreme reveal/cut projections before storing them in a half-float cache.
    return clamp((clip.xy - previous.xy / previous.w) * vec2(0.5,-0.5),vec2(-32.0),vec2(32.0));
}
#ifdef REDUCE_RAW
#ifdef MULTISAMPLED
@group(0) @binding(0) var raw: texture_multisampled_2d<f32>;
@group(0) @binding(1) var raw_depth: texture_depth_multisampled_2d;
#else
@group(0) @binding(0) var raw: texture_2d<f32>;
@group(0) @binding(1) var raw_depth: texture_depth_2d;
#endif
@group(0) @binding(2) var<uniform> data: Data;
// Our Source/effect prepasses export object-only UV motion. Camera motion is
// reconstructed once separately; static geometry needs no depth or matrix reads here.
fn object_motion(v: vec2<f32>, p: vec2<i32>, sample: i32) -> vec2<f32> {
    return select(vec2(0.0),v,dot(v*data.params.zw,v*data.params.zw) >= 0.25);
}
fn raw_motion(p: vec2<i32>) -> vec2<f32> {
#ifdef MULTISAMPLED
    var best = vec2(0.0);
    for (var i = 0u; i < textureNumSamples(raw); i++) {
#ifdef WORLD_RECONSTRUCTION
        let v = object_motion(textureLoad(raw,p,i32(i)).xy,p,i32(i));
#else
        let v = textureLoad(raw,p,i32(i)).xy;
#endif
        if dot(v,v) > dot(best,best) { best = v; }
    }
    return best;
#else
#ifdef WORLD_RECONSTRUCTION
    return object_motion(textureLoad(raw,p,0).xy,p,0);
#else
    return textureLoad(raw,p,0).xy;
#endif
#endif
}
@fragment
fn reduce_raw(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let start = vec2<i32>(in.position.xy) * 4;
    let size = vec2<i32>(textureDimensions(raw));
    var best = vec2(0.0);
    for (var y = 0; y < 4; y++) { for (var x = 0; x < 4; x++) {
        let p = start + vec2(x,y);
        if any(p >= size) { continue; }
        let v = raw_motion(p);
        if dot(v,v) > dot(best,best) { best = v; }
    }}
    return vec4(best, 0.0, 0.0);
}
#else
#ifdef REDUCE_FIELD
@group(0) @binding(0) var field: texture_2d<f32>;
@group(0) @binding(1) var<uniform> data: Data;
@fragment
fn reduce_tiles(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let start = vec2<i32>(in.position.xy) * 8;
    let size = vec2<i32>(textureDimensions(field));
    var best = vec2(0.0);
    for (var y = 0; y < 8; y++) { for (var x = 0; x < 8; x++) {
        let p = start + vec2(x,y);
        if any(p >= size) { continue; }
        let v = textureLoad(field,p,0).xy;
        if dot(v,v) > dot(best,best) { best = v; }
    }}
    return vec4(best,0.0,0.0);
}
@fragment
fn dilate(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let center = vec2<i32>(in.position.xy);
    let size = vec2<i32>(textureDimensions(field));
    let radius = i32(ceil(data.params.x * max(data.params.z, data.params.w) * 0.5 / 32.0)) + 1;
    var best = vec2(0.0);
    for (var y = -radius; y <= radius; y++) { for (var x = -radius; x <= radius; x++) {
        let p = clamp(center + vec2(x,y),vec2(0),size-vec2(1));
        let v = textureLoad(field,p,0).xy;
#ifdef WORLD_RECONSTRUCTION
        let exposed = v*data.params.y;
        let displacement = exposed*min(1.0,data.params.x/max(length(exposed),1e-8))*data.params.zw;
        let span = length(displacement);
        if span < 1.0 { continue; }
        let direction = displacement/span;
        let delta = vec2<f32>(center-p)*32.0;
        if abs(dot(delta,direction)) > span*0.5+48.0 || abs(dot(delta,vec2(-direction.y,direction.x))) > 48.0 { continue; }
#endif
        if dot(v,v) > dot(best,best) { best = v; }
    }}
    return vec4(best,0.0,0.0);
}
#else
@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
@group(0) @binding(2) var<uniform> data: Data;
@group(0) @binding(3) var background: texture_2d<f32>;
#ifdef MULTISAMPLED
@group(0) @binding(4) var motion: texture_multisampled_2d<f32>;
@group(0) @binding(5) var depth: texture_depth_multisampled_2d;
#else
@group(0) @binding(4) var motion: texture_2d<f32>;
@group(0) @binding(5) var depth: texture_depth_2d;
#endif
@group(0) @binding(6) var tiles: texture_2d<f32>;
fn weapon(uv: vec2<f32>) -> vec3<f32> {
    let p = clamp(vec2<i32>(uv * data.params.zw),vec2(0),vec2<i32>(data.params.zw)-vec2(1));
#ifdef MULTISAMPLED
    var velocity = vec2(0.0);
    var coverage = 0.0;
    let count = textureNumSamples(motion);
    for (var i = 0u; i < count; i++) {
        if textureLoad(depth,p,i32(i)) > 0.0 {
            velocity += textureLoad(motion,p,i32(i)).xy;
            coverage += 1.0;
        }
    }
    return vec3(velocity / max(coverage,1.0),coverage / f32(count));
#else
    return vec3(textureLoad(motion,p,0).xy, select(0.0,1.0,textureLoad(depth,p,0)>0.0));
#endif
}
#ifndef WORLD_RECONSTRUCTION
@fragment
fn composite(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let base = textureSampleLevel(frame,linear_sampler,in.uv,0.0);
    let tile_size = vec2<i32>(textureDimensions(tiles));
    let tile = clamp(vec2<i32>(in.uv * vec2<f32>(tile_size)),vec2(0),tile_size-vec2(1));
    let dominant = textureLoad(tiles,tile,0).xy;
    // Maximum UV length bounds every owned sample; max dimension keeps this
    // conservative on wide/tall frames. Empty regions need no MSAA depth reads.
    if data.params.x == 0.0 || length(dominant)*max(data.params.z,data.params.w)*data.params.y < 1.0 { return base; }
    let own = weapon(in.uv);
    // A covered pixel follows its own surface, even when a nearby part moves faster.
    let velocity = select(dominant,own.xy,own.z > 0.0) * data.params.y;
    let pixel_motion = length(velocity * data.params.zw);
    if pixel_motion < 1.0 || data.params.x == 0.0 { return base; }
    let vector = velocity * min(1.0,data.params.x / max(length(velocity),0.00001));
    // Odd tap counts always include the current pixel: thin/slow surfaces cannot
    // lose all coverage merely because even taps straddle their narrow footprint.
    let count = i32(clamp(ceil(length(vector * data.params.zw)),16.0,96.0)) | 1;
    let world = textureSampleLevel(background,linear_sampler,in.uv,0.0);
    var sum = vec3(0.0);
    var coverage = 0.0;
    var valid_taps = 0.0;
    for (var i = 0; i < count; i++) {
        let offset = vector * ((f32(i)+0.5)/f32(count)-0.5);
        let uv = in.uv + offset;
        if any(uv < vec2(0.0)) || any(uv > vec2(1.0)) { continue; }
        valid_taps += 1.0;
        let sample = weapon(uv);
        let exposure = sample.xy * data.params.y;
        let distance = length(offset * data.params.zw);
        let reach = length(exposure * data.params.zw) * 0.5;
        if sample.z == 0.0 || reach < distance { continue; }
        // Reject motion perpendicular to this gather ray, including unrelated hand detail.
        let alignment = abs(dot(normalize(exposure+vec2(1e-12)),normalize(vector)));
        let weight = sample.z * smoothstep(0.5,0.9,alignment);
        sum += textureSampleLevel(frame,linear_sampler,uv,0.0).rgb * weight;
        coverage += weight;
    }
    // Offscreen taps are unknown, not transparent background. Normalize both color
    // and coverage by available samples so clipped guns do not fade at screen edges.
    let normalization = max(valid_taps,1.0);
    let alpha = min(coverage/normalization,1.0);
    if alpha == 0.0 { return select(world,base,own.z < 0.5); }
    let reconstructed = vec4(sum/normalization + world.rgb * (1.0-alpha),base.a);
    return mix(base, reconstructed, smoothstep(1.0,3.0,pixel_motion));
}
#else
@group(0) @binding(7) var camera_filtered: texture_2d<f32>;
// Read all surfaces, including sky/background motion. Reversed depth is larger nearer us.
fn raw_surface(uv: vec2<f32>) -> vec4<f32> {
    let p = clamp(vec2<i32>(uv*data.params.zw),vec2(0),vec2<i32>(data.params.zw)-1);
#ifdef MULTISAMPLED
    var velocity = vec2(0.0);
    var z = 0.0;
    let n = textureNumSamples(motion);
    for (var i = 0u; i < n; i++) {
        let sample_z = textureLoad(depth,p,i32(i));
        if sample_z > 0.0 {
            velocity += textureLoad(motion,p,i32(i)).xy;
        }
        z = max(z,sample_z);
    }
    let object = velocity/f32(n);
    var camera = vec2(0.0);
    if data.flags.x > 0.0 { camera = camera_motion(uv,z,data); }
    return vec4(object+camera,z,min(length(object*data.params.zw),65504.0));
#else
    let z = textureLoad(depth,p,0);
    var object = textureLoad(motion,p,0).xy;
    if z == 0.0 { object = vec2(0.0); }
    var camera = vec2(0.0);
    if data.flags.x > 0.0 { camera = camera_motion(uv,z,data); }
    return vec4(object+camera,z,min(length(object*data.params.zw),65504.0));
#endif
}
fn surface(uv: vec2<f32>) -> vec4<f32> {
#ifdef WORLD_OBJECTS
    let p = clamp(vec2<i32>(uv*data.params.zw),vec2(0),vec2<i32>(data.params.zw)-1);
#ifdef MULTISAMPLED
    var object = vec2(0.0);
    for (var i = 0u; i < textureNumSamples(motion); i++) {
        object += textureLoad(motion,p,i32(i)).xy;
    }
    object /= f32(textureNumSamples(motion));
#else
    let object = textureLoad(motion,p,0).xy;
#endif
    let pixels = length(object*data.params.zw);
    var z = 0.0;
    if pixels >= 1.0 {
        z = background_depth(uv);
        if z == 0.0 { return vec4(0.0); }
    }
    return vec4(object,z,pixels);
#else
    return raw_surface(uv);
#endif
}
fn dominant_motion(uv: vec2<f32>) -> vec2<f32> {
    let size = vec2<i32>(textureDimensions(tiles));
    let tile = clamp(vec2<i32>(uv*vec2<f32>(size)),vec2(0),size-1);
    return textureLoad(tiles,tile,0).xy;
}
fn background_depth(uv: vec2<f32>) -> f32 {
    let p = clamp(vec2<i32>(uv*data.params.zw),vec2(0),vec2<i32>(data.params.zw)-1);
#ifdef MULTISAMPLED
    var z = 0.0;
    for (var i = 0u; i < textureNumSamples(motion); i++) { z = max(z,textureLoad(depth,p,i32(i))); }
    return z;
#else
    return textureLoad(depth,p,0);
#endif
}

struct Destination { own: vec4<f32>, camera: vec2<f32>, relative: vec2<f32> }
fn destination(uv: vec2<f32>) -> Destination {
    var own = surface(uv);
#ifdef WORLD_OBJECTS
    if own.z == 0.0 { own.z = background_depth(uv); }
#endif
    var camera = vec2(0.0);
    if data.flags.x > 0.0 { camera = camera_motion(uv,own.z,data); }
#ifdef WORLD_OBJECTS
    let relative = own.xy;
    own = vec4(own.xy+camera,own.zw);
#else
    let relative = own.xy-camera;
#endif
    return Destination(own,camera,relative);
}
fn limited(v: vec2<f32>) -> vec2<f32> {
    let exposed = v * data.params.y;
    return exposed * min(1.0,data.params.x/max(length(exposed),1e-8));
}
fn taps(vector: vec2<f32>) -> i32 {
    // Subpixel spacing for thin bright lights; odd counts retain the exact center.
    return i32(clamp(ceil(length(vector*data.params.zw)*1.5),2.0,256.0)) | 1;
}
fn camera_gather(uv: vec2<f32>, vector: vec2<f32>) -> vec3<f32> {
    let count = i32(clamp(ceil(length(vector*vec2<f32>(textureDimensions(background)))*1.5),16.0,96.0)) | 1;
    var sum = vec3(0.0);
    for (var i = 0; i < count; i++) {
        let offset = vector * (f32(i)/f32(count-1)-0.5);
        sum += textureSampleLevel(background,linear_sampler,clamp(uv+offset,vec2(0.0),vec2(1.0)),0.0).rgb;
    }
    return sum/f32(count);
}
@fragment
fn composite(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let dominant = dominant_motion(in.uv);
    let original = textureSampleLevel(frame,linear_sampler,in.uv,0.0);
    var base = original;
#ifdef WORLD_OBJECTS
    if data.flags.x > 0.0 {
        let filtered = textureSampleLevel(camera_filtered,linear_sampler,in.uv,0.0);
        base = vec4(filtered.rgb+original.rgb*(1.0-filtered.a),original.a);
    }
    let object_span = length(dominant*data.params.zw)*data.params.y;
    if object_span < data.params.y { return base; }
    if data.flags.x > 0.0 {
        let texel = max(data.params.z/f32(textureDimensions(camera_filtered).x),data.params.w/f32(textureDimensions(camera_filtered).y));
        let camera_span = length(camera_motion(in.uv,0.0,data)*data.params.zw)*data.params.y;
        // Do not repeat a long camera gather for sub-filter-texel object motion.
        // Its contribution is below the camera filter's spatial resolution.
        if object_span <= texel && camera_span >= max(texel*4.0,object_span*8.0) { return base; }
    }
#endif
    if data.params.x == 0.0 { return vec4(0.0); }
    let center = destination(in.uv);
    let own = center.own;
    let camera = center.camera;
    let relative = center.relative;

    let camera_vector = limited(camera);
    let camera_pixels = length(camera_vector*data.params.zw);
    let object_pixels = length(dominant*data.params.zw);
#ifdef WORLD_OBJECTS
    if object_pixels < 1.0 {
#else
    if true {
#endif
        if camera_pixels < 1.0 { return vec4(0.0); }
        let strength = smoothstep(1.0,3.0,camera_pixels);
        return vec4(camera_gather(in.uv,camera_vector)*strength,strength);
    }
    let moving = own.w >= 1.0;
    let vector = limited(select(camera+dominant,own.xy,moving));
    let pixels = length(vector*data.params.zw);
    if pixels < 1.0 { return base; }
    let count = taps(vector);
    var foreground = vec3(0.0);
    var coverage = 0.0;
    var behind = vec3(0.0);
    var background_weight = 0.0;
    for (var i = 0; i < count; i++) {
        let offset = vector * (f32(i)/f32(count-1)-0.5);
        let uv = in.uv+offset;
        if any(uv < vec2(0.0)) || any(uv > vec2(1.0)) { continue; }
        let sample = surface(uv);
        if sample.w < 1.0 {
            if !moving { continue; }
            var z = sample.z;
            if z == 0.0 { z = background_depth(uv); }
            if z <= own.z*1.01 {
                behind += textureSampleLevel(frame,linear_sampler,uv,0.0).rgb;
                background_weight += 1.0;
            }
            continue;
        }
        // A foreground wall must not receive streaks from an object behind it.
        if sample.z < own.z*0.99 && !moving { continue; }
        var sample_motion = sample.xy;
#ifdef WORLD_OBJECTS
        if data.flags.x > 0.0 { sample_motion += camera_motion(uv,sample.z,data); }
#endif
        let exposure = limited(sample_motion);
        if length(offset*data.params.zw) > length(exposure*data.params.zw)*0.5+0.5 { continue; }
        let alignment = abs(dot(normalize(exposure+vec2(1e-12)),normalize(vector)));
        let weight = smoothstep(0.5,0.9,alignment);
        if weight == 0.0 { continue; }
        foreground += textureSampleLevel(frame,linear_sampler,uv,0.0).rgb*weight;
        coverage += weight;
    }
    var backdrop = base.rgb;
    if moving && background_weight > 0.0 { backdrop = behind/background_weight; }
    if !moving && camera_pixels >= 1.0 { backdrop = base.rgb; }
    let alpha = min(coverage/f32(count),1.0);
    let reconstructed = foreground/f32(count)+backdrop*(1.0-alpha);
    let strength = smoothstep(1.0,3.0,pixels)*select(0.0,1.0,moving || alpha > 0.0 || camera_pixels >= 1.0);
#ifdef WORLD_OBJECTS
    return vec4(mix(base.rgb,reconstructed,strength),base.a);
#else
    return vec4(reconstructed*strength,strength);
#endif
}

#endif
#endif
#endif

#endif
