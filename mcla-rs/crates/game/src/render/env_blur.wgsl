// The blurred copy of the cars' environment map (the original shrinks its 512 x 256 dual
// paraboloid map to 64 x 32 and blurs it, PS_BlurHoriz / PS_BlurVert; the paint's soft specular
// reads it, dpSpecFrontSampler). Here: a 16 x 16 cube whose every texel is what that blur gives
// at the texel's direction - the two passes' taps laid out in the dual paraboloid map's own
// space, each looked up in the sharp cube along the direction the map holds there (fs_blur).

@group(0) @binding(0) var t_env: texture_cube<f32>;
@group(0) @binding(1) var s_env: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) face: u32,
};

@vertex
fn vs_blur(@builtin(vertex_index) vi: u32, @builtin(instance_index) face: u32) -> VsOut {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    // (texture coordinates of the face, v down)
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    out.face = face;
    return out;
}

// the direction a cube map's sampler maps to this texel of this face
fn face_dir(face: u32, uv: vec2<f32>) -> vec3<f32> {
    let s = uv.x * 2.0 - 1.0;
    let t = uv.y * 2.0 - 1.0;
    switch face {
        case 0u { return vec3<f32>(1.0, -t, -s); }
        case 1u { return vec3<f32>(-1.0, -t, s); }
        case 2u { return vec3<f32>(s, 1.0, t); }
        case 3u { return vec3<f32>(s, -1.0, -t); }
        case 4u { return vec3<f32>(s, -t, 1.0); }
        default { return vec3<f32>(-s, -t, -1.0); }
    }
}

// A direction's place in the original's dual paraboloid map (512 x 256: two halves side by
// side, +z of the map's own space on the left half, -z on the right, "up" at the top edge's
// middle of each; the map's z is the world's turned round) and back - the inverse of the
// lookup in PS_CarPaint's first instructions.
fn dp_uv(d: vec3<f32>) -> vec2<f32> {
    let m = vec3<f32>(d.x, d.y, -d.z);
    if (m.z >= 0.0) {
        let k = 1.0 / (1.0 + m.z);
        return vec2<f32>((m.x * k + 1.0) * 0.25, (-m.y * k + 1.0) * 0.5);
    }
    let k = 1.0 / (1.0 - m.z);
    return vec2<f32>((-m.x * k + 3.0) * 0.25, (-m.y * k + 1.0) * 0.5);
}

fn dp_dir(uv: vec2<f32>) -> vec3<f32> {
    let y = 2.0 * uv.y - 1.0;
    if (uv.x > 0.5) {
        let x = 4.0 * uv.x - 3.0;
        let r2 = x * x + y * y;
        return vec3<f32>(-2.0 * x, -2.0 * y, 1.0 - r2) / (1.0 + r2);
    }
    let x = 4.0 * uv.x - 1.0;
    let r2 = x * x + y * y;
    return vec3<f32>(2.0 * x, -2.0 * y, r2 - 1.0) / (1.0 + r2);
}

// One texel of the original's 64 x 32 map. It is NOT the mean of an 8 x 8 block of the 512 x 256
// map: the draw that shrinks it (VS_Blit / PS_Blit between PSFastMipMap and PS_BlurHoriz) reads
// the big map with a sampler whose mip filter is "base level only" (fetch word 3 = 0x01280d11:
// mag linear, min linear, mip 2; levels 0 .. 3 allowed but never taken), so a texel is one
// bilinear read at the block's middle - the mean of its FOUR centre texels (8X + 3, 8X + 4).
// In numpy from the original's own sharp dumps this gives its soft map to 0.5 - 1.3 % (the
// lid's texel 0.997 .. 1.013 at four hours); with the 8 x 8 mean the lid's texel came out 4 - 11 %
// too bright (LIGHTING_WORK.md, "The soft map's real chain").
fn small_texel(x: i32, y: i32) -> vec3<f32> {
    let xx = f32(((x % 64) + 64) % 64) * 8.0;
    let yy = f32(clamp(y, 0, 31)) * 8.0;
    var s = vec3<f32>(0.0);
    for (var b = 0; b < 2; b = b + 1) {
        for (var a = 0; a < 2; a = a + 1) {
            let uv = vec2<f32>((xx + 3.5 + f32(a)) / 512.0, (yy + 3.5 + f32(b)) / 256.0);
            // (the original's map holds no more than about 2.3 anywhere at noon - its sky has no
            // sun disc of the main sky's strength; the reads are held to that)
            // (a texel that is not a number - NaN or infinite - counts as 0: tested on the bits, which a
            // "no NaNs" compiler cannot fold away; GitHub #14)
            let t = textureSampleLevel(t_env, s_env, dp_dir(uv), 0.0).rgb;
            let bad = (bitcast<vec3<u32>>(t) & vec3<u32>(0x7f800000u)) == vec3<u32>(0x7f800000u);
            s = s + min(select(t, vec3<f32>(0.0), bad), vec3<f32>(2.3));
        }
    }
    return s * 0.25;
}

// One pass's weights on whole texels: the centre x 0.1 and ten pairs of taps HALF a texel apart
// weighing 0.1, 0.09 .. 0.01, each tap between two texels read bilinearly - together 0.2 on the
// texel itself and 0.22 - 0.04 |m| on the m-th neighbour (m = 1 .. 5), 1.2 in all.
fn pass_weight(m: i32) -> f32 {
    let a = abs(m);
    if (a == 0) { return 0.2; }
    if (a > 5) { return 0.0; }
    return 0.22 - 0.04 * f32(a);
}

// PS_BlurHoriz, then PS_BlurVert, on the 64 x 32 map (recorded constants, the same at every
// hour): the centre x 0.1 and ten pairs of taps HALF a texel apart (c74 ..: k / 128 across, k /
// 64 down) weighing 0.1, 0.09 .. 0.01 (c64 ..) - 1.2 a pass, 1.44 in all, not normalised. The
// map's sampler wraps across and clamps down. The result is taken where the paint would read it:
// bilinearly between the four blurred texels round this direction's place in the map.
@fragment
fn fs_blur(in: VsOut) -> @location(0) vec4<f32> {
    let d = normalize(face_dir(in.face, in.uv));
    let c = dp_uv(d);
    let fx = c.x * 64.0 - 0.5;
    let fy = clamp(c.y * 32.0 - 0.5, 0.0, 31.0);
    let x0 = i32(floor(fx));
    let y0 = i32(floor(fy));
    let tx = fx - f32(x0);
    let ty = fy - f32(y0);
    var sum = vec3<f32>(0.0);
    for (var j = -5; j <= 6; j = j + 1) {
        let wj = (1.0 - ty) * pass_weight(j) + ty * pass_weight(j - 1);
        // (the down pass clamps each of its taps, so a row index past the edge is the edge row
        // of the across-blurred map; the second of the two blurred rows likewise)
        for (var i = -5; i <= 6; i = i + 1) {
            let wi = (1.0 - tx) * pass_weight(i) + tx * pass_weight(i - 1);
            sum = sum + small_texel(x0 + i, y0 + j) * (wi * wj);
        }
    }
    return vec4<f32>(sum, 1.0);
}
