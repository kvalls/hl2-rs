import bevy_pbr::{prepass::bindings::previous_view_uniforms, render::mesh_view_bindings::view};

// Host-owned motion channel: object motion under the current camera, not Bevy's
// camera+object convention. Only our Advanced filters consume these material vectors.
// Camera rotation belongs to the camera blur and camera translation is never blurred (retail's
// forward blur is off), so the camera's move since the last frame is taken out: an object that
// moves with the camera, such as a prop carried while walking, holds still on screen and gets no
// motion.
fn object_motion(current: vec4<f32>, previous: vec4<f32>, clip: mat4x4<f32>) -> vec2<f32> {
    if all(current == previous) { return vec2(0.0); }
    let a = clip*current;
    let b = clip*(previous+vec4(camera_shift(),0.0));
    if a.w <= 1e-8 || b.w <= 1e-8 { return vec2(0.0); }
    let v = (a.xy/a.w-b.xy/b.w)*vec2(0.5,-0.5);
    // The reconstruction caps reach far below one screen; keep near-plane crossings
    // finite in the half-float motion target without changing their direction.
    return v*min(1.0,1.0/max(length(v),1e-8));
}

// The camera's world move since the previous frame (its position from the previous view matrix).
fn camera_shift() -> vec3<f32> {
    let m = previous_view_uniforms.view_from_world;
    let rotation = mat3x3<f32>(m[0].xyz,m[1].xyz,m[2].xyz);
    return view.world_position-(-(transpose(rotation)*m[3].xyz));
}
