// The night fast collector's blur (render/mod.rs `night_collector_blur`): rage_fastmipmap's PSBlur, four taps at the
// texture coordinate +- TexelSize (1 / 1024: half a texel of the 512 map), a quarter each, drawn with the console's
// pixel centres at the texels' corners - so the taps fall on the middles of texels i - 1 and i along each axis and
// each output texel is the mean of the 2 x 2 texels at and before it (the edge's texels for those past it).

@group(0) @binding(0) var t_src: texture_2d<f32>;

struct BlurOut {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn vs_blur(@builtin(vertex_index) i: u32) -> BlurOut {
    var o: BlurOut;
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    o.clip = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

@fragment
fn fs_blur(in: BlurOut) -> @location(0) vec4<f32> {
    let n = vec2<i32>(textureDimensions(t_src)) - vec2<i32>(1);
    let px = vec2<i32>(in.clip.xy);
    let a = clamp(px - vec2<i32>(1), vec2<i32>(0), n);
    let b = clamp(px, vec2<i32>(0), n);
    let s = textureLoad(t_src, vec2<i32>(a.x, a.y), 0) + textureLoad(t_src, vec2<i32>(b.x, a.y), 0)
        + textureLoad(t_src, vec2<i32>(a.x, b.y), 0) + textureLoad(t_src, vec2<i32>(b.x, b.y), 0);
    return s * 0.25;
}
