#import "shaders/source_uniforms.wgsl"::material
#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::{view, clustered_lights, lights, directional_shadow_textures, directional_shadow_textures_comparison_sampler}
#import bevy_render::maths::orthonormalize
#import bevy_pbr::clustered_forward
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var lightmap_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var lightmap_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var iris_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var iris_sampler: sampler;

@group(#{MATERIAL_BIND_GROUP}) @binding(10) var envmap_texture: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(11) var envmap_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(15) var envmap_mask_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(16) var envmap_mask_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(18) var flashlight_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(19) var flashlight_sampler: sampler;
// SDK lightmappedgeneric_ps2_3_x.h CUBEMAP: reflection of the eye vector about the face
// normal, sampled in Source axes. Retail integer HDR stores cubemaps as linear/16 and
// multiplies by ENV_MAP_SCALE 16 (shaderapidx9 c30.z), so texels are capped at 16.
// Tangent frame along the texture's s/t axes from screen-space derivatives (no per-vertex
// tangents): SDK world = mul(vNormal, [tangentS, tangentT, normal]), and VRAD's
// GetBumpNormals orients that frame with s and t, flipping it on left-handed faces.
fn bump_normal(mesh: VertexOutput, n: vec3<f32>, bump: vec3<f32>) -> vec3<f32> {
    let p = mesh.world_position.xyz;
    let dp1 = dpdx(p);
    let dp2 = dpdy(p);
    let duv1 = dpdx(mesh.uv);
    let duv2 = dpdy(mesh.uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let s = dp2perp * duv1.x + dp1perp * duv2.x;
    let t = dp2perp * duv1.y + dp1perp * duv2.y;
    if dot(s, s) < 1e-20 || dot(t, t) < 1e-20 { return n; }
    return normalize(bump.x * normalize(s) + bump.y * normalize(t) + bump.z * n);
}
fn envmap_specular(mesh: VertexOutput, base_alpha: f32) -> vec3<f32> {
    // specularFactor: $normalmapalphaenvmapmask (bump alpha), $envmapmask (RGB) and
    // $basealphaenvmapmask (1 - base alpha). Masks and bump maps use the base UVs.
    let flags = u32(material.envmap_saturation.w + 0.5);
    let mask = textureSample(envmap_mask_texture, envmap_mask_sampler, mesh.uv);
    var factor = vec3(1.0);
    if (flags & 4u) != 0u { factor *= mask.a; }
    if (flags & 2u) != 0u { factor *= mask.rgb; }
    if (flags & 1u) != 0u { factor *= 1.0 - base_alpha; }
    var n = normalize(mesh.world_normal);
    if (flags & 8u) != 0u { n = bump_normal(mesh, n, mask.rgb * 2.0 - 1.0); }
    let to_eye = view.world_position - mesh.world_position.xyz;
    let r = 2.0 * dot(n, to_eye) * n - to_eye;
    var spec = min(textureSample(envmap_texture, envmap_sampler, vec3(r.x, -r.z, r.y)).rgb, vec3(16.0));
    spec *= factor;
    spec *= material.envmap_tint.xyz;
    spec = mix(spec, spec * spec, material.envmap_contrast.xyz);
    spec = mix(vec3(dot(spec, vec3(0.299, 0.587, 0.114))), spec, material.envmap_saturation.xyz);
    let fresnel = pow(1.0 - dot(n, normalize(to_eye)), 5.0);
    return spec * (fresnel * (1.0 - material.envmap_contrast.w) + material.envmap_contrast.w);
}
fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    return select(pow((color + vec3(0.055)) / 1.055, vec3(2.4)), color / 12.92, color <= vec3(0.04045));
}
// Source ambient cube (faces in Source axes) with squared-normal weighting.
fn ambient_light(n: vec3<f32>) -> vec3<f32> {
    let s = vec3(n.x, -n.z, n.y);
    let n2 = s * s;
    let x = select(material.lighting.ambient[0].xyz, material.lighting.ambient[1].xyz, s.x < 0.0);
    let y = select(material.lighting.ambient[2].xyz, material.lighting.ambient[3].xyz, s.y < 0.0);
    let z = select(material.lighting.ambient[4].xyz, material.lighting.ambient[5].xyz, s.z < 0.0);
    return n2.x * x + n2.y * y + n2.z * z;
}
// SDK DoLighting: per light color * cosine (or half-Lambert) * distance/spot attenuation.
fn model_light(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var sum = ambient_light(n);
    let count = i32(material.lighting.params.y);
    for (var i = 0; i < 4; i++) {
        if i >= count { break; }
        let kind = material.lighting.position[i].w;
        let to_light = material.lighting.position[i].xyz - p;
        let d2 = max(dot(to_light, to_light), 1e-6);
        let d = sqrt(d2);
        var l = to_light / d;
        var atten = 1.0;
        if kind > 1.5 {
            l = -material.lighting.direction[i].xyz;
        } else {
            atten = 1.0 / dot(material.lighting.attenuation[i].xyz, vec3(1.0, d, d2));
            if kind > 0.5 {
                let cos_theta = dot(material.lighting.direction[i].xyz, -l);
                var spot = max((cos_theta - material.lighting.spot[i].y) * material.lighting.spot[i].z, 0.0001);
                spot = clamp(pow(spot, material.lighting.spot[i].x), 0.0, 1.0);
                atten *= spot;
            }
        }
        var cosine = dot(n, l);
        if material.lighting.params.z > 0.5 {
            let h = cosine * 0.5 + 0.5;
            cosine = h * h;
        } else {
            cosine = max(cosine, 0.0);
        }
        sum += material.lighting.color[i].xyz * cosine * atten;
    }
    return sum;
}
// Direct world-space beam data for the origin-based viewmodel camera.

// Shared cookie, Lambert and distance calculation for world and viewmodel fragments.
// xyz is added diffuse light; w carries attenuation for the world's shadow blend.
fn flashlight_base(to: vec3<f32>, n: vec3<f32>, projected: vec3<f32>, near: f32, far: f32, linear: f32) -> vec4<f32> {
    if linear <= 0.0 || projected.z <= near { return vec4(0.0); }
    let xy = projected.xy / projected.z;
    if abs(xy.x) > 1.0 || abs(xy.y) > 1.0 { return vec4(0.0); }
    let cookie = textureSampleLevel(flashlight_texture, flashlight_sampler, vec2(xy.x, -xy.y) * 0.5 + 0.5, 0.0).rgb;
    let dist = max(length(to), 0.001);
    let atten = saturate(linear / dist);
    let fade = clamp((dist - far) / (-0.4 * far), 0.0, 1.0);
    let lambert = saturate(dot(n, -to / dist));
    return vec4(cookie * atten * lambert * fade, atten);
}
fn viewmodel_flashlight_color(position: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    if material.viewmodel_flashlight.limits.z < 0.5 { return vec3(0.0); }
    let p = material.lighting.basis * vec4(position, 1.0);
    let n = normalize((material.lighting.basis * vec4(normal, 0.0)).xyz);
    return flashlight_base(p.xyz - material.viewmodel_flashlight.origin_linear.xyz, n,
        (material.viewmodel_flashlight.world_to_beam * p).xyz,
        material.viewmodel_flashlight.limits.x, material.viewmodel_flashlight.limits.y,
        material.viewmodel_flashlight.origin_linear.w).rgb;
}
// Eight-sample Poisson footprint (SDK shader's sampling data), independently applied to
// Bevy's reverse-Z spotlight map. The engine-generated noise/jitter stream is not installed;
// this stable 32-pixel tile supplies rotation without a per-frame texture or CPU update.
const FLASHLIGHT_POISSON = array<vec2<f32>, 8>(
        vec2(0.3475, 0.0042), vec2(0.8806, 0.3430), vec2(-0.0041, -0.6197), vec2(0.0472, 0.4964),
        vec2(-0.3730, 0.0874), vec2(-0.9217, -0.3177), vec2(-0.6289, 0.7388), vec2(0.5744, -0.7741));
fn flashlight_shadow(id: u32, delta: vec3<f32>, direction: vec3<f32>, pixel: vec2<f32>) -> f32 {
    let light = &clustered_lights.data[id];
    let projected = transpose(orthonormalize(-direction)) * delta;
    let distance = -projected.z;
    let uv = projected.xy / (distance * (*light).spot_light_tan_angle) * vec2(0.5, -0.5) + vec2(0.5);
    let depth = min(1.0, (*light).shadow_map_near_z / distance + 0.0005);
    let tile = vec2<u32>(pixel) % vec2<u32>(32u);
    var hash = tile.x * 1664525u + tile.y * 1013904223u + 2246822519u;
    hash = (hash ^ (hash >> 16u)) * 3266489917u;
    let rotation = normalize(vec2<f32>(f32(hash & 65535u), f32(hash >> 16u)) / 32767.5 - vec2(1.0));

    var coverage = 0.0;
    for (var sample = 0u; sample < 8u; sample++) {
        let offset = mat2x2(rotation, vec2(-rotation.y, rotation.x)) * FLASHLIGHT_POISSON[sample] * (6.0 / 1024.0);
#ifdef NO_ARRAY_TEXTURES_SUPPORT
        coverage += textureSampleCompareLevel(directional_shadow_textures, directional_shadow_textures_comparison_sampler, uv + offset, depth);
#else
        coverage += textureSampleCompareLevel(directional_shadow_textures, directional_shadow_textures_comparison_sampler,
            uv + offset, i32(id) + lights.spot_light_shadowmap_offset, depth);
#endif
    }
    return coverage * 0.125;
}
// The HEV flashlight (SDK common_flashlight_fxc.h DoFlashlight), from the host's spot light: its cone
// uses the cookie's square projection (the round cookie fits the cone), its radius is the
// linear attenuation, its range the far plane, its near-plane field the near plane. The projected
// cookie times saturate(linear / d), the Lambert term and the far fade (from 60 % of far).
fn flashlight(world_position: vec4<f32>, n: vec3<f32>, frag_coord: vec2<f32>) -> vec3<f32> {
    let view_z = (view.view_from_world * vec4(world_position.xyz, 1.0)).z;
    let cluster = clustered_forward::view_fragment_cluster_index(frag_coord, view_z, false);
    let ranges = clustered_forward::unpack_clusterable_object_index_ranges(cluster);
    for (var i = ranges.first_spot_light_index_offset; i < ranges.first_reflection_probe_index_offset; i++) {
        let light_id = clustered_forward::get_clusterable_object_id(i);
        let light = &clustered_lights.data[light_id];
        if (*light).position_radius.w <= 0.0 { return vec3(0.0); }
        var d = vec3((*light).light_custom_data.x, 0.0, (*light).light_custom_data.y);
        d.y = sqrt(max(0.0, 1.0 - d.x * d.x - d.z * d.z));
        if ((*light).flags & 2u) != 0u { d.y = -d.y; }
        var up = vec3(0.0, 1.0, 0.0) - d * d.y;
        if dot(up, up) < 1e-6 { up = vec3(0.0, 0.0, -1.0); }
        up = normalize(up);
        let right = cross(d, up);
        let to = world_position.xyz - (*light).position_radius.xyz;
        let tan_half = (*light).spot_light_tan_angle;
        let projected = vec3(dot(to, right) / tan_half, dot(to, up) / tan_half, dot(to, d));
        let beam = flashlight_base(to, n, projected, (*light).shadow_map_near_z, (*light).range, (*light).position_radius.w);
        // Avoid all shadow taps for fragments receiving no flashlight contribution.
        if all(beam.rgb <= vec3(0.0)) { return vec3(0.0); }
        var shadow = 1.0;
        if ((*light).flags & 1u) != 0u {
            let visibility = flashlight_shadow(light_id, to, d, frag_coord);
            // SDK DoFlashlight: the 0.35 fill light is reduced with distance attenuation.
            shadow = mix(mix(visibility, 1.0, 0.35), visibility, beam.a);
        }
        return beam.rgb * shadow;
    }
    return vec3(0.0);
}
@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    var base = textureSample(base_texture, base_sampler, mesh.uv);
    let base_alpha = base.a;
    if material.parameters.w > 0.5 {
        let iris = textureSample(iris_texture, iris_sampler, mesh.uv_b);
        base = vec4(mix(base.rgb, iris.rgb, iris.a), base.a);
    }
    base.a = select(base.a, 1.0, material.parameters.y > 0.5);
    var vertex_color = mesh.color;
    let lit = material.lighting.params.x > 0.5;
    if lit { vertex_color = vec4(1.0, 1.0, 1.0, vertex_color.a); }
    if material.parameters.w < -0.5 {
        let uv = (material.secondary_uv * vec3(mesh.uv, 1.0)).xy;
        base = vec4(base.rgb * textureSample(iris_texture, iris_sampler, uv).rgb, base.a);
        if material.parameters.w > -1.5 || material.parameters.w < -2.5 { vertex_color = vec4(1.0); }
        // SDK UnlitTwoTexture returns alpha 1; distance fade lives in the VTF frames.
        if material.parameters.w < -2.5 { base.a = 1.0; }
    }
    // Static-prop vertex colour is baked illumination, not emissive albedo.
    let material_albedo = base.rgb;
    base *= material.tint * vertex_color;
    if base.a < material.parameters.x { discard; }
#ifdef MAY_DISCARD
    // Alpha-test survivors are opaque. Retaining texture alpha blends the sky back
    // through cutout edges when later cameras compose the frame, creating bright rims.
    base.a = 1.0;
#endif

    let baked = select(textureSample(lightmap_texture, lightmap_sampler, mesh.uv_b).rgb, vec3(1.0), material.parameters.w > 0.5);
    // Lightmaps are linear and unclamped (Source HDR path); white fallbacks are 1.0.
    // Procedural camera images are sampled through an sRGB view and are already linear.
    var rgb = select(srgb_to_linear(base.rgb) * baked, base.rgb, material.parameters.w < -0.5);
    if lit {
        let p = (material.lighting.basis * vec4(mesh.world_position.xyz, 1.0)).xyz;
        let n = normalize((material.lighting.basis * vec4(mesh.world_normal, 0.0)).xyz);
        rgb = srgb_to_linear(base.rgb) * model_light(p, n);
    }
    if material.lighting.params.w > 1.5 && material.parameters.z < 0.5 && material.parameters.w > -0.5 {
        rgb += srgb_to_linear(base.rgb) * viewmodel_flashlight_color(mesh.world_position.xyz, mesh.world_normal);
    } else if material.lighting.params.w < 0.5 {
        rgb += srgb_to_linear(base.rgb) * flashlight(mesh.world_position, normalize(mesh.world_normal), mesh.position.xy);
    }
    // SDK $selfillum: diffuse = lerp(diffuse, material.tint * albedo, base alpha), before specular.
    if material.self_illum.w > 0.5 {
        rgb = mix(rgb, material.self_illum.xyz * srgb_to_linear(material_albedo * material.tint.rgb), base_alpha);
    }
    if material.envmap_tint.w > 0.5 {
        rgb += envmap_specular(mesh, base_alpha);
    }
    // Source FinalOutput TONEMAP_SCALE_LINEAR: the camera's exposure is the tonemap scale.
    rgb *= view.exposure;
    // Bevy Add uses a premultiplied pipeline. Alpha zero retains all of the
    // destination while premultiplying RGB implements Source SrcAlpha/One.
    if material.parameters.z > 0.5 { return vec4(rgb * base.a, 0.0); }
    return vec4(rgb, base.a);
}
