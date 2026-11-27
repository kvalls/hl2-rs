import bevy_pbr::{
    render::mesh_functions,
    prepass::io::{Vertex, VertexOutput},
    render::view_transformations::position_world_to_clip,
};
@if(MOTION_VECTOR_PREPASS)
import bevy_pbr::{prepass::io::FragmentOutput, render::mesh_view_bindings::view};
@if(MOTION_VECTOR_PREPASS)
import package::shaders::object_motion::object_motion;
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var effect_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var effect_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<uniform> parameters: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var<storage, read> quad_rows: array<vec4<f32>>;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let tag = mesh_functions::get_tag(vertex.instance_index);
    if tag != 0u {
        let start = tag >> 16u;
        let count = tag & 0xffffu;
        let q = u32(vertex.position.x + 0.5);
        let c = u32(vertex.position.y + 0.5);
        if q >= count {
            out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
            @if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
                out.unclipped_depth = 2.0;
            }
            return out;
        }
        let base = (start + q) * 6u;
        let row0 = quad_rows[base];
        let row1 = quad_rows[base + 1u];
        let row2 = quad_rows[base + 2u];
        let row3 = quad_rows[base + 3u];
        let row4 = quad_rows[base + 4u];
        let row5 = quad_rows[base + 5u];
        let p01 = select(row0, row1, c == 1u);
        let p23 = select(row2, row3, c == 3u);
        let p = select(p01, p23, c >= 2u);
        let pos = p.xyz;
        let v01 = select(row4.x, row4.y, c == 1u);
        let v23 = select(row4.z, row4.w, c == 3u);
        let uv_y = select(v01, v23, c >= 2u);
        out.world_position = vec4<f32>(pos, 1.0);
        out.position = position_world_to_clip(pos);
        @if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
            out.unclipped_depth = out.position.z;
            out.position.z = min(out.position.z, 1.0);
        }
        @if(VERTEX_UVS_A) {
            out.uv = vec2<f32>(p.w, uv_y);
        }
        @if(VERTEX_UVS_B) {
            out.uv_b = vec2<f32>(0.0);
        }
        @if(NORMAL_PREPASS_OR_DEFERRED_PREPASS) {
            out.world_normal = vec3<f32>(0.0, 1.0, 0.0);
        }
        @if(NORMAL_PREPASS_OR_DEFERRED_PREPASS && VERTEX_TANGENTS) {
            out.world_tangent = vec4<f32>(1.0, 0.0, 0.0, 1.0);
        }
        @if(VERTEX_COLORS) {
            out.color = row5;
        }
        @if(MOTION_VECTOR_PREPASS) {
            out.previous_world_position = out.world_position;
        }
        @if(VERTEX_OUTPUT_INSTANCE_INDEX) {
            out.instance_index = vertex.instance_index;
        }
        @if(VISIBILITY_RANGE_DITHER) {
            let mesh_world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
            out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
                vertex.instance_index, mesh_world_from_local[3]);
        }
        return out;
    }

    let mesh_world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let world_from_local = mesh_world_from_local;
    out.world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0)
    );
    out.position = position_world_to_clip(out.world_position.xyz);
    @if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
        out.unclipped_depth = out.position.z;
        out.position.z = min(out.position.z, 1.0);
    }
    @if(VERTEX_UVS_A) {
        out.uv = vertex.uv;
    }
    @if(VERTEX_UVS_B) {
        out.uv_b = vertex.uv_b;
    }
    @if(NORMAL_PREPASS_OR_DEFERRED_PREPASS && VERTEX_NORMALS) {
        out.world_normal = mesh_functions::mesh_normal_local_to_world(
            vertex.normal,
            vertex.instance_index
        );
    }
    @if(NORMAL_PREPASS_OR_DEFERRED_PREPASS && VERTEX_TANGENTS) {
        out.world_tangent = mesh_functions::mesh_tangent_local_to_world(
            world_from_local,
            vertex.tangent,
            vertex.instance_index
        );
    }
    @if(VERTEX_COLORS) {
        out.color = vertex.color;
    }
    @if(MOTION_VECTOR_PREPASS) {
        let prev_model = mesh_functions::get_previous_world_from_local(vertex.instance_index);
        out.previous_world_position = mesh_functions::mesh_position_local_to_world(
            prev_model,
            vec4<f32>(vertex.position, 1.0)
        );
    }
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) {
        out.instance_index = vertex.instance_index;
    }
    @if(VISIBILITY_RANGE_DITHER) {
        out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
            vertex.instance_index, mesh_world_from_local[3]);
    }
    return out;
}

// The effect's alpha test (parameters.x the cutoff, y > 0.5 ignores the texture's alpha).
fn alpha_test(mesh: VertexOutput) {
    if parameters.x > 0.0 {
        var alpha = 1.0;
        if parameters.y <= 0.5 { alpha = textureSample(effect_texture,effect_sampler,mesh.uv).a; }
        @if(VERTEX_COLORS) {
            alpha *= mesh.color.a;
        }
        if alpha < parameters.x { discard; }
    }
}

@if(MOTION_VECTOR_PREPASS)
@fragment
fn fragment(mesh: VertexOutput) -> FragmentOutput {
    alpha_test(mesh);
    var out: FragmentOutput;
    out.motion_vector = object_motion(mesh.world_position,mesh.previous_world_position,view.unjittered_clip_from_world);
    return out;
}

@else
@fragment
fn fragment(mesh: VertexOutput) {
    alpha_test(mesh);
}
