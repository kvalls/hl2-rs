// Host-owned motion channel: object motion under the current camera, not Bevy's
// camera+object convention. Only our Advanced filters consume these material vectors.
fn object_motion(current: vec4<f32>, previous: vec4<f32>, clip: mat4x4<f32>) -> vec2<f32> {
    if all(current == previous) { return vec2(0.0); }
    let a = clip*current;
    let b = clip*previous;
    if a.w <= 1e-8 || b.w <= 1e-8 { return vec2(0.0); }
    let v = (a.xy/a.w-b.xy/b.w)*vec2(0.5,-0.5);
    // The reconstruction caps reach far below one screen; keep near-plane crossings
    // finite in the half-float motion target without changing their direction.
    return v*min(1.0,1.0/max(length(v),1e-8));
}
