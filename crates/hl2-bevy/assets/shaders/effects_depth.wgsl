#import bevy_pbr::prepass_io::VertexOutput
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var effect_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var effect_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> parameters: vec4<f32>;
#ifdef MOTION_VECTOR_PREPASS
#import bevy_pbr::{prepass_io::FragmentOutput, mesh_view_bindings::view}
#import "shaders/object_motion.wgsl"::object_motion
#endif
@fragment
fn fragment(mesh: VertexOutput)
#ifdef MOTION_VECTOR_PREPASS
 -> FragmentOutput
#endif
{
    if parameters.x > 0.0 {
        var alpha = 1.0;
        if parameters.y <= 0.5 { alpha = textureSample(effect_texture,effect_sampler,mesh.uv).a; }
#ifdef VERTEX_COLORS
        alpha *= mesh.color.a;
#endif
        if alpha < parameters.x { discard; }
    }
#ifdef MOTION_VECTOR_PREPASS
    var out: FragmentOutput;
    out.motion_vector = object_motion(mesh.world_position,mesh.previous_world_position,view.unjittered_clip_from_world);
    return out;
#endif
}
