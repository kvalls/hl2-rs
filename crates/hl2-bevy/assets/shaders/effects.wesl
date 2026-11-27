import bevy_pbr::render::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput},
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
};
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
        var q = 0u;
        var c = 0u;
        @if(VERTEX_POSITIONS) {
            q = u32(vertex.position.x + 0.5);
            c = u32(vertex.position.y + 0.5);
        }
        if q >= count {
            out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
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
        out.world_normal = vec3<f32>(0.0, 1.0, 0.0);
        @if(VERTEX_UVS_A) {
            out.uv = vec2<f32>(p.w, uv_y);
        }
        @if(VERTEX_UVS_B) {
            out.uv_b = vec2<f32>(0.0);
        }
        @if(VERTEX_TANGENTS) {
            out.world_tangent = vec4<f32>(1.0, 0.0, 0.0, 1.0);
        }
        @if(VERTEX_COLORS) {
            out.color = row5;
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
    @if(VERTEX_NORMALS) {
        out.world_normal = mesh_functions::mesh_normal_local_to_world(
            vertex.normal,
            vertex.instance_index
        );
    }
    @if(VERTEX_POSITIONS) {
        out.world_position = mesh_functions::mesh_position_local_to_world(
            world_from_local,
            vec4<f32>(vertex.position, 1.0)
        );
        out.position = position_world_to_clip(out.world_position.xyz);
    }
    @if(VERTEX_UVS_A) {
        out.uv = vertex.uv;
    }
    @if(VERTEX_UVS_B) {
        out.uv_b = vertex.uv_b;
    }
    @if(VERTEX_TANGENTS) {
        out.world_tangent = mesh_functions::mesh_tangent_local_to_world(
            world_from_local,
            vertex.tangent,
            vertex.instance_index
        );
    }
    @if(VERTEX_COLORS) {
        out.color = vertex.color;
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
fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    return select(pow((color + vec3(0.055)) / 1.055, vec3(2.4)), color / 12.92, color <= vec3(0.04045));
}
@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    var color = textureSample(effect_texture, effect_sampler, mesh.uv);
    color.a = select(color.a, 1.0, parameters.y > 0.5);
    // DecalModulate (w = 2): alpha test GREATER 0, mod2x of the frame (2 x texture, blended
    // DST_COLOR x SRC) with TONEMAP_SCALE_NONE. The lit frame is the multiplicand, so neither
    // the vertex light (studio decals carry one for lit decals) nor exposure applies.
    if parameters.w > 1.5 {
        if color.a < parameters.x { discard; }
        return vec4(srgb_to_linear(color.rgb * parameters.z), 1.0);
    }
    // Lit decals (w = 1, DecalBaseTimesLightmapAlphaBlend): texture x the linear light
    // of the surface beneath (vertex colour) x exposure, as the world is lit.
    if parameters.w > 0.5 {
        if color.a < parameters.x { discard; }
        return vec4(srgb_to_linear(color.rgb) * mesh.color.rgb * view.exposure, color.a);
    }
    // Dim beams (w = -1): sprites add in gamma space, so a dim color is a few levels on any
    // background. Converting the product as below would leave a third of that.
    if parameters.w < -0.5 {
        return vec4(srgb_to_linear(color.rgb) * mesh.color.rgb * view.exposure, color.a);
    }
    color *= mesh.color;
    if color.a < parameters.x { discard; }
    // Source sprite shaders end in FinalOutput TONEMAP_SCALE_LINEAR (camera exposure).
    return vec4(srgb_to_linear(color.rgb * parameters.z) * view.exposure, color.a);
}
