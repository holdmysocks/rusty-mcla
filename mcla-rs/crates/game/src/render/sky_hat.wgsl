// The game's own sky dome ("Skyhat"). The two programs in xs_sky.wgsl (appended to this file)
// are the disc's microcode, translated instruction by instruction; this wrapper feeds them:
// the vertex program is run per pixel on the dome point the view ray meets (the game runs it
// on the vertices of a dome mesh and interpolates), then the pixel program on its outputs.

struct Globals {
    view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    ambient_sky: vec4<f32>,
    ambient_ground: vec4<f32>,
    fog_color: vec4<f32>,
    params: vec4<f32>,
    inv_view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var t_stars: texture_2d<f32>;
@group(0) @binding(16) var s_rep: sampler;
@group(1) @binding(2) var t_galaxy: texture_2d<f32>;
@group(1) @binding(3) var t_perlin: texture_2d<f32>;
@group(1) @binding(4) var t_detail: texture_2d<f32>;

struct Hat {
    vs: array<vec4<f32>, 12>,       // c64 c65 c66 c67 c68 c69 c72 c73 c75 c96 c97 c98
    ps: array<vec4<f32>, 24>,       // c64 .. c83, c100 .. c103
    perlin: array<vec4<f32>, 10>,
    pad: array<vec4<f32>, 2>,
};
@group(2) @binding(0) var<uniform> hat: Hat;

struct XsVsOut {
    pos: vec4<f32>,
    o: array<vec4<f32>, 16>,
};

var<private> xs_c: array<vec4<f32>, 256>;

// The pixel program is ps_main_withstarfield: the game draws the dome twice, the variant with
// the moon only inside a stencilled quad round the moon and this one over the rest of the sky
// (in every recording the moon lies below the horizon, MoonPosition (15.6, -15.4, 5.6)).
// StarFieldSampler, GalaxySampler, PerlinNoiseSampler (the Perlin pass's target: no mips),
// HighDetailNoiseSampler - all wrapped
fn xs_s0(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(t_stars, s_rep, uv);
}
fn xs_s1(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(t_galaxy, s_rep, uv, 0.0);
}
fn xs_s2(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(t_perlin, s_rep, uv, 0.0);
}
fn xs_s3(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(t_detail, s_rep, uv);
}

// (ps_dpenvmapsky, the dome in the cars' environment map: the Perlin target and the detail noise)
fn xs_envsky_s0(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(t_perlin, s_rep, uv, 0.0);
}
fn xs_envsky_s1(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(t_detail, s_rep, uv, 0.0);
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_hat(@builtin(vertex_index) i: u32) -> VsOut {
    var o: VsOut;
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    o.clip = vec4<f32>(p, 0.0, 1.0); // reverse-Z: 0 = far plane
    o.ndc = p;
    return o;
}

// vs_main's outputs on the dome point a view ray meets
fn hat_vs(ndc: vec2<f32>) -> XsVsOut {
    let p0 = g.inv_view_proj * vec4<f32>(ndc, 0.5, 1.0);
    var d = normalize(p0.xyz / p0.w - g.cam_pos.xyz);
    // (below the horizon the game shows another model, the ground dome; here the dome's rim)
    d.y = max(d.y, 0.0);
    // The dome (the disc's sky mesh, read out of its file): half an ellipsoid 200 across and
    // 163.67 high - 16 rings of 32 - which the world matrix's scale (5000, 1000, 5000) flattens
    // to a sixth of its width, so most of its rings lie within 20 degrees of the horizon. Its
    // texture coordinates lie flat over it, (0.5 + 0.444 x / 200, 0.5 + 0.444 z / 200), with
    // the lowest five rings pulled outwards to reach the texture's edge at the rim.
    // The mesh point the view ray meets:
    let q = vec3<f32>(d.x * 0.2, d.y, d.z * 0.2);
    let m = q * inverseSqrt((q.x * q.x + q.z * q.z) / 40000.0 + q.y * q.y / 26787.9);
    let rho = length(m.xz) / 200.0;
    var uvr = 0.888 * rho;
    if (rho > 0.8819) {
        // (ring radii / 200 and their texture radii)
        var ring = array<vec2<f32>, 6>(vec2<f32>(0.8819, 0.7828), vec2<f32>(0.9239, 0.8289), vec2<f32>(0.9569, 0.8709), vec2<f32>(0.9808, 0.9116), vec2<f32>(0.9952, 0.9534), vec2<f32>(1.0, 1.0));
        for (var i = 0; i < 5; i = i + 1) {
            if (rho >= ring[i].x) {
                uvr = mix(ring[i].y, ring[i + 1].y, clamp((rho - ring[i].x) / (ring[i + 1].x - ring[i].x), 0.0, 1.0));
            }
        }
    }
    let flat_dir = m.xz / max(length(m.xz), 0.0001);
    var vin: array<vec4<f32>, 16>;
    vin[0] = vec4<f32>(m, 1.0);
    // (v runs WITH the world's z. Until 2026-10-14 this was 0.5 - ...: the clouds lay mirrored north
    // to south, which the one camera at the garage gate - a strip of low sky - did not show. At
    // Selma / Highland, looking south-west at noon with the capture's phase, the original has
    // its cloud bank right of the tower and the port had one on the left: sky cells +19 / +17 /
    // -10; with the sign turned the banks and wisps lie where the original's do, cells within 2;
    // 09:00 and 17:30 likewise. MCLA_DEBUG_VIEW=114: mirrored as before.)
    vin[1] = vec4<f32>(0.5 + 0.5 * uvr * flat_dir.x, 0.5 + 0.5 * uvr * flat_dir.y, 0.0, 0.0);
    if (g.params.w > 113.5 && g.params.w < 114.5) { vin[1].y = 1.0 - vin[1].y; }
    // vs_main's constants: the world matrix (c0 .. c3; the program removes the translation),
    // world x view x projection (c8 .. c11: only the position, which is not used here)
    xs_c[0] = vec4<f32>(5000.0, 0.0, 0.0, 0.0);
    xs_c[1] = vec4<f32>(0.0, 1000.0, 0.0, 0.0);
    xs_c[2] = vec4<f32>(0.0, 0.0, 5000.0, 0.0);
    xs_c[3] = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    xs_c[8] = vec4<f32>(1.0, 0.0, 0.0, 0.0);
    xs_c[9] = vec4<f32>(0.0, 1.0, 0.0, 0.0);
    xs_c[10] = vec4<f32>(0.0, 0.0, 1.0, 0.0);
    xs_c[11] = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    xs_c[64] = hat.vs[0];
    xs_c[65] = hat.vs[1];
    xs_c[66] = hat.vs[2];
    xs_c[67] = hat.vs[3];
    xs_c[68] = hat.vs[4];
    xs_c[69] = hat.vs[5];
    xs_c[72] = hat.vs[6];
    xs_c[73] = hat.vs[7];
    xs_c[75] = hat.vs[8];
    xs_c[96] = hat.vs[9];
    xs_c[97] = hat.vs[10];
    xs_c[98] = hat.vs[11];
    return xs_sky_vs(vin);
}

@fragment
fn fs_hat(in: VsOut) -> @location(0) vec4<f32> {
    let vs = hat_vs(in.ndc);
    // (hat.ps holds the constants of the variant with the moon, c64 .. c83 and c100 .. c103;
    // this program's c64 .. c80 are those without the moon's: c67, c79, c80, c82, c83, c100, c101)
    // (written out, constant places only: filled by a loop the whole xs_c array had to live in memory on drivers that
    // do not unroll it, and every constant read of the program was a memory read - the sky alone 46 ms a frame at
    // 1440p on an RTX 5060, GitHub #12; docs/PERFORMANCE.md 2026-10-28)
    xs_c[64] = hat.ps[0];
    xs_c[65] = hat.ps[1];
    xs_c[66] = hat.ps[2];
    xs_c[67] = hat.ps[4];
    xs_c[68] = hat.ps[5];
    xs_c[69] = hat.ps[6];
    xs_c[70] = hat.ps[7];
    xs_c[71] = hat.ps[8];
    xs_c[72] = hat.ps[9];
    xs_c[73] = hat.ps[10];
    xs_c[74] = hat.ps[11];
    xs_c[75] = hat.ps[12];
    xs_c[76] = hat.ps[13];
    xs_c[77] = hat.ps[14];
    xs_c[78] = hat.ps[17];
    xs_c[79] = hat.ps[22];
    xs_c[80] = hat.ps[23];
    let c = xs_sky_ps(vs.o);
    return vec4<f32>(max(c.rgb, vec3<f32>(0.0)), 1.0);
}

// The dome in the cars' environment map. The game does not draw the dome mesh there but one
// full-screen pass over its dual paraboloid map, ps_dpenvmapsky, which makes the direction out
// of the pixel's place itself (and lays its clouds in the map's own space). Its constants are
// not the main program's: HDRExposure 1.14 against 0.8 and HDRSunExposure 12.9 against 0.1 (the
// map's sky is 1.4 times the screen's, its sun far stronger), its own SunCentre (-226.27,
// 227.27, 11.30), sun size 0.19, AzimuthHeight 1.345, and the sun's colour held to a length of
// 0.017 - the same at every hour recorded. Here the map is a cube: each pixel's direction is
// turned into the place the program would find it at (the inverse of its first 28
// instructions), and the program run from there.
@fragment
fn fs_hat_env(in: VsOut) -> @location(0) vec4<f32> {
    let p0 = g.inv_view_proj * vec4<f32>(in.ndc, 0.5, 1.0);
    let d = normalize(p0.xyz / p0.w - g.cam_pos.xyz);
    // the map's basis (dpMapBasisMtx c48 .. c50): the world's axes with z turned round
    let dl = vec3<f32>(d.x, d.y, -d.z);
    var uv: vec2<f32>;
    if (dl.z <= 0.0) {
        // the half at u > 0.5: direction = (-2 X, -2 Y, R2 - 1) / (1 + R2), X = 4 u - 3, Y = 2 v - 1
        let k = 1.0 / max(1.0 - dl.z, 0.0001);
        uv = vec2<f32>((3.0 - dl.x * k) * 0.25, (1.0 - dl.y * k) * 0.5);
    } else {
        // the half at u <= 0.5: direction = (2 X, -2 Y, 1 - R2) / (1 + R2), X = 4 u - 1
        let k = 1.0 / max(1.0 + dl.z, 0.0001);
        uv = vec2<f32>((1.0 + dl.x * k) * 0.25, (1.0 - dl.y * k) * 0.5);
    }
    xs_c[15] = vec4<f32>(g.cam_pos.xyz, 1.0);
    xs_c[48] = vec4<f32>(1.0, 0.0, 0.0, 0.0);
    xs_c[49] = vec4<f32>(0.0, 1.0, 0.0, 0.0);
    xs_c[50] = vec4<f32>(0.0, 0.0, -1.0, 0.0);
    xs_c[51] = vec4<f32>(g.cam_pos.xyz, 1.0);
    xs_c[64] = vec4<f32>(-226.272, 227.272, 11.296, 0.0);
    xs_c[65] = hat.ps[1];
    xs_c[66] = hat.vs[6];
    xs_c[67] = hat.vs[8];
    let sc = hat.ps[2].xyz;
    xs_c[68] = vec4<f32>(sc * min(1.0, 0.017 / max(length(sc), 0.000001)), 0.0);
    xs_c[69] = hat.vs[9];
    xs_c[70] = vec4<f32>(1.345, 0.0, 0.0, 0.0);
    xs_c[71] = hat.ps[4];
    xs_c[72] = hat.ps[5];
    xs_c[73] = hat.ps[6];
    xs_c[74] = hat.ps[7];
    xs_c[75] = hat.ps[8];
    xs_c[76] = hat.ps[9];
    xs_c[77] = hat.ps[10];
    xs_c[78] = hat.ps[11];
    xs_c[79] = hat.vs[1];
    xs_c[80] = hat.ps[13];
    xs_c[81] = vec4<f32>(0.0, 0.19, 0.0, 0.0);
    xs_c[82] = vec4<f32>(1.14, 0.0, 0.0, 0.0);
    xs_c[83] = vec4<f32>(12.9, 0.0, 0.0, 0.0);
    xs_c[100] = vec4<f32>(32.0, 32.0, 32.0, 0.0);
    var rin: array<vec4<f32>, 16>;
    rin[0] = vec4<f32>(uv, 0.0, 0.0);
    let c = xs_sky_env_ps(rin);
    return vec4<f32>(max(c.rgb, vec3<f32>(0.0)), 1.0);
}
