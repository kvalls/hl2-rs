import bevy_pbr::prepass::io::VertexOutput;
@if(MOTION_VECTOR_PREPASS)
import bevy_pbr::{prepass::io::FragmentOutput, render::mesh_view_bindings::view};
@if(MOTION_VECTOR_PREPASS)
import package::shaders::object_motion::object_motion;
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var effect_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var effect_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var<uniform> parameters: vec4<f32>;

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
