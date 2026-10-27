// The mini sky: the 128 x 128 picture of the sky the window panes mirror (SkyReflectionSampler
// `__miniskyrt__` of PSCityWindowUpper / Lower / LOD, PSPropGlass, PSCityGrimeMetal, PSPropMetal).
// The game fills it every frame, at every hour, with ONE quad right after the Perlin pass:
// vs_MiniSky + ps_miniSky (effect city/0x29901476) - the program in xs_minisky.wgsl, appended to
// this file - into an 8:8:8:8 target with the console's gamma curve, no blending, no blur, no mips.
// The program makes a direction out of the texel's place itself: p = 2 uv - 1, rho = |p|,
// direction = normalize(p rho, 1 - rho) (x, z across, y up) - so the panes' lookup at
// 0.5 + 0.47 R.xz is a mapping of the game's own, not the mirrored ray's true direction. It
// lays the clouds in the map's own space (the Perlin target read at uv, the detail noise at
// 3 uv), adds the sun's disc, and runs on the screen dome's constants of the frame (HDRExposure
// 0.8, HDRSunExposure 0.1). Measured against the map read out of the running game at clear noon
// (work/lighting/ms/t2.py, 10,700 cloud-free texels): mean error 0.02 - 0.19 of 255 when texel
// (i, j) is drawn at uv = (i, j) / 128 (the console's pixel centres: no half texel) and the
// byte is cut off, not rounded.

struct Hat {
    vs: array<vec4<f32>, 12>,       // c64 c65 c66 c67 c68 c69 c72 c73 c75 c96 c97 c98
    ps: array<vec4<f32>, 24>,       // c64 .. c83, c100 .. c103
    perlin: array<vec4<f32>, 10>,
    clamp: vec4<f32>,               // c100 HDRExposureClamp
    pad: vec4<f32>,
};
@group(0) @binding(0) var<uniform> hat: Hat;
@group(0) @binding(1) var s_rep: sampler;
@group(1) @binding(3) var t_perlin: texture_2d<f32>;
@group(1) @binding(4) var t_detail: texture_2d<f32>;

var<private> xs_c: array<vec4<f32>, 256>;

// PerlinNoiseSampler (the Perlin pass's target: wrapped, no mips), HighDetailNoiseSampler
// (wrapped, with its mip chain)
fn xs_mini_s0(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(t_perlin, s_rep, uv, 0.0);
}
fn xs_mini_s1(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(t_detail, s_rep, uv);
}

@vertex
fn vs_mini(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    return vec4<f32>(p, 0.0, 1.0);
}

// linear 0 .. 1 -> the console's gamma byte (the 8888_GAMMA target's write), cut off
fn pwl_encode_floor(c: vec3<f32>) -> vec3<f32> {
    let v = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)) * 1023.0;
    let e = select(select(select(v, 64.0 + (v - 64.0) * 0.5, v >= vec3<f32>(64.0)), 96.0 + (v - 128.0) * 0.25, v >= vec3<f32>(128.0)), 192.0 + (v - 512.0) * 0.125, v >= vec3<f32>(512.0));
    return floor(e + vec3<f32>(0.0001));
}

@fragment
fn fs_mini(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    // ps_miniSky's constants are the screen dome's of the frame (laid side by side with the
    // captures' at 05:45, 12:00 and stormy noon: every register equals one of vs_main's or
    // ps_main_withstarfield_withmoon's): c64 SunCentre, c65 the sun's direction, c66 SkyColor,
    // c67 / c69 AzimuthColor west / east, c68 SunColor, c70 AzimuthHeight, c71 CloudColor,
    // c72 SunsetColor, c73 CloudThreshold, c74 CloudBias, c75 CloudFadeOut, c76 the top clouds'
    // numbers, c77 the top clouds' colour, c78 CloudShadowStrength, c79 AzimuthStrength,
    // c80 thickness / edge / detail scale / detail strength, c81 SunSize, c82 HDRExposure,
    // c83 HDRSunExposure, c100 the weather's HDRExposureClamp (recorded 2.38 1.52 1.48 on clear
    // days, 2 2 2.66 in the storm - a ceiling over values that the 8-bit target cuts at 1 anyway).
    xs_c[64] = hat.ps[0];
    xs_c[65] = hat.ps[1];
    xs_c[66] = hat.vs[6];
    xs_c[67] = hat.vs[8];
    xs_c[68] = hat.ps[2];
    xs_c[69] = hat.vs[9];
    xs_c[70] = hat.vs[10];
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
    xs_c[81] = hat.ps[17];
    xs_c[82] = hat.ps[22];
    xs_c[83] = hat.ps[23];
    xs_c[100] = hat.clamp;
    var rin: array<vec4<f32>, 16>;
    var uv = floor(pos.xy) / 128.0;
    // (texel (64, 64) lies on the zenith itself, where the program divides by rho = 0; the map
    // read out of the game is smooth there - a hair off the pole)
    if (uv.x == 0.5 && uv.y == 0.5) { uv.x = 0.50001; }
    rin[0] = vec4<f32>(uv, 0.0, 0.0);
    let c = xs_minisky_ps(rin);
    // (alpha 1: the mark "drawn" for city.wgsl `mini_sky` - a target never drawn holds 0)
    return vec4<f32>((pwl_encode_floor(c.rgb) + vec3<f32>(0.5)) / 255.0, 1.0);
}
