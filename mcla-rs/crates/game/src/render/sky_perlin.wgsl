// The Perlin noise pass that feeds the sky dome's clouds: the game draws one quad into a
// 256 x 256 target each frame with the pixel program in xs_perlin.wgsl (appended to this file),
// reading perlinnoise_small.dds twelve times.

struct Hat {
    vs: array<vec4<f32>, 12>,
    ps: array<vec4<f32>, 24>,
    perlin: array<vec4<f32>, 10>,   // c64 .. c73
    pad: array<vec4<f32>, 2>,
};
@group(0) @binding(0) var<uniform> hat: Hat;
@group(1) @binding(0) var t_noise1: texture_2d<f32>;
@group(0) @binding(1) var s_rep: sampler;

var<private> xs_c: array<vec4<f32>, 256>;

// (Noise1Sampler: a one-channel texture, wrapped, with its mip chain)
fn xs_s0(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(t_noise1, s_rep, uv).rrrr;
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_perlin(@builtin(vertex_index) i: u32) -> VsOut {
    var o: VsOut;
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    o.clip = vec4<f32>(p, 0.0, 1.0);
    // (the game's quad: uv (0, 0) at the target's top left)
    o.uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    return o;
}

@fragment
fn fs_perlin(in: VsOut) -> @location(0) vec4<f32> {
    for (var i = 0; i < 10; i = i + 1) {
        xs_c[64 + i] = hat.perlin[i];
    }
    var rin: array<vec4<f32>, 16>;
    rin[0] = vec4<f32>(in.uv, 0.0, 0.0);
    return xs_perlin_ps(rin);
}

