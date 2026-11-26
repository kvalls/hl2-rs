import package::shaders::source_uniforms::material;
// Source alpha-tested silhouettes in the flashlight's depth map. Bevy's default
// prepass vertex path provides the same posed GPU skinning and UVs as the colour draw.
import bevy_pbr::prepass::io::VertexOutput;
@if(MOTION_VECTOR_PREPASS)
import bevy_pbr::{prepass::io::FragmentOutput, prepass::bindings::previous_view_uniforms, render::mesh_view_bindings::view};
@if(MOTION_VECTOR_PREPASS)
import package::shaders::object_motion::object_motion;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var base_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var base_sampler: sampler;

// The material's alpha test (parameters.x the cutoff, y > 0.5 ignores the texture's alpha).
fn alpha_test(mesh: VertexOutput) {
    if material.parameters.x > 0.0 {
        let alpha = select(textureSample(base_texture, base_sampler, mesh.uv).a, 1.0, material.parameters.y > 0.5);
        if alpha * material.tint.a < material.parameters.x { discard; }
    }
}

@if(MOTION_VECTOR_PREPASS)
@fragment
fn fragment(mesh: VertexOutput) -> FragmentOutput {
    alpha_test(mesh);
    var out: FragmentOutput;
    if material.lighting.params.w > 1.5 {
        // Viewmodel draw space keeps the a8 camera+object channel unchanged.
        let current = view.unjittered_clip_from_world * mesh.world_position;
        let previous = previous_view_uniforms.clip_from_world * mesh.previous_world_position;
        out.motion_vector = (current.xy/current.w-previous.xy/previous.w)*vec2(0.5,-0.5);
    } else {
        out.motion_vector = object_motion(mesh.world_position,mesh.previous_world_position,view.unjittered_clip_from_world);
    }
    return out;
}

@else
@fragment
fn fragment(mesh: VertexOutput) {
    alpha_test(mesh);
}
