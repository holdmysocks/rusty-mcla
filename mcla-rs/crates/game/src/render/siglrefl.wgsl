// The signal lamps' glow rectangles in the road's reflection picture (siglrefl.rs): the original's
// VS_PassColorThrough + PSSeedRTNoZ (the executable's postfx effect, rgxa container at 0x8281BB00),
// one rect-list draw of 3 vertices a lamp (sub_822FBF00 -> sub_822FA0F0 -> sub_82194B58 / sub_8218F138).
//   VS_PassColorThrough: oPos = the vertex's position as it is (already in clip space, w = 1),
//     o0 = (u, v, z), o2 = the vertex colour.
//   PSSeedRTNoZ: o.rgb = tex(LightGlowTexSampler = __lightglowmap = textures/lightglow.dds, uv).rgb
//     x StreakParams.y x colour.rgb;  o.a = colour.a.   StreakParams = (0.5, I / 4).
// Blend one / one (0x01010101), depth test on without writes (0x00708763: GEQUAL on the
// original's reversed depth - the port's reflection depth is reversed too), no cull.

struct Rect {
    @location(0) r: vec4<f32>,    // x0 y0 x1 y1 (NDC)
    @location(1) z: vec4<f32>,    // depth, StreakParams.y, u at x0, v at y0
    @location(2) col: vec4<f32>,  // the vertex colour
};

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) col: vec4<f32>,
    @location(2) k: f32,
};

@group(0) @binding(0) var t_glow: texture_2d<f32>;
@group(0) @binding(1) var s_glow: sampler;

@vertex
fn vs_siglrefl(@builtin(vertex_index) vi: u32, r: Rect) -> VOut {
    // (the rectangle's corners (x0, y0) (x1, y0) (x0, y1) and the fourth (x1, y1) the rect list adds)
    var cx = array<u32, 6>(0u, 1u, 0u, 1u, 1u, 0u);
    var cy = array<u32, 6>(0u, 0u, 1u, 0u, 1u, 1u);
    let ax = cx[vi];
    let ay = cy[vi];
    var o: VOut;
    o.pos = vec4<f32>(select(r.r.x, r.r.z, ax == 1u), select(r.r.y, r.r.w, ay == 1u), r.z.x, 1.0);
    o.uv = vec2<f32>(select(r.z.z, 1.0 - r.z.z, ax == 1u), select(r.z.w, 1.0 - r.z.w, ay == 1u));
    o.col = r.col;
    o.k = r.z.y;
    return o;
}

@fragment
fn fs_siglrefl(in: VOut) -> @location(0) vec4<f32> {
    let t = textureSample(t_glow, s_glow, in.uv).rgb;
    return vec4<f32>(t * in.k * in.col.rgb, in.col.a);
}
