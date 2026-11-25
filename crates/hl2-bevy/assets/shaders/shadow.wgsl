// A projected shadow on the world (Source SDK shadow_ps2x.fxc): five coverage taps one texel
// apart averaged, less the vertex fade, blend the surface toward the shadow colour; the pipeline
// multiplies the framebuffer by the result.
#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput},
    mesh_functions,
    view_transformations::position_world_to_clip,
}

struct ShadowParameters {
    color: vec4<f32>,
    jitter: vec4<f32>,
    channel: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var shadow_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var shadow_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> parameters: ShadowParameters;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var<storage, read> shadow_verts: array<vec4<f32>>;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let tag = mesh_functions::get_tag(vertex.instance_index);
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
    if tag != 0u {
        let start_vert = (tag >> 16u) * 3u;
        let vert_count = (tag & 0xffffu) * 3u;
#ifdef VERTEX_POSITIONS
        let v = u32(vertex.position.x + 0.5);
#else
        let v = 0u;
#endif
        out.world_normal = vec3<f32>(0.0, 1.0, 0.0);
        if v >= vert_count {
            out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
            out.world_position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
#ifdef VERTEX_UVS_A
            out.uv = vec2<f32>(0.0, 0.0);
#endif
#ifdef VERTEX_UVS_B
            out.uv_b = vec2<f32>(0.0, 0.0);
#endif
#ifdef VERTEX_TANGENTS
            out.world_tangent = vec4<f32>(1.0, 0.0, 0.0, 1.0);
#endif
#ifdef VERTEX_COLORS
            out.color = vec4<f32>(1.0, 1.0, 1.0, 0.0);
#endif
#ifdef VISIBILITY_RANGE_DITHER
            out.visibility_range_dither = 0;
#endif
            return out;
        }
        let base = (start_vert + v) * 2u;
        let p = shadow_verts[base];
        let uv = shadow_verts[base + 1u];
        out.world_position = vec4<f32>(p.xyz, 1.0);
        out.position = position_world_to_clip(p.xyz);
#ifdef VERTEX_UVS_A
        out.uv = uv.xy;
#endif
#ifdef VERTEX_UVS_B
        out.uv_b = vec2<f32>(0.0, 0.0);
#endif
#ifdef VERTEX_TANGENTS
        out.world_tangent = vec4<f32>(1.0, 0.0, 0.0, 1.0);
#endif
#ifdef VERTEX_COLORS
        out.color = vec4<f32>(1.0 - uv.z, 1.0, 1.0, p.w);
#endif
#ifdef VISIBILITY_RANGE_DITHER
        out.visibility_range_dither = 0;
#endif
        return out;
    }

    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
#ifdef VERTEX_POSITIONS
    out.world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0),
    );
    out.position = position_world_to_clip(out.world_position.xyz);
#endif
#ifdef VERTEX_NORMALS
    out.world_normal = mesh_functions::mesh_normal_local_to_world(
        vertex.normal,
        vertex.instance_index,
    );
#else
    out.world_normal = vec3<f32>(0.0, 1.0, 0.0);
#endif
#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = vertex.uv_b;
#endif
#ifdef VERTEX_TANGENTS
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(
        world_from_local,
        vertex.tangent,
        vertex.instance_index,
    );
#endif
#ifdef VERTEX_COLORS
    out.color = vertex.color;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        vertex.instance_index,
        world_from_local[3],
    );
#endif
    return out;
}

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
    coverage = saturate(coverage - mesh.color.a) * mesh.color.r;
#endif
    return vec4(vec3(1.0) + coverage * (parameters.color.rgb - vec3(1.0)), 1.0);
}
