#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::view
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> tint: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var base_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var base_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var<uniform> parameters: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var lightmap_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var lightmap_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var iris_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var iris_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var<uniform> secondary_uv: mat3x3<f32>;
struct ModelLighting {
    params: vec4<f32>,
    basis: mat4x4<f32>,
    ambient: array<vec4<f32>, 6>,
    position: array<vec4<f32>, 4>,
    color: array<vec4<f32>, 4>,
    direction: array<vec4<f32>, 4>,
    attenuation: array<vec4<f32>, 4>,
    spot: array<vec4<f32>, 4>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(9) var<uniform> lighting: ModelLighting;
@group(#{MATERIAL_BIND_GROUP}) @binding(10) var envmap_texture: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(11) var envmap_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(12) var<uniform> envmap_tint: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(13) var<uniform> envmap_contrast: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(14) var<uniform> envmap_saturation: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(15) var envmap_mask_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(16) var envmap_mask_sampler: sampler;
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
    let flags = u32(envmap_saturation.w + 0.5);
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
    spec *= envmap_tint.xyz;
    spec = mix(spec, spec * spec, envmap_contrast.xyz);
    spec = mix(vec3(dot(spec, vec3(0.299, 0.587, 0.114))), spec, envmap_saturation.xyz);
    let fresnel = pow(1.0 - dot(n, normalize(to_eye)), 5.0);
    return spec * (fresnel * (1.0 - envmap_contrast.w) + envmap_contrast.w);
}
fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    return select(pow((color + vec3(0.055)) / 1.055, vec3(2.4)), color / 12.92, color <= vec3(0.04045));
}
// Source ambient cube (faces in Source axes) with squared-normal weighting.
fn ambient_light(n: vec3<f32>) -> vec3<f32> {
    let s = vec3(n.x, -n.z, n.y);
    let n2 = s * s;
    let x = select(lighting.ambient[0].xyz, lighting.ambient[1].xyz, s.x < 0.0);
    let y = select(lighting.ambient[2].xyz, lighting.ambient[3].xyz, s.y < 0.0);
    let z = select(lighting.ambient[4].xyz, lighting.ambient[5].xyz, s.z < 0.0);
    return n2.x * x + n2.y * y + n2.z * z;
}
// SDK DoLighting: per light color * cosine (or half-Lambert) * distance/spot attenuation.
fn model_light(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var sum = ambient_light(n);
    let count = i32(lighting.params.y);
    for (var i = 0; i < 4; i++) {
        if i >= count { break; }
        let kind = lighting.position[i].w;
        let to_light = lighting.position[i].xyz - p;
        let d2 = max(dot(to_light, to_light), 1e-6);
        let d = sqrt(d2);
        var l = to_light / d;
        var atten = 1.0;
        if kind > 1.5 {
            l = -lighting.direction[i].xyz;
        } else {
            atten = 1.0 / dot(lighting.attenuation[i].xyz, vec3(1.0, d, d2));
            if kind > 0.5 {
                let cos_theta = dot(lighting.direction[i].xyz, -l);
                var spot = max((cos_theta - lighting.spot[i].y) * lighting.spot[i].z, 0.0001);
                spot = clamp(pow(spot, lighting.spot[i].x), 0.0, 1.0);
                atten *= spot;
            }
        }
        var cosine = dot(n, l);
        if lighting.params.z > 0.5 {
            let h = cosine * 0.5 + 0.5;
            cosine = h * h;
        } else {
            cosine = max(cosine, 0.0);
        }
        sum += lighting.color[i].xyz * cosine * atten;
    }
    return sum;
}
@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    var base = textureSample(base_texture, base_sampler, mesh.uv);
    let base_alpha = base.a;
    if parameters.w > 0.5 {
        let iris = textureSample(iris_texture, iris_sampler, mesh.uv_b);
        base = vec4(mix(base.rgb, iris.rgb, iris.a), base.a);
    }
    base.a = select(base.a, 1.0, parameters.y > 0.5);
    var vertex_color = mesh.color;
    let lit = lighting.params.x > 0.5;
    if lit { vertex_color = vec4(1.0, 1.0, 1.0, vertex_color.a); }
    if parameters.w < -0.5 {
        let uv = (secondary_uv * vec3(mesh.uv, 1.0)).xy;
        base = vec4(base.rgb * textureSample(iris_texture, iris_sampler, uv).rgb, base.a);
        if parameters.w > -1.5 { vertex_color = vec4(1.0); }
    }
    base *= tint * vertex_color;
    if base.a < parameters.x { discard; }
    let baked = select(textureSample(lightmap_texture, lightmap_sampler, mesh.uv_b).rgb, vec3(1.0), parameters.w > 0.5);
    // Lightmaps are linear and unclamped (Source HDR path); white fallbacks are 1.0.
    // Procedural camera images are sampled through an sRGB view and are already linear.
    var rgb = select(srgb_to_linear(base.rgb) * baked, base.rgb, parameters.w < -0.5);
    if envmap_tint.w > 0.5 {
        rgb += envmap_specular(mesh, base_alpha);
    }
    if lit {
        let p = (lighting.basis * vec4(mesh.world_position.xyz, 1.0)).xyz;
        let n = normalize((lighting.basis * vec4(mesh.world_normal, 0.0)).xyz);
        rgb = srgb_to_linear(base.rgb) * model_light(p, n);
    }
    // Source FinalOutput TONEMAP_SCALE_LINEAR: the camera's exposure is the tonemap scale.
    rgb *= view.exposure;
    // Bevy Add uses a premultiplied pipeline. Alpha zero retains all of the
    // destination while premultiplying RGB implements Source SrcAlpha/One.
    if parameters.z > 0.5 { return vec4(rgb * base.a, 0.0); }
    return vec4(rgb, base.a);
}
