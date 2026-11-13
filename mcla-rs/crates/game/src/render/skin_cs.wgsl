// The figures' skinning on the GPU (D59): one invocation a vertex writes the same 14 words skin.rs `skin_vertex` makes
// on the CPU, into the skinned vertex buffer the figures are drawn from. The arithmetic follows glam's order step by
// step (Mat4::transform_point3 / transform_vector3: x axis * x, + y axis * y, + z axis * z, + w axis; Vec3::normalize_or:
// (x x + y y) + z z, sqrt, its reciprocal, the product; the octahedral map's division by (|x| + |y|) + |z|; Rust's
// signum, which is 1 for +0 and -1 for -0).

// a vertex: 5 vec4 of words - pos.xyz + colour, nrm.xyz + bones 0 / 1, tangent xyzw, weights, uv + bones 2 / 3
@group(0) @binding(0) var<storage, read> src: array<vec4<u32>>;
@group(0) @binding(1) var<storage, read> mats: array<mat4x4<f32>>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;

// one part: its first vertex in this dispatch, its count, where its source and its output start, its figure's matrices
struct Job {
    vstart: u32,
    count: u32,
    src_off: u32,
    dst_off: u32,
    mat_off: u32,
    // bit 0: the character programs' layout, bit 1: an alpha part
    flags: u32,
    fill: u32,
    pad: u32,
}
@group(0) @binding(3) var<storage, read> jobs: array<Job>;

struct Params {
    total: u32,
    njobs: u32,
    // always 0: the products and sums pass through an integer XOR with it, so no compiler can fuse a product into an
    // add (fma) or re-associate a chain of adds - the CPU rounds each operation on its own
    zero: u32,
    pad: u32,
}
@group(0) @binding(4) var<uniform> params: Params;

// a product rounded on its own (see `zero`)
fn mr(a: f32, b: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(a * b) ^ params.zero);
}

fn mv(a: vec4<f32>, b: f32) -> vec4<f32> {
    return bitcast<vec4<f32>>(bitcast<vec4<u32>>(a * b) ^ vec4<u32>(params.zero));
}

fn mv3(a: vec3<f32>, b: f32) -> vec3<f32> {
    return bitcast<vec3<f32>>(bitcast<vec3<u32>>(a * b) ^ vec3<u32>(params.zero));
}

// sums rounded on their own too: DXC compiles without IEEE strictness, and a driver may then re-associate a chain of adds
fn av(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    return bitcast<vec4<f32>>(bitcast<vec4<u32>>(a + b) ^ vec4<u32>(params.zero));
}

fn av3(a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    return bitcast<vec3<f32>>(bitcast<vec3<u32>>(a + b) ^ vec3<u32>(params.zero));
}

fn tp(m: mat4x4<f32>, p: vec3<f32>) -> vec3<f32> {
    var r = mv(m[0], p.x);
    r = av(mv(m[1], p.y), r);
    r = av(mv(m[2], p.z), r);
    r = av(m[3], r);
    return r.xyz;
}

fn tv(m: mat4x4<f32>, v: vec3<f32>) -> vec3<f32> {
    var r = mv(m[0], v.x);
    r = av(mv(m[1], v.y), r);
    r = av(mv(m[2], v.z), r);
    return r.xyz;
}

// The GPU's sqrt and division are not always correctly rounded as the CPU's are: each is the hardware's answer moved to
// the neighbouring float (two steps either way) whose exact residual is the smallest. The residual's product is exact
// by Dekker's split (only rounded products and sums, `mr`).
// a - b rounded on its own: no compiler may fold (c - (c - a)) to a
fn sr(a: f32, b: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(a - b) ^ params.zero);
}

fn ar(a: f32, b: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(a + b) ^ params.zero);
}

fn two_prod(a: f32, b: f32) -> vec2<f32> {
    let p = mr(a, b);
    let ca = mr(4097.0, a);
    let ah = sr(ca, sr(ca, a));
    let al = sr(a, ah);
    let cb = mr(4097.0, b);
    let bh = sr(cb, sr(cb, b));
    let bl = sr(b, bh);
    let e = ar(ar(ar(sr(mr(ah, bh), p), mr(ah, bl)), mr(al, bh)), mr(al, bl));
    return vec2<f32>(p, e);
}

fn finite(x: f32) -> bool {
    return (bitcast<u32>(x) & 0x7f800000u) != 0x7f800000u;
}

// a / b, rounded to nearest even
fn div_rn(a: f32, b: f32) -> f32 {
    let q0 = a / b;
    if (!finite(q0) || q0 == 0.0 || !finite(a) || !finite(b)) {
        return q0;
    }
    var best = q0;
    var br = 3.4028235e38;
    for (var k = -2; k <= 2; k = k + 1) {
        let c = bitcast<f32>(u32(i32(bitcast<u32>(q0)) + k));
        if (!finite(c)) {
            continue;
        }
        let t = two_prod(c, b);
        let r = abs(sr(sr(a, t.x), t.y));
        if (r < br || (r == br && (bitcast<u32>(c) & 1u) == 0u)) {
            br = r;
            best = c;
        }
    }
    return best;
}

