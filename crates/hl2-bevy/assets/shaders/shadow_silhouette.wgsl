// Projected caster triangles, in the viewport of their atlas cell.
struct Projected {
    @builtin(position) p: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) opacity: f32,
}
@vertex fn vs(@location(0) xy: vec2<f32>, @location(1) uv: vec2<f32>,
              @location(2) opacity: f32) -> Projected {
    return Projected(vec4<f32>(xy, 0.0, 1.0), uv, opacity);
}
@group(0) @binding(0) var alpha: texture_2d<f32>;
@fragment fn fs(v: Projected) -> @location(0) vec4<f32> {
    let xy = vec2<i32>(fract(v.uv) * vec2<f32>(textureDimensions(alpha)));
    return vec4<f32>(textureLoad(alpha, xy, 0).a * v.opacity, 0.0, 0.0, 1.0);
}
