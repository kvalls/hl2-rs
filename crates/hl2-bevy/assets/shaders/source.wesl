import package::shaders::source_uniforms::material;
import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::mesh_functions;
import bevy_pbr::render::mesh_view_bindings::{view, clustered_lights, lights, directional_shadow_textures, directional_shadow_textures_comparison_sampler};
import bevy_render::maths::orthonormalize;
import bevy_pbr::render::clustered_forward;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var base_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var base_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var lightmap_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var lightmap_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var iris_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var iris_sampler: sampler;

@group(constants::MATERIAL_BIND_GROUP) @binding(10) var envmap_texture: texture_cube<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(11) var envmap_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(15) var envmap_mask_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(16) var envmap_mask_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(18) var flashlight_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(19) var flashlight_sampler: sampler;
// Iris projections, two world-space rows per eye mesh; the mesh's second UV holds its slot.
@group(constants::MATERIAL_BIND_GROUP) @binding(20) var<storage, read> eye_rows: array<vec4<f32>>;
// Per-frame material values (tint colour, texture transform columns), four rows per slot.
@group(constants::MATERIAL_BIND_GROUP) @binding(21) var<storage, read> material_rows: array<vec4<f32>>;
// Model lighting rows (material_slots::lighting, 31 rows per slot).
@group(constants::MATERIAL_BIND_GROUP) @binding(22) var<storage, read> light_rows: array<vec4<f32>>;
fn material_tint() -> vec4<f32> {
    if material.dynamic.x < 0.0 || material.dynamic.y < 0.5 { return material.tint; }
    return vec4(material_rows[u32(material.dynamic.x) * 4u].rgb, material.tint.a);
}
fn material_secondary_uv() -> mat3x3<f32> {
    if material.dynamic.x < 0.0 { return material.secondary_uv; }
    let row = u32(material.dynamic.x) * 4u;
    return mat3x3(material_rows[row + 1u].xyz, material_rows[row + 2u].xyz, material_rows[row + 3u].xyz);
}
// Model lighting from its entity's slot when the draw's MeshTag (> 0) or material.dynamic.z
// names one, else the material's own.
@if(SOURCE_MODEL_LIT)
fn light_slot_index(instance_index: u32) -> i32 {
    let tag = mesh_functions::get_tag(instance_index);
    if tag > 0u { return i32(tag - 1u); }
    return select(-1, i32(material.dynamic.z), material.dynamic.z >= 0.0);
}
@if(SOURCE_MODEL_LIT)
fn light_row(slot: i32, i: u32) -> vec4<f32> { return light_rows[u32(slot) * 31u + i]; }
@if(SOURCE_MODEL_LIT)
fn light_params(slot: i32) -> vec4<f32> {
    if slot >= 0 { return light_row(slot, 0u); }
    return material.lighting.params;
}
@if(SOURCE_MODEL_LIT)
fn light_basis(slot: i32) -> mat4x4<f32> {
    if slot >= 0 { return mat4x4(light_row(slot, 1u), light_row(slot, 2u), light_row(slot, 3u), light_row(slot, 4u)); }
    return material.lighting.basis;
}
@if(SOURCE_MODEL_LIT)
fn light_ambient(slot: i32, i: u32) -> vec4<f32> {
    if slot >= 0 { return light_row(slot, 5u + i); }
    return material.lighting.ambient[i];
}
@if(SOURCE_MODEL_LIT)
fn light_position(slot: i32, i: u32) -> vec4<f32> {
    if slot >= 0 { return light_row(slot, 11u + i); }
    return material.lighting.position[i];
}
@if(SOURCE_MODEL_LIT)
fn light_color(slot: i32, i: u32) -> vec4<f32> {
    if slot >= 0 { return light_row(slot, 15u + i); }
    return material.lighting.color[i];
}
@if(SOURCE_MODEL_LIT)
fn light_direction(slot: i32, i: u32) -> vec4<f32> {
    if slot >= 0 { return light_row(slot, 19u + i); }
    return material.lighting.direction[i];
}
@if(SOURCE_MODEL_LIT)
fn light_attenuation(slot: i32, i: u32) -> vec4<f32> {
    if slot >= 0 { return light_row(slot, 23u + i); }
    return material.lighting.attenuation[i];
}
@if(SOURCE_MODEL_LIT)
fn light_spot(slot: i32, i: u32) -> vec4<f32> {
    if slot >= 0 { return light_row(slot, 27u + i); }
    return material.lighting.spot[i];
}
// SDK lightmappedgeneric_ps2_3_x.h CUBEMAP: reflection of the eye vector about the face
// normal, sampled in Source axes. Retail integer HDR stores cubemaps as linear/16 and
// multiplies by ENV_MAP_SCALE 16 (shaderapidx9 c30.z), so texels are capped at 16.
// Tangent frame along the texture's s/t axes from screen-space derivatives (no per-vertex
// tangents): SDK world = mul(vNormal, [tangentS, tangentT, normal]), and VRAD's
// GetBumpNormals orients that frame with s and t, flipping it on left-handed faces.
@if(SOURCE_ENVMAP)
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
@if(SOURCE_ENVMAP)
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
@if(SOURCE_MODEL_LIT)
fn ambient_light(slot: i32, n: vec3<f32>) -> vec3<f32> {
    let s = vec3(n.x, -n.z, n.y);
    let n2 = s * s;
    let x = select(light_ambient(slot, 0u).xyz, light_ambient(slot, 1u).xyz, s.x < 0.0);
    let y = select(light_ambient(slot, 2u).xyz, light_ambient(slot, 3u).xyz, s.y < 0.0);
    let z = select(light_ambient(slot, 4u).xyz, light_ambient(slot, 5u).xyz, s.z < 0.0);
    return n2.x * x + n2.y * y + n2.z * z;
}
// SDK DoLighting: per light color * cosine (or half-Lambert) * distance/spot attenuation.
@if(SOURCE_MODEL_LIT)
fn model_light(slot: i32, p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var sum = ambient_light(slot, n);
    let params = light_params(slot);
    let count = i32(params.y);
    for (var i = 0; i < 4; i++) {
        if i >= count { break; }
        let pos = light_position(slot, u32(i));
        let kind = pos.w;
        let to_light = pos.xyz - p;
        let d2 = max(dot(to_light, to_light), 1e-6);
        let d = sqrt(d2);
        var l = to_light / d;
        var atten = 1.0;
        let dir = light_direction(slot, u32(i)).xyz;
        if kind > 1.5 {
            l = -dir;
        } else {
            atten = 1.0 / dot(light_attenuation(slot, u32(i)).xyz, vec3(1.0, d, d2));
            if kind > 0.5 {
                let spot_terms = light_spot(slot, u32(i));
                let cos_theta = dot(dir, -l);
                var spot = max((cos_theta - spot_terms.y) * spot_terms.z, 0.0001);
                spot = clamp(pow(spot, spot_terms.x), 0.0, 1.0);
                atten *= spot;
            }
        }
        var cosine = dot(n, l);
        if params.z > 0.5 {
            let h = cosine * 0.5 + 0.5;
            cosine = h * h;
        } else {
            cosine = max(cosine, 0.0);
        }
        sum += light_color(slot, u32(i)).xyz * cosine * atten;
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
@if(SOURCE_MODEL_LIT)
fn viewmodel_flashlight_color(slot: i32, position: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    if material.viewmodel_flashlight.limits.z < 0.5 { return vec3(0.0); }
    let basis = light_basis(slot);
    let p = basis * vec4(position, 1.0);
    let n = normalize((basis * vec4(normal, 0.0)).xyz);
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
        @if(NO_ARRAY_TEXTURES_SUPPORT) {
            coverage += textureSampleCompareLevel(directional_shadow_textures, directional_shadow_textures_comparison_sampler, uv + offset, depth);
        }
        @if(!NO_ARRAY_TEXTURES_SUPPORT) {
            coverage += textureSampleCompareLevel(directional_shadow_textures, directional_shadow_textures_comparison_sampler,
            uv + offset, i32(id) + lights.spot_light_shadowmap_offset, depth);
        }
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
fn has_spot_lights() -> bool {
    if lights.cluster_dimensions.w != 1u { return true; }
    let ranges = clustered_forward::unpack_clusterable_object_index_ranges(0u);
    return ranges.first_spot_light_index_offset < ranges.first_reflection_probe_index_offset;
}
@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    var base = textureSample(base_texture, base_sampler, mesh.uv);
    // DecalModulate (SDK decalmodulate_dx9.cpp): alpha test, then the frame times twice the
    // texture (DST_COLOR, SRC_COLOR): grey leaves it unchanged. Unlit and without the tonemap scale.
    if material.parameters.z > 1.5 {
        @if(SOURCE_MAY_DISCARD) {
            if base.a < material.parameters.x { discard; }
        }
        return vec4(srgb_to_linear(base.rgb * 2.0), 1.0);
    }
    if material.parameters.w > 1.5 {
        // WorldVertexTransition (SDK lightmappedgeneric_ps2_3_x.h): the second base, at the
        // same coordinates, by the vertex alpha a displacement vertex carries; alpha too.
        base = mix(base, textureSample(iris_texture, iris_sampler, mesh.uv), mesh.color.a);
    }
    let base_alpha = base.a;
    var eye = false;
    @if(SOURCE_MODEL_LIT) {
        eye = material.parameters.w > 0.5 && material.parameters.w < 1.5;
        if eye {
            let slot = u32(mesh.uv_b.x + 0.5) * 2u;
            let at = vec4(mesh.world_position.xyz, 1.0);
            var iris_uv = vec2(0.0);
            if slot + 1u < arrayLength(&eye_rows) {
                iris_uv = vec2(dot(eye_rows[slot], at), dot(eye_rows[slot + 1u], at));
            }
            let iris = textureSample(iris_texture, iris_sampler, iris_uv);
            base = vec4(mix(base.rgb, iris.rgb, iris.a), base.a);
        }
    }
    base.a = select(base.a, 1.0, material.parameters.y > 0.5);
    var vertex_color = mesh.color;
    // Vertex alpha is a blend weight there, never opacity (camera materials use it below).
    if material.parameters.w > -0.5 { vertex_color.a = 1.0; }
    var lparams = material.lighting.params;
    var lslot = -1;
    var lit = false;
    @if(SOURCE_MODEL_LIT) {
        lslot = light_slot_index(mesh.instance_index);
        lparams = light_params(lslot);
        lit = lparams.x > 0.5;
        if lit { vertex_color = vec4(1.0, 1.0, 1.0, vertex_color.a); }
    }
    if material.parameters.w < -0.5 {
        let uv = (material_secondary_uv() * vec3(mesh.uv, 1.0)).xy;
        base = vec4(base.rgb * textureSample(iris_texture, iris_sampler, uv).rgb, base.a);
        if material.parameters.w > -1.5 || material.parameters.w < -2.5 { vertex_color = vec4(1.0); }
        // SDK UnlitTwoTexture returns alpha 1; distance fade lives in the VTF frames.
        if material.parameters.w < -2.5 { base.a = 1.0; }
    }
    // Static-prop vertex colour is baked illumination, not emissive albedo.
    let material_albedo = base.rgb;
    base *= material_tint() * vertex_color;
    @if(SOURCE_MAY_DISCARD) {
        if base.a < material.parameters.x { discard; }
    }
    @if(MAY_DISCARD) {
        // Alpha-test survivors are opaque. Retaining texture alpha blends the sky back
        // through cutout edges when later cameras compose the frame, creating bright rims.
        base.a = 1.0;
    }

    var linear_base = base.rgb;
    var rgb = base.rgb;
    if material.parameters.w >= -0.5 {
        var baked = vec3(1.0);
        @if(SOURCE_MODEL_LIT) {
            if !eye {
                baked = textureSample(lightmap_texture, lightmap_sampler, mesh.uv_b).rgb;
            }
        }
        @if(!SOURCE_MODEL_LIT) {
            if lparams.w < 0.5 {
                baked = textureSample(lightmap_texture, lightmap_sampler, mesh.uv_b).rgb;
            }
        }
        // Lightmaps are linear and unclamped (Source HDR path); white fallbacks are 1.0.
        // Procedural camera images are sampled through an sRGB view and are already linear.
        linear_base = srgb_to_linear(base.rgb);
        rgb = linear_base * baked;
    }
    @if(SOURCE_MODEL_LIT) {
        if lit {
            let basis = light_basis(lslot);
            let p = (basis * vec4(mesh.world_position.xyz, 1.0)).xyz;
            let n = normalize((basis * vec4(mesh.world_normal, 0.0)).xyz);
            rgb = linear_base * model_light(lslot, p, n);
        }
        if lparams.w > 1.5 && material.parameters.z < 0.5 && material.parameters.w > -0.5 {
            rgb += linear_base * viewmodel_flashlight_color(lslot, mesh.world_position.xyz, mesh.world_normal);
        } else if lparams.w < 0.5 && has_spot_lights() {
            rgb += linear_base * flashlight(mesh.world_position, normalize(mesh.world_normal), mesh.position.xy);
        }
    }
    @if(!SOURCE_MODEL_LIT) {
        if lparams.w < 0.5 && has_spot_lights() {
            rgb += linear_base * flashlight(mesh.world_position, normalize(mesh.world_normal), mesh.position.xy);
        }
    }
    // SDK $selfillum: diffuse = lerp(diffuse, material.tint * albedo, base alpha), before specular.
    if material.self_illum.w > 0.5 {
        rgb = mix(rgb, material.self_illum.xyz * srgb_to_linear(material_albedo * material_tint().rgb), base_alpha);
    }
    @if(SOURCE_ENVMAP) {
        if material.envmap_tint.w > 0.5 {
            rgb += envmap_specular(mesh, base_alpha);
        }
    }
    // Source FinalOutput TONEMAP_SCALE_LINEAR: the camera's exposure is the tonemap scale.
    rgb *= view.exposure;
    // Bevy Add uses a premultiplied pipeline. Alpha zero retains all of the
    // destination while premultiplying RGB implements Source SrcAlpha/One.
    if material.parameters.z > 0.5 { return vec4(rgb * base.a, 0.0); }
    return vec4(rgb, base.a);
}
