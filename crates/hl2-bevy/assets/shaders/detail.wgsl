// Detail sprites (SDK CDetailObjectSystem fast path, orientation 2): each vertex carries its
// sprite's origin (position), corner offset (uv_b: right, up) and baked linear color. The quad
// faces the view drawing it in the horizontal plane, so player and monitor views each get
// their own facing and fade.
#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput},
    mesh_functions,
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var atlas: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var atlas_sampler: sampler;
// (maximum distance squared, fade start distance squared, unused, unused)
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> fade: vec4<f32>;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    let origin = (world_from_local * vec4(vertex.position, 1.0)).xyz;
    // Source +Z is Bevy +Y; one Source unit is one world unit.
    let up = vec3(0.0, 1.0, 0.0);
    var to_view = view.world_position - origin;
    let distance_squared = dot(to_view, to_view);
    to_view.y = 0.0;
    var facing = vec3(0.0, 0.0, 1.0);
    if dot(to_view, to_view) > 1e-6 {
        facing = normalize(to_view);
    }
    // The viewer's right: cross(view direction, up) with the view direction toward the sprite.
    let right = cross(-facing, up);
    let world = origin + right * vertex.uv_b.x + up * vertex.uv_b.y;
    out.world_position = vec4(world, 1.0);
    out.position = position_world_to_clip(world);
    out.world_normal = facing;
    out.uv = vertex.uv;
    out.uv_b = vertex.uv_b;
    // Linear in squared distance, quantized to a byte like the CPU vertex color.
    let alpha = floor(clamp((fade.x - distance_squared) / (fade.x - fade.y), 0.0, 1.0) * 255.0) / 255.0;
    out.color = vec4(vertex.color.rgb, vertex.color.a * alpha);
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // UnlitGeneric with $vertexcolor/$vertexalpha: texture (sRGB view) times vertex color.
    let color = textureSample(atlas, atlas_sampler, in.uv) * in.color;
    if color.a <= 0.0 {
        discard;
    }
    // FinalOutput with the HDR tone-map scale (camera exposure).
    return vec4(color.rgb * view.exposure, color.a);
}
