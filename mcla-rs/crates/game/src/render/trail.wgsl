// The lamps' light trails (trail.rs): the original's VS_PassColorThroughXForm + PSTrailSeedRT, blend
// one / one into the halo picture after the glows. The vertices are in view space (the original draws
// them with world = view = identity), the program's CentrePosition is the trail's newest point.
// Translated from the microcode:
//   o1 = the centre in clip space as (0.5 (x + w), 0.5 (w - y), w, w): its place on the screen in uv
//   s = the depth map at the centre; the ribbon's pixel is drawn only where s > 1 - z and z > 0 (z its
//       own depth; the original's depth map holds 1 - z, so: where the scene at the centre lies before
//       the ribbon's pixel). The port's depth is reversed (1 near, 0 far): scene > z and z < 1.
//   a = alpha x (1 - sat(|uv x 2 - 1|))^2 x sat(1 - w / 150) (w the centre's distance)
//   out = sat(a x rgb - 0.05)

struct U {
    proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var depth: texture_depth_2d;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) z: f32,
    @location(2) @interpolate(flat) centre: vec4<f32>,
    @location(3) col: vec4<f32>,
};

@vertex
fn vs_trail(@location(0) pos: vec4<f32>, @location(1) uv: vec4<f32>, @location(2) col: vec4<f32>, @location(3) centre: vec4<f32>) -> VsOut {
    var o: VsOut;
    let c = u.proj * vec4<f32>(pos.xyz, 1.0);
    o.clip = c;
    o.uv = uv.xy;
    o.z = c.z / c.w;
    let m = u.proj * vec4<f32>(centre.xyz, 1.0);
    o.centre = vec4<f32>(0.5 * (m.x + m.w), 0.5 * (m.w - m.y), m.w, m.w);
    o.col = col;
    return o;
}

@fragment
fn fs_trail(in: VsOut) -> @location(0) vec4<f32> {
    let cuv = in.centre.xy / in.centre.w;
    let dims = vec2<i32>(textureDimensions(depth));
    let p = clamp(vec2<i32>(cuv * vec2<f32>(dims)), vec2<i32>(0), dims - vec2<i32>(1));
    let s = textureLoad(depth, p, 0);
    if (!(s > in.z && in.z < 1.0)) {
        discard;
    }
    let q = in.uv * 2.0 - vec2<f32>(1.0);
    let r = clamp(sqrt(abs(dot(q, q))), 0.0, 1.0);
    let a = in.col.a * (1.0 - r) * (1.0 - r) * clamp(1.0 - in.centre.z * 0.0066666668, 0.0, 1.0);
    return vec4<f32>(clamp(a * in.col.rgb - vec3<f32>(0.05), vec3<f32>(0.0), vec3<f32>(1.0)), 0.0);
}
