// The environment cube's mip levels as the original builds its map's (PSFastMipMap of embedded:/rage_fastmipmap, loaded by F-58842; draws 347 - 349 of
// the env chain: levels 1 .. 3 of the 512 x 256 map, each from the level above): four bilinear taps at +-0.5 of the
// DESTINATION's texel (TexelSize c88 = 1 / the destination's size, x 0.5) round its centre, x 0.25. Each tap lands on a
// corner between source texels, so a level is a centred 4 x 4 box of the one above (texels 2i - 1 .. 2i + 2) - read from
// the running original at noon: its level 1 against the box of its level 0, mean abs error 0.33 % (a 2 x 2 box 2.5 %,
// the box shifted a texel 2.4 - 3.8 %; work/police17). Here per cube face, clamped at the face's edge.

@group(0) @binding(0) var t_src: texture_2d<f32>;
@group(0) @binding(1) var s_src: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_mip(@builtin(vertex_index) vi: u32) -> VsOut {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    return out;
}

@fragment
fn fs_mip(in: VsOut) -> @location(0) vec4<f32> {
    // (half the destination's texel = one source texel)
    let h = 1.0 / vec2<f32>(textureDimensions(t_src));
    let a = textureSampleLevel(t_src, s_src, in.uv + vec2<f32>(-h.x, -h.y), 0.0);
    let b = textureSampleLevel(t_src, s_src, in.uv + vec2<f32>(h.x, -h.y), 0.0);
    let c = textureSampleLevel(t_src, s_src, in.uv + vec2<f32>(h.x, h.y), 0.0);
    let d = textureSampleLevel(t_src, s_src, in.uv + vec2<f32>(-h.x, h.y), 0.0);
    return (a + b + c + d) * 0.25;
}