// sqrt(d) for d >= 0, rounded to nearest (a square root is never half way between two floats)
fn sqrt_rn(d: f32) -> f32 {
    let s0 = sqrt(d);
    if (!(d > 0.0) || !finite(d)) {
        return s0;
    }
    var best = s0;
    var br = 3.4028235e38;
    for (var k = -2; k <= 2; k = k + 1) {
        let c = bitcast<f32>(u32(i32(bitcast<u32>(s0)) + k));
        let t = two_prod(c, c);
        let r = abs(sr(sr(d, t.x), t.y));
        if (r < br) {
            br = r;
            best = c;
        }
    }
    return best;
}

fn norm_or(v: vec3<f32>, f: vec3<f32>) -> vec3<f32> {
    let d = ar(ar(mr(v.x, v.x), mr(v.y, v.y)), mr(v.z, v.z));
    let r = div_rn(1.0, sqrt_rn(d));
    // (glam: the reciprocal finite and above 0)
    if (r == r && r <= 3.4028235e38 && r > 0.0) {
        return mv3(v, r);
    }
    return f;
}

fn signum(x: f32) -> f32 {
    return select(1.0, -1.0, (bitcast<u32>(x) >> 31u) == 1u);
}

fn oct(v: vec3<f32>) -> vec2<f32> {
    let s = ar(ar(abs(v.x), abs(v.y)), abs(v.z));
    let n = vec3<f32>(div_rn(v.x, s), div_rn(v.y, s), div_rn(v.z, s));
    if (n.z >= 0.0) {
        return n.xy;
    }
    return vec2<f32>(mr(sr(1.0, abs(n.y)), signum(n.x)), mr(sr(1.0, abs(n.x)), signum(n.y)));
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let g = gid.x;
    if (g >= params.total) {
        return;
    }
    // (the part this vertex is in: the last job starting at or before it)
    var lo = 0u;
    var hi = params.njobs;
    while (hi - lo > 1u) {
        let mid = (lo + hi) / 2u;
        if (jobs[mid].vstart <= g) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let job = jobs[lo];
    let i = g - job.vstart;
    let s = (job.src_off + i) * 5u;
    let a0 = src[s];
    let a1 = src[s + 1u];
    let tn = bitcast<vec4<f32>>(src[s + 2u]);
    let w = bitcast<vec4<f32>>(src[s + 3u]);
    let a4 = src[s + 4u];
    let p = bitcast<vec3<f32>>(a0.xyz);
    let nv = bitcast<vec3<f32>>(a1.xyz);
    var b = array<u32, 4>(a1.w & 0xffffu, a1.w >> 16u, a4.z & 0xffffu, a4.z >> 16u);
    var pos = vec3<f32>(0.0);
    var nrm = vec3<f32>(0.0);
    var tan = vec3<f32>(0.0);
    for (var k = 0u; k < 4u; k = k + 1u) {
        if (w[k] > 0.0) {
            let m = mats[job.mat_off + b[k]];
            pos = av3(pos, mv3(tp(m, p), w[k]));
            nrm = av3(nrm, mv3(tv(m, nv), w[k]));
            tan = av3(tan, mv3(tv(m, tn.xyz), w[k]));
        }
    }
    let chars = (job.flags & 1u) != 0u;
    let alpha = select(0.0, 1.0, (job.flags & 2u) != 0u);
    let n = norm_or(nrm, vec3<f32>(0.0, 1.0, 0.0));
    var uv1 = vec2<f32>(0.0, 77.0);
    var uv2y = 0.0;
    var col = 0xffffffffu;
    if (chars) {
        let o = oct(norm_or(tan, vec3<f32>(1.0, 0.0, 0.0)));
        var sgn = 0.0;
        if (tn.w < -0.5) {
            sgn = 4.0;
        } else if (tn.w < 0.5) {
            sgn = 8.0;
        }
        uv1 = vec2<f32>(ar(o.x, sgn), 77.0);
        uv2y = o.y;
        col = a0.w;
    }
    let o = (job.dst_off + i) * 14u;
    dst[o] = bitcast<u32>(pos.x);
    dst[o + 1u] = bitcast<u32>(pos.y);
    dst[o + 2u] = bitcast<u32>(pos.z);
    dst[o + 3u] = bitcast<u32>(n.x);
    dst[o + 4u] = bitcast<u32>(n.y);
    dst[o + 5u] = bitcast<u32>(n.z);
    dst[o + 6u] = a4.x;
    dst[o + 7u] = a4.y;
    dst[o + 8u] = bitcast<u32>(uv1.x);
    dst[o + 9u] = bitcast<u32>(uv1.y);
    dst[o + 10u] = col;
    dst[o + 11u] = job.fill;
    dst[o + 12u] = bitcast<u32>(alpha);
    dst[o + 13u] = bitcast<u32>(uv2y);
}
