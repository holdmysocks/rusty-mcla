// Uber shader for static city geometry and vehicle/prop objects.

struct Globals {
    view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,      // xyz, w = time
    sun_dir: vec4<f32>,      // xyz normalized (towards sun), w = sun intensity
    sun_color: vec4<f32>,
    ambient_sky: vec4<f32>,
    ambient_ground: vec4<f32>,
    fog_color: vec4<f32>,    // rgb, a = density
    params: vec4<f32>,       // x = fog start, y = fog end, z = exposure, w = unused
    inv_view_proj: mat4x4<f32>,
    sky_zenith: vec4<f32>,   // rgb, a = day factor
    sky_horizon: vec4<f32>,  // rgb, a = star brightness
    sky_sunset: vec4<f32>,   // rgb, a = sunset strength
    true_sun: vec4<f32>,
    moon_dir: vec4<f32>,
    light_vp: mat4x4<f32>,
    shadow_params: vec4<f32>,  // x = strength, y = texel size
    cloud_color: vec4<f32>,
    cloud_shade: vec4<f32>,
    tone: vec4<f32>,         // x = exposure, y = white, z = saturation, w = 1: night programs
    grade: vec4<f32>,        // rgb = twice the colour correction, w = OcclusionInfo.w
    fog_east: vec4<f32>,     // rgb, w = lean towards the horizon colours
    fog_west: vec4<f32>,
    dome: vec4<f32>,
    car_key: vec4<f32>,      // the cars' key lamp at night: the direction towards it
    car_key_col: vec4<f32>,  // ... its colour x strength
    key_dir: vec4<f32>,      // the key light under the night programs: the direction towards it
    key_col: vec4<f32>,      // ... its colour
    collector: vec4<f32>,    // the fast shadow collector's map: xy = its centre (world x, z), z = 1: kept up; w = 1: the road's reflection picture (t_refl) is there for this view
    car_key2: vec4<f32>,     // the cars' OTHER light slot at night (the hour's light, or a second lamp that beat it): the direction towards it; w = 1: use it
    car_key2_col: vec4<f32>, // ... its colour x strength
    car_amb: vec4<f32>,      // the garage: rgb = the cars' own ambient (ambYPos), w = 1: in force; car_key2 is then a position (its w = 2)
};
@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(2) var t_shadow: texture_depth_2d;
@group(0) @binding(3) var s_shadow: sampler_comparison;
@group(0) @binding(4) var t_scrape: texture_2d<f32>;
@group(0) @binding(5) var t_env: texture_cube<f32>;
@group(0) @binding(6) var s_env: sampler;
@group(0) @binding(7) var t_lamp_grid: texture_2d<u32>;
@group(0) @binding(8) var t_lamp_data: texture_2d<f32>;
@group(0) @binding(9) var t_env_soft: texture_cube<f32>;
@group(0) @binding(10) var t_fc: texture_2d<f32>;
@group(0) @binding(11) var t_refl: texture_2d<f32>;
// the mini sky (sky_mini.wgsl): 128 x 128, bytes in the console's gamma, alpha 1 where drawn
@group(0) @binding(14) var t_minisky: texture_2d<f32>;
// The original's four sun shadow cascades (render/mod.rs `Gpu::set_cascades`): PS_ShadowBlend's
// constants c64 .. c87 and its depth map (s15: four tiles of 640 x 640, two by two).
struct Casc {
    m: array<mat4x4<f32>, 4>,  // gLightMatrixArr: world -> the map's uv and depth, per tile
    top: vec4<f32>,            // gTopPlanes c82: the rectangles' long axis
    right: vec4<f32>,          // gRightPlanes c85: their short axis
    top_off: vec4<f32>,        // c83
    top_half: vec4<f32>,       // c84
    right_off: vec4<f32>,      // c86
    right_half: vec4<f32>,     // c87
    params: vec4<f32>,         // x = 1: drawn this frame; y, z = gFadeOutLastSMap (82, 78)
};
@group(0) @binding(12) var<uniform> casc: Casc;
@group(0) @binding(13) var t_casc: texture_depth_2d;

// The city's lamps at night, as PSMultiLight reads them: the cell of the grid under the point
// (512 x 640 cells over the city: uv = world.xz x c96.xy + c96.zw) names up to four lamps; each gives
//   colour x sat(intensity / distance squared) x n.l x sat(cos x scale + bias)
// (the cone of the street lights' spots; 1 for lamps that shine all round) and the sum is taken
// three times over (ShaderGlobals.w) and by gPropNFLColor.w (1.5 x m_NFLFade).
fn grid_lamps(wpos: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    return grid_lamps_by(wpos, n, g.ambient_ground.x);
}

// (`nfl`: gPropNFLColor.w - 1.5 x m_NFLFade outdoors, 1 at every hour for interiors)
fn grid_lamps_by(wpos: vec3<f32>, n: vec3<f32>, nfl: f32) -> vec3<f32> {
    let uv = wpos.xz * vec2<f32>(1.0 / 6320.0, 1.0 / 4463.75) + vec2<f32>(0.5485759, 0.5282554);
    if (uv.x < 0.0 || uv.x >= 1.0 || uv.y < 0.0 || uv.y >= 1.0) {
        return vec3<f32>(0.0);
    }
    let cell = textureLoad(t_lamp_grid, vec2<i32>(uv * vec2<f32>(512.0, 640.0)), 0);
    var sum = vec3<f32>(0.0);
    for (var k = 0; k < 4; k = k + 1) {
        let i = i32(cell[k]);
        if (i == 0) {
            continue;
        }
        let at = vec2<i32>((i % 256) * 4, i / 256);
        let p = textureLoad(t_lamp_data, at, 0);
        let c = textureLoad(t_lamp_data, at + vec2<i32>(1, 0), 0);
        let axis = textureLoad(t_lamp_data, at + vec2<i32>(2, 0), 0);
        let bias = textureLoad(t_lamp_data, at + vec2<i32>(3, 0), 0).x;
        let d = p.xyz - wpos;
        let d2 = max(dot(d, d), 0.0001);
        let dn = d * inverseSqrt(d2);
        sum = sum + c.rgb * (clamp(p.w / d2, 0.0, 1.0) * clamp(dot(n, dn), 0.0, 1.0) * clamp(dot(dn, axis.xyz) * axis.w + bias, 0.0, 1.0));
    }
    return sum * 3.0 * nfl;
}

// The one lamp the environment map's night program takes (see its use).
fn grid_lamp_env(wpos: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let uv = wpos.xz * vec2<f32>(1.0 / 6320.0, 1.0 / 4463.75) + vec2<f32>(0.5485759, 0.5282554);
    if (uv.x < 0.0 || uv.x >= 1.0 || uv.y < 0.0 || uv.y >= 1.0) {
        return vec3<f32>(0.0);
    }
    let i = i32(textureLoad(t_lamp_grid, vec2<i32>(uv * vec2<f32>(512.0, 640.0)), 0).x);
    if (i == 0) {
        return vec3<f32>(0.0);
    }
    let at = vec2<i32>((i % 256) * 4, i / 256);
    let p = textureLoad(t_lamp_data, at, 0);
    let c = textureLoad(t_lamp_data, at + vec2<i32>(1, 0), 0);
    let axis = textureLoad(t_lamp_data, at + vec2<i32>(2, 0), 0);
    let bias = textureLoad(t_lamp_data, at + vec2<i32>(3, 0), 0).x;
    let d = p.xyz - wpos;
    let d2 = max(dot(d, d), 0.0001);
    let dn = d * inverseSqrt(d2);
    return c.rgb * (clamp(p.w * g.ambient_ground.x / d2, 0.0, 1.0) * clamp(dot(n, dn), 0.0, 1.0) * clamp(dot(dn, axis.xyz) * axis.w + bias, 0.0, 1.0));
}

// (diagnostics 240 - 243: lamp `k` of the cell alone, as grid_lamps counts it)
fn grid_lamp_one(wpos: vec3<f32>, n: vec3<f32>, k: i32) -> vec3<f32> {
    let uv = wpos.xz * vec2<f32>(1.0 / 6320.0, 1.0 / 4463.75) + vec2<f32>(0.5485759, 0.5282554);
    let cell = textureLoad(t_lamp_grid, vec2<i32>(uv * vec2<f32>(512.0, 640.0)), 0);
    let i = i32(cell[k]);
    if (i == 0) { return vec3<f32>(0.0); }
    let at = vec2<i32>((i % 256) * 4, i / 256);
    let p = textureLoad(t_lamp_data, at, 0);
    let c = textureLoad(t_lamp_data, at + vec2<i32>(1, 0), 0);
    let axis = textureLoad(t_lamp_data, at + vec2<i32>(2, 0), 0);
    let bias = textureLoad(t_lamp_data, at + vec2<i32>(3, 0), 0).x;
    let d = p.xyz - wpos;
    let d2 = max(dot(d, d), 0.0001);
    let dn = d * inverseSqrt(d2);
    return c.rgb * (clamp(p.w / d2, 0.0, 1.0) * clamp(dot(n, dn), 0.0, 1.0) * clamp(dot(dn, axis.xyz) * axis.w + bias, 0.0, 1.0)) * 3.0 * g.ambient_ground.x;
}

// The night map's ROAD (PSMultiLightEnv 0x2db6456f.., instructions 6 - 63, work/lighting/em_prog.py
// sv_env_18.25 ?7e3c): TWO lamps of the cell, each colour x 2 x sat(256 x intensity / d^2) x n.l x
// cone - the program doubles the reach term (57: r0.zw = 2 x r4.w, 2 x r1.y) and takes no
// gPropNFLColor.w at all (the walls' program multiplies the intensity by c99.w; this one has no
// such constant): the map's road is as lamp-lit at 18:15 and 05:45 as at midnight.
fn grid_lamps_env_road(wpos: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let uv = wpos.xz * vec2<f32>(1.0 / 6320.0, 1.0 / 4463.75) + vec2<f32>(0.5485759, 0.5282554);
    if (uv.x < 0.0 || uv.x >= 1.0 || uv.y < 0.0 || uv.y >= 1.0) {
        return vec3<f32>(0.0);
    }
    let cell = textureLoad(t_lamp_grid, vec2<i32>(uv * vec2<f32>(512.0, 640.0)), 0);
    var sum = vec3<f32>(0.0);
    for (var k = 0; k < 2; k = k + 1) {
        let i = i32(cell[k]);
        if (i == 0) {
            continue;
        }
        let at = vec2<i32>((i % 256) * 4, i / 256);
        let p = textureLoad(t_lamp_data, at, 0);
        let c = textureLoad(t_lamp_data, at + vec2<i32>(1, 0), 0);
        let axis = textureLoad(t_lamp_data, at + vec2<i32>(2, 0), 0);
        let bias = textureLoad(t_lamp_data, at + vec2<i32>(3, 0), 0).x;
        let d = p.xyz - wpos;
        let d2 = max(dot(d, d), 0.0001);
        let dn = d * inverseSqrt(d2);
        sum = sum + c.rgb * (2.0 * clamp(p.w / d2, 0.0, 1.0) * clamp(dot(n, dn), 0.0, 1.0) * clamp(dot(dn, axis.xyz) * axis.w + bias, 0.0, 1.0));
    }
    return sum;
}

// The lamps' specular on walls with a specular map (PSMultiLight of the BumpSpec / Grime
// effects): per lamp the same reach and cone, without n.l, times s / (2E - (2E - 1) s) with
// s = sat(r . l), r the view ray mirrored at the surface and E the material's SpecularExponent.
fn grid_lamps_spec(wpos: vec3<f32>, r: vec3<f32>, e2: f32) -> vec3<f32> {
    let uv = wpos.xz * vec2<f32>(1.0 / 6320.0, 1.0 / 4463.75) + vec2<f32>(0.5485759, 0.5282554);
    if (uv.x < 0.0 || uv.x >= 1.0 || uv.y < 0.0 || uv.y >= 1.0) {
        return vec3<f32>(0.0);
    }
    let cell = textureLoad(t_lamp_grid, vec2<i32>(uv * vec2<f32>(512.0, 640.0)), 0);
    var sum = vec3<f32>(0.0);
    for (var k = 0; k < 4; k = k + 1) {
        let i = i32(cell[k]);
        if (i == 0) {
            continue;
        }
        let at = vec2<i32>((i % 256) * 4, i / 256);
        let p = textureLoad(t_lamp_data, at, 0);
        let c = textureLoad(t_lamp_data, at + vec2<i32>(1, 0), 0);
        let axis = textureLoad(t_lamp_data, at + vec2<i32>(2, 0), 0);
        let bias = textureLoad(t_lamp_data, at + vec2<i32>(3, 0), 0).x;
        let d = p.xyz - wpos;
        let d2 = max(dot(d, d), 0.0001);
        let dn = d * inverseSqrt(d2);
        let sr = clamp(dot(r, dn), 0.0, 1.0);
        sum = sum + c.rgb * (clamp(p.w / d2, 0.0, 1.0) * clamp(dot(dn, axis.xyz) * axis.w + bias, 0.0, 1.0) * sr / (e2 - (e2 - 1.0) * sr));
    }
    return sum * 3.0 * g.ambient_ground.x;
}

// The interior programs' lamps (PSMultiLightInterior; work/lighting/tun_rules.py): the same cell,
// gPropNFLColor.w = 1 at every hour. `count`: the cell's first lamps taken - four in the
// CityDefault-type and road forms, THREE in the tile form (CityTunnelTile / CityGrimeMetal read
// only half of the second index texture). x: the diffuse sum (x n.l); y: the specular sum,
// s / (e2 - (e2 - 1) s) with s = sat(r . l), for the forms that have one (e2 = 0: none).
fn interior_lamps(wpos: vec3<f32>, n: vec3<f32>, r: vec3<f32>, e2: f32, count: i32) -> array<vec3<f32>, 2> {
    var out = array<vec3<f32>, 2>(vec3<f32>(0.0), vec3<f32>(0.0));
    let uv = wpos.xz * vec2<f32>(1.0 / 6320.0, 1.0 / 4463.75) + vec2<f32>(0.5485759, 0.5282554);
    if (uv.x < 0.0 || uv.x >= 1.0 || uv.y < 0.0 || uv.y >= 1.0) {
        return out;
    }
    let cell = textureLoad(t_lamp_grid, vec2<i32>(uv * vec2<f32>(512.0, 640.0)), 0);
    for (var k = 0; k < count; k = k + 1) {
        let i = i32(cell[k]);
        if (i == 0) {
            continue;
        }
        let at = vec2<i32>((i % 256) * 4, i / 256);
        let p = textureLoad(t_lamp_data, at, 0);
        let c = textureLoad(t_lamp_data, at + vec2<i32>(1, 0), 0);
        let axis = textureLoad(t_lamp_data, at + vec2<i32>(2, 0), 0);
        let bias = textureLoad(t_lamp_data, at + vec2<i32>(3, 0), 0).x;
        let d = p.xyz - wpos;
        let d2 = max(dot(d, d), 0.0001);
        let dn = d * inverseSqrt(d2);
        let w = clamp(p.w / d2, 0.0, 1.0) * clamp(dot(dn, axis.xyz) * axis.w + bias, 0.0, 1.0);
        out[0] = out[0] + c.rgb * (w * clamp(dot(n, dn), 0.0, 1.0));
        if (e2 > 0.0) {
            let sr = clamp(dot(r, dn), 0.0, 1.0);
            out[1] = out[1] + c.rgb * (w * sr / (e2 - (e2 - 1.0) * sr));
        }
    }
    out[0] = out[0] * 3.0;
    out[1] = out[1] * 3.0;
    return out;
}

// One tap of the cascades' depth map as the original's sampler s15 gives it: the depth itself,
// filtered between the four texels round the point (fetch constant: linear, no mips, clamped).
fn casc_depth(uv: vec2<f32>) -> f32 {
    let size = vec2<i32>(textureDimensions(t_casc));
    let p = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * vec2<f32>(size) - vec2<f32>(0.5);
    let i = floor(p);
    let f = p - i;
    let a = vec2<i32>(i);
    let lo = clamp(a, vec2<i32>(0), size - vec2<i32>(1));
    let hi = clamp(a + vec2<i32>(1), vec2<i32>(0), size - vec2<i32>(1));
    let d00 = textureLoad(t_casc, lo, 0);
    let d10 = textureLoad(t_casc, vec2<i32>(hi.x, lo.y), 0);
    let d01 = textureLoad(t_casc, vec2<i32>(lo.x, hi.y), 0);
    let d11 = textureLoad(t_casc, hi, 0);
    return mix(mix(d00, d10, f.x), mix(d01, d11, f.x), f.y);
}

// The sun's shadow as the original's screen-space collector holds it (PS_ShadowBlend with
// VS_ShadowBlend, city effect 0xb6de4415; the scene programs read it doubled):
//   - fade = sat((distance from the camera - 82) / 78)                       (gFadeOutLastSMap)
//   - a tile holds the point when |p . top + c83| < c84 and |p . right + c86| < c87; the
//     tiles are nested, the tile taken is 4 - (the number that hold it): the smallest
//   - four taps of that tile's depth at p's place in it, on the arms of a cross of
//     1, 3/4, 1/2, 1/4 x (the number of tiles holding the point) texels, turned by the angle
//     3 x + 7.138 y of the pixel (x, y of a 1280 x 720 frame): the stipple of its edges
//   - each tap is lit when the map's depth there >= the point's own (no bias here: the bias
//     is in the depth draws), pcf = their mean: 0, 1/4, .. 1
//   - sun = (1 - m_ShadowFade) + m_ShadowFade x (fade + (1 - fade) x pcf)
// Where no tile holds the point the program's matrix index runs past the array into the
// constants behind it (c80 .. c83) and the taps fall together: lit or not by the sign of that
// arithmetic, kept as it is.
// The taps are taken only where the face looks at the light - n . l > 0 of the VERTEX normal
// (VS_ShadowBlend o1.x, instructions 5 - 7): elsewhere pcf = 0. That is what keeps a wall the
// light only grazes dark although its normal map turns some texels to the light.
// (Diagnostics 69: without that gate.)
fn sun_shadow_casc(wpos: vec3<f32>, n: vec3<f32>) -> f32 {
    let k = g.shadow_params.x;
    let fade = clamp((distance(wpos, g.cam_pos.xyz) - casc.params.y) / casc.params.z, 0.0, 1.0);
    var pcf = 0.0;
    if (fade < 1.0 && (dot(n, g.sun_dir.xyz) > 0.0 || (g.params.w > 68.5 && g.params.w < 69.5))) {
        let t = vec4<f32>(dot(wpos, casc.top.xyz)) + casc.top_off;
        let r = vec4<f32>(dot(wpos, casc.right.xyz)) + casc.right_off;
        let held = vec4<f32>(casc.top_half > abs(t)) * vec4<f32>(casc.right_half > abs(r));
        let count = dot(held, held);
        let tile = i32(4.0 - count);
        let clip = g.view_proj * vec4<f32>(wpos, 1.0);
        var px = (clip.xy / clip.w * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5)) * vec2<f32>(1280.0, 720.0);
        // (casc.params.w = 1, MCLA_CASC_PXGRID: at the centres of the original's 1280 x 720 pixels)
        if (casc.params.w > 0.5) { px = floor(px) + vec2<f32>(0.5); }
        let ang = fract((3.0 * px.x + 7.138 * px.y) * 0.15915494 + 0.5) * 6.2831855 - 3.1415927;
        let sn = sin(ang);
        let cs = cos(ang);
        let rad = count * vec4<f32>(0.000585938, 0.00078125, 0.000390625, 0.000195313);
        var q = vec4<f32>(0.0);
        if (tile < 4) {
            q = casc.m[tile] * vec4<f32>(wpos, 1.0);
            // (car materials: the look-up as before the measured shift - casc.right.w, see `set_cascades`)
            if (mat.mode == 6u || mat.mode == 7u || mat.mode == 8u || mat.mode == 10u || mat.mode == 12u || mat.mode == 13u || mat.mode == 17u || mat.mode == 18u || mat.mode == 19u) {
                q = q + vec4<f32>(casc.right.w, casc.right.w, 0.0, 0.0);
            }
        } else {
            q = wpos.x * vec4<f32>(0.0546875, 0.109375, 0.0, 0.0) + wpos.y * vec4<f32>(0.0625, 0.125, 0.0, 0.0) + wpos.z * casc.top + casc.top_off;
        }
        let zref = q.z * q.w;
        let lit = vec4<f32>(
            step(zref, casc_depth(q.xy + vec2<f32>(rad.y * cs, -rad.y * sn))),
            step(zref, casc_depth(q.xy + vec2<f32>(rad.x * sn, rad.x * cs))),
            step(zref, casc_depth(q.xy + vec2<f32>(-rad.z * cs, rad.z * sn))),
            step(zref, casc_depth(q.xy + vec2<f32>(-rad.w * sn, -rad.w * cs))));
        pcf = dot(lit, vec4<f32>(0.25));
        // (diagnostics 68: the tile's number - 0, 1/4 .. 3/4, 1 where none holds the point)
        if (g.params.w > 67.5 && g.params.w < 68.5) { return f32(tile) * 0.25; }
    }
    return (1.0 - k) + k * (fade + (1.0 - fade) * pcf);
}

// Sun visibility at a world position (1 = lit). Where the cascades are drawn (MCLA_CASCADES=1,
// not yet the default - see main.rs at `casc_vps`): the original's cascades and four turned
// taps (sun_shadow_casc). Otherwise, and under diagnostics 67: the port's single map of
// +-95 m with a 3x3 PCF and its own fade at the map's edge.
fn sun_shadow(wpos: vec3<f32>, n: vec3<f32>) -> f32 {
    if (g.shadow_params.x <= 0.0) {
        return 1.0;
    }
    if (casc.params.x > 0.5 && !(g.params.w > 66.5 && g.params.w < 67.5)) {
        return sun_shadow_casc(wpos, n);
    }
    let p = g.light_vp * vec4<f32>(wpos + n * 0.005, 1.0);
    let uv = p.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    let t = g.shadow_params.y;
    // Receiver-plane PCF: compare each tap against the receiver's depth there,
    // instead of hiding slope acne with a large constant offset. The old 0.0012
    // offset represented 1.08 metres in the 899 m light depth range.
    let duv_dx = dpdx(uv);
    let duv_dy = dpdy(uv);
    let dz_dx = dpdx(p.z);
    let dz_dy = dpdy(p.z);
    let det = duv_dx.x * duv_dy.y - duv_dx.y * duv_dy.x;
    var depth_gradient = vec2<f32>(0.0);
    if (abs(det) > 1e-12) {
        depth_gradient = vec2<f32>(dz_dx * duv_dy.y - duv_dx.y * dz_dy,
            duv_dx.x * dz_dy - dz_dx * duv_dy.x) / det;
    }
    // Five millimetres in world space, converted by the light projection.
    let bias = 0.005 * length(vec3<f32>(g.light_vp[0].z, g.light_vp[1].z, g.light_vp[2].z));
    // Derivatives above must run before this per-fragment early return.
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || p.z < 0.0 || p.z > 1.0) {
        return 1.0;
    }
    var s = 0.0;
    for (var dx = -1; dx <= 1; dx = dx + 1) {
        for (var dy = -1; dy <= 1; dy = dy + 1) {
            let offset = vec2<f32>(f32(dx), f32(dy)) * t;
            s = s + textureSampleCompareLevel(t_shadow, s_shadow, uv + offset,
                p.z + dot(depth_gradient, offset) - bias);
        }
    }
    s = s / 9.0;
    // fade out towards the edge of the shadowed area
    let e = max(abs(uv.x - 0.5), abs(uv.y - 0.5)) * 2.0;
    let fade = smoothstep(0.85, 1.0, e);
    return mix(mix(1.0 - g.shadow_params.x, 1.0, s), 1.0, fade);
}

// One tap of the shadow map: is this point in the sun (1) or not (0)?
fn sun_tap(wpos: vec3<f32>) -> f32 {
    let p = g.light_vp * vec4<f32>(wpos, 1.0);
    let uv = p.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || p.z < 0.0 || p.z > 1.0) {
        return 1.0;
    }
    let bias = 0.05 * length(vec3<f32>(g.light_vp[0].z, g.light_vp[1].z, g.light_vp[2].z));
    return textureSampleCompareLevel(t_shadow, s_shadow, uv, p.z - bias);
}

// The original's "fast shadow collector" at a point: its map (fast_collector.wgsl - 256 x 256
// texels of 1 m round the camera: the far city's models from above - the on / off sun shadow on
// their ground, faded to lit with the distance, a grey by their height on the roofs - blurred),
// red for the ground and green for 8 m above it. `p.y` is not looked at; `high` picks the layer. Lit outside the map.
// (Diagnostics 51, or no map: the stand-in before it - four taps of the shadow map at p.)
fn fast_collector_at(p: vec3<f32>, high: bool) -> f32 {
    if ((g.params.w > 50.5 && g.params.w < 51.5) || g.collector.z < 0.5) {
        return 0.25 * (sun_tap(p + vec3<f32>(-1.5, 0.0, -1.5)) + sun_tap(p + vec3<f32>(-1.5, 0.0, 0.5)) + sun_tap(p + vec3<f32>(0.5, 0.0, -1.5)) + sun_tap(p + vec3<f32>(0.5, 0.0, 0.5)));
    }
    let uv = (p.xz - g.collector.xy) * (1.0 / 256.0) + vec2<f32>(0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return 1.0;
    }
    let c = textureSampleLevel(t_fc, s_env, uv, 0.0);
    return select(c.r, c.g, high);
}

// ... at the ground (for callers that ask less than 4 m up)
fn fast_collector(p: vec3<f32>) -> f32 {
    return fast_collector_at(p, false);
}

// ... and 8 m up (for those that ask 4 m up or more)
fn fast_collector_high(p: vec3<f32>) -> f32 {
    return fast_collector_at(p, true);
}

// The cars' second light slot (LIGHTING_WORK.md "The car's light slots, lid and glass"): the game
// picks each car's two lights itself (sub_8235E3D8 -> sub_822F51A0: the two strongest of the lamps
// by sat(Intensity / d^2) x cone and the hour's light by 2 x (0.30 r + 0.59 g + 0.11 b)) and
// eases them; where a second lamp beats the hour's light the cars do not get the hour's light.
// main.rs car_key_light hands the slot over; by day (w = 0) the scene's own light stands.
// (garage round, 2026-10-13: in the garage the cars' lights are POINTS at hand - gLightPosDir of the car
// programs there: c16 = 4 m above the car (its strength, -c39.w, is 0 on the disc), c17 = 1 m above the
// camera, both with range 1000. car_key2.w = 2 marks the second as a position: the direction is taken
// per pixel, and the colour carries sat(1 - d / 1000) over the 0.9 every use site has built in for the
// lights 100 m off. `car_wp`: the pixel's world position, set at the head of fs_main.)
var<private> car_wp: vec3<f32>;
fn car_kd() -> vec3<f32> {
    if (g.car_key2.w > 1.5) { return normalize(g.car_key2.xyz - car_wp); }
    return select(g.key_dir.xyz, g.car_key2.xyz, g.car_key2.w > 0.5);
}
fn car_kc() -> vec3<f32> {
    if (g.car_key2.w > 1.5) { return g.car_key2_col.rgb * (clamp(1.0 - distance(g.car_key2.xyz, car_wp) * 0.001, 0.0, 1.0) / 0.9); }
    return select(g.key_col.rgb, g.car_key2_col.rgb, g.car_key2.w > 0.5);
}

// The collector as the cars' programs read it: PS_CarPaint, PS_Chrome and PS_BumpSpec all take
// the fetch's x, and the map is an ordinary 8888 texture (fetch swizzle BGRA with 8-in-32 like
// every other 8888 texture of the frame), so x is the RED the collector's pass wrote - and
// PS_ShadowFastBlend writes o0.x = the test at the face's OWN height, o0.y = the one 8 m up
// (instructions 23 - 39). So the cars read the ground layer, at the pixel's own x, z: the car
// stands in its own and the kerb's shade when the sun is low (2026-10-14; the 8 m layer was
// read here before - diagnostics 96 brings that back).
fn car_collector(p: vec3<f32>) -> f32 {
    if (g.params.w > 95.5 && g.params.w < 96.5) { return fast_collector_at(p + vec3<f32>(0.0, 7.5, 0.0), true); }
    return fast_collector_at(p, false);
}

// The collector as the plants meet it. The game's map is not the fresh on / off value: only a
// few ground pieces are written each frame, the rest is blurred again and again and fills in
// by diffusion (LIGHTING_WORK.md, "How the collector's gradient comes about") - shade under a
// block reads 0.4 - 0.9, never 0. A stand-in for the settled field - the on / off value averaged
// over rings of 6 and 14 m - was tried and made the plants WORSE (the bushes across the street
// 31.8 32.7 17.6 against 27.2 28.2 15.9 with the near taps and 19.6 20.7 12.6 in the original;
// 09:00 5.0 -> 5.4): it stays as diagnostics 50 only, the near taps are the rule until the map
// is built here as its own pass.
fn soft_collector(p: vec3<f32>) -> f32 {
    return soft_collector_at(p, false);
}

fn soft_collector_high(p: vec3<f32>) -> f32 {
    return soft_collector_at(p, true);
}

fn soft_collector_at(p: vec3<f32>, high: bool) -> f32 {
    let near = fast_collector_at(p, high);
    if (!(g.params.w > 49.5 && g.params.w < 50.5)) { return near; }
    var ring = 0.0;
    for (var i = 0; i < 8; i = i + 1) {
        let a = f32(i) * 0.7853982;
        let d = vec3<f32>(cos(a), 0.0, sin(a));
        ring = ring + sun_tap(p + d * 6.0) + sun_tap(p + d.zyx * vec3<f32>(-1.0, 1.0, 1.0) * 14.0 * 0.7071 + d * 14.0 * 0.7071);
    }
    return (near * 4.0 + ring) / 20.0;
}

// The same without the floor: 0 in shade, 1 in light (the foliage's light, see PSPropFoliage).
fn sun_visibility(wpos: vec3<f32>, n: vec3<f32>) -> f32 {
    let k = g.shadow_params.x;
    if (k <= 0.0) {
        return 1.0;
    }
    return clamp((sun_shadow(wpos, n) - (1.0 - k)) / k, 0.0, 1.0);
}

struct LightSet {
    count: vec4<u32>,
    pos_range: array<vec4<f32>, 48>,
    color: array<vec4<f32>, 48>,
    dir: array<vec4<f32>, 48>,
};
@group(0) @binding(1) var<uniform> lights: LightSet;

@group(1) @binding(0) var t_diffuse: texture_2d<f32>;
@group(1) @binding(1) var s_diffuse: sampler;
@group(1) @binding(2) var t_second: texture_2d<f32>;
@group(1) @binding(3) var t_third: texture_2d<f32>;
@group(1) @binding(4) var t_fourth: texture_2d<f32>;

struct Material {
    mode: u32,     // see MatMode in render/mod.rs
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};
@group(2) @binding(0) var<uniform> mat: Material;

struct Object {
    model: mat4x4<f32>,
    color: vec4<f32>,     // paint colour (rgb), a = metallic amount
    params: vec4<f32>,    // x = headlights on, y = brake lights, z = damage, w = unused
    dents: array<vec4<f32>, 6>,   // car bodies: where they were hit (xyz, own space), depth 0..1
    color2: vec4<f32>,    // car paint: second colour, w = fade type
    fade_min: vec4<f32>,  // the body's box: lowest corner
    fade_size: vec4<f32>, // and size
    tint: vec4<f32>,      // window tint: the car's m_vWindowTintColorNew (colour, w = darkness 0.372 stock .. 0.847; 0 = none given)
    interior: array<vec4<f32>, 5>, // cabin colours A, B, C, gauge colours A, B (w 1 = chosen)
    fill: vec4<f32>,      // added to the ambient: the night fill light at the object's place
    lamp_a: array<vec4<f32>, 19>, // a car's lamps by lamp id: bulb colours (row 0 w = 1: a traffic car's table)
    lamp_b: array<vec4<f32>, 19>, // and cover (lens) colours
    area_a: array<vec4<f32>, 5>,  // the player's paint areas after the body (doors, hood, bumpers, skirts): first colour, w 1 = own paint
    area_b: array<vec4<f32>, 5>,  // and second colour, w = paint type + 8 x fade
};
@group(3) @binding(0) var<uniform> obj: Object;
// a car's vertex damage map (damagemap.rs, the original's DmgTex): r the dents, g / a the layout,
// b the scrapes; all zero on everything else
@group(3) @binding(1) var t_dmg: texture_2d<f32>;

// The paint area (1 doors, 2 hood, 3 front bumper, 4 rear bumper, 5 skirts) whose own paint a
// fragment of the car's paint takes; 0 = the body's. The part's vertices carry 8 + area in
// uv2.y (vehicles.rs; the original's table of areas per part slot, 0x82050FD0).
fn paint_area(uv2y: f32) -> u32 {
    let k = i32(round(uv2y)) - 8;
    if (k >= 1 && k <= 5) {
        if (obj.area_a[k - 1].w > 0.5) { return u32(k); }
    }
    return 0u;
}

// A directional fade of the car's paint, PER VERTEX as VS_CarPaint (cars/0x0d96b4f8, instructions
// 19 - 32) has it: t = sat(dot(DL5 / DL7, rest position - DL6)) - DL5 the fade's axis ((0, 0, 1)
// front to back, (1, 0, 0) left to right, (0, 1, 0) bottom to top; zero for every other fade), DL6 /
// DL7 the body's box - and the vertex colour lerp(DL2, DL3, sat(2 t - 0.5)), the change taking
// the middle half of the box. The position is the undented one. Fades None / ColorShift and the
// matte paints have no axis: 0, the first colour. (On the program in numbers:
// work/lighting/ar_px.py, 20000 points, 3e-16.)
fn paint_fade_s(pos: vec3<f32>, uv2y: f32) -> f32 {
    var fade = u32(round(max(obj.color2.w, 0.0)));
    let area = paint_area(uv2y);
    if (area > 0u) { fade = u32(round(obj.area_b[area - 1u].w)) / 8u; }
    if (fade < 2u || fade > 4u) { return 0.0; }
    let rel = (pos - obj.fade_min.xyz) / obj.fade_size.xyz;
    var t = rel.z;
    if (fade == 3u) { t = rel.x; }
    if (fade == 4u) { t = rel.y; }
    return clamp(2.0 * clamp(t, 0.0, 1.0) - 0.5, 0.0, 1.0);
}

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv0: vec2<f32>,
    @location(3) uv1: vec2<f32>,
    @location(4) color: vec4<f32>,
    @location(5) color1: vec4<f32>,
    @location(8) uv2: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) wpos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv0: vec2<f32>,
    @location(3) uv1: vec2<f32>,
    @location(4) color: vec4<f32>,
    @location(5) color1: vec4<f32>,
    @location(6) uv2: vec2<f32>,
    @location(7) lpos: vec3<f32>,
    @location(8) fill: vec4<f32>,   // instances: the night fill light at the place, a = 1: has a value
    @location(9) neon_hl: f32,      // neon signs: the sun's highlight, per vertex (see vs_main)
    // plants: their own lamp at night (vs_inst; zero on everything else) - the world position and
    // strength, the colour and the cone's scale, the direction and the cone's offset
    @location(10) @interpolate(flat) lamp_pos: vec4<f32>,
    @location(11) @interpolate(flat) lamp_col: vec4<f32>,
    @location(12) @interpolate(flat) lamp_dir: vec4<f32>,
    // bump-mapped walls and props: the binormal's sign as the mesh stores it, 0: +1, 1: -1, 0.5: none
    // (world::mark_binormal lays it into the normal's length; w = 1 - 2 x this)
    @location(13) bflip: f32,
    // car paint: how far the vertex lies in a directional fade, 0 = first colour .. 1 = second (vs_body, paint_fade_s)
    @location(14) paint_s: f32,
    // car bodies: the damage map at the vertex (vs_body) - x the scrape (sat(b) x a), y the soot
    @location(15) dmg: vec2<f32>,
};

// The console's own decoding of a "gamma" texture: not the sRGB curve but four straight pieces
// (0 .. 64 x 1, .. 96 x 2, .. 192 x 4, .. 255 x 8 of 1023; the reference translator's
// PWLGammaToLinear), applied to the filtered sample. The textures here are uploaded as sRGB, so
// the sample comes decoded that way: encoded again, then decoded the console's way.
// (dark texels come out brighter than under sRGB - 32 of 255 gives 0.031 against 0.014 - bright
// ones a few per cent darker.) This IS the reference's decoding of the city's textures: with the
// tone map's own encode (post.wgsl: exponent Gamma = 0.4545, not sRGB's) the day frames agree to
// 1 .. 2 levels a cell, where sRGB decoding left the port 4 .. 9 too dark in dim places
// (LIGHTING_WORK.md, "The encode and the textures' decoding"). MCLA_DEBUG_VIEW=40: sRGB as before.
fn xenos_gamma(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let e = select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055), x * 12.92, x <= vec3<f32>(0.0031308)) * 255.0;
    let k = select(select(select(vec3<f32>(1.0), vec3<f32>(2.0), e >= vec3<f32>(64.0)), vec3<f32>(4.0), e >= vec3<f32>(96.0)), vec3<f32>(8.0), e >= vec3<f32>(192.0));
    let off = select(select(select(vec3<f32>(0.0), vec3<f32>(-64.0), e >= vec3<f32>(64.0)), vec3<f32>(-256.0), e >= vec3<f32>(96.0)), vec3<f32>(-1024.0), e >= vec3<f32>(192.0));
    var l = e * k + off;
    l = l + trunc(l * k / 1024.0);
    return l / 1023.0;
}
fn xenos_gamma_on() -> bool {
    return !(g.params.w > 39.5 && g.params.w < 40.5);
}
// (a gamma texture's sample, decoded the console's way unless the diagnostics view asks otherwise)
fn xgm(c: vec3<f32>) -> vec3<f32> {
    if (xenos_gamma_on()) { return xenos_gamma(c); }
    return c;
}
// (the windows' fixed reflection pictures; MCLA_DEBUG_VIEW=102: sRGB as before, for A/B)
fn xgp(c: vec3<f32>) -> vec3<f32> {
    if (g.params.w > 101.5 && g.params.w < 102.5) { return c; }
    return xgm(c);
}
// (trial, MCLA_DEBUG_VIEW=91: the same decoding for every other material's diffuse - cars, props,
// plants, people)
fn xenos_gamma_all() -> bool {
    return g.params.w > 90.5 && g.params.w < 91.5;
}
// A texture uploaded as it is (no decoding by the GPU), decoded here: the sRGB curve, and the
// console's four straight pieces on the stored value (see xenos_gamma). For the far city's night
// picture, whose fetch constant carries the gamma flag like the day picture's.
fn srgb_decode(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(pow((x + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)), x / 12.92, x <= vec3<f32>(0.04045));
}
// (the cars' textures carry no gamma flag: their programs take the stored value as it is. The
// textures here are uploaded as sRGB, so the sample comes decoded: encoded again.)
fn car_raw(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055), x * 12.92, x <= vec3<f32>(0.0031308));
}
fn pwl_decode(c: vec3<f32>) -> vec3<f32> {
    let e = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0;
    let k = select(select(select(vec3<f32>(1.0), vec3<f32>(2.0), e >= vec3<f32>(64.0)), vec3<f32>(4.0), e >= vec3<f32>(96.0)), vec3<f32>(8.0), e >= vec3<f32>(192.0));
    let off = select(select(select(vec3<f32>(0.0), vec3<f32>(-64.0), e >= vec3<f32>(64.0)), vec3<f32>(-256.0), e >= vec3<f32>(96.0)), vec3<f32>(-1024.0), e >= vec3<f32>(192.0));
    var l = e * k + off;
    l = l + trunc(l * k / 1024.0);
    return l / 1023.0;
}

// ---- The road as the game's own programs draw it (xs_road.wgsl, appended to this file:
// VSCityRoad and PSCityRoad translated from the microcode). The vertex program runs per pixel
// on the interpolated vertex; its inputs in the order it fetches them: position, normal, the
// grime's colour and weight, occlusion (the second colour's alpha), the first two texture
// coordinates in one, the third. ----
struct XsVsOut {
    pos: vec4<f32>,
    o: array<vec4<f32>, 16>,
};
var<private> xs_c: array<vec4<f32>, 256>;
var<private> xs_road_sun: f32;
// DiffuseSampler, DecalSampler, GrimeSampler (all three gamma-decoded by their fetch constants)
fn xs_road_s0(uv: vec2<f32>) -> vec4<f32> {
    let t = textureSample(t_diffuse, s_diffuse, uv);
    if (xenos_gamma_on()) { return vec4<f32>(xenos_gamma(t.rgb), t.a); }
    return t;
}
fn xs_road_s1(uv: vec2<f32>) -> vec4<f32> {
    let t = textureSample(t_second, s_diffuse, uv);
    if (xenos_gamma_on()) { return vec4<f32>(xenos_gamma(t.rgb), t.a); }
    return t;
}
fn xs_road_s2(uv: vec2<f32>) -> vec4<f32> {
    let t = textureSample(t_third, s_diffuse, uv);
    if (xenos_gamma_on()) { return vec4<f32>(xenos_gamma(t.rgb), t.a); }
    return t;
}
// PuddleSampler (the material's own map, on half the diffuse coordinates; a road without one
// has the empty texture there: taken as the middle)
fn xs_road_s3(uv: vec2<f32>) -> vec4<f32> {
    let t = textureSample(t_fourth, s_diffuse, uv);
    if (t.r + t.g + t.b + t.a <= 0.0) { return vec4<f32>(0.5); }
    return t;
}
// ReflectionSampler: the game's planar reflection of the city in the wet road - its own pass,
// a mirrored low-detail city and the sky (refl.wgsl, `Gpu::reflection_pass`), read at the
// pixel's place on the screen (the programs make it from the projected position: u = (x / w +
// 1) / 2, v = (1 - y / w) / 2, plus the puddle's ripple). collector.w = 1: the picture is there
// for this view. Without it (the environment map's views, MCLA_DEBUG_VIEW 63) the older
// stand-in: the surroundings along the mirrored view ray, as the cars' map or its stand-in has
// them - set by road_program.
var<private> xs_road_refl: vec3<f32>;
fn xs_road_s4(uv: vec2<f32>) -> vec4<f32> {
    if (g.collector.w > 0.5) {
        return vec4<f32>(textureSampleLevel(t_refl, s_env, uv, 0.0).rgb, 0.25);
    }
    return vec4<f32>(xs_road_refl, 1.0);
}
// ShadowCollectorSampler: the screen-space sun visibility, stored halved
fn xs_road_s13_off(uv: vec2<f32>, texels: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(xs_road_sun * 0.5);
}

fn road_program(wpos: vec3<f32>, nrm: vec3<f32>, color: vec4<f32>, occlusion: f32, uv01: vec4<f32>, uv2: vec2<f32>, sun: f32) -> vec3<f32> {
    // VSCityRoad: c8 .. c11 world x view x projection (the position: the pixel program makes
    // the screen place of the reflection picture from it), c15 the
    // camera, c16 gLightPosDir, c39 gLightColor, c43 gReflectionParams (m_RoadReflection twice,
    // the reflection plane's height, 1), c56 OcclusionInfo, c72 FresnelExponent, c73
    // SpecularExponent - the materials' values are the same on every road of the city
    xs_c[8] = g.view_proj[0];
    xs_c[9] = g.view_proj[1];
    xs_c[10] = g.view_proj[2];
    xs_c[11] = g.view_proj[3];
    xs_c[15] = vec4<f32>(g.cam_pos.xyz, 1.0);
    xs_c[16] = vec4<f32>(-g.sun_dir.xyz, 100.0);
    xs_c[39] = vec4<f32>(g.sun_color.rgb * g.sun_dir.w, -1.0);
    xs_c[43] = vec4<f32>(g.key_dir.w, g.key_col.w, g.cam_pos.y - 2.0, 1.0);
    xs_c[56] = vec4<f32>(g.ambient_ground.y, 0.05, 1.0, g.grade.w);
    xs_c[72] = vec4<f32>(10.0, 0.0, 0.0, 0.0);
    xs_c[73] = vec4<f32>(8.0, 0.0, 0.0, 0.0);
    // (38: FresnelExponent 0 - the far intensity everywhere)
    if (g.params.w > 37.5 && g.params.w < 38.5) { xs_c[72] = vec4<f32>(0.0); }
    var vin: array<vec4<f32>, 16>;
    vin[0] = vec4<f32>(wpos, 1.0);
    vin[1] = vec4<f32>(nrm, 0.0);
    vin[2] = color;
    vin[3] = vec4<f32>(0.0, 0.0, 0.0, occlusion);
    vin[4] = uv01;
    vin[5] = vec4<f32>(uv2, 0.0, 0.0);
    let vs = xs_road_vs(vin);
    // PSCityRoad: c26 gLightAmbient, c100 SpecularTint, c101 / c102 SpecularIntensityNear / Far,
    // c103 DecalSpecularIntensity, c104 SpecularColorMixer, c105 the reflection's terms
    xs_c[26] = vec4<f32>(g.ambient_sky.rgb, 1.0);
    xs_c[100] = vec4<f32>(1.0, 1.0, 1.0, 0.0);
    xs_c[101] = vec4<f32>(0.4, 0.0, 0.0, 0.0);
    xs_c[102] = vec4<f32>(6.0, 0.0, 0.0, 0.0);
    xs_c[103] = vec4<f32>(0.33, 0.0, 0.0, 0.0);
    xs_c[104] = vec4<f32>(0.7, 0.0, 0.0, 0.0);
    xs_c[105] = vec4<f32>(0.04, 0.15, 0.1, 0.0);
    // (diagnostics 35: without the specular - the three intensities zeroed, as an override on the running game does it)
    if (g.params.w > 34.5 && g.params.w < 35.5) {
        xs_c[101] = vec4<f32>(0.0);
        xs_c[102] = vec4<f32>(0.0);
        xs_c[103] = vec4<f32>(0.0);
    }
    // (36: the far intensity set to the near one; 37: near and decal intensity zero)
    if (g.params.w > 35.5 && g.params.w < 36.5) { xs_c[102] = vec4<f32>(0.4, 0.0, 0.0, 0.0); }
    if (g.params.w > 36.5 && g.params.w < 37.5) { xs_c[101] = vec4<f32>(0.0); xs_c[103] = vec4<f32>(0.0); }
    xs_road_sun = sun;
    xs_road_refl = env_color(reflect(normalize(wpos - g.cam_pos.xyz), nrm));
    return max(xs_road_ps(vs.o).rgb, vec3<f32>(0.0));
}

// ---- The road at night as the game's own programs draw it (xs_roadn.wgsl, appended to this
// file: VSCityRoadNight and PSMultiLight of the road's effect). Its lamp tables are read
// through these: the game's two grid textures hold a cell's four lamps as byte pairs (column
// x 2, row of its 128 x 256 table), the tables position x z + cone, colour / 2 + intensity /
// 256, axis + height - the port's own grid and table handed over in that form. ----
fn xs_lamp_pair(i: u32) -> vec2<f32> {
    return vec2<f32>(f32((i % 128u) * 2u), f32(i / 128u)) / 255.0;
}
fn xs_lamp_cell(uv: vec2<f32>) -> vec4<u32> {
    let c = clamp(vec2<i32>(floor(uv * vec2<f32>(512.0, 640.0))), vec2<i32>(0), vec2<i32>(511, 639));
    return textureLoad(t_lamp_grid, c, 0);
}
fn xs_lamp_s12(uv: vec2<f32>) -> vec4<f32> {
    let c = xs_lamp_cell(uv);
    return vec4<f32>(xs_lamp_pair(c.x), xs_lamp_pair(c.y));
}
fn xs_lamp_s11(uv: vec2<f32>) -> vec4<f32> {
    let c = xs_lamp_cell(uv);
    return vec4<f32>(xs_lamp_pair(c.z), xs_lamp_pair(c.w));
}
fn xs_lamp_at(uv: vec2<f32>) -> vec2<i32> {
    let i = clamp(i32(floor(uv.y * 256.0)), 0, 255) * 128 + clamp(i32(floor(uv.x * 128.0)), 0, 127);
    return vec2<i32>((i % 256) * 4, i / 256);
}
fn xs_lamp_s9(uv: vec2<f32>) -> vec4<f32> {
    let at = xs_lamp_at(uv);
    let p = textureLoad(t_lamp_data, at, 0);
    return vec4<f32>(p.x, p.z, textureLoad(t_lamp_data, at + vec2<i32>(2, 0), 0).w, textureLoad(t_lamp_data, at + vec2<i32>(3, 0), 0).x);
}
fn xs_lamp_s7(uv: vec2<f32>) -> vec4<f32> {
    let at = xs_lamp_at(uv);
    return vec4<f32>(textureLoad(t_lamp_data, at + vec2<i32>(1, 0), 0).rgb, textureLoad(t_lamp_data, at, 0).w / 256.0);
}
fn xs_lamp_s8(uv: vec2<f32>) -> vec4<f32> {
    let at = xs_lamp_at(uv);
    return vec4<f32>(textureLoad(t_lamp_data, at + vec2<i32>(2, 0), 0).xyz * 0.5 + 0.5, (textureLoad(t_lamp_data, at, 0).y + 31.0) / 110.0);
}
// FastShadowCollectorSampler: at night the game binds a map that reads 1 where the collector was
fn xs_roadn_s13(uv: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(1.0);
}

// The two head lamps the night programs take as their first "dynamic" lights (the third is
// the key light): the first two of the light set marked as such.
fn head_lamps() -> vec2<i32> {
    var h = vec2<i32>(-1, -1);
    for (var i = 0; i < i32(min(lights.count.x, 48u)); i = i + 1) {
        if (lights.color[i].w > 1.5 && lights.color[i].w < 2.5) {
            if (h.x < 0) { h.x = i; } else if (h.y < 0) { h.y = i; }
        }
    }
    return h;
}

// The night programs' fourth dynamic light (the light manager's dynamic list +576, record 3 -
// sub_822F7CE0): the race checkpoint's flickering light (checkpoints.rs, the light set's entry
// with colour.w 3: its strength in dir.w, all round).
fn cp_light() -> i32 {
    for (var i = 0; i < i32(min(lights.count.x, 48u)); i = i + 1) {
        if (lights.color[i].w > 2.5) {
            return i;
        }
    }
    return -1;
}

fn road_night_program(wpos: vec3<f32>, nrm: vec3<f32>, color: vec4<f32>, color1: vec4<f32>, uv01: vec4<f32>, uv2: vec2<f32>) -> vec3<f32> {
    // VSCityRoadNight: c8 .. c11 world x view x projection, c15 the camera, c43
    // gReflectionParams, c56 OcclusionInfo
    xs_c[8] = g.view_proj[0];
    xs_c[9] = g.view_proj[1];
    xs_c[10] = g.view_proj[2];
    xs_c[11] = g.view_proj[3];
    xs_c[15] = vec4<f32>(g.cam_pos.xyz, 1.0);
    xs_c[43] = vec4<f32>(g.key_dir.w, g.key_col.w, g.cam_pos.y - 2.0, 1.0);
    xs_c[56] = vec4<f32>(g.ambient_ground.y, 0.05, g.ambient_ground.z, 0.0);
    var vin: array<vec4<f32>, 16>;
    vin[0] = vec4<f32>(wpos, 1.0);
    vin[1] = vec4<f32>(nrm, 0.0);
    vin[2] = color;
    // (the second colour: the baked night light, and the occlusion in its alpha)
    vin[3] = color1;
    vin[4] = uv01;
    vin[5] = vec4<f32>(uv2, 0.0, 0.0);
    let vs = xs_roadn_vs(vin);
    // PSMultiLight: c26 gLightAmbient; c63 (hour, .., 0.2, 3: the lamps' gain); c84 .. c86 the
    // three dynamic lights' colours (a column each: two head lamps, the key light), c87 their
    // intensities (500, 500, 1e10), c88 .. c90 their positions, c91 .. c93 the spots' axes
    // (back along the beam, x 4.4696), c94 towards the key light, c95 the cones' bias; c96 ..
    // c98 the lamp grid's scale and bias, the tables' origin and extent; c99.w gPropNFLColor.w
    xs_c[14] = vec4<f32>(0.0, 0.0, -1.0, 0.0);
    xs_c[26] = vec4<f32>(g.ambient_sky.rgb, 1.0);
    xs_c[63] = vec4<f32>(g.sun_color.w, 0.0, 0.2, 3.0);
    let h = head_lamps();
    var hp: array<vec3<f32>, 2>;
    var hc: array<vec3<f32>, 2>;
    var hd: array<vec3<f32>, 2>;
    for (var k = 0; k < 2; k = k + 1) {
        hp[k] = vec3<f32>(0.0, -100000.0, 0.0);
        hc[k] = vec3<f32>(0.0);
        hd[k] = vec3<f32>(0.0, 1.0, 0.0);
        if (h[k] >= 0) {
            hp[k] = lights.pos_range[h[k]].xyz;
            hc[k] = lights.color[h[k]].rgb;
            hd[k] = -lights.dir[h[k]].xyz * 4.4696;
        }
    }
    let kp = g.key_dir.xyz * 100000.0;
    // (the fourth: the checkpoint's light - black, far below, when there is none)
    let cl = cp_light();
    var cc = vec3<f32>(0.0);
    var cpp = vec3<f32>(0.0, -100000.0, 0.0);
    var ci = 0.0;
    if (cl >= 0) {
        cc = lights.color[cl].rgb;
        cpp = lights.pos_range[cl].xyz;
        ci = lights.dir[cl].w;
    }
    xs_c[84] = vec4<f32>(hc[0].r, hc[1].r, g.key_col.r, cc.r);
    xs_c[85] = vec4<f32>(hc[0].g, hc[1].g, g.key_col.g, cc.g);
    xs_c[86] = vec4<f32>(hc[0].b, hc[1].b, g.key_col.b, cc.b);
    xs_c[87] = vec4<f32>(500.0, 500.0, 10000000000.0, ci);
    xs_c[88] = vec4<f32>(hp[0].x, hp[1].x, kp.x, cpp.x);
    xs_c[89] = vec4<f32>(hp[0].y, hp[1].y, kp.y, cpp.y);
    xs_c[90] = vec4<f32>(hp[0].z, hp[1].z, kp.z, cpp.z);
    xs_c[91] = vec4<f32>(hd[0].x, hd[1].x, 0.0, 0.0);
    xs_c[92] = vec4<f32>(hd[0].y, hd[1].y, 0.0, 0.0);
    xs_c[93] = vec4<f32>(hd[0].z, hd[1].z, 0.0, 0.0);
    xs_c[94] = vec4<f32>(g.key_dir.xyz, -435311680.0);
    xs_c[95] = vec4<f32>(-2.872, -2.872, 1.0, 1.0);
    xs_c[96] = vec4<f32>(1.0 / 6320.0, 1.0 / 4463.75, 0.5485759, 0.5282554);
    xs_c[97] = vec4<f32>(-3467.0, -31.0, -2358.0, 0.0);
    xs_c[98] = vec4<f32>(6320.0, 110.0, 4463.75, 0.0);
    xs_c[99] = vec4<f32>(0.0, 0.0, 0.0, g.ambient_ground.x);
    xs_c[100] = vec4<f32>(1.0, 1.0, 1.0, 0.0);
    xs_c[101] = vec4<f32>(0.4, 0.0, 0.0, 0.0);
    xs_c[102] = vec4<f32>(0.33, 0.0, 0.0, 0.0);
    xs_c[103] = vec4<f32>(0.7, 0.0, 0.0, 0.0);
    xs_c[104] = vec4<f32>(0.04, 0.15, 0.1, 0.0);
    xs_road_refl = env_color(reflect(normalize(wpos - g.cam_pos.xyz), nrm));
    return max(xs_roadn_ps(vs.o).rgb, vec3<f32>(0.0));
}

// ---- The sea as the game's own programs draw it (water work, 2026-10-14; xs_water.wgsl, appended
// to this file - work/water/gen_water.py). Recorded (work/lighting/cap/np3_*, work/mm/cap/gps_whole):
// CityOceanWater (detailed: VSCityOceanWater of city/0x487d7df0, the vertex colour moves waves) and
// CityOceanWaterLOD (flat, city/0x25bee0f2) with the one PSCityOceanWater - by day AND at night (the
// main view never takes PSMultiLight for the sea), opaque (blend 0x00010001, colour mask rgb). The
// program, in words: base = mix(DiffuseSampler, WaveFoamSampler, 2/3 foam); diffuse part =
// base x 0.5 x (gLightAmbient x 0.5 + gLightColor) - no n.l; the ripple normal from three taps of
// WaveSampler; out = mix(diffuse part, 0.5 x (ReflectionSampler + 0.066 x highlight), sat(w^2 x
// fresnel x (1 - foam)^2)), fresnel = 0.05 + 0.95 (1 - |n.v|)^2. Seen from above (the GPS map) the
// fresnel is 0.05 and the sea is nearly the dark diffuse part; at a glancing angle it is half the
// reflection picture.
// The vertex program runs per vertex, as in the original (`water_vertex`, called by vs_body: the
// waves move the vertices). Its inputs in fetch order: detailed -
// position, colour (rgba as the port holds it: the runtime fetch 8888 swizzle zyxw), the two
// coordinate sets in one (16_16_16_16 float, yxwz); LOD - position, the first coordinate set (0, 1).
// DiffuseSampler, WaveFoamSampler: gamma-decoded by their fetch constants; WaveSampler as stored
// (the batch's third texture, a raw copy - world.rs `upload`).
// (The detailed sea's maps are bound with few levels in the original - fetch constants of the beach
// captures, work/water/fetchc.py: levels 0 .. 2 for the diffuse, 0 .. 3 for foam and ripples, 4:1
// anisotropy - and so are the port's copies: the shared dictionary's l_sha_ground_water has 3 levels,
// l_sha_water_ripple_n 4, the sectors' own foam 4. The far sea's draws bind copies with whole chains,
// 0 .. 5 / 0 .. 6, which the port does not pick - see WATER_WORK.md.)
fn xs_water_s0(uv: vec2<f32>) -> vec4<f32> {
    let t = textureSample(t_diffuse, s_diffuse, uv);
    if (xenos_gamma_on()) { return vec4<f32>(xenos_gamma(t.rgb), t.a); }
    return t;
}
fn xs_water_s1(uv: vec2<f32>) -> vec4<f32> {
    let t = textureSample(t_second, s_diffuse, uv);
    if (xenos_gamma_on()) { return vec4<f32>(xenos_gamma(t.rgb), t.a); }
    return t;
}
fn xs_water_s2(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(t_third, s_diffuse, uv);
}
// ReflectionSampler: the same picture the road reads (refl.wgsl, the mirrored far city and the sky),
// at the place the vertex program hands over - the pixel's place on the screen in the main view, its
// place in the paraboloid half in the cars' map (VSCityOceanWaterEnv: o2 = (0.5 + 0.5 x, 0.5 - 0.5 y)).
// Without the picture: the surroundings along the mirrored view ray (as the road's stand-in).
var<private> xs_water_pic: bool;
var<private> xs_water_refl: vec3<f32>;
fn xs_water_s3(uv: vec2<f32>) -> vec4<f32> {
    if (xs_water_pic) {
        return vec4<f32>(textureSampleLevel(t_refl, s_env, uv, 0.0).rgb, 1.0);
    }
    return vec4<f32>(xs_water_refl, 1.0);
}

// VSCityOceanWater, PER VERTEX (vs_body, for the vertices world.rs `mark_water` marks: uv2.x = WATER_MARK,
// uv2.y 0 detailed / 1 far): c8 .. c11 world x view x projection, c63 ShaderGlobals (hour, seconds, ..) -
// y the clock the waves and the scrolling run on (recorded 262.4 / 26973.9: a running count of seconds;
// here the port's own), c73 UseCPVControls (1 on every recorded draw). Its outputs ride in the vertex's
// own slots, interpolated as the original's are: o1 (the moved place) as the world position and the
// clip position from it, o0 / o5 / o6 (the wave frame) as normal / color1.xyz / lpos, o4.x (foam) as
// color1.w, o3 (the four coordinates) as uv0 + uv1, o7 (colour x UseCPVControls) as color. o2 (the
// screen place) is rebuilt per pixel from the interpolated place - it is linear in it.
fn water_vertex(in: VsIn, wp: vec3<f32>, o_in: VsOut) -> VsOut {
    var o = o_in;
    xs_c[8] = g.view_proj[0];
    xs_c[9] = g.view_proj[1];
    xs_c[10] = g.view_proj[2];
    xs_c[11] = g.view_proj[3];
    xs_c[63] = vec4<f32>(g.sun_color.w, g.cloud_shade.a / 0.0022, 0.2, 3.0);
    xs_c[73] = vec4<f32>(1.0, 0.0, 0.0, 0.0);
    var vin: array<vec4<f32>, 16>;
    vin[0] = vec4<f32>(wp, 1.0);
    var vs: XsVsOut;
    if (in.uv2.y > 0.5) {
        vin[1] = vec4<f32>(in.uv0, 0.0, 1.0);
        vs = xs_water_lod_vs(vin);
    } else {
        vin[1] = in.color;
        vin[2] = vec4<f32>(in.uv0, in.uv1);
        vs = xs_water_vs(vin);
    }
    o.wpos = vs.o[1].xyz;
    o.clip = g.view_proj * vec4<f32>(o.wpos, 1.0);
    o.normal = vs.o[0].xyz;
    o.color1 = vec4<f32>(vs.o[5].xyz, vs.o[4].x);
    o.lpos = vs.o[6].xyz;
    o.uv0 = vs.o[3].xy;
    o.uv1 = vs.o[3].zw;
    o.color = vs.o[7];
    return o;
}

fn water_program(in: VsOut) -> vec3<f32> {
    var o: array<vec4<f32>, 16>;
    o[0] = vec4<f32>(in.normal, 0.0);
    o[1] = vec4<f32>(in.wpos, 0.0);
    let c = g.view_proj * vec4<f32>(in.wpos, 1.0);
    o[2] = vec4<f32>(0.5 * (c.x + c.w), 0.5 * (c.w - c.y), c.w, 0.0);
    // (the cars' map: the place in the half, 256 x 256 - VSCityOceanWaterEnv's o2)
    if (g.moon_dir.w > 0.5) { o[2] = vec4<f32>(in.clip.xy / 256.0, 1.0, 0.0); }
    o[3] = vec4<f32>(in.uv0, in.uv1);
    o[4] = vec4<f32>(in.color1.w, 0.0, 0.0, 0.0);
    o[5] = vec4<f32>(in.color1.xyz, 0.0);
    o[6] = vec4<f32>(in.lpos, 0.0);
    o[7] = in.color;
    // PSCityOceanWater: c15 the camera (gViewInverse's last row), c16 gLightPosDir (the way the light
    // travels: by day the sun's, at night the key light's from below the horizon - GPS frame at 01:34
    // (-0.762, 0.648, 0.011) with gLightColor 0.034 grey), c26 gLightAmbient, c27 gInvColorExpBias
    // (alpha only - not written), c39 gLightColor, c63 ShaderGlobals (y: the clock)
    xs_c[15] = vec4<f32>(g.cam_pos.xyz, 1.0);
    if (g.tone.w > 0.5) {
        xs_c[16] = vec4<f32>(-g.key_dir.xyz, 100.0);
        xs_c[39] = vec4<f32>(g.key_col.rgb, -1.0);
    } else {
        xs_c[16] = vec4<f32>(-g.sun_dir.xyz, 100.0);
        xs_c[39] = vec4<f32>(g.sun_color.rgb * g.sun_dir.w, -1.0);
    }
    xs_c[26] = vec4<f32>(g.ambient_sky.rgb, 1.0);
    xs_c[27] = vec4<f32>(0.0625, 0.0, 1.875, 0.0);
    xs_c[63] = vec4<f32>(g.sun_color.w, g.cloud_shade.a / 0.0022, 0.2, 3.0);
    xs_water_pic = g.collector.w > 0.5 && !(g.params.w > 750.5 && g.params.w < 751.5);
    xs_water_refl = env_color(reflect(normalize(in.wpos - g.cam_pos.xyz), vec3<f32>(0.0, 1.0, 0.0)));
    return max(xs_water_ps(o).rgb, vec3<f32>(0.0));
}

fn vs_body(in: VsIn) -> VsOut {
    var o: VsOut;
    var lp = in.pos;
    // Car bodies (params.w carries 16): the vertex damage map (damagemap.rs; VS_CarPaint 22 - 49,
    // the same in every body program). The map is read at the vertex's direction from the car's
    // origin, (x, z) / |p| x 0.5 + 0.5, one texel; the vertex moves dmgParams.w (0.2) x dent x
    // layout towards dmgParams.xyz = (0, 1.3, 0). (The normal is bent there by the map's slope
    // along the vertex's tangent and binormal: the port's vertex has neither - not done.)
    var dmg = vec2<f32>(0.0);
    var dmg_d = 0.0;
    if (obj.params.w > 15.5) {
        let pl = length(in.pos);
        let dd = select(vec2<f32>(0.0), in.pos.xz / pl, pl > 1e-6);
        let ts = textureDimensions(t_dmg);
        let tx = min(vec2<u32>(max(dd * 0.5 + vec2<f32>(0.5), vec2<f32>(0.0)) * vec2<f32>(ts)), ts - vec2<u32>(1u));
        let t = textureLoad(t_dmg, tx, 0);
        let to = vec3<f32>(0.0, 1.3, 0.0) - in.pos;
        if (dot(to, to) > 1e-12) {
            lp = lp + normalize(to) * (t.r * t.g * 0.2);
        }
        // (o2.w: the scrape, sat(b) x a; o8.y's first factors: sat(r + b) x sat(1 - 2 p.y) - which
        // of the position's values it takes is not certain)
        dmg.x = clamp(t.b, 0.0, 1.0) * t.a;
        dmg_d = clamp(t.r + t.b, 0.0, 1.0) * clamp(1.0 - 2.0 * in.pos.y, 0.0, 1.0);
    } else if (obj.dents[3].y > 0.5) {
        // A wheel (vehicles.rs: dents[3] = (its hub's scrape sum, 1)): the tyre's dirt as VS_ParallaxRubber2
        // (cars/0x315a5780, 22 - 34, 47, 70, 76) has it - o7.w = 0.5 x TireDirtColor.w x sqrt(max(t - t^2, 0)),
        // t = (|p.yz| - 0.406) / 0.095 of the unit wheel model: across the side wall, most in its middle.
        let tw = (length(in.pos.yz) - 0.406) * 10.526315;
        dmg.x = 0.5 * obj.dents[3].x * sqrt(max(tw - tw * tw, 0.0));
    }
    let wp = obj.model * vec4<f32>(lp, 1.0);
    o.clip = g.view_proj * wp;
    o.wpos = wp.xyz;
    // (a vertex without a stored normal - the packed word all zero, 7 % of the cut-out bump-spec
    // batches' vertices, the dry weeds at the walls' feet: handed on as zero; fs_main
    // reads the mark by its length. 2026-10-15, MCLA_DEBUG_VIEW=400: as before)
    o.normal = normalize((obj.model * vec4<f32>(in.normal, 0.0)).xyz);
    // (world.rs marks it: the vertical at a quarter of the length - x 1.5 or x 2 by world::mark_binormal)
    if (dot(in.normal, in.normal) < 0.3 && !(g.params.w > 399.5 && g.params.w < 400.5)) { o.normal = vec3<f32>(0.0); }
    o.bflip = clamp(round(2.0 * (length(in.normal) - 1.0)) * 0.5, 0.0, 1.0);
    o.uv0 = in.uv0;
    o.uv1 = in.uv1;
    o.uv2 = in.uv2;
    o.color = in.color;
    o.color1 = in.color1;
    o.lpos = in.pos;
    o.fill = vec4<f32>(0.0);
    // (car paint's directional fade, per vertex; the material's mode is not visible to this stage -
    // only the paint branch of the pixel stage reads it)
    o.paint_s = paint_fade_s(in.pos, in.uv2.y);
    // (the soot: x (0.33 + 0.66 (1 - n.v)) at the vertex)
    dmg.y = dmg_d * (0.33 + 0.66 * (1.0 - clamp(dot(o.normal, normalize(g.cam_pos.xyz - wp.xyz)), 0.0, 1.0)));
    o.dmg = dmg;
    // Neon signs' highlight of the sun, PER VERTEX as VSCityNeon (city/0x25a6120c) has it - o4.w:
    //   SpecularIntensity x pow(sat(V . (2 (n.l) n - l)), SpecularExponent) x sat(4 n.l)
    // (l towards the light, V towards the camera, both at the vertex; against the program on
    // 4000 points, work/lighting/neonhl_verify.py: 5e-14.) A poster is two triangles, so the
    // highlight is a smooth gradient over the whole of it and not a spot. The material's
    // exponent and intensity ride in the first colour word of the neon batches
    // (world::mark_window); the material's mode is not visible to this stage, so the value is
    // worked out for every vertex and only the neon branch of the pixel stage reads it.
    let hl_l = select(g.key_dir.xyz, g.sun_dir.xyz, g.sun_dir.w > 0.0);
    let hl_nl = clamp(dot(o.normal, hl_l), 0.0, 1.0);
    let hl_vr = clamp(dot(normalize(g.cam_pos.xyz - wp.xyz), o.normal * (2.0 * hl_nl) - hl_l), 0.0, 1.0);
    o.neon_hl = (round(in.color.g * 255.0) / 100.0) * pow(max(hl_vr, 1e-20), max(round(in.color.r * 255.0), 1.0)) * clamp(4.0 * hl_nl, 0.0, 1.0);
    // Traffic (its lamp rows marked ambient, lamp_a[0].w): VSAmbientCarPaint and VSAmbientGlass hand
    // their pixel programs the mirror ray and the fresnel worked out AT THE VERTEX (o5 / o3; the paint
    // also n.v, o4.w), interpolated over the triangle (work/trafpaint/dis/amb_carpaint.txt 74 - 102,
    // amb_glass.txt 68 - 86). Here in `fill` (ray, fresnel) and `neon_hl` (n.v + 4: the signs'
    // branches read it as >= -0.5, the props' gamma test as < -0.5). The programs' ray is V - 2 (n.v) N;
    // the port's maps take the opposite, reflect(-V, N). Glass vertices: uv2.x = 1 (vehicles.rs).
    if (obj.params.w > 15.5 && obj.lamp_a[0].w > 0.5) {
        let vv = normalize(g.cam_pos.xyz - wp.xyz);
        let nv = dot(o.normal, vv);
        let ray = 2.0 * nv * o.normal - vv;
        if (in.uv2.x > 0.5) {
            o.fill = vec4<f32>(ray, 0.025 + 0.825 * pow(clamp(1.0 - abs(nv), 0.0, 1.0), 4.5125));
        } else {
            let tbv = PAINT_B[min((u32(round(obj.params.w)) / 32u) % 8u, 5u)];
            o.fill = vec4<f32>(normalize(ray), tbv.y + (tbv.z - tbv.y) * pow(clamp(1.0 - nv, 0.0, 1.0), tbv.x));
            o.neon_hl = nv + 4.0;
        }
    }
    // the sea: the game's vertex program on this vertex (world::mark_water, `water_vertex`)
    if (in.uv2.x < -77000.0) { o = water_vertex(in, wp.xyz, o); }
    return o;
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    return vs_body(in);
}

// The cars' environment map as the original draws it (VSCity*Env, e.g. VSCityBumpSpecEnv 10 -
// 24): no projection matrix - every VERTEX is turned into its direction from the map's eye and
// laid on a paraboloid, x = d.x / (1 + d.fwd), y = d.y / (1 + d.fwd), the depth its distance x
// 0.0005; one half looks along the map's +z (the world's -z), the other along -z, mirrored
// left to right. Lines between vertices stay STRAIGHT in the map - a long pole is a straight
// stroke where the true projection bends it - and that is what the paint mirrors.
// g.moon_dir.w: 1 the front half, 2 the back half (each has a target of its own here).
@vertex
fn vs_env_dp(in: VsIn) -> VsOut {
    var o = vs_body(in);
    let d = o.wpos - g.cam_pos.xyz;
    let dist = max(length(d), 0.0001);
    let m = vec3<f32>(d.x, d.y, -d.z) / dist;
    var xy = vec2<f32>(0.0);
    var fwd = m.z;
    if (g.moon_dir.w < 1.5) {
        xy = m.xy / max(1.0 + m.z, 0.02);
    } else {
        xy = vec2<f32>(-m.x, m.y) / max(1.0 - m.z, 0.02);
        fwd = -m.z;
    }
    // (a corner BEHIND the half lies outside the picture, the further behind the further out; a
    // thing wholly behind is then a triangle between two such places and its straight edge can
    // still cut across the picture - the original draws each model only into the half its
    // bounding sphere reaches, which the port's merged batches cannot. Pushed further out along
    // their own direction such corners keep the ground that spans both halves covering its part
    // and take what lies wholly behind out of sight.)
    // WHAT THE ORIGINAL RASTERISES behind a half (2026-10-15; cap/sv_env_12, work/lighting/
    // eh_state.py, em_prog.py): the position the Env programs put out is (x, y, FORWARD distance x
    // 0.0005, 1) - VSCityBumpSpecEnv 19 `r0.x = r6.y x c254.w`, the half's forward component of
    // the offset from the eye, NOT the distance; 24 `oPos = r0.zwxx` with w the constant 1 - and
    // the draws clip (clip control 0x00080000: clipping on, D3D clip space 0 <= z <= w, no user
    // planes; viewport z scale -1, offset 1; depth test >=). So the GPU cuts every triangle at the
    // plane through the eye across the half's axis (z = 0), along the STRAIGHT edge between the
    // warped corners - a corner behind the plane lies outside the unit circle, the further behind
    // the further out, as 1 / (1 + d.fwd) has it, with no clamp -, what lies wholly behind is not
    // drawn, and the half's 256 x 256 scissor cuts the rest (the viewport is the whole 512 x 256,
    // x x 0.5 -+ 0.5). No push, no test per pixel. MCLA_DEBUG_VIEW=370: the port's two
    // inventions as until then (corners behind pushed out, depth by distance, the 60 degree drop
    // in fs_main).
    if (g.params.w > 369.5 && g.params.w < 370.5) {
        xy = xy * (1.0 + 8.0 * max(-fwd, 0.0));
        o.clip = vec4<f32>(xy, clamp(1.0 - dist * 0.0005, 0.0001, 1.0), 1.0);
        return o;
    }
    let xy0 = select(vec2<f32>(-m.x, m.y), m.xy, g.moon_dir.w < 1.5) / max(1.0 + fwd, 0.000001);
    // (MCLA_DEBUG_VIEW=371: cut at the half's plane, as the clip control word reads)
    if (g.params.w > 370.5 && g.params.w < 371.5) {
        o.clip = vec4<f32>(xy0, 1.0 - fwd * dist * 0.0005, 1.0);
        return o;
    }
    // ... but the pictures of the original's map say the GPU does NOT cut at that plane: the
    // road's triangles that cross it go on past the cut's chord with their texture unbroken
    // (out/eh_hole_o12.png: the original where the cut would leave holes - a third of the
    // ground of each half, numpy model eh_rast.py clip), while nothing that lies wholly behind
    // a half shows in it (no sheets over the sky; eh_rast.py noclip covers the back half with
    // one decal sheet). What agrees (eh_rast.py h3): a triangle with all three corners behind the
    // plane is thrown away, one with a corner in front is drawn WHOLE, straight edges between
    // the warped corners. Here: fill.a carries "in front" per corner (0.25 / 0; under 0.5, so it
    // is no fill light) and fs_main drops the fragments of triangles where it is 0 throughout;
    // the depth runs on behind the plane (0.5 - forward x 0.00025: nothing is cut).
    o.fill.a = select(0.0, 0.25, fwd > 0.0);
    o.clip = vec4<f32>(select(vec2<f32>(-m.x, m.y), m.xy, g.moon_dir.w < 1.5) / max(1.0 + fwd, 0.0001), 0.5 - fwd * dist * 0.00025, 1.0);
    return o;
}

struct InstIn {
    @location(6) pos_scale: vec4<f32>,
    @location(7) rot_tint: vec4<f32>,
    @location(9) fill: vec4<f32>,
    @location(10) lamp_pos: vec4<f32>,
    @location(11) lamp_col: vec4<f32>,
    @location(12) lamp_dir: vec4<f32>,
};

// A plant's own lamp at night - the one light the original's three plant programs take besides
// the fill, the ambient and the dynamic lights (the leaf and trunk vertex programs: c46 / c45 /
// c41; VSPropInstanceFoliageNight + PSMultiLight: o4 / o5 / o6 from the instance stream):
//   colour x sat(n.l) x sat(strength / d^2) x sat(l . direction x scale + offset)
// l from the lit point towards the lamp. No fade with the hour: the constants are the same at
// 18:30, 23:30 and 05:45. (LIGHTING_WORK.md, "The plants' own lamp at night".)
fn plant_lamp(wpos: vec3<f32>, n: vec3<f32>, lp: vec4<f32>, lc: vec4<f32>, ld: vec4<f32>) -> vec3<f32> {
    if (lp.w <= 0.0) { return vec3<f32>(0.0); }
    let d = lp.xyz - wpos;
    let d2 = max(dot(d, d), 1e-8);
    let l = d * inverseSqrt(d2);
    return lc.rgb * (clamp(dot(n, l), 0.0, 1.0) * clamp(lp.w / d2, 0.0, 1.0) * clamp(dot(l, ld.xyz) * lc.w + ld.w, 0.0, 1.0));
}

// Instanced props: position / uniform scale / yaw per instance; the tint is passed
// in color1.rgb's place via the vertex colour.
fn quat_rot(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

@vertex
fn vs_inst(in: VsIn, inst: InstIn) -> VsOut {
    var o: VsOut;
    // (plants: the quaternion's squared length is the height scale over the width scale - the
    // original stretches a plant's y axis apart from x and z; 1 on everything else)
    let tall = dot(inst.rot_tint, inst.rot_tint);
    let iq = inst.rot_tint * inverseSqrt(max(tall, 1e-8));
    var ipos = in.pos * vec3<f32>(inst.pos_scale.w, inst.pos_scale.w * tall, inst.pos_scale.w);
    var wp = quat_rot(iq, ipos) + inst.pos_scale.xyz;
    // Pass 8: a plant's impostor card (uv1.y = 104; props/impostor.rs) - VSPropInstanceFoliage.
    // The corner is pos = (side sign, top sign), the type's constants ride in the normal:
    // (half the box's width, CenterY, HeightY). The quad is the plant's own axis (lean and
    // height from the instance), widened by the camera-direction's side vector and tilted by
    // the tile's up vector's level part, so that it shows the picture as the tile camera saw it:
    //   up = axis x height scale, C = pos + up CenterY, top / bottom = C +- up HeightY
    //   P = (top or bottom) + sideSign SideVec width + topSign BackVec width
    //   SideVec = side W / 2, BackVec = (U.x, 0, U.z) W / 2, side = normalize(c.z, 0, -c.x), U = c x side
    let imp_card = in.uv1.y > 103.5 && in.uv1.y < 104.5;
    if (imp_card) {
        let cb = -normalize(vec3<f32>(g.view_proj[0][3], g.view_proj[1][3], g.view_proj[2][3]));
        let s0 = vec3<f32>(cb.z, 0.0, -cb.x);
        let side = select(vec3<f32>(1.0, 0.0, 0.0), s0 * inverseSqrt(max(dot(s0, s0), 1e-12)), dot(s0, s0) > 1e-10);
        let tu = cross(cb, side);
        let up = quat_rot(iq, vec3<f32>(0.0, 1.0, 0.0)) * (inst.pos_scale.w * tall);
        let hw = in.normal.x * inst.pos_scale.w;
        wp = inst.pos_scale.xyz + up * (in.normal.y + in.normal.z * in.pos.y) + side * (in.pos.x * hw) + vec3<f32>(tu.x, 0.0, tu.z) * (in.pos.y * hw);
        // (hOut: metres above the card's foot - top.y - bottom.y at the top corners, 0 below)
        ipos = vec3<f32>(0.0, up.y * in.normal.z * (in.pos.y + 1.0), 0.0);
    }
    o.clip = g.view_proj * vec4<f32>(wp, 1.0);
    o.wpos = wp;
    // (for the foliage's light: the vertex's height above the instance's foot, and the foot's height)
    // Fade with distance, as the original's gPropFadeInfo does it (MAP_WORK.md pass 3; the rules
    // are props.rs `PropModel::reach`). Prop models carry their bounding-sphere radius in uv1.x
    // and 100 + kind in uv1.y: 0 a prop (gone over the last 8 m of R (80 / tan(lens / 2) - 0.5)),
    // 1 a plant's model (the last 4 m of its distance), 2 a hedge, 3 a plant's card (in over the
    // 4 m before the model goes - not for the card-only placements, marked by a negative
    // quaternion w - and out over the last quarter of its reach). The fade leaves here as an
    // offset on the marker in lpos.z and is dithered in the pixel program.
    var fade = 1.0;
    if (in.uv1.y > 99.5 && in.uv1.x > 0.0) {
        let cot = length(vec3<f32>(g.view_proj[0][1], g.view_proj[1][1], g.view_proj[2][1]));
        let half_deg = degrees(atan(1.0 / max(cot, 0.01)));
        let depth = distance(wp, g.cam_pos.xyz);
        let r = in.uv1.x;
        let s = inst.pos_scale.w * max(tall, 1.0);
        let s2 = min(s, 2.0);
        let kind = i32(round(in.uv1.y)) - 100;
        let model = 2.0 * r / tan(half_deg * 0.007853982) * s2;
        if (kind == 1) {
            fade = clamp((model - depth) * 0.25, 0.0, 1.0);
        } else if (kind == 3 || kind == 4) {
            let far = min(2.0 * r / tan(half_deg * 0.000436332) * s2, 2600.0);
            fade = clamp((far - depth) / (far * 0.25), 0.0, 1.0);
            if (inst.rot_tint.w >= 0.0) { fade = min(fade, clamp((depth - (model - 8.0)) * 0.25, 0.0, 1.0)); }
        } else if (kind == 2) {
            let far = min(2.0 * r / tan(half_deg * 0.001308997) * s2, 2600.0);
            fade = clamp((far - depth) / (far * 0.25), 0.0, 1.0);
        } else {
            let far = min(r * s * (80.0 * cot - 0.5), 2250.0);
            fade = clamp((far - depth) * 0.125, 0.0, 1.0);
        }
    }
    o.lpos = vec3<f32>(ipos.y, inst.pos_scale.y, -10000.0 - (1.0 - fade) * 100.0);
    o.normal = normalize(quat_rot(iq, select(in.normal, vec3<f32>(0.0, 1.0, 0.0), imp_card)));
    o.bflip = select(clamp(round(2.0 * (length(in.normal) - 1.0)) * 0.5, 0.0, 1.0), 0.0, imp_card || (in.uv1.y > 100.5 && in.uv1.y < 105.5));
    o.uv0 = in.uv0;
    // Traffic signals carry two states in the instance scale, (a + 3 b) / 1000 above 1:
    // a for the lamps facing along the prop's z axis, b for those facing along x (the
    // cross street). One mast usually carries heads for both.
    let code = i32(round(fract(inst.pos_scale.w) * 1000.0));
    let facing_z = abs(in.normal.z) >= abs(in.normal.x);
    let sig = select(code / 3, code % 3, facing_z);
    o.uv1 = vec2<f32>(f32(sig), in.uv1.y);
    o.uv2 = in.uv2;
    o.color = in.color;
    o.color1 = in.color1;
    o.fill = inst.fill;
    // (below zero: the mark of an instanced prop for the pixel stage - their diffuse textures are
    // fetched with the console's gamma decoding like the city's, the cars' are not)
    o.neon_hl = -1.0;
    // Plants: the lamp of the instance's record. The cone as VSPropInstanceFoliageNight unpacks it:
    // o = outer byte x 2 - 1, i = inner byte x 2 - 1; both <= 0: scale 0, offset 1 (no cone),
    // else scale = 1 / (i - o), offset = -o / (i - o).
    let cone_o = inst.lamp_dir.w * 2.0 - 1.0;
    let cone_i = inst.lamp_col.w * 2.0 - 1.0;
    var cone = vec2<f32>(0.0, 1.0);
    if ((cone_o > 0.0 || cone_i > 0.0) && cone_i != cone_o) { cone = vec2<f32>(1.0, -cone_o) / (cone_i - cone_o); }
    o.lamp_pos = vec4<f32>(inst.pos_scale.xyz + inst.lamp_pos.xyz, inst.lamp_pos.w);
    o.lamp_col = vec4<f32>(inst.lamp_col.rgb, cone.x);
    o.lamp_dir = vec4<f32>(inst.lamp_dir.xyz * 2.0 - vec3<f32>(1.0), cone.y);
    // (a plant's model: the leaf program takes the world normal as the matrix gives it, NOT
    // normalised - its length, the plant's scale along the normal, rides below the mark: -1 - length)
    if (in.uv1.y > 100.5 && in.uv1.y < 102.5) {
        o.neon_hl = -1.0 - length(in.normal * vec3<f32>(inst.pos_scale.w, inst.pos_scale.w * tall, inst.pos_scale.w));
    }
    return o;
}

// For the debug views: what the tone map (post.wgsl) will show as this display colour.
fn to_scene(d: vec3<f32>) -> vec3<f32> {
    let v = clamp(d, vec3<f32>(0.0), vec3<f32>(0.985)) / max(g.grade.rgb, vec3<f32>(0.001));
    let y = clamp(dot(v, vec3<f32>(0.2125, 0.7154, 0.0721)), 0.0, 0.985);
    let w = max(g.tone.y, 0.001);
    let l = 0.5 * w * (sqrt((1.0 - y) * (1.0 - y) + 4.0 * y / w) - (1.0 - y));
    return v * (l / max(y, 0.0001)) / max(g.tone.x, 0.001);
}

// The scene target is linear and HDR, as the original's: exposure, tone curve, bloom and colour
// correction happen in the post chain (post.wgsl).
fn tonemap(c: vec3<f32>) -> vec3<f32> {
    return max(c, vec3<f32>(0.0));
}

// The mini sky: the 128 x 128 picture of the dome ALONE (vs_MiniSky / ps_miniSky, drawn each frame
// with the screen dome's constants - HDRExposure 0.8, the same azimuth keys) that the upper
// windows' SkyReflectionSampler holds. No houses in it: a pane that faces a wall still mirrors
// sky. Looked up at 0.5 + 0.47 r, r = R'.xz (the view ray mirrored with the normal's y raised
// by the height / 150). The map is not made here: the dome's colour rule without its clouds
// stands in, at the dome point whose flat coordinate lies that far out (as refl.wgsl's window
// models do). Read out of the running game at 09:00 (work/lighting/l3_minisky.py): zenith
// 0.29 0.43 0.49, at 0.47 from the middle 0.37 .. 0.43 0.43 .. 0.46 0.42 by the azimuth.
// (Since the mini sky round this is only the fall-back: MCLA_MINISKY_OLD=1, MCLA_DEBUG_VIEW=660,
// or no sky dome constants - the garage.)
fn mini_sky_old(r: vec2<f32>) -> vec3<f32> {
    let rl = max(length(r), 0.0001);
    let rho = clamp(0.94 * rl / 0.888, 0.0, 1.0);
    let d = normalize(vec3<f32>(200.0 * rho * r.x / rl, 163.67 * sqrt(max(1.0 - rho * rho, 0.0)), 200.0 * rho * r.y / rl));
    let az = mix(g.sky_sunset.rgb, g.sky_horizon.rgb, d.x * 0.5 + 0.5);
    return (g.sky_zenith.rgb + (az - g.sky_zenith.rgb) * (g.dome.x * (1.0 - clamp(d.y * g.dome.y, 0.0, 1.0)))) * g.dome.z;
}

// The real thing (mini sky round): the map itself, drawn each frame by the game's own program
// (sky_mini.wgsl: ps_miniSky with the screen dome's constants - clouds and the sun's disc in it),
// read as every consumer's SkyReflectionSampler reads it: at 0.5 + 0.47 r, clamped at the
// edge, bilinear, no mips, bytes decoded with the console's gamma curve (fetch constant
// 0x810048fe). The stand-in above reached the horizon's grey at a picture radius of 0.444;
// the program's own mapping (direction = (p |p|, 1 - |p|), p = 0.94 r) keeps the zenith's blue
// far longer. MCLA_DEBUG_VIEW=660: the stand-in; 661: decoded before filtering (the console's
// order - the reference recompilation filters first); 662: the map's nearest texel.
fn mini_sky(r: vec2<f32>) -> vec3<f32> {
    let uv = clamp(vec2<f32>(0.5) + 0.47 * r, vec2<f32>(0.0), vec2<f32>(1.0));
    let t = textureSampleLevel(t_minisky, s_env, uv, 0.0);
    if (t.a < 0.5 || (g.params.w > 659.5 && g.params.w < 660.5)) { return mini_sky_old(r); }
    if (g.params.w > 660.5 && g.params.w < 662.5) {
        let p = uv * 128.0 - vec2<f32>(0.5);
        let i0 = vec2<i32>(floor(p));
        let f = p - floor(p);
        let lo = vec2<i32>(0);
        let hi = vec2<i32>(127);
        if (g.params.w > 661.5) { return pwl_decode(textureLoad(t_minisky, clamp(vec2<i32>(floor(uv * 128.0)), lo, hi), 0).rgb); }
        let a = pwl_decode(textureLoad(t_minisky, clamp(i0, lo, hi), 0).rgb);
        let b = pwl_decode(textureLoad(t_minisky, clamp(i0 + vec2<i32>(1, 0), lo, hi), 0).rgb);
        let c = pwl_decode(textureLoad(t_minisky, clamp(i0 + vec2<i32>(0, 1), lo, hi), 0).rgb);
        let d = pwl_decode(textureLoad(t_minisky, clamp(i0 + vec2<i32>(1, 1), lo, hi), 0).rgb);
        return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
    }
    return pwl_decode(t.rgb);
}

// procedural environment used for reflections (sky gradient, ground, sun highlight)
fn env_color(d: vec3<f32>) -> vec3<f32> {
    let up = clamp(d.y, -1.0, 1.0);
    // same model as the sky dome (sky.wgsl), without stars
    var sky = mix(g.sky_zenith.rgb, g.sky_horizon.rgb, pow(1.0 - max(up, 0.0), 6.0)) * 1.25;
    // street level: asphalt below, a band of building fronts around the horizon
    let lit = g.sun_color.rgb * g.sun_dir.w;
    var ground = g.ambient_ground.rgb * 0.30 + lit * 0.045;
    var city = g.ambient_ground.rgb * 0.55 + lit * 0.07;
    if (g.dome.w > 0.5) {
        // With the weather's keys as the game has them (sky_zenith / sky_horizon / sky_sunset are
        // the dome's keys and ambient_ground holds other things - see main.rs `lighting`): the
        // dome's own rule for the sky, and the ground and the house fronts as the original's
        // environment map shows them: asphalt 0.06 at noon under a light of about 1, 0.03 at
        // night under the lamps; walls about three times that.
        let az = mix(g.sky_sunset.rgb, g.sky_horizon.rgb, d.x * 0.5 + 0.5);
        sky = (g.sky_zenith.rgb + (az - g.sky_zenith.rgb) * (g.dome.x * (1.0 - clamp(d.y * g.dome.y, 0.0, 1.0)))) * g.dome.z;
        let light = g.ambient_sky.rgb + lit * max(g.sun_dir.y, 0.0) + vec3<f32>(0.45, 0.38, 0.24) * (g.ambient_ground.x / 1.5);
        ground = light * 0.06;
        city = light * 0.18;
    }
    var c = mix(ground, city, smoothstep(-0.08, 0.02, up));
    c = mix(c, sky, smoothstep(0.10, 0.42, up));
    let sun = pow(max(dot(d, g.sun_dir.xyz), 0.0), 180.0) * g.sun_dir.w;
    // (shadow_params.w: the cube map of the real surroundings is there to be mirrored)
    if (g.shadow_params.w > 0.5) {
        let real = textureSampleLevel(t_env, s_env, vec3<f32>(d.x, d.y, d.z), 0.0).rgb;
        // (under the weather's keys the map is taken as it is, as the game's programs take
        // theirs: the sun in it is the dome's own. MCLA_DEBUG_VIEW 39: the older gains.)
        if (g.dome.w > 0.5 && !(g.params.w > 38.5 && g.params.w < 39.5)) { return real; }
        return real * 1.15 + g.sun_color.rgb * sun * 2.5;
    }
    return c + g.sun_color.rgb * sun * 2.5;
}

// What the fast shadow collector holds in full sun: 1 (read out of the running game at noon -
// 255 over the open street, both channels; a trial with 0.5 made the cars grey).
const FC_LIT: f32 = 1.0;

// The blurred copy of the surroundings (dpSpecFrontSampler: the 64 x 32 environment map after a
// Gaussian blur; here the 16 x 16 cube of env_blur.wgsl).
// PS_CarbonFiber's highlight (cars/0x84fc57d9, lines 63 - 89): exp(-2 ((h.b / ax)^2 + (h.t / ay)^2) / (1 + sat(h.n))) / (4 pi ax ay)
fn cf_ward(h: vec3<f32>, t: vec3<f32>, b: vec3<f32>, n: vec3<f32>, ax: f32, ay: f32) -> f32 {
    let hb = dot(h, b) / ax;
    let ht = dot(h, t) / ay;
    return exp(-2.0 * (hb * hb + ht * ht) / (1.0 + clamp(dot(h, n), 0.0, 1.0))) / (12.5664 * ax * ay);
}
fn cf_unit(a: vec3<f32>) -> vec3<f32> { return a * inverseSqrt(max(dot(a, a), 1e-24)); }
fn env_soft(d: vec3<f32>) -> vec3<f32> {
    if (g.shadow_params.w < 0.5) {
        return env_color(d);
    }
    // PS_BlurVert's end: x imgExposure (0.473 by day, 1.29 by night: car_key.w), half way to
    // grey, a contrast of imgExp (1.023) about 0.5, nothing below 0
    var c = textureSampleLevel(t_env_soft, s_env, d, 0.0).rgb * g.car_key.w;
    c = mix(c, vec3<f32>(dot(c, vec3<f32>(0.3, 0.59, 0.11))), 0.5);
    return max((c - vec3<f32>(0.5)) * 1.023 + vec3<f32>(0.5), vec3<f32>(0.0));
}

// The paint types (DirtyMatte, Matte, Gloss, Metallic, Pearl, Sparkle), as PS_CarPaint takes
// them: A = DL0 (diffuse, specular and ambient intensity, specular exponent), B = DL1 (fresnel
// exponent, minimum and maximum, paint falloff), C = pearlescence and DL4 (noise scale,
// strength, exponent). Replaced at start-up by the disc's own tuning files.
//#PAINT_BEGIN
const PAINT_A: array<vec4<f32>, 6> = array<vec4<f32>, 6>(vec4<f32>(0.375, 0.15, 0.375, 1.0), vec4<f32>(0.375, 0.2, 0.375, 1.0), vec4<f32>(0.375, 0.025, 0.375, 1.0), vec4<f32>(0.225, 2.0, 0.2, 1.475), vec4<f32>(0.225, 1.45, 0.275, 1.5), vec4<f32>(0.225, 0.95, 0.5, 1.5));
const PAINT_B: array<vec4<f32>, 6> = array<vec4<f32>, 6>(vec4<f32>(4.5, 0.0, 0.0, 0.1), vec4<f32>(4.5, 0.0, 0.0, 0.05), vec4<f32>(4.5, 0.07, 0.625, 1.0), vec4<f32>(4.5, 0.07, 0.625, 1.0), vec4<f32>(4.5, 0.07, 0.625, 1.0), vec4<f32>(4.5, 0.013, 0.625, 1.0));
const PAINT_C: array<vec4<f32>, 6> = array<vec4<f32>, 6>(vec4<f32>(0.29, 0.068, 0.295, 1.0), vec4<f32>(0.29, 0.068, 0.0, 1.0), vec4<f32>(1.0, 20.0, 0.0, 1.0), vec4<f32>(0.0, 13.975, 0.3275, 1.0), vec4<f32>(0.1625, 14.0, 0.5, 1.0), vec4<f32>(0.235, 4.125, 0.685, 1.0));
//#PAINT_END

// The normal a paint's flakes tilt (PS_CarPaint 55 - 59: normalize(N + (noise - 0.5) x DL4.y)), by
// the paint type - the very expressions of the paint's lower half in fs_main, where the soft
// highlight is looked up along it; the two-tone shift of the colour goes by the same normal.
fn paint_flake_n(n: vec3<f32>, uv0: vec2<f32>, ti: u32, scraped: f32) -> vec3<f32> {
    var pc = PAINT_C;
    let tc = pc[ti];
    let uvn = uv0 * tc.y * 4.0;
    let tp = max(length(dpdx(uvn)), length(dpdy(uvn))) * 128.0;
    let fade = exp2(-2.0 * max(log2(max(tp, 0.0001)) - 1.0, 0.0));
    let cell = floor(uvn * 128.0);
    let flake = fract(sin(vec3<f32>(dot(cell, vec2<f32>(127.1, 311.7)), dot(cell, vec2<f32>(269.5, 183.3)), dot(cell, vec2<f32>(419.2, 371.9)))) * 43758.5453) - vec3<f32>(0.5);
    return normalize(n + flake * tc.z * fade * (1.0 - scraped));
}

fn fresnel(n: vec3<f32>, v: vec3<f32>, f0: f32) -> f32 {
    let c = clamp(1.0 - max(dot(n, v), 0.0), 0.0, 1.0);
    return f0 + (1.0 - f0) * c * c * c * c * c;
}

// The light a car's own lamps throw on its body (siColor0 / siColor1 of the cars' vertex
// programs, c61 / c60, weighed by two bytes of the first vertex colour and screened together):
// siColor0 - weight byte 1 - is the head lamps' lit colour (bulb x cover x state of lamp id 3:
// recorded (0.616, 0.796, 1) on the R8 with its lights on, zero with them off); it is what lights
// the number plate's recess. siColor1 - byte 2 - is the brake lamps': bulb x cover of id 0 times
// 2.44 x state^2.2 (two recorded states, 0.25 -> (0.431, 0.0138, 0) and 0.769 -> (5.112, 0.163, 0):
// the exponent and the factor are a fit through those two). siColor2 (byte 3) was zero in every
// recording. Which byte is which: the runtime fetch word of the draws (0x400600d1 on the BumpSpec
// family, 0x400602c2 on the paint), work/lighting/rtfetch.py. `wts` = the bytes 1, 2, 3 as the
// car meshes carry them in the first colour. Traffic (their own programs) and cabin parts (whose
// two colours are the gauge colours, not passed here; marked 32 in the colour's alpha) get none.
fn car_own_lamps(wts: vec4<f32>) -> vec3<f32> {
    if (obj.lamp_a[0].w > 0.5 || (u32(round(wts.a * 255.0)) & 32u) != 0u || wts.a > 0.5) { return vec3<f32>(0.0); }
    let st0 = max(0.25 * obj.params.x, clamp(obj.params.y * 4.0, 0.0, 1.0));
    let s0 = obj.lamp_a[3].rgb * obj.lamp_b[3].rgb * (clamp(obj.params.x, 0.0, 1.0) * wts.r);
    let s1 = obj.lamp_a[0].rgb * obj.lamp_b[0].rgb * (2.44 * pow(st0, 2.2) * wts.g);
    return s0 + s1 - clamp(s0 * s1, vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    car_wp = in.wpos;
    var albedo = textureSample(t_diffuse, s_diffuse, in.uv0);
    // (diagnostics 542 .. 545, p2day: the upper windows' diffuse read at level 0 .. 3; 546: at one level coarser than the sampler's choice)
    if (mat.mode == 3u && g.params.w > 541.5 && g.params.w < 545.5) { albedo = textureSampleLevel(t_diffuse, s_diffuse, in.uv0, round(g.params.w) - 542.0); }
    if (mat.mode == 3u && g.params.w > 545.5 && g.params.w < 546.5) { albedo = textureSampleBias(t_diffuse, s_diffuse, in.uv0, 1.0); }
    // (neon2, 2026-10-15) Signs: a lit sign's light is far from linear in its textures - D x m x (0.6 + 2.8 m)
    // of two pictures decoded AFTER the filter (see `sign_raw`), white clipping in the tone map - so how wide the
    // filter reaches changes the sign's mean, not only its sharpness. The original reads them at 1280 x 720 with
    // 4 : 1 anisotropy (fetch words of the neon draws: linear / linear / linear, aniso 3, levels 0 .. 2); this
    // window has more rows and an 8 : 1 sampler. The sectors' sign textures are read with the original's
    // footprint: the derivatives scaled to 720 rows, the minor axis no less than a quarter of the major one
    // (main view only). In a 1280 x 720 window it changes nothing (SKATEBOARD box to 0.1); at 1600 x 900 the
    // second place's frame reads 0.8 with it and 1.0 without. MCLA_DEBUG_VIEW=442: the window's own footprint.
    let sg_c = g.view_proj * vec4<f32>(in.wpos, 1.0);
    let sg_s = clamp(1.0 / max(360.0 * abs(dpdy(sg_c.y / sg_c.w)), 0.0001), 0.5, 4.0);
    let sg_on = (mat.mode == 5u || mat.mode == 24u) && in.neon_hl > -0.5 && g.tone.w < 1.5 && !(g.params.w > 441.5 && g.params.w < 442.5);
    let sg_dx0 = dpdx(in.uv0) * sg_s;
    let sg_dy0 = dpdy(in.uv0) * sg_s;
    // (the plain footprint, for fetches made in branches: the in-car glass' noise and mask)
    let uv0_dx = dpdx(in.uv0);
    let uv0_dy = dpdy(in.uv0);
    if (sg_on) {
        let dm = vec2<f32>(textureDimensions(t_diffuse));
        let lx = max(length(sg_dx0 * dm), 0.000001);
        let ly = max(length(sg_dy0 * dm), 0.000001);
        albedo = textureSampleGrad(t_diffuse, s_diffuse, in.uv0, sg_dx0 * max(1.0, ly / (4.0 * lx)), sg_dy0 * max(1.0, lx / (4.0 * ly)));
    }
    // The cars' map, a paraboloid half (vs_env_dp; moon_dir.w = 1 / 2): the warp is per vertex, so a
    // triangle with corners far apart in direction - one beside or behind the half's eye - is a
    // straight-edged slab that can lie across the picture where the thing itself is not: at the
    // Selma / Highland crossing (car 735.9 20.8 -850.5) such slabs covered half the sky of the map
    // and the car's mirror and soft highlight went black (the original's map, read out of the game
    // there, has a clean sky: it draws each model only into the half its sphere reaches and its
    // pieces are small). A fragment is kept only where the place it stands for (the interpolated
    // world position) lies in front of the half and within 60 degrees of the picture point's own
    // direction. MCLA_DEBUG_VIEW=113: without, as until 2026-10-14.
    // (2026-10-15: only with MCLA_DEBUG_VIEW=370 - the original clips at the half's plane instead, vs_env_dp)
    if (g.moon_dir.w > 0.5 && g.params.w > 369.5 && g.params.w < 370.5) {
        let q = vec2<f32>(in.clip.x / 128.0 - 1.0, 1.0 - in.clip.y / 128.0);
        let r2 = dot(q, q);
        let f = (1.0 - r2) / (1.0 + r2);
        let pd = vec3<f32>(q * (1.0 + f), f);
        let dw = in.wpos - g.cam_pos.xyz;
        var md = vec3<f32>(dw.x, dw.y, -dw.z) / max(length(dw), 0.0001);
        if (g.moon_dir.w > 1.5) { md = vec3<f32>(-md.x, md.y, -md.z); }
        let dp_ghost = md.z < -0.1 || dot(md, pd) < 0.5;
        if (dp_ghost) { discard; }
    }
    // (the cars' map: a triangle wholly behind the half's plane is not drawn - vs_env_dp)
    if (g.moon_dir.w > 0.5 && !(g.params.w > 369.5 && g.params.w < 371.5) && in.fill.a <= 0.0) { discard; }
    // (... and of a FAR model - world::LOD_MARK in uv2.y - only what lies in front of the plane:
    // the original's far ground does not lie over its road where both run on behind the plane
    // (there the last one drawn wins, and the roads come after their sector's far model); the
    // port's depth would put the far model's sheet across the road. MCLA_DEBUG_VIEW=372: whole.)
    if (g.moon_dir.w > 0.5 && !(g.params.w > 369.5 && g.params.w < 372.5) && in.uv2.y > 2000000.0 && in.clip.z > 0.5) { discard; }
    // The city's programs all draw with back faces culled (register 0x2205 = 2 on every PSCity*
    // draw of the captures, the cut-out ones too); the port's meshes are not wound consistently,
    // so it draws both sides - and of a two-sided pair laid in one plane (the shop awnings: a
    // top face and an under face) the wrong one could win and stand unlit at a low sun. The
    // stored normal agrees with the game's winding (formats example tangent_stats, mode near):
    // a face whose stored normal points away from the side it is seen from is a back face.
    // (MCLA_DEBUG_VIEW=98: both sides as before)
    if ((mat.mode < 6u || mat.mode == 14u || mat.mode >= 20u) && in.neon_hl > -0.5 && !(g.params.w > 97.5 && g.params.w < 98.5)) {
        var face_n = cross(dpdx(in.wpos), dpdy(in.wpos));
        let fl = length(face_n);
        let sl = length(in.normal);
        if (fl > 0.0 && sl > 0.01) {
            // (The cars' map, 2026-10-13: there the original culls by the winding of the WARPED triangle - the
            // rasteriser's own test on what VSCity*Env puts out - and a wall that crosses the half's plane close
            // to the eye is turned over by the warp (its corner behind the plane lands far out on the other
            // side). Downtown on Grand Ave. (cap/np1_12; work/lighting/ed_flip.py) the far model's 113 m wall
            // 10 m beside the car is such a face: seen from its front, wound backwards in the half - not drawn
            // by the original; drawn here it lay as one dark sheet over the half's whole sky and the car's lid,
            // roof and rear glass mirrored it (lid 187 178 144 against 231 228 211). In the halves the picture
            // looks along the axis with x to the right, so cross(dpdx, dpdy) of the world position points AWAY
            // from the viewer for a triangle the warp leaves as it is, and towards him for one it turns over:
            // the sign alone is the warped winding. MCLA_DEBUG_VIEW=620: the true side, as until then.)
            if (g.moon_dir.w > 0.5 && !(g.params.w > 619.5 && g.params.w < 620.5)) {
                face_n = -face_n;
            } else if (dot(face_n, g.cam_pos.xyz - in.wpos) < 0.0) { face_n = -face_n; }
            if (dot(in.normal, face_n) < -0.3 * sl * fl) { discard; }
        }
    }
    // Props are drawn with back faces culled too (2026-10-15): register 0x2205 = 0x00018002 on every
    // PSPropDefault / PSPropBumpSpec / PSPropTrafficSignal draw of the captures and on their night
    // programs' draws (the plants, their trunks and the lamp glass have 0x00018000: both sides). The
    // port drew both: of the parking sign on the pole by the garage gate (la_prop_sign_custparking_01x,
    // a plate 6 cm thick seen from the front at a flat angle) the BACK face stood out 3 px beside the
    // front one and, facing the afternoon sun, shone orange where the original shows the wall behind.
    // The same test as the city's: the stored normal against the face seen. Diagnostics 336: both sides.
    if (mat.mode == 9u && in.lpos.z < -9000.0 && in.uv2.x < 0.5 && in.uv2.y < 0.5 && !(in.uv1.y > 100.5 && in.uv1.y < 104.5) && !(g.params.w > 335.5 && g.params.w < 336.5)) {
        var pface_n = cross(dpdx(in.wpos), dpdy(in.wpos));
        let pfl = length(pface_n);
        let psl = length(in.normal);
        if (pfl > 0.0 && psl > 0.01) {
            if (dot(pface_n, g.cam_pos.xyz - in.wpos) < 0.0) { pface_n = -pface_n; }
            if (dot(in.normal, pface_n) < -0.3 * psl * pfl) { discard; }
        }
    }
    // (diagnostics 320 - 322, shopw: the shop panes not drawn)
    if (g.params.w > 319.5 && g.params.w < 322.5 && mat.mode == 22u) { discard; }
    // (332 / 333 / 334: panes with a stored normal pointing to -x not drawn / panes west of x 792 / panes east of it)
    if (g.params.w > 331.5 && g.params.w < 332.5 && mat.mode == 22u && in.normal.x < -0.3) { discard; }
    if (g.params.w > 332.5 && g.params.w < 333.5 && mat.mode == 22u && in.wpos.x < 792.0) { discard; }
    if (g.params.w > 333.5 && g.params.w < 334.5 && mat.mode == 22u && in.wpos.x >= 792.0) { discard; }
    // (329: each pane layer halves what is behind it - a layer count)
    if (g.params.w > 328.5 && g.params.w < 329.5 && mat.mode == 22u) { return vec4<f32>(0.0, 0.0, 0.0, 0.5); }
    // (the city's own materials: their diffuse, grime, decal and window maps are "gamma" textures)
    let xg = xenos_gamma_on() && (mat.mode < 6u || mat.mode == 14u || mat.mode >= 20u);
    // (props, plants and street furniture - PSPropDefault, PSPropBumpSpec, PSPropFoliage, the
    // trunks, lamp glass: sampler 0 carries the gamma flag too, work/lighting/gammascan.py; the
    // cars' textures do not. MCLA_DEBUG_VIEW=92: props as before)
    // (2026-10-14: NOT a plant's impostor card - uv1.y 104: its picture is a linear 8888 surface
    // read without the flag, fetch word 0x81024802; the leaf texture is decoded when the picture
    // is drawn, props/impostor.wgsl. MCLA_DEBUG_VIEW=112: decoded here as before)
    let card_pic = mat.mode == 15u && in.uv1.y > 103.5 && in.uv1.y < 104.5 && !(g.params.w > 111.5 && g.params.w < 112.5);
    let xg_prop = xenos_gamma_on() && in.neon_hl < -0.5 && !xg && !card_pic && !(g.params.w > 91.5 && g.params.w < 92.5);
    // (people - the skinned programs' diffuse, sampler 1, carries the flag as well; uv1.y = 77 is
    // the mark of a figure)
    let xg_fig = xenos_gamma_on() && mat.mode == 9u && in.uv1.y > 76.5 && in.uv1.y < 77.5;
    // (neon2, 2026-10-15: the SIGNS' two textures - CityNeon, CityBillboard, CityStoreSign batches of the sectors,
    // world.rs `upload` - are uploaded as stored and decoded here, AFTER the filter: the reference's fetch filters
    // the stored bytes and puts the filtered value through the pieces. Decoded per texel and filtered linear, as
    // every sRGB texture format does, white lettering bleeds into the dark strokes beside it: a minified sign's
    // dark parts came out up to twice too light (SKATEBOARD at the gate, LIGHTING_WORK.md). The batches that got
    // the raw copies carry bit 14 of uv2.y (world.rs `upload`); props' signs (neon_hl < -0.5) and a sign whose
    // texture was not found for a raw copy keep their sRGB textures. MCLA_DEBUG_VIEW=441: as before (world.rs
    // then makes no raw copies and sets no mark).
    let sign_raw = (mat.mode == 5u || mat.mode == 24u) && in.neon_hl > -0.5 && (u32(round(in.uv2.y)) & 16384u) != 0u && !(g.params.w > 440.5 && g.params.w < 441.5);
    if (sign_raw) {
        albedo = vec4<f32>(select(srgb_decode(albedo.rgb), pwl_decode(albedo.rgb), vec3<bool>(xenos_gamma_on())), albedo.a);
    } else if (xg || xg_prop || xg_fig || xenos_gamma_all()) { albedo = vec4<f32>(xenos_gamma(albedo.rgb), albedo.a); }
    // (the far city's day picture - batches marked by world::mark_lod: decoded the console's
    // piecewise way at every hour, see "The far city" below; diagnostics 84: sRGB as it stood)
    let lod_pwl = !xg && (mat.mode == 0u || mat.mode == 1u) && (u32(round(in.uv2.y)) & 2097152u) != 0u && g.dome.w > 0.5 && g.ambient_ground.w > -0.5 && !(g.params.w > 83.5 && g.params.w < 84.5);
    if (lod_pwl) { albedo = vec4<f32>(xenos_gamma(albedo.rgb), albedo.a); }
    // leaves right in front of the camera would hide the car: cut them away
    if (mat.mode == 15u && distance(in.wpos, g.cam_pos.xyz) < 5.0) {
        discard;
    }
    // (a sector cross-fading with its far model: world.rs writes its alpha into the last dent
    // slot of the sector's object uniform, marked by w = -7; dithered like the props' fade)
    if (obj.dents[5].w < -6.5 && obj.dents[5].x < 0.999) {
        if (obj.dents[5].x <= fract(52.9829189 * fract(dot(in.clip.xy + vec2<f32>(17.0, 5.0), vec2<f32>(0.06711056, 0.00583715))))) {
            discard;
        }
    }
    // (instanced props and plants fading out or in: see vs_inst)
    if (in.lpos.z < -10000.5) {
        let fade = 1.0 + (in.lpos.z + 10000.0) * 0.01;
        if (fade <= fract(52.9829189 * fract(dot(in.clip.xy, vec2<f32>(0.06711056, 0.00583715))))) {
            discard;
        }
    }
    // Glass props (2026-10-13, coverage item 2 - the door leaves of Grand Ave.: la_prop_*glasspane*, props.rs
    // MatMode::PropGlass). The effect xPropGlass, both programs run in numpy on the captures' own constants
    // against these forms (work/lighting/dg/propglass.py, 4000 points: 7e-9 / 1e-8):
    //   day, PSPropGlass f10de4e9f0432a69 (draw 2977 of np1_12):
    //     rgb   = ReflectionIntensity x (0.1 + 0.4 (1 - |n.v|)) x lerp(mini sky, ReflectSphereDay, its alpha)
    //             + 0.4 x gLightColor x sat(v . (2 (n.l) n - l))^12 x sat(4 n.l)
    //     the picture at (0.5 - 0.9 R.x, 0.5 - 0.47 R.y), the mini sky at 0.5 + 0.47 R'.xz with the normal's y
    //     raised by the height / 15 (c254.w = 0.06667 here; the upper windows have / 150)
    //   night, PSMultiLight f1dde8c47d9ff3e2 (draws 1835 of np1_23.5, 1910 of np1_18.25):
    //     rgb   = 0.4 x (0.3 + 0.7 (1 - sat(n.v))) x ReflectSphereNight + the same highlight under the hour's light
    //   alpha = Transparency (c100, 0.4) at every hour; blend SrcAlpha / InvSrcAlpha, both sides, no depth
    //   writes; no lamps, no ambient, no occlusion, no height fade, no shadow term in either.
    // It was drawn as MatMode::BlackGloss - the cars' opaque black trim: a black sheet over the lit lobby by
    // day (door box -37), a white mirror at dusk and night (+116 / +77).
    // uv2 = (Transparency, ReflectionIntensity). MCLA_DEBUG_VIEW=580: not drawn; 581: the pane's colour opaque.
    if (mat.mode == 26u) {
        let n26 = normalize(in.normal);
        let v26 = normalize(g.cam_pos.xyz - in.wpos);
        let nv26 = dot(n26, v26);
        let r26 = n26 * (2.0 * nv26) - v26;
        let pic_uv26 = vec2<f32>(0.5 - 0.9 * r26.x, 0.5 - 0.47 * r26.y);
        let night26 = g.tone.w > 0.5;
        let l_dir26 = select(g.sun_dir.xyz, g.key_dir.xyz, night26);
        let l_col26 = select(g.sun_color.rgb * g.sun_dir.w, g.key_col.rgb, night26);
        let nl26 = dot(n26, l_dir26);
        let hl26 = l_col26 * (0.4 * pow(clamp(dot(v26, n26 * (2.0 * nl26) - l_dir26), 0.0, 1.0), 12.0) * clamp(4.0 * nl26, 0.0, 1.0));
        var lit26 = vec3<f32>(0.0);
        if (night26) {
            let pn26 = xgp(textureSampleLevel(t_third, s_diffuse, pic_uv26, 0.0).rgb);
            lit26 = pn26 * (0.4 * (0.3 + 0.7 * (1.0 - clamp(nv26, 0.0, 1.0)))) + hl26;
        } else {
            let pd26s = textureSampleLevel(t_second, s_diffuse, pic_uv26, 0.0);
            let nv26b = nv26 + v26.y * in.wpos.y * 0.0666667;
            let sky26 = mini_sky(n26.xz * (2.0 * nv26b) - v26.xz);
            lit26 = mix(sky26, xgp(pd26s.rgb), pd26s.a) * (in.uv2.y * (0.1 + 0.4 * (1.0 - abs(nv26)))) + hl26;
        }
        let to26 = in.wpos - g.cam_pos.xyz;
        let d26 = length(to26);
        let fg26 = clamp(1.0 - exp(-(d26 - g.params.x) / max(g.params.y - g.params.x, 1.0)), 0.0, 1.0);
        let hz26 = mix(g.fog_east.rgb, g.fog_west.rgb, 0.5 + 0.5 * to26.x / max(d26, 0.001));
        let fog26 = g.fog_color.rgb + (hz26 - g.fog_color.rgb) * g.fog_east.w;
        let col26 = mix(min(lit26, vec3<f32>(1.9921875)), fog26, clamp(fg26 * g.fog_color.a, 0.0, 1.0));
        return vec4<f32>(tonemap(col26), select(in.uv2.x, 1.0, g.params.w > 580.5 && g.params.w < 581.5));
    }
    // (pass 8: a plant's impostor card - its texture is the type's picture, alpha 1.2 x a as
    // PSPropFoliage of the impostor effect writes it for the alpha-to-coverage)
    let imp_card = mat.mode == 15u && in.uv1.y > 103.5 && in.uv1.y < 104.5;
    // (2026-10-14: a plant's MODEL leaves are not cut at half alpha. Their draws have alpha
    // reference 0 and alpha-to-mask on (colour control 0x8700001c), and the pixel programs
    // write a boosted alpha: by day PSPropFoliage 6ebf788b1c5456fc, instructions 3 - 10,
    // sat(a x (1.3 + 0.95 sat((lod - 1) / 6))), lod the texture's computed level; at night
    // f14129accd056373, sat(2 a) - so the share of a pixel a leaf covers is that alpha, and
    // the hedges stay dense at a distance. Here: dithered. MCLA_DEBUG_VIEW=111: the cut at 0.5)
    let leaf_model = mat.mode == 15u && !imp_card && in.lpos.z < -9000.0 && !(g.params.w > 110.5 && g.params.w < 111.5);
    if (leaf_model) {
        let lts = vec2<f32>(textureDimensions(t_diffuse));
        let ldx = dpdx(in.uv0) * lts;
        let ldy = dpdy(in.uv0) * lts;
        let llod = 0.5 * log2(max(max(dot(ldx, ldx), dot(ldy, ldy)), 1e-8));
        let cov = clamp(albedo.a * select(1.3 + 0.95 * clamp((llod - 1.0) / 6.0, 0.0, 1.0), 2.0, g.tone.w > 0.5), 0.0, 1.0);
        if (cov <= fract(52.9829189 * fract(dot(in.clip.xy + vec2<f32>(31.0, 71.0), vec2<f32>(0.06711056, 0.00583715))))) {
            discard;
        }
    } else if ((mat.mode == 1u || mat.mode == 13u || mat.mode == 15u) && albedo.a < select(0.5, 0.41667, imp_card) && !(mat.mode == 1u && g.moon_dir.w > 0.5)) {
        // (not in the cars' map - moon_dir.w > 0.5, the paraboloid halves: its passes draw the
        // cut-out batches with alpha test and alpha-to-mask off, colour control 0xaa000004 where
        // the main view's have 0x8700001c - a fence is a solid sheet there)
        discard;
    }
    // (the cars' BumpSpecAlpha parts - marked in uv2.y, bit 16 of the colour's alpha: cut out as before)
    if (mat.mode == 9u && in.uv2.y > 0.5 && obj.params.w > 15.5 && (u32(round(in.color.a * 255.0)) & 16u) != 0u && albedo.a < 0.5) {
        discard;
    }
    // characters' hair cards (flagged per vertex): cut out, the fringe fades to dark
    if (mat.mode == 9u && in.uv2.x > 0.5) {
        if (albedo.a < 0.5) {
            discard;
        }
        albedo = vec4<f32>(albedo.rgb * albedo.a * albedo.a, 1.0);
    }
    var second_uv = in.uv0;
    if (mat.mode == 3u || mat.mode == 4u || mat.mode == 14u || mat.mode == 20u || ((mat.mode == 0u || mat.mode == 1u) && in.uv2.x > 0.5)) { second_uv = in.uv1; }
    // signs: lit between the material's own clock times (sign_on), their strength (sign_gain)
    var sign_on = false;
    var sign_gain = 0.0;
    if (mat.mode == 5u || mat.mode == 24u) {
        let a = u32(round(in.uv2.x));
        let b = u32(round(in.uv2.y));
        let hour = g.sun_color.w;
        sign_on = hour > f32(a & 255u) / 8.0 || hour < f32((a >> 8u) & 255u) / 8.0;
        sign_gain = f32((a >> 16u) & 255u) / 8.0;
        // (diagnostics 330: the signs' own light off - NeonIntensity 0, as the override captures cap/ne_0 of the original)
        // (331: also the ambient alone, no highlight; 332: also everything but the ambient)
        if (g.params.w > 329.5 && g.params.w < 332.5) { sign_gain = 0.0; }
        // (shop signs: the night picture on the second UV set. Neon signs: the program has ONE
        // texture coordinate input and takes the neon map with it, scaled by the flip-book -
        // VSCityNeon and the night draws' own vertex program both - so the map lies on the
        // diffuse's coordinates. It stood on the second set until 2026-10-12, which left the
        // poster over the start a flat grey; diagnostics 49 puts it back there.)
        if ((b & 4096u) != 0u && (mat.mode == 24u || (g.params.w > 48.5 && g.params.w < 49.5))) { second_uv = in.uv1; }
        // (the neon map as a flip-book: NumAnimsU x NumAnimsV frames, AnimSpeed frames an
        // hour of the game's clock - the shader steps it by its time-of-day global)
        let nu = max(f32(b & 7u), 1.0);
        let nv = max(f32((b >> 3u) & 7u), 1.0);
        if (nu * nv > 1.5) {
            let frame = floor(hour * f32((b >> 6u) & 63u));
            let row = floor(frame / nu);
            second_uv = (second_uv + vec2<f32>(frame - nu * row, row - nv * floor(row / nv))) / vec2<f32>(nu, nv);
        }
    }
    var second = textureSample(t_second, s_diffuse, second_uv);
    let sg_dx1 = dpdx(second_uv) * sg_s;
    let sg_dy1 = dpdy(second_uv) * sg_s;
    if (sg_on) {
        // (signs: the second picture with the original's footprint, as the first above)
        let dm = vec2<f32>(textureDimensions(t_second));
        let lx = max(length(sg_dx1 * dm), 0.000001);
        let ly = max(length(sg_dy1 * dm), 0.000001);
        second = textureSampleGrad(t_second, s_diffuse, second_uv, sg_dx1 * max(1.0, ly / (4.0 * lx)), sg_dy1 * max(1.0, lx / (4.0 * ly)));
    }
    // (a far model's second texture is its day picture again)
    if (sign_raw) {
        second = vec4<f32>(select(srgb_decode(second.rgb), pwl_decode(second.rgb), vec3<bool>(xenos_gamma_on())), second.a);
    } else if (xg || lod_pwl) { second = vec4<f32>(xenos_gamma(second.rgb), second.a); }
    // (car windows of the full glass program - vehicles.rs marks them lens -1 - carry the detail mask
    // (window_detail_mask.dds) and the crack map (glass_test.dds) in the two slots: the pane itself is
    // drawn white as before; the glass branch below reads the mask's alpha and the crack map)
    var glass_mask = 1.0;
    if (mat.mode == 12u && in.uv2.y < -0.5) {
        glass_mask = albedo.a;
        albedo = vec4<f32>(1.0);
        second = vec4<f32>(1.0);
    }
    let ng = normalize(select(in.normal, vec3<f32>(0.0, 1.0, 0.0), length(in.normal) < 0.01));
    var n = ng;
    // (diagnostics 310: what draws a pixel - red = material mode / 32, green = prop instance, blue = sign lit; alpha-free)
    if (g.params.w > 309.5 && g.params.w < 310.5) { return vec4<f32>(to_scene(vec3<f32>(f32(mat.mode) / 32.0, select(0.0, 1.0, in.lpos.z < -9000.0), select(0.0, 1.0, sign_on) + select(0.0, 0.5, (u32(round(in.uv2.y)) & 8192u) != 0u))), 1.0); }
    // (diagnostics 312 / 313 / 314, signs only: the second texture as sampled, the first, (strength / 8, flip-book frames / 8, second set in use))
    if (g.params.w > 311.5 && g.params.w < 314.5 && (mat.mode == 5u || mat.mode == 24u)) {
        let dv = select(select(vec3<f32>(sign_gain / 8.0, f32(u32(round(in.uv2.y)) & 63u) / 64.0, select(0.0, 1.0, (u32(round(in.uv2.y)) & 4096u) != 0u)), albedo.rgb, g.params.w < 313.5), second.rgb, g.params.w < 312.5);
        return vec4<f32>(to_scene(dv), 1.0);
    }
    // (diagnostics 316, signs only: red = first texture's width / 512, green = second's, blue = fract(u x 8))
    if (g.params.w > 315.5 && g.params.w < 316.5 && mat.mode == 5u) {
        return vec4<f32>(to_scene(vec3<f32>(f32(textureDimensions(t_diffuse).x) / 512.0, f32(textureDimensions(t_second).x) / 512.0, fract(in.uv0.x * 8.0))), 1.0);
    }
    // (diagnostics 317, signs only, yes / no: red = first texture's green above 0.6, green = second texture's green above 0.4, blue = the vertex's occlusion byte above 0.5;
    //  318: red = first above 0.4, green = second above 0.25, blue = night colour's green above 0.5)
    if (g.params.w > 316.5 && g.params.w < 318.5 && mat.mode == 5u) {
        let lo = g.params.w > 317.5;
        return vec4<f32>(to_scene(0.9 * vec3<f32>(step(select(0.6, 0.4, lo), albedo.g), step(select(0.4, 0.25, lo), second.g), step(0.5, select(in.color1.a, in.color1.g, lo)))), 1.0);
    }
    // (diagnostics 315, signs only: red = u within 0.90 .. 0.97, green = v within 0.80 .. 0.86 (the wall lamps' piece of l_sha_gfxlight2), blue = v within 0.14 .. 0.20)
    if (g.params.w > 314.5 && g.params.w < 315.5 && mat.mode == 5u) {
        let fu = fract(in.uv0);
        return vec4<f32>(to_scene(vec3<f32>(select(0.0, 0.9, fu.x > 0.90 && fu.x < 0.97), select(0.0, 0.9, fu.y > 0.80 && fu.y < 0.86), select(0.0, 0.9, fu.y > 0.14 && fu.y < 0.20))), 1.0);
    }
    // (diagnostics 293: the cars' stored tangent sign, bflip - green w +1, magenta w -1, yellow w 0 - on the car bodies / parts: obj.params.w > 15.5)
    if (g.params.w > 292.5 && g.params.w < 293.5 && obj.params.w > 15.5) { return vec4<f32>(to_scene(select(select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 1.0), in.bflip > 0.75), vec3<f32>(1.0, 1.0, 0.0), in.bflip > 0.25 && in.bflip < 0.75)), 1.0); }
    // (for normal maps without stored tangents - the tyres' side walls: the surface's own
    // frame from how position and texture coordinate change across the screen)
    let tb_dp1 = dpdx(in.wpos);
    let tb_dp2 = dpdy(in.wpos);
    let tb_duv1 = dpdx(in.uv0);
    let tb_duv2 = dpdy(in.uv0);
    // Walls with a HeightSpecularSampler (PSCityBumpSpec and its kin): the map's alpha and red
    // tilt the normal along the tangent and binormal (N - x T - y B, both from 2 c - 1), its
    // green (2 g - 1) masks the specular. The tangent frame is rebuilt from the derivatives of
    // position and texture coordinate.
    var wall_spec = vec4<f32>(0.0);
    // (diagnostics 300 / 301: the bump frame's tangent and binormal)
    var dbg_t = vec3<f32>(0.0);
    var dbg_b = vec3<f32>(0.0);
    // (the ground programs with such a map - PSCityGroundGrime, PSCityGroundGrimeDecal: the
    // sidewalks and plazas - do the same; their map is the material's fourth texture and their
    // terms are the same in every material of the city: exponent 8, intensity 0.33, mixer 0.5,
    // white tint. A material without the map has the empty texture there: all zero.)
    var hs = vec4<f32>(0.0);
    var hs_k = 0u;
    var d14 = vec2<f32>(0.0);
    if (mat.mode == 0u || mat.mode == 1u) {
        hs = textureSample(t_third, s_diffuse, in.uv0);
        hs_k = u32(round(in.uv2.y)) & 1048575u;
        // (a wall whose map is not there samples the empty texture, all zero - which read as a tilt of a
        // whole tangent plus a whole binormal and a mask of -1: the bronze door frames of the shop at the
        // gate, l_sha_win_bs. Flat and without the highlight instead, as modes 20 and 14 below have it.
        // MCLA_DEBUG_VIEW=251: as before (250, the binormal's old sign, too).)
        if (hs.r + hs.g + hs.a <= 0.0 && !(g.params.w > 249.5 && g.params.w < 251.5)) { hs_k = 0u; }
    } else if (mat.mode == 20u) {
        hs = textureSample(t_fourth, s_diffuse, in.uv0);
        if (hs.r + hs.g + hs.a > 0.0) { hs_k = 8u + 33u * 256u + 5u * 32768u; }
    } else if (mat.mode == 14u) {
        // Walls with a decal and a grime layer (PSCityGrimeDecal and its night program, PSMultiLight
        // of city/0xbabd0a08): the bump-spec walls' map (their FOURTH texture) and highlight, with
        // the material's own terms - world::mark_decal_spec lays them on the third UV set as whole
        // multiples of 64. (2026-10-14, work/lighting/gw_wallL.py: on the shop wall of the left
        // view at 23:30 the port's diffuse sum was the original's to the third decimal - 0.190
        // 0.151 0.100 - and the highlight, 0.0225 0.0187 0.0100, was missing: 6 % of the wall in
        // display terms. MCLA_DEBUG_VIEW=219: without, as before.)
        hs = textureSample(t_fourth, s_diffuse, in.uv0);
        d14 = round(in.uv2 / 64.0);
        if (d14.y > 0.5 && d14.x > -0.5 && hs.r + hs.g + hs.a > 0.0 && !(g.params.w > 218.5 && g.params.w < 219.5)) { hs_k = 1u; }
    }
    // Instanced props by day (2026-10-14; PSPropDefault 4c6e93d24108070b and PSPropBumpSpec
    // bbf9f06a3ad41e31 run on the customer-parking sign's own vertices with the capture's
    // constants, work/lighting/pr/pr_eval.py): texture x (gLightAmbient + gLightColor x n.l x sun)
    // - no occlusion, no colour stream - plus the walls' kind of highlight,
    //   SpecularIntensity x pow(sat(V . (2 (n.l) n - l)), SpecularExponent) x sat(4 n.l) x sun^3
    //   x lerp(gLightColor, texture, SpecularColorMixer)   [BumpSpec: x (2 g - 1) of its map, the
    //   normal tilted by the map's red and alpha as on the walls]
    // with the material's own terms (PropDefault 8 / 0.33 / 0.7; BumpSpec 175 / 0.7 / 0.25, 25 /
    // 0.5 / 0.5, 15 / 1 / 0.9 ...; props.rs packs them into the first colour word and binds the
    // map as third texture). The generic object sheen that stood here (0.25 x (n.h)^16 x the
    // light's colour + a tenth of the mirrored surroundings) is not in either program: it was
    // the props' surplus in the sun - of the size of the diffuse term on a dark pole.
    // Not the plants' trunks (uv1.y 101 / 102: PSPropFoliageTrunk). MCLA_DEBUG_VIEW=150: as before.
    // Plant trunks by DAY (2026-10-15, PSPropFoliageTrunk 1c6267a307253175 read from capture tg_12, the 43
    // instructions by hand, LIGHTING_WORK.md "Two shop windows ..."): the bump-spec form WITH the vertex's
    // occlusion byte c and the leaves' collector value C in the shadow's place -
    //   diffuse x c x (gLightAmbient + gLightColor x sat(N'.L) x C)
    //   + (c C)^3 x SpecularIntensity x (2 g - 1) x sat(4 N'.L) x (V.R)^SpecularExponent x lerp(gLightColor, diffuse, mixer)
    //   C = lerp(collector.x, collector.y, h / 8) one metre TOWARDS the sun from the pixel (xz - c16.xz),
    //   h the height above the plant's origin, not clamped; N' tilted by the map's alpha / red as on the props.
    // MCLA_DEBUG_VIEW=335: as before (mesh normal, the sun's shadow map, no highlight).
    let trunk_d = mat.mode == 9u && in.lpos.z < -9000.0 && in.uv2.x < 0.5 && in.uv2.y < 0.5 && g.tone.w < 0.5 && in.uv1.y > 100.5 && in.uv1.y < 102.5 && g.dome.w > 0.5 && g.ambient_ground.w > -0.5 && !(g.params.w > 334.5 && g.params.w < 335.5);
    let prop_hl = mat.mode == 9u && in.lpos.z < -9000.0 && in.uv2.x < 0.5 && in.uv2.y < 0.5 && g.tone.w < 0.5 && (!(in.uv1.y > 100.5 && in.uv1.y < 104.5) || trunk_d) && !(g.params.w > 149.5 && g.params.w < 150.5);
    // Instanced props at NIGHT (2026-10-14, work/lighting/gw_prop.py, gw_propvs.py: their two night
    // pixel programs of capture gw_L against rules, 2.4e-15 / 2.1e-14):
    //   plain (d3e24fe4abc0b712, vertex programs ?c2f5a877.. / ?20c121a8..):
    //     texture x H x (gLightAmbient + 1.25 |w| F + key + head lamps + the cell's four lamps)
    //   bump-spec (96f8d43ba6028207, ?09935c2c.. / ?632e71a5..): the walls' form -
    //     (texture x (the same sum) + tint x intensity x (2 g - 1) x lerp(1, texture, mixer) x
    //      (0.3125 |w| F + the lamps' highlights)) x H, the normal tilted by the map
    // with F = gPropNFLColor.rgb x (1 - 0.8 sat(y / 15)), y the vertex's height in the model (both
    // vertex programs: sat(0.0667 y) x -0.8 + 1 into an interpolant's w) - the fill light fades
    // up the prop - and H the walls' fade with height. Nothing is occluded. The port had the sum
    // with the whole fill, no highlight and no H. MCLA_DEBUG_VIEW=218: as before.
    let prop_n = mat.mode == 9u && in.lpos.z < -9000.0 && in.uv2.x < 0.5 && in.uv2.y < 0.5 && g.tone.w > 0.5 && in.uv1.y > 99.5 && !(in.uv1.y > 100.5 && in.uv1.y < 104.5) && !(g.params.w > 217.5 && g.params.w < 218.5);
    // (the bump-spec ones - props.rs marks the batches with a height / specular map in bit 1 of the
    // first colour word's alpha. Of the bump-spec rule the fill light's highlight is on; the map's
    // tilt and the lamps' highlights are NOT - with both on the customer-parking sign went to 85
    // against 82 (its map tilts the normal by 0.9 along the binormal, whose sign and scale in these
    // vertex programs are not read yet). MCLA_DEBUG_VIEW=217 turns them on.)
    let prop_nb = prop_n && (u32(round(in.color.a * 255.0)) & 2u) != 0u;
    // (2026-10-15: both ON. The frame is known - VSPropBumpSpec hands over the instance matrix x T and x (cross(T, N) x w),
    // each NORMALISED (listing lines 28, 40 - 48), w the stored tangent's sign, carried in the normal's length (`bflip`) -
    // and over the prop files the stored tangent is the direction u grows in (props.rs MCLA_PROP_TAN_DUMP), which is the
    // T built below. On the customer-parking sign, whose map is flat, the rule run per pixel with the capture's constants
    // gives 0.1257 0.0951 0.0447 against the port's 0.1254 0.0949 0.0444 (work/lighting/bp_sign.py).
    // MCLA_DEBUG_VIEW=217 now turns the tilt and the lamps' highlights OFF: as it stood.)
    let prop_nb_full = prop_nb && !(g.params.w > 216.5 && g.params.w < 217.5);
    if (mat.mode == 9u) {
        let hs9 = textureSample(t_third, s_diffuse, in.uv0);
        if (prop_hl || prop_nb) {
            // (no map: flat, mask 1)
            hs = select(vec4<f32>(0.5, 1.0, 0.5, 0.5), hs9, hs9.r + hs9.g + hs9.a > 0.0);
            hs_k = 1u;
        }
    }
    if (mat.mode == 0u || mat.mode == 1u || mat.mode == 20u || mat.mode == 14u || prop_hl || prop_nb) {
        // (diagnostics 34: the height / specular map as sampled - red, alpha, green)
        if (g.params.w > 33.5 && g.params.w < 34.5) { return vec4<f32>(to_scene(vec3<f32>(hs.r, hs.a, hs.g)), 1.0); }
        let dpx = dpdx(in.wpos);
        let dpy = dpdy(in.wpos);
        let dux = dpdx(in.uv0);
        let duy = dpdy(in.uv0);
        // (bit 20 of the packed value marks an interior, see `interior` below; the map's terms lie under it)
        if (hs_k != 0u) {
            // The game's frame is the mesh's stored tangent T and cross(T, N) x w (VSCityBumpSpec;
            // the stored w is 1 on every city mesh). Over 1.9 million triangles of the city the
            // stored tangent is the direction the texture's u grows in (examples/tangent_stats
            // of the formats crate) - so cross(T, N) points along growing v wherever the texture
            // is not mirrored and against it where it is (a fifth of the triangles). The frame
            // here is that: T from the derivatives of position and texture coordinate, the
            // binormal from the cross product, not from v.
            let p2 = cross(dpy, ng);
            let p1 = cross(ng, dpx);
            let t0 = p2 * dux.x + p1 * duy.x;
            // (the sum above is dP/du times N . (dP/dx x dP/dy), whose sign is the screen's
            // handedness, not the surface's: taken out)
            let t = t0 * inverseSqrt(max(dot(t0, t0), 1e-12)) * select(-1.0, 1.0, dot(cross(dpx, dpy), ng) >= 0.0);
            // (2026-10-14: x w, the sign the mesh stores in its tangent word - the vertex programs
            // hand over cross(T, N) x w, VSCityBumpSpec 12 - 13, 23; +1 on two thirds of the city's
            // bump-mapped vertices, -1 on the rest, and with it the binormal points the way v grows
            // on 98 % of the triangles. MCLA_DEBUG_VIEW=250: +1 everywhere, as before.)
            let bw = select(1.0 - 2.0 * in.bflip, 1.0, g.params.w > 249.5 && g.params.w < 250.5);
            var b = cross(t, ng) * bw;
            // (the props' vertex programs normalise the binormal after the instance matrix)
            if (prop_hl || prop_nb) { b = b * inverseSqrt(max(dot(b, b), 1e-12)); }
            dbg_t = t;
            dbg_b = b;
            n = normalize(ng - t * (hs.r * 2.0 - 1.0) - b * (hs.a * 2.0 - 1.0));
            if (prop_nb && !prop_nb_full) { n = ng; }
            let k = hs_k;
            wall_spec = vec4<f32>(f32(k & 255u), f32((k >> 8u) & 127u) / 100.0, f32((k >> 15u) & 15u) / 10.0, hs.g * 2.0 - 1.0);
            // (diagnostics 43: the mask cut at zero, as it stood until 2026-10-12)
            if (g.params.w > 42.5 && g.params.w < 43.5) { wall_spec.w = max(wall_spec.w, 0.0); }
            if (mat.mode == 14u) {
                // (the exponent's class, the mixer in twentieths, the intensity in hundredths)
                let c14 = u32(d14.x);
                var e14 = array<f32, 8>(8.0, 15.0, 25.0, 50.0, 100.0, 150.0, 175.0, 255.0);
                wall_spec = vec4<f32>(e14[c14 & 7u], d14.y * 0.01, f32((c14 >> 3u) & 31u) * 0.05, hs.g * 2.0 - 1.0);
            }
            if (prop_hl || prop_nb) {
                // (exponent, intensity and mixer in hundredths; a batch that does not carry them - all 255: PropDefault's)
                let pc = round(in.color.rgb * 255.0);
                wall_spec = vec4<f32>(pc.r, pc.g * 0.01, pc.b * 0.01, hs.g * 2.0 - 1.0);
                if (pc.b > 100.5) { wall_spec = vec4<f32>(8.0, 0.33, 0.7, hs.g * 2.0 - 1.0); }
            }
        }
    }
    // A city vertex WITHOUT a stored normal under the bump-spec programs (2026-10-15; the dry weeds at
    // the foot of the walls: cut-out cards of PSCityBumpSpec 384b588271b6fd6b, 30 % of whose vertices
    // have the packed normal word 0 - work/lighting/pv_vtx.py, pv_zn.py). The program builds
    // N' = normalise(N - x B - y T) with B = cross(T, N) w (instructions 7 - 12): with N = 0 the
    // binormal is 0 too and N' = -sign(2 r - 1) T - the stored tangent (the way u grows), turned by
    // the map's red about its middle: a horizontal vector on an upright card, so that a high sun
    // gives it little. The port had the vertical (or normalise(0)) there: the weeds stood in full
    // noon sun, +29 in the right view's box. MCLA_DEBUG_VIEW=400: as before.
    if ((mat.mode == 0u || mat.mode == 1u) && in.neon_hl > -0.5 && !(length(in.normal) >= 0.01) && !(g.params.w > 399.5 && g.params.w < 400.5)) {
        let z_dpx = dpdx(in.wpos);
        let z_dpy = dpdy(in.wpos);
        let z_dux = dpdx(in.uv0);
        let z_duy = dpdy(in.uv0);
        let z_t = (z_dpx * z_duy.y - z_dpy * z_dux.y) * select(-1.0, 1.0, z_dux.x * z_duy.y - z_dux.y * z_duy.x >= 0.0);
        let z_l = dot(z_t, z_t);
        if (z_l > 0.0) {
            n = z_t * inverseSqrt(z_l) * select(1.0, -1.0, hs.r * 2.0 - 1.0 > 0.0);
            wall_spec = vec4<f32>(0.0);
        }
    }
    let v = normalize(g.cam_pos.xyz - in.wpos);
    let night = clamp(1.0 - g.sky_zenith.a * 2.0, 0.0, 1.0);
    // (gLightAmbient: one colour for every direction)
    // (+ the night fill light at the object's place: traffic cars, gLightAmbient + map / 4 x m_NFLFade)
    // (pass 9: figures - skin.rs, the generic object mode with uv1.y = 77 - carry theirs in the
    // second colour: rgb = four times the sum; the player's car shows 0.4 of the whole ambient
    // at night, a recorded factor of unknown origin, in obj.fill.w - 0 = none)
    let fig_fill = select(vec3<f32>(0.0), in.color1.rgb * 0.25, mat.mode == 9u && in.uv1.y > 76.5 && in.uv1.y < 77.5);
    var hemi = (g.ambient_sky.rgb + obj.fill.rgb + fig_fill) * select(1.0, obj.fill.w, obj.fill.w > 0.0);
    // (the garage: the car's programs get their own ambient there - c57 ambYPos = sm_vGarageAmbientLighting,
    // a grey of 0.196, where the room's programs hold TODGarage's (0.035, 0.051, 0.027); the player's car is
    // the object with the night factor in obj.fill.w. Diagnostics 711: the room's ambient on the car.)
    if (g.car_amb.w > 0.5 && obj.fill.w > 0.0 && !(g.params.w > 710.5 && g.params.w < 711.5)) { hemi = g.car_amb.rgb; }
    let shadow = sun_shadow(in.wpos, ng);
    var ndl = max(dot(n, g.sun_dir.xyz), 0.0) * shadow;
    // PSPropFoliage: leaves are not lit by the light's colour or the ambient at all - the
    // texture times a brightness read from the "fast shadow collector" (a 256 m top-down map of
    // sun visibility round the camera, blurred; the leaves take its value 8 m above the ground).
    // Here: the sun's visibility at the leaf itself, softened.
    // (the collector holds two values per metre of ground: visibility at the ground and 8 m above
    // it; a leaf blends them by its height in the plant, sat(h / 8))
    var leaf = 1.0;
    // (trunks by day: c x C for the highlight's cube; -1 = not a trunk)
    var trunk_cut = -1.0;
    var trunk_c = 1.0;
    if (trunk_d) {
        let tsp = vec3<f32>(in.wpos.x + g.sun_dir.x, in.lpos.y, in.wpos.z + g.sun_dir.z);
        if (g.shadow_params.x > 0.0) { trunk_c = mix(soft_collector(tsp), soft_collector_high(tsp), in.lpos.x * 0.125); }
        trunk_cut = in.color1.a * trunk_c;
    }
    if (mat.mode == 15u && g.tone.w < 0.5 && g.shadow_params.x > 0.0) {
        if (in.lpos.z < -9000.0) {
            // instanced plants: lpos = (height above the plant's foot, the foot's height)
            let foot = vec3<f32>(in.wpos.x, in.lpos.y, in.wpos.z);
            leaf = mix(soft_collector(foot + vec3<f32>(0.0, 0.3, 0.0)), soft_collector_high(foot + vec3<f32>(0.0, 8.0, 0.0)), clamp(in.lpos.x * 0.125, 0.0, 1.0));
            // (2026-10-14, a plant's MODEL - VSPropFoliage 6457b0b6e2d0c45a, instructions 39 -
            // 47, run on numbers by work/lighting/plantday_verify.py: the map is read two metres
            // down the light's way from the leaf, and the height that blends ground -> 8 m
            // layer is taken there too: sat((h - 2 sun.y) / 8). 112: as before)
            if (!(in.uv1.y > 103.5 && in.uv1.y < 104.5) && !(g.params.w > 111.5 && g.params.w < 112.5)) {
                let lsp = foot - vec3<f32>(2.0 * g.sun_dir.x, 0.0, 2.0 * g.sun_dir.z);
                leaf = mix(soft_collector(lsp), soft_collector_high(lsp), clamp((in.lpos.x - 2.0 * g.sun_dir.y) * 0.125, 0.0, 1.0));
            } else if (!(g.params.w > 111.5 && g.params.w < 112.5)) {
                // (a card - PSPropFoliage eeff113e5fd82321, instructions 5 - 13: ONE metre down
                // the light's way, the height as it is: sat(h / 8))
                let csp = foot - vec3<f32>(g.sun_dir.x, 0.0, g.sun_dir.z);
                leaf = mix(soft_collector(csp), soft_collector_high(csp), clamp(in.lpos.x * 0.125, 0.0, 1.0));
            }
        } else {
            leaf = soft_collector(in.wpos + vec3<f32>(0.0, 2.0, 0.0));
        }
    }
    let h = normalize(g.sun_dir.xyz + v);
    let ndh = max(dot(n, h), 0.0);
    // (vertex occlusion, lifted towards 1 by OcclusionInfo.w)
    var occ = mix(in.color1.a, 1.0, g.grade.w);
    var base = albedo.rgb;
    var spec = 0.0;
    var gloss = 16.0;
    // rims: how much of a mirror the zone is (below 0: by its brightness, as the model is)
    var rim_mirror = -1.0;
    // (the sun's highlight on car paint goes by the paint type's specular intensity)
    var spec_k = 1.0;
    var alpha = 1.0;
    var emissive = vec3<f32>(0.0);
    // paint scraped off (damage): no clear coat there - the damage map's bare metal mask (PS_CarPaint's
    // m); its rubbed paint
    var scraped = 0.0;
    var rubbed = 0.0;

    if (mat.mode == 25u) {
        // PSCityNeonTint: the tube's day colour
        let ti = u32(round(in.uv2.x));
        base = vec3<f32>(f32(ti & 127u), f32((ti >> 7u) & 127u), f32((ti >> 14u) & 127u)) / 127.0;
    } else if (mat.mode == 21u) {
        // PSCityTerrain / PSCityGrass: the channel map (on the second UV set) weights three
        // textures - its red, green and blue, normalised as a vector - e.g. dirt, rock and
        // brush on a hillside
        let cm = textureSample(t_diffuse, s_diffuse, in.uv1).rgb;
        let w = cm / max(length(cm), 0.001);
        base = second.rgb * w.r + xgm(textureSample(t_third, s_diffuse, in.uv0).rgb) * w.g + xgm(textureSample(t_fourth, s_diffuse, in.uv0).rgb) * w.b;
    } else if (mat.mode == 4u || mat.mode == 20u) {
        // The ground, as the game's own pixel shaders compose it (read from their microcode):
        // PSCityRoad / PSCityTunnelRoad take the diffuse map's RED channel as a grey level -
        // the green channel is their specular mask; PSCitySidewalk, PSCityGroundGrime* and
        // PSCityRiverRoad use the map in its own colours.
        var ground = albedo.rgb;
        if (mat.mode == 4u) {
            ground = vec3<f32>(albedo.r);
        }
        // grime layer (cracks, tar, tyre marks) on the third UV set, weighted per vertex
        // (mix(ground, grime.rgb * vertex colour, grime.a * vertex alpha) in all of them)
        var grime = textureSample(t_third, s_diffuse, in.uv2);
        if (xg) { grime = vec4<f32>(xenos_gamma(grime.rgb), grime.a); }
        if (mat.mode == 4u) {
            // (PSCityRoad: the decal atlas first, the grime over it)
            base = mix(ground, second.rgb, second.a);
            // (the cars' map - moon_dir.w > 0.5 - has the road without its grime layer: PSCityRoadEnv
            // and the night map's road program fetch the diffuse and the decal map alone.
            // Diagnostics 210: with it, as until 2026-10-14)
            if (!(g.moon_dir.w > 0.5) || (g.params.w > 209.5 && g.params.w < 210.5)) {
                base = mix(base, grime.rgb * in.color.rgb, grime.a * in.color.a);
            }
        } else {
            // (PSCityGroundGrimeDecal, instr. 15 - 18: the decal atlas - lane lines, arrows,
            // crossings, manholes, on the second UV set - first, the grime over it, as on the road)
            base = mix(ground, second.rgb, second.a);
            base = mix(base, grime.rgb * in.color.rgb, grime.a * in.color.a);
        }
        // rain: wet asphalt is darker and mirrors the sky and the lights
        // (the ground-grime programs know no wetness: PSCityGroundGrime has no gReflectionParams)
        // (and under the weather's keys the road's wetness is the road program's own business by
        // day; at night its lamp-lit program, PSMultiLight's road variant, is not ported with
        // its wet terms yet - the mirror and highlight this older code laid on a wet night road
        // were several times what the original shows, so there the road stays as in the dry)
        // (interior floors - the sector's interior uniform: their programs have no wet terms at all; 75: as before)
        let dry_in = obj.dents[5].w < -6.5 && obj.dents[5].y > 0.5 && !(g.params.w > 74.5 && g.params.w < 75.5);
        let wet = select(g.shadow_params.z, 0.0, dry_in || (g.dome.w > 0.5 && g.ambient_ground.w > -0.5 && (mat.mode == 20u || g.tone.w > 0.5)));
        base = base * (1.0 - 0.4 * wet);
        spec = 2.2 * wet;
        gloss = 70.0;
    } else if (mat.mode == 14u) {
        base = mix(albedo.rgb, second.rgb, second.a);
        // PSCityGrimeDecal (walls with a decal and a grime layer, read from its listing): over
        // the decal the grime as on the grime walls - by its alpha and the first colour word's,
        // tinted by that colour - on the mesh's third texture coordinates. (The batch's third
        // texture; a decal without a grime map has the empty texture there: nothing.)
        // (less the specular terms' whole multiples of 64, world::mark_decal_spec)
        let gr14s = textureSample(t_third, s_diffuse, in.uv2 - 64.0 * round(in.uv2 / 64.0));
        let gr14 = vec4<f32>(xgm(gr14s.rgb), gr14s.a);
        base = mix(base, gr14.rgb * in.color.rgb, gr14.a * in.color.a);
    } else if (mat.mode == 0u || mat.mode == 1u) {
        // grime overlay: an RGBA dirt layer blended in by its own alpha and the vertex weight
        // (PSCityGrime: mix(diffuse, grime * vertex colour, grime alpha * vertex alpha), the
        // grime on the second UV set)
        base = mix(albedo.rgb, second.rgb * mix(vec3<f32>(1.0), in.color.rgb, step(0.5, in.uv2.x)), second.a * in.color.a);
    } else if (mat.mode == 6u) {
        // car paint: tint from object colour, strong specular + fresnel reflection
        occ = 1.0;
        // (the damage, looked up here - the flake's normal below needs to know where the paint is
        // gone - and laid on after the paint as before)
        // (PS_CarPaint's scrapes, s = the vertex's scrape: the scrape texture at uv0 x (4, 3) - its
        // alpha the scratches, its green the rubbed paint; k = sat(2 x scratches x (0.75 s + 0.25
        // [s > 0.125])) cuts through the livery, m = sat(k - livery's alpha) is bare metal)
        let s6 = in.dmg.x;
        var k6 = 0.0;
        if (s6 > 0.0) {
            let sc = textureSample(t_scrape, s_diffuse, in.uv0 * vec2<f32>(4.0, 3.0));
            k6 = clamp(2.0 * sc.a * (0.75 * s6 + 0.25 * select(0.0, 1.0, s6 > 0.125)), 0.0, 1.0);
            rubbed = sc.g * s6;
        }
        // livery: the vinyl texture lies in the body's unwrap (first UV set)
        let vinyl = textureSample(t_third, s_diffuse, in.uv0);
        scraped = clamp(k6 - vinyl.a, 0.0, 1.0);
        // (the flakes: x sat(1 - 2 m))
        let scraped6 = 1.0 - clamp(1.0 - 2.0 * scraped, 0.0, 1.0);
        // The paint's two colours (PS_CarPaint's DL2 and DL3). Two-tone: the first face-on,
        // the second at a glancing angle, by (n.v)^PaintFalloff. The fades (VS_CarPaint): how
        // far along the body's box the point lies - front to back, left to right or bottom to
        // top - the change taking the middle half of it (sat(2 t - 0.5)).
        // (a paint area with its own entry - the player's doors, hood, bumpers, skirts - is
        // drawn with that entry's two colours, type and fade: sub_823580D0 per part)
        var pc_a = obj.color.rgb;
        var pc_b = obj.color2.rgb;
        var fade = u32(round(obj.color2.w));
        var pti = min((u32(round(obj.params.w)) / 32u) % 8u, 5u);
        let p_area = paint_area(in.uv2.y);
        if (p_area > 0u) {
            let pk = u32(round(obj.area_b[p_area - 1u].w));
            pc_a = obj.area_a[p_area - 1u].rgb;
            pc_b = obj.area_b[p_area - 1u].rgb;
            fade = pk / 8u;
            pti = min(pk % 8u, 5u);
        }
        // What the two programs do with the constants sub_823580D0 sets (2026-10-13, paint areas
        // round; LIGHTING_WORK.md "The paint areas' fades"):
        //   vertex colour  C = lerp(DL2, DL3, sat(2 t - 0.5))            (VS_CarPaint 19 - 32, o3)
        //   two-tone       F = lerp(DL3, DL2, sat(|nf . v| ^ DL1.w))    (PS_CarPaint 60, 90 - 93)
        //   paint            = lerp(F, C, DL2.w)                         (PS_CarPaint 94 - 95)
        // nf the FLAKE's normal (the one the soft highlight is looked up along), DL1.w the paint
        // type's falloff, DL2.w the fade's strength: 1 for the three directional fades - the
        // falloff the game still passes with them has no weight, they are the vertex colour and
        // nothing else - 0 for None (both colours the first, falloff 0), ColorShift and the matte
        // paints (second colour (0.6, 0.6, 0.55), the type's falloff: 0.05 / 0.1, a near-flat
        // shift that only shows at the rim). Run on numbers against this branch for the six
        // kinds, work/lighting/ar_px.py: 4e-14. Until then the two-tone took the smooth normal
        // (0.19 off on a metallic's flakes close up, 0.16 on dirty matte) and the directional
        // fades were worked out per pixel (the same but for the kinks of sat(2 t - 0.5) inside a
        // triangle). Diagnostics 690: as before.
        var paint = pc_a;
        if (g.params.w > 689.5 && g.params.w < 690.5) {
            if (fade == 1u) {
                var pbt0 = PAINT_B;
                paint = mix(pc_b, pc_a, pow(clamp(dot(n, v), 0.0, 1.0), max(pbt0[pti].w, 0.05)));
            } else if (fade >= 2u) {
                let rel = (in.lpos - obj.fade_min.xyz) / obj.fade_size.xyz;
                var t = rel.z;
                if (fade == 3u) { t = rel.x; }
                if (fade == 4u) { t = rel.y; }
                paint = mix(pc_a, pc_b, clamp(2.0 * clamp(t, 0.0, 1.0) - 0.5, 0.0, 1.0));
            }
        } else if (fade == 1u) {
            var pbt = PAINT_B;
            let nf6 = paint_flake_n(n, in.uv0, pti, scraped6);
            paint = mix(pc_b, pc_a, clamp(pow(abs(dot(nf6, v)), pbt[pti].w), 0.0, 1.0));
        } else if (fade >= 2u) {
            paint = mix(pc_a, pc_b, in.paint_s);
        }
        // (diagnostics 691: the directional fade's vertex value itself; 692: the two-tone's weight)
        if (g.params.w > 690.5 && g.params.w < 691.5) { return vec4<f32>(to_scene(vec3<f32>(in.paint_s)), 1.0); }
        if (g.params.w > 691.5 && g.params.w < 692.5) {
            var pbt2 = PAINT_B;
            return vec4<f32>(to_scene(vec3<f32>(clamp(pow(abs(dot(paint_flake_n(n, in.uv0, pti, scraped6), v)), pbt2[pti].w), 0.0, 1.0))), 1.0);
        }
        // (a part painted by itself carries its colour in the vertex colour, alpha set)
        if (in.color.a > 0.5 && p_area == 0u) { paint = in.color.rgb; }
        base = mix(paint, vinyl.rgb, clamp(vinyl.a - k6, 0.0, 1.0));
        spec = 1.2;
        gloss = 110.0;
    } else if (mat.mode == 7u) {
        // chrome / metal
        occ = 1.0;
        base = vec3<f32>(0.75);
        spec = 2.5;
        gloss = 160.0;
    } else if (mat.mode == 13u || mat.mode == 19u) {
        // black gloss plastic / opaque black glass / grills
        occ = 1.0;
        base = vec3<f32>(0.02, 0.02, 0.022);
        spec = 1.2;
        gloss = 90.0;
    } else if (mat.mode == 12u) {
        // vehicle glass: dark tint, blended, mostly reflection
        occ = 1.0;
        base = vec3<f32>(0.03, 0.035, 0.04);
        alpha = 0.62;
        spec = 2.0;
        gloss = 160.0;
        // (the glass over a lamp: a red lens over the tail lamps, clear over the others)
        // (the garage's window tint: its colour, and ten steps from light to limousine - the
        // steps' own opacities are not on the disc)
        // (w is the record's darkness: the tint menu's row k = (w - 0.372) / 0.0528, 1..9)
        if (obj.tint.w > 0.4 && in.uv2.y < 0.5 && in.uv2.x < 0.5) {
            base = obj.tint.rgb * 0.14 + vec3<f32>(0.01);
            alpha = 0.30 + 0.065 * ((obj.tint.w - 0.372) / 0.0528 - 1.0);
        }
        if (in.uv2.y > 0.5 && g.dome.w > 0.5 && g.ambient_ground.w > -0.5) {
            // The lamps' lenses under the weather's keys, as recorded (PS_Light's glass
            // variant, BumpedTintColors c209 = (0, 0.0035, 0.005, 0.372) on every lens of the
            // car, tail lamps included): smoked glass a good third opaque - the red is the lamp
            // behind it, not the lens.
            base = vec3<f32>(0.0, 0.0035, 0.005);
            alpha = 0.372;
        } else if (in.uv2.y > 1.5) {
            alpha = 0.16;
        } else if (in.uv2.y > 0.5) {
            base = vec3<f32>(0.42, 0.02, 0.015);
            alpha = 0.78;
        }
    } else if (mat.mode == 8u) {
        // car lights: diffuse lens + emissive when on
        occ = 1.0;
        // The lamp type is stored in the second UV set (same order as the glow table of
        // tune/vehicle): 0 brake, 1 tail, 2 signal, 3 head, 4 high beam, 6 reverse, 8 fog,
        // 12 third brake, 13 police red, 14 police blue, 15-18 turn signals.
        let lamp = i32(round(in.uv1.x));
        let lights_on = obj.params.x;
        let braking = clamp(obj.params.y * 4.0, 0.0, 1.0);
        let state = i32(round(obj.params.w)) % 16;
        let siren = state % 4;
        let reversing = f32(state / 4);
        var glow = 0.0;
        var tint = vec3<f32>(1.0);
        if (lamp == 0 || lamp == 12) {
            // (deep red: brighter than this the tone curve washes a brake lamp out to pink)
            glow = 1.7 * braking + 0.25 * lights_on;
            tint = vec3<f32>(1.0, 0.045, 0.03);
        } else if (lamp == 1) {
            glow = 0.8 * lights_on + 0.5 * braking;
            tint = vec3<f32>(1.0, 0.045, 0.03);
        } else if (lamp == 3) {
            glow = 2.8 * lights_on;
            tint = vec3<f32>(1.0, 0.97, 0.9);
        } else if (lamp == 6) {
            glow = 2.0 * reversing;
        } else if (lamp == 13) {
            glow = 5.0 * f32(siren == 1);
            tint = vec3<f32>(1.0, 0.08, 0.05);
        } else if (lamp == 14) {
            glow = 5.0 * f32(siren == 2);
            tint = vec3<f32>(0.1, 0.25, 1.0);
        }
        // The light texture is a grey reflector pattern; the lens colour follows the lamp type.
        var lens = vec3<f32>(0.75, 0.76, 0.80);
        if (lamp == 0 || lamp == 1 || lamp == 12 || lamp == 13) {
            lens = vec3<f32>(0.55, 0.035, 0.03);
        } else if (lamp == 2 || lamp >= 15) {
            lens = vec3<f32>(0.85, 0.38, 0.04);
        } else if (lamp == 14) {
            lens = vec3<f32>(0.06, 0.12, 0.6);
        } else if (in.uv2.y > 0.5 && lamp != 6) {
            // (the unlit faces of a tail lamp - the "noglow" types - are its red lens too)
            lens = vec3<f32>(0.42, 0.03, 0.025);
        } else if (lamp == 5 || lamp == 7 || (lamp >= 9 && lamp <= 11) || lamp > 18) {
            // (elsewhere they are the lamp's bright-metal reflector)
            lens = vec3<f32>(0.42, 0.43, 0.45);
        }
        let pattern = max(albedo.rgb, vec3<f32>(0.0));
        base = lens * (0.18 + 1.6 * pattern);
        emissive = (0.35 + 2.0 * pattern) * tint * glow;
        spec = 0.8;
        gloss = 60.0;
    } else if (mat.mode == 2u) {
        // glass / water: blended
        occ = 1.0;
        alpha = max(albedo.a, 0.35);
        spec = 1.5;
        gloss = 120.0;
    } else if (mat.mode == 16u) {
        // traffic signal head: the left strip of the texture holds the red / yellow /
        // green lamps top to bottom, the right half the pedestrian hand and walking man
        occ = 1.0;
        let state = i32(round(in.uv1.x));
        let sat = max(albedo.r, max(albedo.g, albedo.b)) - min(albedo.r, min(albedo.g, albedo.b));
        let lamp_px = smoothstep(0.06, 0.20, sat);
        var on = 0.0;
        if (in.uv0.x < 0.30) {
            let lamp = clamp(i32(floor(in.uv0.y * 3.0)), 0, 2);
            on = f32(lamp == state);
        } else if (in.uv0.x > 0.55 && in.uv0.y > 0.22) {
            // "don't walk" hand unless the parallel traffic has green
            let hand = in.uv0.x < 0.79;
            on = f32(hand != (state == 2));
            emissive = albedo.rgb * 1.2 * on * smoothstep(0.25, 0.6, max(albedo.r, albedo.g));
        }
        if (g.params.w > 7.5 && g.params.w < 8.5) { return vec4<f32>(to_scene(vec3<f32>(f32(state) / 3.0, in.uv0.y, lamp_px)), 1.0); }
        base = albedo.rgb * mix(1.0, 0.22, lamp_px * (1.0 - on));
        emissive = emissive + albedo.rgb * lamp_px * on * 3.2 * f32(in.uv0.x < 0.30);
        spec = 0.3;
    } else if (mat.mode == 18u) {
        // Wheel rim (shader RimMain): uv1.x selects one of five material zones - zone 0
        // is the dark barrel / recesses, the others polished metal - and the vertex
        // colour alpha carries baked occlusion.
        let zone = i32(round(in.uv1.x));
        occ = mix(0.25, 1.0, in.color.a);
        if (zone >= 10) {
            // a tyre with its set of textures: black rubber, the lettering's colour from the
            // first map, the side wall's relief from the second (a normal map)
            let nm = second.xyz * 2.0 - vec3<f32>(1.0);
            let p2 = cross(tb_dp2, ng);
            let p1 = cross(ng, tb_dp1);
            let tan = p2 * tb_duv1.x + p1 * tb_duv2.x;
            var bit = p2 * tb_duv1.y + p1 * tb_duv2.y;
            // (2026-10-14: the game's binormal is cross(T, N) x w, w the stored tangent word's sign - the car's vertices
            // carry it in their normal's length, bflip - taken to the world by the model matrix, which for a mirrored one
            // is cross(T, N) x det. Here: T from the derivatives, the binormal's length and direction from the rule.
            // MCLA_DEBUG_VIEW=290: bit by derivatives as before; 291: bit x (1 - 2 bflip); 292: the rule against the old, red same / green reversed)
            let tr_b = cross(tan * inverseSqrt(max(dot(tan, tan), 1e-12)), ng) * ((1.0 - 2.0 * in.bflip) * select(-1.0, 1.0, dot(cross(obj.model[0].xyz, obj.model[1].xyz), obj.model[2].xyz) >= 0.0) * length(bit));
            if (g.params.w > 291.5 && g.params.w < 292.5) { return vec4<f32>(to_scene(select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), dot(tr_b, bit) >= 0.0)), 1.0); }
            if (g.params.w > 290.5 && g.params.w < 291.5) { bit = bit * (1.0 - 2.0 * in.bflip); }
            else if (!(g.params.w > 289.5 && g.params.w < 290.5)) { bit = tr_b; }
            let inv = inverseSqrt(max(max(dot(tan, tan), dot(bit, bit)), 1e-12));
            n = normalize(tan * inv * nm.x * 1.6 + bit * inv * nm.y * 1.6 + ng * max(nm.z, 0.2));
            base = vec3<f32>(0.035, 0.035, 0.038) + albedo.rgb * 0.85;
            spec = 0.45;
            gloss = 24.0;
            // (the side wall's sheen is the surroundings' on the relief: `g_wh_brake`)
            rim_mirror = 0.05;
            occ = 1.0;
        } else if (zone == 8) {
            // a brake's rotor: its texture is the pattern, the metal is made here
            base = albedo.rgb * 2.4;
            spec = 0.9;
            gloss = 40.0;
            rim_mirror = 0.10;
            occ = mix(0.55, 1.0, in.color.a);
        } else if (zone == 9) {
            // its caliper: painted, or its own texture
            base = mix(in.color.rgb, albedo.rgb * in.color.rgb * 1.5, step(0.5, in.uv1.y));
            spec = 0.5;
            gloss = 50.0;
            rim_mirror = 0.04;
            occ = mix(0.55, 1.0, in.color.a);
        } else if (zone == 0) {
            base = vec3<f32>(0.05);
            spec = 0.2;
            gloss = 30.0;
        } else {
            base = vec3<f32>(0.62, 0.63, 0.65);
            spec = 1.6;
            gloss = 120.0;
            // the finish chosen in the garage (MATERIALS/PAINT: spoke A, spoke B, lip), which
            // a wheel's object carries where a body has its dents
            let finish = obj.dents[clamp(zone - 1, 0, 2)];
            if (finish.w > 3.5) {
                // carbon fibre (rim material 2): PS_Multi draws it a flat 0.2, no light, no mirror
                // (approx. here, where the rims are not drawn by that program's rule: the same grey, unlit terms off)
                base = vec3<f32>(0.2);
                spec = 0.0;
                gloss = 1.0;
                rim_mirror = 0.0;
            } else if (finish.w > 2.5) {
                // brushed: the metal dull, its highlight wide
                // (reference `g_rm_1`: pale satin metal)
                base = vec3<f32>(0.70, 0.71, 0.73);
                spec = 0.5;
                gloss = 18.0;
                rim_mirror = 0.10;
            } else if (finish.w > 1.5) {
                rim_mirror = 0.42;
            } else if (finish.w > 0.5) {
                // painted: the colour under a clear coat
                base = finish.rgb;
                spec = 0.6;
                gloss = 70.0;
                rim_mirror = 0.07;
            }
        }
    } else if (mat.mode == 17u) {
        // Cabin trim: the leather / cloth / plastic / carpet maps are pale patterns that
        // the game multiplies with the chosen interior colour (charcoal by default).
        occ = 1.0;
        base = albedo.rgb * vec3<f32>(0.16, 0.16, 0.17);
        // the colours chosen in the garage, by the mesh's zone (uv1.x): the leathers 9, 10,
        // 11 are INTERIOR COLOR A (dash top, upper doors), B (wheel rim, door panels,
        // bolsters) and C (seat centres); zone 7 (around the dials, the console) is GAUGE
        // COLOR A and the carpet (0) GAUGE COLOR B (references `g_ic_a..c`, `s11/int_zones`)
        // (corrected: which zone takes which of the five groups A..E is the vehicle's own table
        // - its .r83, read in vehicles.rs, 100 + group in uv2.y - not fixed zones; the R8's is
        // carpet 0 -> C, the plain zones 6 / 7 / 8 -> E / D / C, the leathers 9 / 10 / 11 -> A / B /
        // C, the cloth 16 -> B. The gauge colours colour no trim. Groups 5 and up are not the
        // garage's: the stock look.)
        let zid = i32(round(in.uv1.x));
        var slot = i32(round(in.uv2.y)) - 100;
        if (slot > 4) { slot = -1; }
        if (slot >= 0) {
            let cabin = obj.interior[slot];
            if (cabin.w > 0.5) {
                // (approx., by eye as before - now by the zone's kind of surface, which is what the
                // original's surface values go by: carpet 0..2 dim, the plain zones 6..8, the rest)
                var strength = 0.55;
                if (zid >= 6 && zid <= 8) { strength = 0.30; }
                if (zid <= 2) { strength = 0.15; }
                base = albedo.rgb * cabin.rgb * strength;
            }
        }
        spec = 0.15;
    } else if (mat.mode == 9u) {
        // generic object (interior trim etc.)
        occ = 1.0;
        spec = 0.25;
        // (instanced props by day: their programs' own highlight, see `prop_hl`)
        if (prop_hl || prop_n) { spec = 0.0; }
    } else if (mat.mode == 10u) {
        // tyre rubber
        occ = 1.0;
        base = vec3<f32>(0.035, 0.035, 0.038);
        spec = 0.15;
        gloss = 12.0;
    } else if (mat.mode == 11u) {
        // checkpoint marker: unlit, pulsing, blended
        // (obj.params.x: flicker speed of the game's checkpoint lights, 0 = slow pulse)
        var pulse = 0.75 + 0.25 * sin(g.cam_pos.w * 6.0 + in.wpos.y * 3.0);
        if (obj.params.x > 0.0) {
            let t = g.cam_pos.w * obj.params.x;
            pulse = 0.72 + 0.28 * fract(sin(floor(t) * 12.9898) * 43758.5453) + 0.1 * sin(in.wpos.y * 0.8 - g.cam_pos.w * 9.0);
        }
        let fade = 1.0 - clamp((in.wpos.y - obj.params.y) / max(obj.params.z, 0.1), 0.0, 1.0);
        return vec4<f32>(obj.color.rgb * pulse * 2.0, obj.color.a * fade);
    }

    if (g.params.w > 5.5 && g.params.w < 6.5) { return vec4<f32>(to_scene(albedo.rgb), 1.0); }
    if (g.params.w > 6.5 && g.params.w < 7.5) { return vec4<f32>(to_scene(vec3<f32>(f32(mat.mode) / 32.0, f32(mat.mode % 4u) / 3.0, f32(mat.mode % 3u) / 2.0)), 1.0); }
    if (g.params.w > 0.5 && g.params.w < 1.5) { return vec4<f32>(to_scene(vec3<f32>(shadow)), 1.0); }
    // (diagnostics 360 / 361: the sun's visibility as a NUMBER - (shadow - floor) / (1 - floor) in 64 steps, its high
    // three bits in view 360 and its low three in 361, one bit a channel, so that no tone curve touches it;
    // work/lighting/se_num.py puts the two shots together)
    if (g.params.w > 359.5 && g.params.w < 361.5) {
        let v6 = u32(clamp((shadow - (1.0 - g.shadow_params.x)) / max(g.shadow_params.x, 1e-4), 0.0, 1.0) * 63.0 + 0.5);
        let b3 = select(v6 & 7u, v6 >> 3u, g.params.w < 360.5);
        return vec4<f32>(f32(b3 & 4u) * 0.25, f32(b3 & 2u) * 0.5, f32(b3 & 1u), 1.0);
    }
    // (diagnostics 68: which cascade holds the point - sun_shadow_casc)
    if (g.params.w > 67.5 && g.params.w < 68.5) { return vec4<f32>(to_scene(vec3<f32>(shadow, fract(shadow * 4.0 + 0.5), 1.0 - shadow)), 1.0); }
    if (g.params.w > 1.5 && g.params.w < 2.5) { return vec4<f32>(to_scene(n * 0.5 + 0.5), 1.0); }
    if (g.params.w > 2.5 && g.params.w < 3.5) { return vec4<f32>(to_scene(in.color.rgb), 1.0); }
    if (g.params.w > 3.5 && g.params.w < 4.5) { return vec4<f32>(to_scene(in.color1.rgb), 1.0); }
    // (71: the night fill colour by place - the instance's own and the object's, work/map/pass7_notes.md)
    if (g.params.w > 70.5 && g.params.w < 71.5) { return vec4<f32>(to_scene(in.fill.rgb + obj.fill.rgb + fig_fill), 1.0); }
    // (diagnostics 24: the first colour set by bands - alpha > 0.9 red, > 0.6 green, > 0.3 blue, > 0.05 grey, else black; 25 the same for its blue)
    if (g.params.w > 23.5 && g.params.w < 25.5) {
        let q = select(in.color.a, in.color.b, g.params.w > 24.5);
        var c = vec3<f32>(0.0);
        if (q > 0.9) { c = vec3<f32>(0.3, 0.0, 0.0); } else if (q > 0.6) { c = vec3<f32>(0.0, 0.3, 0.0); } else if (q > 0.3) { c = vec3<f32>(0.0, 0.0, 0.3); } else if (q > 0.05) { c = vec3<f32>(0.08); }
        return vec4<f32>(c, 1.0);
    }
    // (31: the second texture; 32: the surroundings along the mirror ray)
    if (g.params.w > 30.5 && g.params.w < 31.5) { return vec4<f32>(to_scene(second.rgb), 1.0); }
    if (g.params.w > 31.5 && g.params.w < 32.5) { return vec4<f32>(to_scene(env_color(reflect(-v, n))), 1.0); }
    if (g.params.w > 4.5 && g.params.w < 5.5) { return vec4<f32>(to_scene(vec3<f32>(in.color1.a, in.color.a, 0.0)), 1.0); }
    // The original's rule (PSCityDefault, PSCityBumpSpec, PSCityGrime, PSCityLOD ...):
    //   diffuse * (gLightAmbient * occlusion + gLightColor * n.l * occlusion * sun)
    // with sun = twice the shadow collector's value: 1 in light, 1 - m_ShadowFade in shade
    // (`ndl` carries it already).
    let ao = occ;
    var light_in = (hemi + g.sun_color.rgb * g.sun_dir.w * ndl) * occ;
    if (mat.mode == 4u) {
        // (PSCityRoad: the sun's share carries the occlusion twice)
        light_in = (hemi + g.sun_color.rgb * g.sun_dir.w * ndl * occ) * occ;
    }
    // (far, 2026-10-13: the FAR city's models by day - VSCityLOD 3b5e093d268b22f5 + PSCityLOD 93307a3906a73ef9, read from
    // capture np1_12 and run per pixel on the file's meshes (work/lighting/fc_px.py, fc_sun.py):
    //   day picture x (gLightAmbient + gLightColor x sat(n.l) x 2 x collector.y)
    // - NO occlusion: the models' one colour word is the packed ground height, which the program does not fetch.
    // Its first byte, 0, stood here as the occlusion: the far city by day was black under the fog.
    // Diagnostics 640: as it stood; 641: the sun's share alone; 642: the ambient alone.)
    let fc_lod_day = g.tone.w < 0.5 && (mat.mode == 0u || mat.mode == 1u) && (u32(round(in.uv2.y)) & 2097152u) != 0u && !(g.params.w > 639.5 && g.params.w < 640.5);
    if (fc_lod_day) {
        light_in = hemi + g.sun_color.rgb * g.sun_dir.w * ndl;
        if (g.params.w > 640.5 && g.params.w < 641.5) { light_in = g.sun_color.rgb * g.sun_dir.w * ndl; }
        if (g.params.w > 641.5 && g.params.w < 642.5) { light_in = hemi; }
    }
    // The cars' map by day (moon_dir.w > 0.5; listings by work/lighting/em_prog.py sv_env_12):
    // - the road, PSCityRoadEnv: `base x occlusion x (gLightAmbient + gLightColor x n.l)` on the
    //   vertex normal - the occlusion ONCE (the port had the main view's twice: the ground of the
    //   noon map at 0.78 of the original's);
    // - the far city, PSCityLODEnv: `day picture x (gLightAmbient + gLightColor x n.l')`, no
    //   vertex colours (the far models carry none: occlusion 0 here made them black), and n.l'
    //   = n.l / sqrt(|n|^2 + u^2), u the first texture coordinate - the program normalises the
    //   light by the length of the whole interpolant (instruction 4, a dp4 over normal and u).
    // Diagnostics 210: as until 2026-10-14.
    if (g.moon_dir.w > 0.5 && g.tone.w < 0.5 && !(g.params.w > 209.5 && g.params.w < 210.5)) {
        let nv_len = max(length(in.normal), 0.0001);
        let ndl_v = max(dot(in.normal, g.sun_dir.xyz), 0.0);
        if (mat.mode == 4u) {
            light_in = (hemi + g.sun_color.rgb * g.sun_dir.w * (ndl_v / nv_len)) * occ;
        }
        if ((mat.mode == 0u || mat.mode == 1u) && (u32(round(in.uv2.y)) & 2097152u) != 0u) {
            light_in = hemi + g.sun_color.rgb * g.sun_dir.w * (ndl_v / sqrt(nv_len * nv_len + in.uv0.x * in.uv0.x));
        }
    }
    // Night (m_NFLFade > 0): the game draws the city with PSMultiLight -
    //   diffuse x occlusion x (gLightAmbient + vertex night light x 1.25 x gPropNFLColor.w + lamps)
    // darkened towards the ground by OcclusionInfo (below). ambient_ground = (gPropNFLColor.w,
    // OcclusionInfo.x, OcclusionInfo.z); the lamps come further down (`local`).
    let night_prog = g.tone.w > 0.5 && (mat.mode < 6u || mat.mode >= 20u || mat.mode == 14u);
    // (gantry, 2026-10-13: BILLBOARDS have a night program too - PSMultiLight a0f4eb680ec2fd29 of city/0x70ecad23 behind
    // the night draws' own vertex program 62c08fb0bd63e688, 15 draws in capture np2_23.5; the freeway's gantry signs are
    // billboards. On 4000 points with the game's lamp tables, 6e-15 (work/lighting/gy_bill.py):
    //   (diffuse x occ x (ambient + 1.25 |w| V + key + head lamps + the cell's FOUR lamps)
    //    + 0.5 x lerp(1, diffuse, 0.5) x (0.3125 |w| V + the four lamps' highlights at exponent 2 x 25)) x H
    //   + diffuse x m (0.3 + 0.7 m) x on            (m the neon map, on from the vertex program's clock)
    // - the walls' night form with the billboard's literals. It ran the DAY program round the clock here: no lamps,
    // no baked light - the 101's signs, each with a lamp of its own 2.6 m in front, stayed dark.
    // MCLA_DEBUG_VIEW=560: the day program at night, as before; 561: without the highlight; 562: without its own light.)
    let gy_bill_n = night_prog && mat.mode == 5u && in.uv2.x > 0.5 && (u32(round(in.uv2.y)) & 4096u) == 0u && g.dome.w > 0.5 && !(g.params.w > 559.5 && g.params.w < 560.5);
    // (terrain at night: its lights come unoccluded - see below)
    var terr_n = false;
    // (a neon sign's own light under the night program: added after the fade with height)
    var neon_em = vec3<f32>(0.0);
    // (shop signs under the night program: the night picture and its alpha, laid over after the fade - see mode 24)
    var store_n = vec4<f32>(0.0);
    if (night_prog) {
        // (+ the key light, the third of the program's "dynamic" lights: gLightColor x n.l, no shadow)
        light_in = (hemi + in.color1.rgb * 1.25 * g.ambient_ground.x + grid_lamps(in.wpos, n) + g.key_col.rgb * max(dot(n, g.key_dir.xyz), 0.0)) * occ;
        // (2026-10-15: the neon signs' night program takes the cell's first THREE lamps, diffuse and
        // highlight - work/lighting/og_neon.py; the fourth of the poster's cell is what lit the
        // poster over the start: with its own light switched off (pixel c104 = 0 on the running game)
        // the original's poster holds the ambient alone, 2.8 4.9 5.8 thousandths, and nothing
        // once the ambient is off too; the port had 10 11 13 of lamp light there. Diagnostics 333:
        // four lamps, as before.)
        let neon3 = mat.mode == 5u && in.uv2.x > 0.5 && (u32(round(in.uv2.y)) & 4096u) != 0u && !(g.params.w > 332.5 && g.params.w < 333.5);
        if (neon3) {
            light_in = (hemi + in.color1.rgb * 1.25 * g.ambient_ground.x + interior_lamps(in.wpos, n, n, 0.0, 3)[0] * g.ambient_ground.x + g.key_col.rgb * max(dot(n, g.key_dir.xyz), 0.0)) * occ;
        }
        if (g.tone.w > 1.5) {
            // The environment map's night program (PSMultiLightEnv): the ambient and the cell's
            // first lamp alone - colour x sat(intensity x gPropNFLColor.w / d^2) x n.l x cone,
            // without the main view's threefold. (It also adds a grey of a quarter of
            // gPropNFLColor.w x sat(0.75 - 0.75 x an interpolant not identified; left out - the
            // original's map has its walls at 0.05 - 0.1, which leaves no room for it on them.)
            let main_night = light_in;
            light_in = (hemi + grid_lamp_env(in.wpos, n)) * occ;
            // (2026-10-14, the program read again - work/lighting/out/em_bs_1815.txt, PSMultiLightEnv
            // of the BumpSpec / Grime walls, instructions 5 - 47: its colour interpolant (baked night
            // light, occlusion) is overwritten before any use, so NO vertex colour and NO occlusion,
            // and the "grey" IS there: `diffuse x (gLightAmbient + 0.25 x gPropNFLColor.w x sat(0.75 -
            // 0.75 n.y) + first lamp + dynamic lights)` on the vertex normal - 0.28 on a wall at
            // 23:30 (gPropNFLColor.w 1.5), 0.07 at 18:15. The room it "had not" was the occlusion
            // and the lamp. Only those walls run it: the Default, Neon, GrimeDecal, Terrain,
            // GroundGrimeDecal, StoreInterior and Billboard batches of the night map are drawn
            // with the MAIN view's PSMultiLight behind their Env vertex programs - the rule
            // above. Diagnostics 210: every wall by the old reading.)
            // The GRIME walls' night program in the map (VSCityGrimeEnv + PSMultiLightEnv 0xc006d09e..,
            // em_prog.py sv_env_23.5 VSCityGrimeEnv) is of the road's kind instead: `diffuse x
            // occlusion x (V + two lamps, doubled, unfaded + dynamic lights)` - no ambient, no
            // grey, and the diffuse map alone (no grime layer).
            if (!(g.params.w > 209.5 && g.params.w < 210.5)) {
                let nvn = in.normal / max(length(in.normal), 0.0001);
                if ((mat.mode == 0u || mat.mode == 1u) && in.uv2.x > 0.5 && in.uv2.x < 2.5) {
                    light_in = (in.color1.rgb + grid_lamps_env_road(in.wpos, nvn) + g.key_col.rgb * max(dot(nvn, g.key_dir.xyz), 0.0)) * occ;
                    base = albedo.rgb;
                } else if ((mat.mode == 0u || mat.mode == 1u) && in.uv2.x < 0.5 && hs_k != 0u) {
                    light_in = hemi + vec3<f32>(0.25 * abs(g.ambient_ground.x) * clamp(0.75 - 0.75 * nvn.y, 0.0, 1.0)) + grid_lamp_env(in.wpos, nvn) + g.key_col.rgb * max(dot(nvn, g.key_dir.xyz), 0.0);
                } else {
                    light_in = main_night;
                }
            }
            // The ROAD of the night map is another program (city/0x8667f75a, the pass's own
            // vertex program 0x7e3c740f.. and pixel program 0x2db6456f..; listing by
            // work/lighting/envnight_prog.py): `base x occlusion x (V + the cell's lamps + the
            // dynamic lights)` - V the vertex's baked night light as it is (not x 1.25 x
            // gPropNFLColor.w) and NO ambient in it. The lamp-lit asphalt of the original's map
            // is brown and 1.5 - 5 times the port's below the horizon (23:30 and 05:45 maps read
            // out of the game). Diagnostics 100: the walls' rule, as before.
            if (mat.mode == 4u && !(g.params.w > 99.5 && g.params.w < 100.5)) {
                light_in = (in.color1.rgb + grid_lamp_env(in.wpos, n) + g.key_col.rgb * max(dot(n, g.key_dir.xyz), 0.0)) * occ;
                // (2026-10-14: its lamps are two, doubled and not faded - grid_lamps_env_road. 210: as before)
                if (!(g.params.w > 209.5 && g.params.w < 210.5)) {
                    light_in = (in.color1.rgb + grid_lamps_env_road(in.wpos, n) + g.key_col.rgb * max(dot(n, g.key_dir.xyz), 0.0)) * occ;
                }
            }
            // The FAR CITY of the night map (VSCityLODEnvNight + its PSMultiLightEnv, 0xb06e1515..):
            // `day picture x (night picture + the cell's lamps + the dynamic lights)` - the night
            // picture as it is (x 1, where the main view has x 1.25 x gPropNFLColor.w), no
            // ambient, no occlusion. The port's map had the walls' rule: no lit windows at all.
            if ((mat.mode == 0u || mat.mode == 1u) && (u32(round(in.uv2.y)) & 2097152u) != 0u && !(g.params.w > 99.5 && g.params.w < 100.5)) {
                light_in = pwl_decode(hs.rgb) + grid_lamp_env(in.wpos, n) + g.key_col.rgb * max(dot(n, g.key_dir.xyz), 0.0);
                // (2026-10-15, the listing read again - em_prog.py sv_env_23.5 VSCityLODEnvNight, PS
                // instructions 9 - 15 fetch BOTH pairs of LightIndexGridSampler1, 22: intensity x 256
                // with no gPropNFLColor.w, 57: the reach doubled, 61 / 62: two lamp colours - the
                // road's kind: two lamps, doubled, unfaded. The far models' street level in the map's
                // low band read 0.010 / 0.018 against 0.019 / 0.028 at night. Diagnostics 420: one
                // lamp x gPropNFLColor.w, as before.)
                if (!(g.params.w > 419.5 && g.params.w < 420.5)) {
                    light_in = pwl_decode(hs.rgb) + grid_lamps_env_road(in.wpos, n) + g.key_col.rgb * max(dot(n, g.key_dir.xyz), 0.0);
                }
                // (diagnostics 421: no lamps at all; 422: the night picture alone)
                if (g.params.w > 420.5 && g.params.w < 421.5) { light_in = pwl_decode(hs.rgb) + g.key_col.rgb * max(dot(n, g.key_dir.xyz), 0.0); }
                if (g.params.w > 421.5 && g.params.w < 422.5) { light_in = pwl_decode(hs.rgb); }
            }
        }
        if (g.tone.w < 1.5 && (mat.mode == 0u || mat.mode == 1u) && (u32(round(in.uv2.y)) & 2097152u) != 0u) {
            // The far city (VSCityLODNight + PSMultiLight of city/0xb6de4415, probed with
            // work/lighting/wallnight.py lod): no vertex colours, no occlusion - the day picture
            // times (gLightAmbient + the night picture x 1.25 x gPropNFLColor.w + the lamps).
            // The night picture is the batch's third texture (world::mark_lod), uploaded as it
            // is; the game's fetch decodes it like the diffuse.
            // (2026-10-14, work/lighting/og_lod.py - the rule against the program on 4000 points,
            // 2.7e-15: `day x H x (ambient + night x 1.25 x |gPropNFLColor.w| + key + head lamps +
            // the cell's first THREE lamps)` - the program reads half of the second index
            // texture; both pictures' fetch constants carry the gamma flag, and the original's
            // planar reflection picture - linear values of this very program, no fog or tone
            // curve between - shows what that flag does: the console's PIECEWISE decoding of
            // BOTH pictures (work/lighting/og_lodfit.py, stormy 21:00: the level of 28,751 far
            // city pixels 1.01 / 1.00 / 1.00 of the original's with it, 0.83 / 0.79 / 0.76 with
            // sRGB; the night picture's implied curve bin by bin on the piecewise one - 0.030
            // at a stored 0.125, sRGB 0.014). The day picture is decoded where `albedo` is
            // taken. Diagnostics 84: sRGB day picture, night picture to the power 2.2, four
            // lamps - as it stood; 88: the night picture by the sRGB curve.)
            var np = pwl_decode(hs.rgb);
            var lod_lamps = interior_lamps(in.wpos, n, n, 0.0, 3)[0] * g.ambient_ground.x;
            if (g.params.w > 83.5 && g.params.w < 84.5) { np = pow(hs.rgb, vec3<f32>(2.2)); lod_lamps = grid_lamps(in.wpos, n); }
            if (g.params.w > 87.5 && g.params.w < 88.5) { np = srgb_decode(hs.rgb); }
            light_in = hemi + np * 1.25 * abs(g.ambient_ground.x) + lod_lamps + g.key_col.rgb * max(dot(n, g.key_dir.xyz), 0.0);
        }
        // Hillsides and lawns (PSMultiLight of city/0x16ea3597 CityTerrain and 0xb7b45988
        // CityGrass, work/lighting/og_terrain.py: rule against program 2.8e-8 / 2.7e-15):
        //   base x H x c x (ambient x c x occlusion + 1.25 x |gPropNFLColor.w| x V + key + head
        //   lamps + four lamps), c the fast collector's sample (1 at night)
        // - the occlusion cuts the AMBIENT ONLY: the baked light, the lamps and the head lamps
        // come unoccluded (the wall programs occlude the whole sum). CityTerrain also tilts the
        // normal by its three normal maps, fading out over the first three mip levels - not
        // bound here. Diagnostics 82: the walls' sum, as it stood.
        if (g.tone.w < 1.5 && mat.mode == 21u && !(g.params.w > 81.5 && g.params.w < 82.5)) {
            terr_n = true;
            light_in = hemi * occ + in.color1.rgb * (1.25 * abs(g.ambient_ground.x)) + grid_lamps(in.wpos, n) + g.key_col.rgb * max(dot(n, g.key_dir.xyz), 0.0);
        }
        // (diagnostics, MCLA_DEBUG_VIEW 20 / 21 / 22: the ambient, the vertex light, the lamps alone)
        if (g.params.w > 330.5 && g.params.w < 331.5) { light_in = hemi * occ; }
        if (g.params.w > 331.5 && g.params.w < 332.5) { light_in = light_in - hemi * occ; }
        if (g.params.w > 19.5 && g.params.w < 20.5) { light_in = hemi * occ; }
        if (g.params.w > 20.5 && g.params.w < 21.5) { light_in = in.color1.rgb * 1.25 * g.ambient_ground.x * occ; }
        if (g.params.w > 21.5 && g.params.w < 22.5) { light_in = grid_lamps(in.wpos, n) * occ; }
        if (g.params.w > 22.5 && g.params.w < 23.5) { light_in = g.key_col.rgb * max(dot(n, g.key_dir.xyz), 0.0) * occ * 4.0; }
        // (diagnostics 200 - 202 / 203 - 205 / 206 - 208: the binary digits of x - 700 / z + 1000 / y, metres
        // 128 .. 0.5 - to place a pixel in the world; run with --no-post)
        if (g.params.w > 199.5 && g.params.w < 208.5) {
            let dk = u32(g.params.w - 200.0 + 0.5);
            let dv = select(select(in.wpos.y, in.wpos.z + 1000.0, dk / 3u == 1u), in.wpos.x - 700.0, dk / 3u == 0u);
            let dq = u32(max(dv, 0.0) * 2.0);
            let sh = 6u - (dk % 3u) * 3u;
            light_in = vec3<f32>(f32((dq >> (sh + 2u)) & 1u), f32((dq >> (sh + 1u)) & 1u), f32((dq >> sh) & 1u)) * 3.0;
        }
    }
    // Interiors - tunnels, car parks, garages: the sector batches flagged 4, marked by
    // world::mark_grime in bit 20 of uv2.y - are drawn with PSMultiLightInterior at every hour:
    // the night program with gPropNFLColor.w = 1 (recorded at noon and at 23:30, MAP_WORK.md C3).
    // No sun reaches them. (Its shadow collector term is not read yet.)
    // (Ground batches of interiors - tunnel roads, car park floors - cannot carry the mark in
    // their vertices: their sector's interior object uniform has it, dents[5] = (.., 1, .., -7).)
    let interior_obj = obj.dents[5].w < -6.5 && obj.dents[5].y > 0.5 && (mat.mode < 6u || mat.mode >= 20u);
    // The interior programs run on numbers (2026-10-13, work/lighting/tun_rules.py: each rule
    // against its program, exact). With V = the second colour's rgb, occ its alpha, lamps = the
    // cell's lamps x 3 (gPropNFLColor.w = 1), z = ShaderGlobals.z (8 + f by day, 4 at night):
    //   CityDefault / BumpSpec / Grime walls, CityRoad floors:
    //     base x occ x (1.25 V + gLightColor x n.l x k + head lamps x k' + 4 lamps [+ ambient, z < 4 only])
    //       by day k = k' = (2 collector - f) / (1 - f); at night k = 0 (the key light is off), k' = 1
    //     + spec colour x (0.3125 V + the lamps' highlights)   - NOT occluded; walls with a
    //       HeightSpecular map: tint x intensity x (2 g - 1) x lerp(1, base, mixer); roads:
    //       lerp(0.4, 0.33, decal alpha squared) x the diffuse map's green x lerp(1, base, 0.7),
    //       and no lamp highlights
    //     all of it x the height term 1 + sat((y - OcclusionInfo.x) x 0.05) x (OcclusionInfo.z - 1)
    //   CityTunnelRoad: the same as CityRoad with k = 0 at EVERY hour (no sun by day) and the
    //     head lamps on while z < 8 (by day their colours are zero anyway)
    //   CityTunnelTile / CityGrimeMetal: no occlusion, no height term, no shadow term, THREE lamps:
    //     base x (1.25 V + the frame's light x n.l + head lamps + 3 lamps)
    //     + (2 g - 1) x (0.3125 V + the 3 lamps' highlights, exponent 2 x SpecularExponent)
    //     + ReflectionIntensity x (1 - grime) x 0.2 x (1 + (1 - n.v)^4) x the interior picture
    //       (ReflectSphereInterior.dds, the batch's fourth texture) at (0.5 - 0.9 R.x, 0.5 - 0.47 R.y)
    // (ShaderGlobals.z >= 23, "flag 16": V itself x 2 x collector - in no recording; not done.)
    // Diagnostics: 75 the interiors as they stood before this; 76 tunnel roads as the other
    // interior floors; 77 the tiles' older mirror term (the surroundings' map); 78 without the
    // unoccluded specular terms.
    // (world::mark_tunnel_road: TUNNEL_ROAD_MARK = 512 added to the grime coordinate's y)
    let tunnel_road = interior_obj && mat.mode == 4u && in.uv2.y > 256.0 && !(g.params.w > 75.5 && g.params.w < 76.5);
    let int_new = !(g.params.w > 74.5 && g.params.w < 75.5) && !(g.params.w > 69.5 && g.params.w < 70.5);
    var int_tile = false;
    var int_wall = false;
    var int_spec = vec3<f32>(0.0);
    if (((mat.mode == 0u || mat.mode == 1u) && (u32(round(in.uv2.y)) & 1048576u) != 0u) || interior_obj) {
        // (pass 7, work/map/pass7_notes.md: the program takes NO gLightAmbient while
        // ShaderGlobals.z >= 4 - 8 + (1 - m_ShadowFade) by day, 4 at night in the recordings -
        // and by day it has the sun: gLightColor x n.l x (2 x collector - f) / (1 - f), which is 0
        // in shade. The tunnel tile / grime metal effects' own interior program does not multiply
        // by the occlusion at all and takes the light without a shadow term - their vertices
        // carry an occlusion of 0.0 - 0.2, which made the 2nd Street tunnel black here.)
        let tile_in = (mat.mode == 0u || mat.mode == 1u) && in.uv2.x > 2.5;
        var dir_in = g.sun_color.rgb * g.sun_dir.w * max(dot(n, g.sun_dir.xyz), 0.0);
        if (g.tone.w > 0.5) {
            // (night: the grey from below is switched off in the CityDefault form, on in the tiles')
            dir_in = select(vec3<f32>(0.0), g.key_col.rgb * max(dot(n, g.key_dir.xyz), 0.0), tile_in);
        } else if (!tile_in) {
            dir_in = dir_in * sun_visibility(in.wpos, ng);
        }
        // (the pass 7 line; kept for diagnostics 75 - the newer forms follow)
        if (!int_new) { light_in = (in.color1.rgb * 1.25 + grid_lamps_by(in.wpos, n, 1.0) + dir_in) * select(occ, 1.0, tile_in); }
        // (diagnostics 70: the line as it stood before pass 7)
        if (g.params.w > 69.5 && g.params.w < 70.5) { light_in = (hemi + in.color1.rgb * 1.25 + grid_lamps_by(in.wpos, n, 1.0)) * occ; }
        if (int_new) {
            int_wall = mat.mode == 0u || mat.mode == 1u;
            int_tile = tile_in;
            let v1 = in.color1.rgb;
            let no_spec = g.params.w > 77.5 && g.params.w < 78.5;
            if (tile_in) {
                let il = interior_lamps(in.wpos, n, reflect(-v, n), select(0.0, 2.0 * max(wall_spec.x, 1.0), hs_k != 0u), 3);
                light_in = v1 * 1.25 + il[0] + dir_in;
                if (hs_k != 0u && !no_spec) { int_spec = (il[1] + v1 * 0.3125) * wall_spec.w; }
                if (!(g.params.w > 76.5 && g.params.w < 77.5)) {
                    let rv = reflect(-v, n);
                    let pic = xgm(textureSampleLevel(t_fourth, s_diffuse, vec2<f32>(0.5 - 0.9 * rv.x, 0.5 - 0.47 * rv.y), 0.0).rgb);
                    // (diagnostics 79: the picture alone, as sampled)
                    if (g.params.w > 78.5 && g.params.w < 79.5) { return vec4<f32>(to_scene(pic), 1.0); }
                    let fr4 = 1.0 - clamp(dot(n, v), 0.0, 1.0);
                    int_spec = int_spec + pic * ((in.uv2.x - 3.0) * (1.0 - second.a * in.color.a) * 0.2 * (1.0 + fr4 * fr4 * fr4 * fr4));
                }
            } else {
                let wall_hs = int_wall && hs_k != 0u;
                let il = interior_lamps(in.wpos, n, reflect(-v, n), select(0.0, 2.0 * max(wall_spec.x, 1.0), wall_hs), 4);
                light_in = (v1 * 1.25 + il[0] + select(dir_in, vec3<f32>(0.0), tunnel_road)) * occ;
                if (wall_hs && !no_spec) {
                    int_spec = mix(vec3<f32>(1.0), base, wall_spec.z) * (wall_spec.y * wall_spec.w) * (il[1] + v1 * 0.3125);
                } else if (mat.mode == 4u && !no_spec) {
                    int_spec = v1 * 0.3125 * (mix(0.4, 0.33, second.a * second.a) * albedo.g) * mix(vec3<f32>(1.0), base, 0.7);
                }
            }
        }
    }
    if (mat.mode == 15u && g.tone.w < 0.5) {
        // Leaves (VSPropFoliage, VSPropFoliageGenImpostorDay): 0.75 x vertex occlusion x
        // (gLightAmbient + gLightColor x (0.1 + 0.9 n.l)). Plants at a distance are impostors
        // rendered with that into an 8-bit target and the whole of it is then multiplied by the
        // collector's brightness (PSPropFoliage) - black in shade; the modelled leaves near the
        // camera keep their ambient share.
        let nl0 = 0.1 + 0.9 * max(dot(n, g.sun_dir.xyz), 0.0);
        if (in.lpos.z < -9000.0) {
            light_in = min(0.75 * occ * (hemi + g.sun_color.rgb * g.sun_dir.w * nl0), vec3<f32>(1.0) / max(base, vec3<f32>(0.004))) * leaf;
            // (2026-10-14: that is the CARD's rule - the picture lit, then the whole of it times
            // the collector. A plant's model is drawn by VSPropFoliage + PSPropFoliage
            // 6ebf788b1c5456fc: 0.75 c x (gLightAmbient + gLightColor x (0.1 + 0.9 sat(N.L)) x
            // collector) - the ambient is NOT shaded by the collector, c is the occlusion byte
            // as it is, N the world normal as the matrix gives it, not normalised (the plant's
            // scale leaks into N.L). plantday_verify.py: 2e-8. 112: as before)
            if (!imp_card && !(g.params.w > 111.5 && g.params.w < 112.5)) {
                let n_raw_d = n * select(1.0, max(-1.0 - in.neon_hl, 0.0), in.neon_hl < -1.0001);
                light_in = 0.75 * in.color1.a * (hemi + g.sun_color.rgb * g.sun_dir.w * (0.1 + 0.9 * clamp(dot(n_raw_d, g.sun_dir.xyz), 0.0, 1.0)) * leaf);
                // (diagnostics 414 / 415: a plant model's leaves without the sun's term / without the ambient - the running
                //  original's counterpart: vertex constants 39 / 26 zeroed on PSPropFoliage 6ebf788b1c5456fc, work/lighting/sn_ovr.py)
                if (g.params.w > 413.5 && g.params.w < 414.5) { light_in = 0.75 * in.color1.a * hemi; }
                // (416 / 417: the sun's term alone with a light of 0.3 grey / the ambient alone at 0.3 grey - the original with constants 39 / 26 set so)
                if (g.params.w > 415.5 && g.params.w < 416.5) { light_in = vec3<f32>(0.75 * in.color1.a * 0.3 * (0.1 + 0.9 * clamp(dot(n_raw_d, g.sun_dir.xyz), 0.0, 1.0)) * leaf); }
                if (g.params.w > 416.5 && g.params.w < 417.5) { light_in = vec3<f32>(0.75 * in.color1.a * 0.3); }
                if (g.params.w > 414.5 && g.params.w < 415.5) { light_in = 0.75 * in.color1.a * g.sun_color.rgb * g.sun_dir.w * (0.1 + 0.9 * clamp(dot(n_raw_d, g.sun_dir.xyz), 0.0, 1.0)) * leaf; }
                // (diagnostics 403 - 409, a plant model's leaf terms as greys (the tone map works on the luminance: one term a view):
                //  403 the occlusion byte / 4, 404 (0.1 + 0.9 sat(N.L)) / 4, 405 grey 0.125, 406 grey 0.25, 407 the collector value C / 4,
                //  408 its ground layer / 4, 409 its 8 m layer / 4 - read with work/lighting/pv_diag.py)
                if (g.params.w > 402.5 && g.params.w < 413.5) {
                    let dsp = vec3<f32>(in.wpos.x - 2.0 * g.sun_dir.x, in.lpos.y, in.wpos.z - 2.0 * g.sun_dir.z);
                    var dg = in.color1.a;
                    if (g.params.w > 403.5) { dg = 0.1 + 0.9 * clamp(dot(n_raw_d, g.sun_dir.xyz), 0.0, 1.0); }
                    if (g.params.w > 404.5) { dg = 0.5; }
                    if (g.params.w > 405.5) { dg = 1.0; }
                    if (g.params.w > 406.5) { dg = leaf; }
                    if (g.params.w > 407.5) { dg = soft_collector(dsp); }
                    if (g.params.w > 408.5) { dg = soft_collector_high(dsp); }
                    if (g.params.w > 409.5) { dg = soft_collector(vec3<f32>(825.2, 0.0, -850.6)); }
                    if (g.params.w > 410.5) { dg = soft_collector_high(vec3<f32>(825.2, 0.0, -850.6)); }
                    if (g.params.w > 411.5) { dg = soft_collector_high(vec3<f32>(827.5, 0.0, -854.5)); }
                    if (g.params.w > 412.5) { dg = soft_collector(vec3<f32>(819.6, 0.0, -808.4)); }
                    return vec4<f32>(to_scene(vec3<f32>(0.25 * dg)), 1.0);
                }
            }
        } else {
            light_in = 0.75 * occ * (hemi + g.sun_color.rgb * g.sun_dir.w * nl0 * leaf);
        }
    }
    if (trunk_d) {
        light_in = in.color1.a * (hemi + g.sun_color.rgb * g.sun_dir.w * clamp(dot(n, g.sun_dir.xyz), 0.0, 1.0) * trunk_c);
    }
    if (imp_card && g.tone.w < 0.5) {
        // Pass 8, PSPropFoliage of the impostor effect: the picture (the model lit by the leaf
        // rule above, drawn by props/impostor.rs) times the collector's sun, ground / 8 m up by
        // the height above the card's foot.
        light_in = vec3<f32>(leaf);
    }
    if (mat.mode == 9u && in.uv1.y > 99.5 && g.tone.w > 0.5) {
        // Props at night (instanced models: uv1.y = 100 + kind). The original lights them with
        // lamps stored with each instance, like the plants; here the grid's lamps, the ambient
        // and the key light. (Their own PSMultiLight is not read.)
        // (pass 7: + the night fill light at the prop's place, gPropNFLColor.rgb x 1.25 x |w| with
        // rgb = trunc(map / 4) / 255 x w - the instance's colour; zero where there is none)
        light_in = (hemi + in.fill.rgb * (1.25 * g.ambient_ground.x * g.ambient_ground.x) + grid_lamps(in.wpos, n) + g.key_col.rgb * max(dot(n, g.key_dir.xyz), 0.0)) * occ;
        if (prop_n) {
            light_in = hemi + in.fill.rgb * ((1.0 - 0.8 * clamp(in.lpos.x * 0.0667, 0.0, 1.0)) * 1.25 * g.ambient_ground.x * g.ambient_ground.x) + grid_lamps(in.wpos, n) + g.key_col.rgb * max(dot(n, g.key_dir.xyz), 0.0);
        }
    }
    // Plants under the night programs: what is lit how (the three programs of the original)
    // (93, and foliage that is not an instance - it has no record: the lamp grid as before)
    let plant_old = (g.params.w > 92.5 && g.params.w < 93.5) || in.lpos.z >= -9000.0;
    let plant_card_n = mat.mode == 15u && g.tone.w > 0.5 && in.uv1.y > 102.5 && in.uv1.y < 104.5;
    let plant_leaf_n = mat.mode == 15u && g.tone.w > 0.5 && !plant_card_n && in.lpos.z < -9000.0;
    let plant_trunk_n = mat.mode == 9u && g.tone.w > 0.5 && in.uv1.y > 100.5 && in.uv1.y < 102.5 && in.lpos.z < -9000.0 && !plant_old;
    if (mat.mode == 15u && g.tone.w > 0.5) {
        // Plants at night. The original lights a plant with gLightAmbient, the night fill light
        // of its place (pass 7), ONE lamp of its own - baked into its instance record, not the
        // lamp grid - and the dynamic lights (the two head lamps: the local lights below; the
        // third, MoonLightColor 0.051 from below-west: the key light here).
        //   leaf models (vertex program 9fa5cebc56a89902, pixel = diffuse x this):
        //     0.75 sat(0.3 + c) x (ambient + fill + lamp) + dynamic lights
        //     c = the vertex's occlusion byte; N the world normal NOT normalised (in.neon_hl)
        //   cards (VSPropInstanceFoliageNight + PSMultiLight c0175330ac1577e4), per pixel:
        //     ambient + fill + lamp + head lamps + sat(n . c94) x c100, n = the picture's normal
        //   fill = colour x 0.5 x (1 - 0.5 sat(h / 10)) x (0.3 + 0.7 sat(0.75 + N.y (1 - 1.75 sat(h / 8)))) x s
        //     colour = the record's bytes x s on models (c74), as they are on cards; s = gPropNFLColor.w;
        //     h = the height above the plant's foot
        // (93: as before - the lamp grid's lamps, no occlusion)
        var plant_fill = vec3<f32>(0.2, 0.18235, 0.14706) * (0.2 * g.ambient_ground.x);
        // (pass 8: an impostor card's normal is its picture's - the second texture, world space)
        if (imp_card && dot(second.rgb, second.rgb) > 0.01) { n = normalize(second.rgb * 2.0 - vec3<f32>(1.0)); }
        var n_raw = n;
        if (plant_leaf_n && !plant_old) { n_raw = n * max(-1.0 - in.neon_hl, 0.0); }
        if (in.fill.a > 0.5) {
            let hgt = select(0.0, max(in.lpos.x, 0.0), in.lpos.z < -9000.0);
            let s_nfl = g.ambient_ground.x;
            plant_fill = in.fill.rgb * (0.5 * (1.0 - 0.5 * clamp(hgt * 0.1, 0.0, 1.0)) * (0.3 + 0.7 * clamp(0.75 + n_raw.y * (1.0 - 1.75 * clamp(hgt * 0.125, 0.0, 1.0)), 0.0, 1.0)) * s_nfl * select(s_nfl, 1.0, plant_card_n));
        }
        let moon = g.key_col.rgb * clamp(dot(n_raw, g.key_dir.xyz), 0.0, 1.0);
        if (plant_old) {
            light_in = hemi + plant_fill + grid_lamps(in.wpos, n) + moon;
        } else if (plant_leaf_n) {
            light_in = (0.75 * clamp(0.3 + in.color1.a, 0.0, 1.0)) * (hemi + plant_fill + plant_lamp(in.wpos, n_raw, in.lamp_pos, in.lamp_col, in.lamp_dir)) + moon;
        } else {
            light_in = hemi + plant_fill + plant_lamp(in.wpos, n, in.lamp_pos, in.lamp_col, in.lamp_dir) + moon;
        }
    }
    if (plant_trunk_n) {
        // Palm trunks and the plants' other solid parts at night (PropFoliageTrunk; vertex program
        // 92a760d3ba27649f, pixel = diffuse x this): c x (ambient + fill + lamp) - the fill by the
        // leaves' rule with the normalised normal, c the vertex's occlusion byte as it is; no
        // dynamic lights (no head lamps, no moon), no lamp grid.
        let hgt_t = max(in.lpos.x, 0.0);
        let s_t = g.ambient_ground.x;
        let fill_t = in.fill.rgb * (0.5 * (1.0 - 0.5 * clamp(hgt_t * 0.1, 0.0, 1.0)) * (0.3 + 0.7 * clamp(0.75 + n.y * (1.0 - 1.75 * clamp(hgt_t * 0.125, 0.0, 1.0)), 0.0, 1.0)) * s_t * s_t);
        light_in = in.color1.a * (hemi + fill_t + plant_lamp(in.wpos, n, in.lamp_pos, in.lamp_col, in.lamp_dir));
    }
    // OUTDOOR CityGrimeMetal / CityTunnelTile (city/0xa39f9566, 0x9084708c; work/lighting/og_tile.py,
    // each rule against its program on 4000 points: night 5.9e-9, day 2.3e-8 / 6.3e-8).
    //   night, PSMultiLight (the same code in both effects) - the interior form with the ambient:
    //     base x (1.25 |w| V + ambient x occlusion + key + head lamps + the cell's first THREE lamps)
    //     + (2 g - 1) x (the three lamps' highlights, exponent 2 x SpecularExponent, + 0.25 x (1.25 |w| V + ambient x occlusion))
    //     + ReflectionIntensity x (1 - grime) x 0.2 x (1 + (1 - n.v)^4) x ReflectSphereNight.dds at (0.5 - 0.9 R.x, 0.5 - 0.47 R.y)
    //     - no occlusion but on the ambient, no height term; w = gPropNFLColor.w
    //   day, PSCityGrimeMetal: base x occlusion x (ambient + light x n.l x s) + the sun's highlight
    //     + 0.5 x (1 - n.v)^4 x (1 - grime) x lerp(mini sky x ReflectionIntensity x s x occlusion,
    //       ReflectSphereDay.rgb, its alpha)          s = twice the shadow collector's sample
    //     the day picture at (0.5 - 0.9 R.x, 0.5 - 0.47 R.y), the mini sky (the 128 x 128 map of
    //     the windows) at 0.5 + 0.47 R'.xz, R' mirrored with the normal's y raised by height / 150.
    //     PSCityTunnelTile: the same with s = ShaderGlobals.z (the shade's level) - not told apart here.
    // The batch's fourth texture is the day picture (world.rs `extra_textures`); there is no
    // slot for the night one, so the night mirror term is left out (about RI x 0.2 .. 0.4 of
    // that picture). The mini sky is not made here: the surroundings' map stands in, looked up
    // where the dome's flat layout would put that coordinate (as refl.wgsl's window panes do).
    // The day highlight stays the walls' (the metal's own is a half-vector lobe rising to
    // SpecularIntensityMax at a glancing angle; its two constants are not carried).
    // Diagnostics 81: all of it as it stood (the walls' sums, the surroundings' map mirrored).
    let tile_out = (mat.mode == 0u || mat.mode == 1u) && in.uv2.x > 2.5 && !int_tile && (u32(round(in.uv2.y)) & 1048576u) == 0u
        && g.tone.w < 1.5 && g.dome.w > 0.5 && g.ambient_ground.w > -0.5 && !(g.params.w > 80.5 && g.params.w < 81.5);
    // (diagnostics 89: the walls' marks - magenta: this outdoor metal form applies; red: a
    // wall marked as metal / tile (uv2.x = 3 + ReflectionIntensity) that it does not apply to -
    // an interior; dark green: a grime wall; dark grey: the other walls)
    if (g.params.w > 88.5 && g.params.w < 89.5 && (mat.mode == 0u || mat.mode == 1u)) {
        var mk = select(vec3<f32>(0.1), vec3<f32>(0.0, 0.3, 0.0), in.uv2.x > 0.5);
        if (in.uv2.x > 2.5) { mk = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(1.0, 0.0, 1.0), tile_out); }
        return vec4<f32>(to_scene(mk), 1.0);
    }
    var tile_n = false;
    var tile_spec = vec3<f32>(0.0);
    if (tile_out && night_prog) {
        tile_n = true;
        let soft_t = in.color1.rgb * (1.25 * abs(g.ambient_ground.x)) + hemi * occ;
        let il_t = interior_lamps(in.wpos, n, reflect(-v, n), select(0.0, 2.0 * max(wall_spec.x, 1.0), hs_k != 0u), 3);
        light_in = soft_t + il_t[0] * g.ambient_ground.x + g.key_col.rgb * max(dot(n, g.key_dir.xyz), 0.0);
        if (hs_k != 0u) { tile_spec = (il_t[1] * g.ambient_ground.x + 0.25 * soft_t) * wall_spec.w; }
    }
    var lit = base * light_in;
    if (tile_out && !night_prog) {
        let nv_t = dot(n, v);
        let fr_t = 1.0 - clamp(nv_t, 0.0, 1.0);
        let rv_t = reflect(-v, n);
        let day_ts = textureSampleLevel(t_fourth, s_diffuse, vec2<f32>(0.5 - 0.9 * rv_t.x, 0.5 - 0.47 * rv_t.y), 0.0);
        let day_t = vec4<f32>(xgp(day_ts.rgb), day_ts.a);
        let r2_t = 2.0 * (nv_t + v.y * in.wpos.y * 0.00666667) * n.xz - v.xz;
        let rl_t = max(length(r2_t), 0.0001);
        let rho_t = clamp(0.94 * rl_t / 0.888, 0.0, 1.0);
        // (mini sky round: the map itself - sampler 0 of PSCityGrimeMetal is the windows' SkyReflectionSampler;
        // MCLA_DEBUG_VIEW=663 or MCLA_MINISKY_OLD=1: the surroundings' map at the dome's flat layout as before)
        var sky_t = mini_sky(r2_t);
        if ((g.params.w > 662.5 && g.params.w < 663.5) || textureLoad(t_minisky, vec2<i32>(0, 0), 0).a < 0.5) {
            sky_t = env_color(normalize(vec3<f32>(200.0 * rho_t * r2_t.x / rl_t, 163.67 * sqrt(max(1.0 - rho_t * rho_t, 0.0)), 200.0 * rho_t * r2_t.y / rl_t)));
        }
        let m_t = mix(sky_t * ((in.uv2.x - 3.0) * shadow * occ), day_t.rgb, day_t.a);
        tile_spec = m_t * (0.5 * fr_t * fr_t * fr_t * fr_t * (1.0 - second.a * in.color.a));
    }
    // (interior tiles and metal: the program's own mirror term is in `int_spec`; 77: this older one)
    if (!tile_out && (mat.mode == 0u || mat.mode == 1u) && in.uv2.x > 2.5 && (!int_tile || (g.params.w > 76.5 && g.params.w < 77.5))) {
        // PSCityGrimeMetal / PSCityTunnelTile: the surroundings mirrored, ReflectionIntensity
        // strong, by (1 - n.v)^4 - only at a glancing angle - and not where grime covers the metal
        let fr = pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 4.0);
        lit = lit + env_color(reflect(-v, n)) * (in.uv2.x - 3.0) * 2.0 * fr * (1.0 - second.a * in.color.a) * mix(0.5, 1.0, shadow);
    }
    // (interior walls: the interior programs have no sun highlight, and their lamp highlights are in `int_spec`)
    if (hs_k != 0u && !int_wall && !prop_n) {
        // (the mask, 2 g - 1 of the map, is NOT cut at zero: PSCityBumpSpec run in numpy with
        // a noon capture's constants, work/lighting/bumpspec_day.py, gives for green 1 / 0.5 /
        // 0 a highlight of +0.68 / 0 / -0.68 - a map darker than a half takes the sun's
        // highlight off the wall. The ground's programs are taken to do the same.)
        // PSCityBumpSpec / PSCityGrime: (v . (2 (n.l) n - l))^SpecularExponent x SpecularIntensity,
        // coloured SpecularColorMixer of the way from the light's colour to the texture's, masked
        // by the map's green, faded in over the first quarter of n.l, and cut by the cube of
        // occlusion x sun (shade and corners lose it fast)
        let nl0 = max(dot(n, g.sun_dir.xyz), 0.0);
        let rl = n * (2.0 * nl0) - g.sun_dir.xyz;
        // (plant trunks: the cube of occlusion byte x collector value, see `trunk_d`)
        let cut = select(occ * shadow, trunk_cut, trunk_cut >= 0.0);
        lit = max(lit + mix(g.sun_color.rgb, base, wall_spec.z) * pow(clamp(dot(v, rl), 0.0, 1.0), max(wall_spec.x, 1.0)) * wall_spec.y * wall_spec.w * clamp(4.0 * nl0, 0.0, 1.0) * g.sun_dir.w * cut * cut * cut, vec3<f32>(0.0));
    }
    // (the interior programs' unoccluded terms: the baked light's and the lamps' highlights, the tiles' mirror)
    lit = lit + int_spec + tile_spec;
    if (night_prog && hs_k != 0u && !int_wall && !tile_n) {
        // (SpecularTint x SpecularIntensity x the map's mask x lerp(1, diffuse, SpecularColorMixer).
        // The mask is 2 g - 1 as it stands, not cut at zero: where the map's green is under a
        // half the lamps' highlight is taken OFF the wall - PSMultiLight of city/0xcf5e188e run
        // against this rule, work/lighting/bumpspec_night.py: the same to four decimals with
        // the signed mask, 5 - 10 % of the lamp light off with the cut one.)
        // The baked light's own highlight (2026-10-14): beside the lamps' the program adds
        // 0.3125 x |gPropNFLColor.w| x V - V the second colour's rgb - under the same specular
        // colour, NOT occluded: PSMultiLight of BumpSpec 0xcf5e188e, Grime 0x1bc58d09, GrimeDecal
        // 0xbabd0a08 and the ground's 0xa49b99a4 / 0xeaed87d3 against
        //   (base x occ x (1.25 |w| V + key + head lamps + 4 lamps + ambient)
        //    + tint x intensity x (2 g - 1) x lerp(1, base, mixer) x (0.3125 |w| V + the lamps' highlights)) x H
        // on 4000 points each: 3.4e-15 at most (work/lighting/og_rules.py, og_road.py; on the
        // ground the occluded part is also x the fast collector's sample, 1 at night). Not in
        // the environment map's program. Diagnostics 80: without it, as it stood.
        var baked_hl = in.color1.rgb * (0.3125 * abs(g.ambient_ground.x));
        if (g.tone.w > 1.5 || (g.params.w > 79.5 && g.params.w < 80.5)) { baked_hl = vec3<f32>(0.0); }
        lit = max(lit + mix(vec3<f32>(1.0), base, wall_spec.z) * (wall_spec.y * (hs.g * 2.0 - 1.0)) * (grid_lamps_spec(in.wpos, reflect(-v, n), 2.0 * max(wall_spec.x, 1.0)) + baked_hl), vec3<f32>(0.0));
    }
    if (prop_nb && hs_k != 0u) {
        // (bump-spec props at night: the lamps' highlights and the fill light's, see `prop_n`)
        let fill_hl = in.fill.rgb * ((1.0 - 0.8 * clamp(in.lpos.x * 0.0667, 0.0, 1.0)) * 0.3125 * g.ambient_ground.x * g.ambient_ground.x);
        lit = max(lit + mix(vec3<f32>(1.0), base, wall_spec.z) * (wall_spec.y * wall_spec.w) * (select(vec3<f32>(0.0), grid_lamps_spec(in.wpos, reflect(-v, n), 2.0 * max(wall_spec.x, 1.0)), prop_nb_full) + fill_hl), vec3<f32>(0.0));
    }
    if (spec > 0.0) {
        let f = fresnel(n, v, 0.04);
        let s = pow(ndh, gloss) * spec * g.sun_dir.w * shadow;
        let r = reflect(-v, n);
        let env = env_color(r);
        // (car parts read from their programs, work/lighting bmprobe.py / bs3.py / cglassprobe.py;
        // vehicles.rs marks their vertices in uv2. MCLA_DEBUG_VIEW 72: all of them as before;
        // 73: without the cars' own-lamp light)
        // (obj.params.w >= 16: a car's body - props and figures share the modes)
        let cp_on = g.dome.w > 0.5 && g.ambient_ground.w > -0.5 && obj.params.w > 15.5 && !(g.params.w > 71.5 && g.params.w < 72.5);
        if (mat.mode == 7u && in.uv2.x > 0.5 && cp_on) {
            // PS_BrakeRotor of brushedmetal.fxc (cars/0xb140ff44: brake discs, suspension arms,
            // door and trim metal, some exhaust tips), exact against the program:
            //   out = tint x (0.125 x ambient x k^2 + SOFT surroundings x ReflectColorMod x k)
            // k = 0.25 + 0.75 x baked occlusion; tint = BrushedMetalTintDownload, 0.75 in every
            // recorded draw; ReflectColorMod the material's (uv2.x - 1). No light, no collector,
            // no own-lamp term in the pixel program - and the blurred map, not the sharp one.
            let k7 = 0.25 + 0.75 * in.color1.a;
            lit = 0.75 * (0.125 * hemi * k7 * k7 + env_soft(r) * ((in.uv2.x - 1.0) * k7));
            spec_k = 0.0;
        } else if (mat.mode == 13u && in.uv2.x > 1.5 && cp_on) {
            // VS_CarbonFiber + PS_CarbonFiber of cars/0x84fc57d9 (the R8's side blades), read from the
            // listing (LIGHTING_WORK.md, "Carbon fibre"):
            //   base = c64 x c65 = 0.03;  dif = base x diffuseModC (0.1025) x screen of the car's two
            //   lights, each colour x 0.9 x sat(4 n.l) x n.l  - NO ambient term, no collector
            //   m    = lerp(dif, surroundings (sharp), f),  f = 0.005 + 0.1075 (1 - n.v)^5
            //   spec = base x 1.5 x screen of the two lights' colour x 0.9 x sat(4 n.l) x
            //          exp(-2 ((h.b / ax)^2 + (h.t / ay)^2) / (1 + h.n)) / (4 pi ax ay)
            //   out  = screen(m, spec) x (0.5 + 0.5 occ)
            // ax / ay = 0.0775 .. 0.7925 by the weave map's alpha and its complement; the weave map
            // (256 x 256 normal map) is not bound here: its middle, ax = ay = 0.435, no bump.
            // (2026-10-13, blade round - what the reading above had missed, found by running the capture's own
            // VS_CarbonFiber + PS_CarbonFiber on the blade's real vertices, work/lighting/bl_px.py / bl_refl.py:
            //  1. VS_CarbonFiber turns the normal by the car's matrix TWICE: lines 43 - 45 take it to the world
            //     (rows c64.. = the body's matrix; the world rows c0..c2 are the identity), 54 - 61 normalise, and
            //     62 - 64 multiply by the SAME rows again. The pixel program's N, the mirror ray and the fresnel
            //     weight all come from M (M n) - to 1e-6 on all 503 vertices; the tangent is M t, once. So the
            //     blade is lit and mirrors as if it were turned about the car's up axis by the car's own heading:
            //     at the gate its normals look away from the viewer (N.V < 0: fresnel at its maximum, no sun) and
            //     the "light in the lower half" is the street mirrored at full weight. VS_CarPaint / VS_BumpSpec
            //     turn theirs once.
            //  2. the weave map (`__cfnormalmap` = cf_bump2.dds of the global car textures, vehicles.rs binds it
            //     as these meshes' second texture, read as stored): n' = T x + cross(T, N) w y + N z of 2 c - 1,
            //     the highlight's frame B' = cross(T, n'), T' = cross(cross(T, N) w, n'), each normalised; its
            //     alpha (0 or 1 by strand) gives ax = lerp(0.0775, 0.7925, a), ay by 1 - a. The diffuse term and
            //     the mirror take N, not n'.
            // T is the direction u grows in (stored T . dP/du = 0.98 over the blade's triangles, bl_tan.py), w the
            // stored sign (`bflip`). MCLA_DEBUG_VIEW=520: as it stood (the true normal, no map); 521: the
            // twice-turned normal without the map; 522: that normal as a colour; 523 / 524: the map's rgb / alpha; 525: green where the map is bound, red where it is missing.)
            let cf_old = g.params.w > 519.5 && g.params.w < 520.5;
            var n_cf = normalize(mat3x3<f32>(normalize(obj.model[0].xyz), normalize(obj.model[1].xyz), normalize(obj.model[2].xyz)) * n);
            if (cf_old) { n_cf = n; }
            var wv_cf = textureSample(t_second, s_diffuse, in.uv0);
            if (cf_old || (g.params.w > 520.5 && g.params.w < 521.5) || textureDimensions(t_second).x < 8u) { wv_cf = vec4<f32>(0.5, 0.5, 1.0, 0.5); }
            if (g.params.w > 521.5 && g.params.w < 522.5) { return vec4<f32>(to_scene(n_cf * 0.5 + 0.5), 1.0); }
            if (g.params.w > 522.5 && g.params.w < 523.5) { return vec4<f32>(to_scene(wv_cf.rgb), 1.0); }
            if (g.params.w > 523.5 && g.params.w < 524.5) { return vec4<f32>(to_scene(vec3<f32>(wv_cf.a)), 1.0); }
            if (g.params.w > 524.5 && g.params.w < 525.5) { return vec4<f32>(to_scene(select(vec3<f32>(0.0, 0.8, 0.0), vec3<f32>(0.8, 0.0, 0.0), textureDimensions(t_second).x < 8u)), 1.0); }
            let cf_t0 = cross(tb_dp2, ng) * tb_duv1.x + cross(ng, tb_dp1) * tb_duv2.x;
            var cf_t = cf_t0 * (inverseSqrt(max(dot(cf_t0, cf_t0), 1e-24)) * select(-1.0, 1.0, dot(cross(tb_dp1, tb_dp2), ng) >= 0.0));
            if (dot(cf_t0, cf_t0) < 1e-24) { cf_t = cf_unit(cross(vec3<f32>(0.0, 1.0, 0.0), n_cf) + vec3<f32>(1e-6, 0.0, 0.0)); }
            let cf_b = cross(cf_t, n_cf) * select(1.0 - 2.0 * in.bflip, 1.0, in.bflip > 0.25 && in.bflip < 0.75);
            let cf_mv = wv_cf.xyz * 2.0 - 1.0;
            let cf_n = cf_unit(cf_t * cf_mv.x + cf_b * cf_mv.y + n_cf * cf_mv.z);
            let cf_bb = cf_unit(cross(cf_t, cf_n));
            let cf_tt = cf_unit(cross(cf_b, cf_n));
            let cf_ax = mix(0.0775, 0.7925, wv_cf.a);
            let cf_ay = mix(0.0775, 0.7925, 1.0 - wv_cf.a);
            let nvc = clamp(dot(n_cf, v), 0.0, 1.0);
            let fc_ = 0.005 + 0.1075 * pow(1.0 - nvc, 5.0);
            let nls_c = max(dot(n_cf, g.sun_dir.xyz), 0.0);
            let nlk_c = max(dot(n_cf, car_kd()), 0.0);
            let nlc_c = max(dot(n_cf, g.car_key.xyz), 0.0);
            let hs_c = normalize(g.sun_dir.xyz + v);
            let hk_c = normalize(car_kd() + v);
            let hc_c = normalize(g.car_key.xyz + v);
            let ws_c = g.sun_color.rgb * (g.sun_dir.w * 0.9 * clamp(4.0 * nls_c, 0.0, 1.0));
            let wk_c = car_kc() * (0.9 * clamp(4.0 * nlk_c, 0.0, 1.0));
            let wc_c = g.car_key_col.rgb * (0.9 * clamp(4.0 * nlc_c, 0.0, 1.0));
            let dh_c = ws_c * nls_c + wk_c * nlk_c;
            let dl_c = wc_c * nlc_c;
            let dif_c = (dh_c + dl_c - clamp(dh_c * dl_c * 0.1025, vec3<f32>(0.0), vec3<f32>(1.0))) * (0.03 * 0.1025);
            let sh_c = (ws_c * cf_ward(hs_c, cf_tt, cf_bb, cf_n, cf_ax, cf_ay) + wk_c * cf_ward(hk_c, cf_tt, cf_bb, cf_n, cf_ax, cf_ay)) * 1.5;
            let sl_c = wc_c * (cf_ward(hc_c, cf_tt, cf_bb, cf_n, cf_ax, cf_ay) * 1.5);
            let sp_c = (sh_c + sl_c - clamp(sh_c * sl_c, vec3<f32>(0.0), vec3<f32>(1.0))) * 0.03;
            let m_c = mix(dif_c, env_color(reflect(-v, n_cf)), fc_);
            lit = (m_c + sp_c - clamp(m_c * sp_c, vec3<f32>(0.0), vec3<f32>(1.0))) * (0.5 + 0.5 * in.color1.a);
            base = vec3<f32>(0.0);
            emissive = vec3<f32>(0.0);
            spec_k = 0.0;
        } else if (mat.mode == 13u && in.uv2.x > 0.5 && cp_on) {
            // PS_GlassFront of blackopaqueglass.fxc (cars/0x5df91aa0: opaque black panes):
            //   out = 0.25 x surroundings x sat(0.25 + 0.75 (1 - n.v)^5 + max(lum - 0.5, 0) / 4)
            // back faces black; no light term (reflectFresnelExp 5, Min 0.25, Max 1 of the material).
            let nv13 = dot(n, v);
            let m13 = clamp(0.25 + 0.75 * pow(1.0 - clamp(nv13, 0.0, 1.0), 5.0) + 0.25 * max(dot(env, vec3<f32>(0.3, 0.59, 0.11)) - 0.5, 0.0), 0.0, 1.0);
            lit = 0.25 * env * m13 * step(0.0, dot(in.normal, v));
            base = vec3<f32>(0.0);
            emissive = vec3<f32>(0.0);
            spec_k = 0.0;
        } else if (mat.mode == 18u && i32(round(in.uv1.x)) < 8 && obj.dents[clamp(i32(round(in.uv1.x)) - 1, 0, 2)].w > 0.5 && g.dome.w > 0.5 && g.ambient_ground.w > -0.5 && !(g.params.w > 134.5 && g.params.w < 135.5)) {
            // VS_Common + PS_Multi of cars/0x9ba9add5 (the rims), read from the listing (LIGHTING_WORK.md,
            // "Rims"): each vertex's zone picks a colour (c206 + zone) and a "material hookup" (c211 + zone):
            //   1: surroundings (sharp, along the mirror ray) x (0.125 + 1.1852 s) x colour^0.45 x occ
            //   3: the SOFT surroundings x (0.25 + 1.0667 s) x colour^0.45 x occ
            //   0: lerp(colour x light, surroundings x (0.5 + 0.6667 s), w x occ),
            //      w = sat(0.07 + 0.555 (1 - n.v)^4.5 + max(lum - 1, 0) / 4), light = the vertex
            //      program's screen(screen of the two lights x 0.9 x n.l, ambient) x occ
            //   s = sat(collector - 0.25); occ = the rim's baked occlusion (colour alpha).
            // The recorded R8 (rim finish 1 of the save on all zones: chrome) has hookup 1 with colour
            // 0.9743 on zones 0 - 2, hookup 3 on zone 3, 0 on zone 4; its far-side wheels are a 960-index
            // model with the colour / 8 (0.1218) and the soft map left bound. Here by the finish the
            // wheel's object carries: 2 -> hookup 1, 1 -> hookup 0, 3 -> hookup 3. Not ported: the fade along the
            // axle (sat(1 - 3.7037 (x + 0.22)) of the mesh's own x). MCLA_DEBUG_VIEW=135: the zones as before.
            let fin18 = obj.dents[clamp(i32(round(in.uv1.x)) - 1, 0, 2)];
            let occ18 = in.color.a;
            var fc18 = 127.0 / 255.0;
            if (g.tone.w < 0.5 && g.shadow_params.x > 0.0) {
                fc18 = car_collector(in.wpos);
            } else if (g.tone.w > 0.5 && g.car_amb.w < 0.5) {
                fc18 = 1.0;
            }
            let s18 = clamp(fc18 * FC_LIT - 0.25, 0.0, 1.0);
            let soft18 = env_soft(r);
            let col18 = pow(max(fin18.rgb, vec3<f32>(0.000001)), vec3<f32>(0.45));
            // (zone 3 keeps the rim file's own hookup, 3, whatever the finish: recorded)
            if (fin18.w > 3.5 && i32(round(in.uv1.x)) != 3) {
                // hookup 2 (the record's rim material 2, Carbon Fiber): `r0.yzw = 0.2` - a flat grey,
                // neither light nor surroundings nor occlusion (PS_Multi instructions 96 - 99)
                lit = vec3<f32>(0.2);
            } else if (fin18.w > 2.5 || i32(round(in.uv1.x)) == 3) {
                lit = soft18 * col18 * ((0.25 + 1.0667 * s18) * occ18);
            } else if (fin18.w > 1.5) {
                lit = env * col18 * ((0.125 + 1.1852 * s18) * occ18);
            } else {
                let nls18 = max(dot(n, g.sun_dir.xyz), 0.0);
                let nlk18 = max(dot(n, car_kd()), 0.0);
                let nlc18 = max(dot(n, g.car_key.xyz), 0.0);
                let lh18 = (g.sun_color.rgb * (g.sun_dir.w * nls18) + car_kc() * nlk18) * (0.9 * occ18);
                let ll18 = g.car_key_col.rgb * (nlc18 * 0.9 * occ18);
                var lt18 = lh18 + ll18 - clamp(lh18 * ll18, vec3<f32>(0.0), vec3<f32>(1.0));
                let am18 = hemi * occ18;
                lt18 = lt18 + am18 - clamp(lt18 * am18, vec3<f32>(0.0), vec3<f32>(1.0));
                let e18 = env * (0.5 + 0.66667 * s18);
                let w18 = clamp(0.07 + 0.555 * pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 4.5) + 0.25 * max(dot(e18, vec3<f32>(0.3, 0.59, 0.11)) - 1.0, 0.0), 0.0, 1.0) * occ18;
                lit = mix(fin18.rgb * lt18, e18, w18);
            }
            base = vec3<f32>(0.0);
            emissive = vec3<f32>(0.0);
            spec_k = 0.0;
        } else if (mat.mode == 18u && i32(round(in.uv1.x)) >= 10 && g.dome.w > 0.5 && g.ambient_ground.w > -0.5 && !(g.params.w > 125.5 && g.params.w < 126.5)) {
            // VS_ParallaxRubber2 + PS_ParallaxRubber2_false of cars/0x315a5780 (the tyres), the
            // pixel program run in numpy on hand-made interpolators (work/lighting/l3_tyre3.py):
            //   light = screen(screen(L0, L1) x s, AmbientIntensity (0.75) x ambYPos)
            //           each light: colour x n.l x sat(4 n.l) x 0.9 x DiffuseIntensity (0.775);
            //           s = sat(collector - 0.25) x 4 / 3 on BOTH lights
            //   env   = SOFT surroundings ^ (2 - diffuse alpha x normal map alpha) x diffuse alpha^2
            //           x (0.125 + 0.875 (1 - n.h)),  h between the normal and the eye
            //   out   = w x screen(diffuse x light, env)      w = o6.w, from the vertex colour
            // No highlight of the sun or the lamps at all: the hand rule that stood here (a
            // lobe of exponent 24, strength 0.45) turned the side walls orange under a low
            // sun - gate, camera turned left, 17:30: 194 109 57 against the original's black.
            // The texture as stored (car textures carry no gamma flag). MCLA_DEBUG_VIEW=126:
            // as it stood; 127: w = 1.
            var fct = 127.0 / 255.0;
            if (g.tone.w < 0.5 && g.shadow_params.x > 0.0) {
                fct = car_collector(in.wpos);
            } else if (g.tone.w > 0.5 && g.car_amb.w < 0.5) {
                fct = 1.0;
            }
            let st_ = clamp(fct * FC_LIT - 0.25, 0.0, 1.0) * 1.33333;
            let nlst = max(dot(n, g.sun_dir.xyz), 0.0);
            let nlkt = max(dot(n, car_kd()), 0.0);
            let nlct = max(dot(n, g.car_key.xyz), 0.0);
            let lht = (g.sun_color.rgb * (g.sun_dir.w * nlst * clamp(4.0 * nlst, 0.0, 1.0)) + car_kc() * (nlkt * clamp(4.0 * nlkt, 0.0, 1.0))) * (0.9 * 0.775);
            let llt = g.car_key_col.rgb * (nlct * clamp(4.0 * nlct, 0.0, 1.0) * 0.9 * 0.775);
            var ltt = (lht + llt - clamp(lht * llt, vec3<f32>(0.0), vec3<f32>(1.0))) * st_;
            let amt = hemi * 0.75;
            ltt = ltt + amt - clamp(ltt * amt, vec3<f32>(0.0), vec3<f32>(1.0));
            var wt = in.color.a;
            if (g.params.w > 126.5 && g.params.w < 127.5) { wt = 1.0; }
            // (the tyre's dirt, TireDirtColor (0.44, 0.35, 0.25) with .w = its hub's scrape sum: m = normal
            // map alpha x the vertex's dirt (vs_body, VsOut::dmg.x); the colour goes to the dirt by m and the
            // surroundings' share x sat(1 - 6 m) - PS_ParallaxRubber2_false 17 - 24, 84 - 86, in numpy
            // work/glassdmg/py/tyre_dirt.py: exact at m = 1)
            let mt = second.a * in.dmg.x;
            let dift = mix(car_raw(albedo.rgb), vec3<f32>(0.44, 0.35, 0.25), mt) * ltt;
            let nht = sqrt(clamp(0.5 + 0.5 * dot(n, v), 0.0, 1.0));
            let envt = pow(max(env_soft(reflect(-v, n)), vec3<f32>(0.000001)), vec3<f32>(2.0 - albedo.a * second.a)) * (albedo.a * albedo.a * (0.125 + 0.875 * (1.0 - nht))) * clamp(1.0 - 6.0 * mt, 0.0, 1.0);
            lit = (dift + envt - clamp(dift * envt, vec3<f32>(0.0), vec3<f32>(1.0))) * wt;
            base = vec3<f32>(0.0);
            emissive = vec3<f32>(0.0);
            spec_k = 0.0;
        } else if (mat.mode == 9u && in.uv2.y > 0.5 && cp_on && (u32(round(in.color.a * 255.0)) & 80u) == 80u) {
            // VS_Plate + PS_Plate of cars/0x94eca68d (the number plate), exact against the program
            // (work/lighting/c2_plateprobe.py, 40 random inputs, 3e-5):
            //   out   = (plate picture, the letters laid in)^1.6 x light        (the picture as stored)
            //   light = screen(screen(screen(screen(first x s, second), ambient x occ), own / 2), own / 2)
            //   each of the car's two lights x n.l x sat(4 n.l) x 0.9 x occ; s = sat(collector - 0.25) x 4 / 3
            // on the FIRST light only; own = the car's own lamps (siColor0 .. 2 by the colour's bytes).
            // No reflection, no highlight - which is why the plate is cream under a low yellow sun.
            // (vehicles.rs marks the plate's vertices: the colour's alpha = 16 + 64. MCLA_PLATE_OLD=1: unmarked.)
            let occp = in.color1.a;
            var fcp = 127.0 / 255.0;
            if (g.tone.w < 0.5 && g.shadow_params.x > 0.0) {
                fcp = car_collector(in.wpos);
            } else if (g.tone.w > 0.5 && g.car_amb.w < 0.5) {
                fcp = 1.0;
            }
            let sp_ = clamp(fcp * FC_LIT - 0.25, 0.0, 1.0) * 1.33333;
            let nlsp = max(dot(n, g.sun_dir.xyz), 0.0);
            let nlkp = max(dot(n, car_kd()), 0.0);
            let nlcp = max(dot(n, g.car_key.xyz), 0.0);
            let lhp = (g.sun_color.rgb * (g.sun_dir.w * nlsp * clamp(4.0 * nlsp, 0.0, 1.0)) + car_kc() * (nlkp * clamp(4.0 * nlkp, 0.0, 1.0))) * (0.9 * occp);
            let llp = g.car_key_col.rgb * (nlcp * clamp(4.0 * nlcp, 0.0, 1.0) * 0.9 * occp);
            let lamp_first_p = g.car_key_col.w > 0.5;
            let l0p = select(lhp, llp, lamp_first_p) * sp_;
            let l1p = select(llp, lhp, lamp_first_p);
            var ltp = l0p + l1p - clamp(l0p * l1p, vec3<f32>(0.0), vec3<f32>(1.0));
            let amp = hemi * occp;
            ltp = ltp + amp - clamp(ltp * amp, vec3<f32>(0.0), vec3<f32>(1.0));
            let ownp = 0.5 * car_own_lamps(vec4<f32>(in.color.rgb, 0.0));
            ltp = ltp + ownp - clamp(ltp * ownp, vec3<f32>(0.0), vec3<f32>(1.0));
            ltp = ltp + ownp - clamp(ltp * ownp, vec3<f32>(0.0), vec3<f32>(1.0));
            lit = pow(max(car_raw(albedo.rgb), vec3<f32>(0.000001)), vec3<f32>(1.6)) * ltp;
            base = vec3<f32>(0.0);
            emissive = vec3<f32>(0.0);
            spec_k = 0.0;
        } else if (mat.mode == 9u && in.uv2.y > 0.5 && cp_on) {
            // PS_BumpSpec_true_true of bumpspec.fxc / bumpspecalpha.fxc (cars/0x601ee749,
            // 0x297774d0: dark trim, mirror glass, badges ...), exact against the program for
            // n.v >= 0 (work/lighting/bs3.py, 300 random inputs):
            //   colour = texture x baseColor
            //   light  = screen(screen(screen(L, ambient x occ), own), own)
            //            L = screen of the car's two lights, each x n.l x sat(4 n.l) x s x occ,
            //            s = sat((collector - 0.25) x 4 / 3); own = the car's own lamps (above)
            //   lit    = screen(colour x light, SOFT surroundings ^ SpecMapExp x SpecularColor x (0.5 + 0.5 occ))
            //   mirror = 0.5 x surroundings x lerp(1, colour, metallic x (1 - (1 - n.v)^5)) x occ
            //   out    = lerp(lit, mirror, w),  w = sm^5 + (sm - sm^5) (1 - n.v)^5, sm = smoothness
            // The mirrors' glass is this with smoothness 1: half the surroundings and nothing else.
            // The material's terms ride in the vertices (vehicles.rs): uv2.y - 1 = base r + 256 g,
            // uv1.x = base b + 256 x 32 SpecMapExp, -uv2.x - 1 = 200 smoothness + 256 x 200
            // SpecularColor, the colour's alpha x 255 = 10 metallic (+ 16 cut-out, + 32 cabin,
            // + 64 GrillOpaque).
            let pa9 = u32(round(in.uv2.y)) - 1u;
            let pb9 = u32(round(max(in.uv1.x, 0.0)));
            let pc9 = u32(round(max(-in.uv2.x, 1.0))) - 1u;
            let pd9 = u32(round(in.color.a * 255.0));
            // (the texture as stored - the cars' textures are not gamma textures; MCLA_DEBUG_VIEW=103:
            // decoded, as it stood)
            let tex9 = select(car_raw(albedo.rgb), albedo.rgb, g.params.w > 102.5 && g.params.w < 103.5);
            let col9 = tex9 * vec3<f32>(f32(pa9 & 255u), f32((pa9 >> 8u) & 255u), f32(pb9 & 255u)) / 255.0;
            let e9 = f32((pb9 >> 8u) & 255u) / 32.0;
            let sm9 = f32(pc9 & 255u) / 200.0;
            let k9 = f32((pc9 >> 8u) & 255u) / 200.0;
            let m9 = f32(pd9 & 15u) / 10.0;
            let occ9 = in.color1.a;
            var fc9 = 127.0 / 255.0;
            if (g.tone.w < 0.5 && g.shadow_params.x > 0.0) {
                fc9 = car_collector(in.wpos);
            } else if (g.tone.w > 0.5 && g.car_amb.w < 0.5) {
                fc9 = 1.0;
            }
            let q9 = clamp((fc9 * FC_LIT - 0.25) * 1.33333, 0.0, 1.0) * occ9;
            let nls9 = max(dot(n, g.sun_dir.xyz), 0.0);
            let nlk9 = max(dot(n, car_kd()), 0.0);
            let nlc9 = max(dot(n, g.car_key.xyz), 0.0);
            // (x 0.9: the lights' range factor, as in the paint - every car program that takes the two
            // lights has it: VS_BumpSpec, VS_Common, VS_Emissive, VS_Glass, VS_Layout, VS_Plate, and in
            // the pixel programs PS_Light, PS_CarbonFiber, PS_ParallaxRubber2 - work/lighting/carvs_range.py)
            let att9 = select(0.9, 1.0, g.params.w > 98.5 && g.params.w < 99.5);
            let lh9 = (g.sun_color.rgb * (g.sun_dir.w * nls9 * clamp(4.0 * nls9, 0.0, 1.0)) + car_kc() * (nlk9 * clamp(4.0 * nlk9, 0.0, 1.0))) * (q9 * att9);
            let ll9 = g.car_key_col.rgb * (nlc9 * clamp(4.0 * nlc9, 0.0, 1.0) * q9 * att9);
            var lt9 = lh9 + ll9 - clamp(lh9 * ll9, vec3<f32>(0.0), vec3<f32>(1.0));
            let am9 = hemi * occ9;
            lt9 = lt9 + am9 - clamp(lt9 * am9, vec3<f32>(0.0), vec3<f32>(1.0));
            var own9 = car_own_lamps(in.color);
            if (g.params.w > 72.5 && g.params.w < 73.5) { own9 = vec3<f32>(0.0); }
            lt9 = lt9 + own9 - clamp(lt9 * own9, vec3<f32>(0.0), vec3<f32>(1.0));
            lt9 = lt9 + own9 - clamp(lt9 * own9, vec3<f32>(0.0), vec3<f32>(1.0));
            let dif9 = col9 * lt9;
            // (grillopaque.fxc, PS_BumpSpec_SpecMap of cars/0x3d436fa4 - bit 64: the same program
            // with this term times the texture's colour; work/lighting/bs6.py)
            let sp9 = pow(max(env_soft(r), vec3<f32>(0.000001)), vec3<f32>(e9)) * (k9 * (0.5 + 0.5 * occ9)) * select(vec3<f32>(1.0), tex9, (pd9 & 64u) != 0u);
            let lit9 = dif9 + sp9 - clamp(dif9 * sp9, vec3<f32>(0.0), vec3<f32>(1.0));
            let f59 = pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 5.0);
            let s59 = sm9 * sm9 * sm9 * sm9 * sm9;
            let w9 = s59 + (sm9 - s59) * f59;
            let mir9 = env * 0.5 * (vec3<f32>(1.0) + (col9 - vec3<f32>(1.0)) * (m9 * (1.0 - f59))) * occ9;
            lit = mix(lit9, mir9, w9);
            // (74: the own-lamp light alone)
            if (g.params.w > 73.5 && g.params.w < 74.5) { lit = own9; }
            base = vec3<f32>(0.0);
            emissive = vec3<f32>(0.0);
            spec_k = 0.0;
        } else if (mat.mode == 7u && g.dome.w > 0.5 && g.ambient_ground.w > -0.5) {
            // PS_Chrome, the whole of it: the surroundings mirrored (fading to the ambient
            // colour on tight curves, as the paint's mirror does but without the doubling),
            // times 0.125 + 1.185 x sat(collector - 0.25), times the baked occlusion. No light
            // term, no highlight of its own: the sun's glint on chrome is the sun in the map.
            // (no sun shadows - a storm, the frame of 06:00 / 18:00 sharp: the game binds an 8 x 8
            // grey of 127 where the collector is)
            var fc_c = 127.0 / 255.0;
            if (g.tone.w < 0.5 && g.shadow_params.x > 0.0) {
                fc_c = car_collector(in.wpos);
            } else if (g.tone.w > 0.5 && g.car_amb.w < 0.5) {
                fc_c = 1.0;
            }
            let cur_c = clamp(clamp(length(dpdx(n)) - 0.15, 0.0, 1.0) + clamp(length(dpdy(n)) - 0.15, 0.0, 1.0), 0.0, 1.0);
            lit = mix(env, hemi, cur_c) * (0.125 + 1.18518 * clamp(fc_c - 0.25, 0.0, 1.0)) * in.color1.a;
            spec_k = 0.0;
        } else if (mat.mode == 7u) {
            // polished metal: mostly mirror, darker face-on so spokes and lips read
            lit = (env * 0.34 * mix(0.45, 1.0, clamp(f * 3.0, 0.0, 1.0)) + lit * 0.07) * albedo.rgb;
        } else if (mat.mode == 6u) {
            // PS_CarPaint with the car's paint type (in the object's fourth parameter, above
            // its lamp state). The mirror: DL1.y + (DL1.z - DL1.y) * (1 - n.v)^DL1.x of the
            // surroundings - nothing at all on matte paint. The flakes: noise of DL4's scale
            // tilts the normal by its strength before the surroundings are looked up. The
            // soft specular: the surroundings again, DL0.y strong, between the paint's own
            // colour (times their brightness) and their own by the pearlescence. Diffuse and
            // ambient light by DL0.x and DL0.z, here relative to the metallic type the car
            // files themselves carry.
            var ti = min(select(3u, (u32(round(obj.params.w)) / 32u) % 8u, obj.params.w > 15.5), 5u);
            // (a paint area with its own entry has its own paint type)
            let ti_area = paint_area(in.uv2.y);
            if (ti_area > 0u) { ti = min(u32(round(obj.area_b[ti_area - 1u].w)) % 8u, 5u); }
            var pa = PAINT_A;
            var pb = PAINT_B;
            var pc = PAINT_C;
            let ta = pa[ti];
            let tb = pb[ti];
            let tc = pc[ti];
            let ndv = clamp(dot(n, v), 0.0, 1.0);
            if (g.ambient_ground.w > -0.5) {
                // The game's own rule (VS_CarPaint + PS_CarPaint, LIGHTING_WORK.md), DL0 = ta
                // (diffuse, specular, ambient, specular power + 1), DL1 = tb (fresnel exponent,
                // least and most reflection, falloff), tc = (pearlescence, noise scale, flake
                // strength):
                //   light  = screen(sun x n.l x DL0.x x s + lamp x n.l x DL0.x, ambient x DL0.z)
                //   mirror = lerp(paint x light, surroundings x k, fresnel (+ a quarter of what
                //            the surroundings exceed 1 by))
                //   + sat(DL0.y x |soft|^(DL0.w - 1) x lerp(paint x lum(soft), soft, pearl) x k)
                // s = sat((fast shadow collector - 0.25) / 0.75), k = 0.5 + 0.5 s; "soft" is the
                // blurred copy of the surroundings looked up along the flake's normal.
                // (without sun shadows - a storm, the frame of 06:00 / 18:00 sharp - the game binds
                // an 8 x 8 grey of 127 where the collector is: s = 0.33, k = 0.67)
                var fc = 127.0 / 255.0;
                if (g.tone.w < 0.5 && g.shadow_params.x > 0.0) {
                    // (the collector holds two layers - the sun's visibility at the ground and 8 m
                    // above it; the paint reads the GROUND one, the fetch's x - see car_collector.
                    // The earlier reading, "the upper one", had the two bytes the wrong way round.)
                    fc = car_collector(in.wpos);
                } else if (g.tone.w > 0.5 && g.car_amb.w < 0.5) {
                    // (at night the game binds another map where the collector was - 512 x 512,
                    // one 8-bit channel, the cars' shadows under the street lamps (PS_ShadowNight):
                    // 255 round the car, nowhere below 210 in a frame read out of the running
                    // game at 23:30 with its real layout. An earlier reading of it as four
                    // channels gave "0.14" and a night car without its lamp light: wrong.)
                    fc = 1.0;
                }
                let sk = clamp((fc * FC_LIT - 0.25) * 1.33333, 0.0, 1.0);
                let kk = 0.5 + 0.66667 * clamp(fc * FC_LIT - 0.25, 0.0, 1.0);
                // (the body's baked occlusion: the first colour set's alpha; the lights take
                // 0.125 + 0.875 of it, the mirror and the specular the value itself)
                // (vehicles.rs hands it on in the second colour's alpha; 1 where a mesh has none)
                let ao_v = in.color1.a;
                // (traffic and parked cars - their lamp rows marked ambient, lamp_a[0].w - are drawn by
                // VSAmbientCarPaint / PSAmbientCarPaint of cars/0xdacfb063 instead: the lights take the
                // occlusion byte as it is, no 0.125 + 0.875 of it; the rest of their rule below.
                // MCLA_DEBUG_VIEW 698: traffic through the player's CarPaint rule, with the same paint value)
                let amb6 = obj.lamp_a[0].w > 0.5 && obj.params.w > 15.5 && !(g.params.w > 697.5 && g.params.w < 698.5);
                let aok = select(0.125 + 0.875 * ao_v, ao_v, amb6);
                // The car's two lights: the hour's light (the sun; from dusk to dawn what the
                // light track holds - the last of the sun, then a faint grey from below) and the
                // strongest city lamp (nothing by day). Only the FIRST of the two is weighed by the
                // collector's s, the second joins the ambient unshadowed:
                //   light = screen(first x s, screen(second, ambient))
                // Which comes first is settled when the car is made and kept from then on (the
                // lamp for a car made at night, the hour's light for one made by day): car_key_col.w.
                // (VS_CarPaint 82 - 99: each of the two x sat(1 - distance / range), and the game
                // hands both lights over as points 100 m off the car with a range of 1000 -
                // gLightPosDir.w, at every hour recorded: 0.9. Diagnostics 99: without, as before.)
                let att6 = select(0.9, 1.0, g.params.w > 98.5 && g.params.w < 99.5);
                let l_hour = (g.sun_color.rgb * g.sun_dir.w * max(dot(n, g.sun_dir.xyz), 0.0) + car_kc() * max(dot(n, car_kd()), 0.0)) * (ta.x * aok * att6);
                let l_lamp = g.car_key_col.rgb * max(dot(n, g.car_key.xyz), 0.0) * (ta.x * aok * att6);
                let lamp_first = g.car_key_col.w > 0.5;
                let l0 = select(l_hour, l_lamp, lamp_first) * sk;
                let l1 = select(l_lamp, l_hour, lamp_first);
                let la0 = hemi * (ta.z * aok);
                let la = l1 + la0 - clamp(l1 * la0, vec3<f32>(0.0), vec3<f32>(1.0));
                let light_sl = l0 + la - clamp(l0 * la, vec3<f32>(0.0), vec3<f32>(1.0));
                // (+ the car's own lamps on the panels round them, screened in once and not
                // weighed by the occlusion - VS_CarPaint / PS_CarPaint run with the lamp colours
                // set, work/lighting/paintsi.py; car_own_lamps above. Diagnostics 72 / 73: without.)
                var own6 = car_own_lamps(in.color);
                if (g.params.w > 71.5 && g.params.w < 73.5) { own6 = vec3<f32>(0.0); }
                let light = light_sl + own6 - clamp(light_sl * own6, vec3<f32>(0.0), vec3<f32>(1.0));
                let dif = base * light;
                // (tight curves: the mirror blurs and fades to the ambient colour)
                let cur = clamp(2.0 * (clamp(length(dpdx(n)) - 0.15, 0.0, 1.0) + clamp(length(dpdy(n)) - 0.15, 0.0, 1.0)), 0.0, 1.0);
                let envm = mix(env, hemi, cur) * kk;
                let fr = (tb.y + (tb.z - tb.y) * pow(1.0 - ndv, tb.x)) * step(0.0005, tb.z);
                let w = clamp(fr + 0.25 * step(0.001, tb.y) * max(dot(envm, vec3<f32>(0.3, 0.59, 0.11)) - 1.0, 0.0), 0.0, 1.0) * (1.0 - scraped) * ao_v;
                // the flakes: a 128 x 128 noise, uv x DL4.x x 4 (ObjNoiseUvMod), whose mips fade to
                // flat grey quickly (62 -> 16 -> 3 of 255 by the second level, bias -1)
                let uvn = in.uv0 * tc.y * 4.0;
                let tp = max(length(dpdx(uvn)), length(dpdy(uvn))) * 128.0;
                let fade = exp2(-2.0 * max(log2(max(tp, 0.0001)) - 1.0, 0.0));
                let cell = floor(uvn * 128.0);
                let flake = fract(sin(vec3<f32>(dot(cell, vec2<f32>(127.1, 311.7)), dot(cell, vec2<f32>(269.5, 183.3)), dot(cell, vec2<f32>(419.2, 371.9)))) * 43758.5453) - vec3<f32>(0.5);
                let nf = normalize(n + flake * tc.z * fade * clamp(1.0 - 2.0 * scraped, 0.0, 1.0));
                let soft = env_soft(reflect(-v, nf));
                let s_lum = dot(soft, vec3<f32>(0.3, 0.59, 0.11));
                let sp = clamp(mix(base * s_lum, soft, tc.x) * (ta.y * pow(max(length(soft), 0.00001), ta.w - 1.0) * kk * ao_v), vec3<f32>(0.0), vec3<f32>(1.0)) * (1.0 - scraped) * ao_v;
                // (the soft highlight goes on as a SCREEN, a + b - sat(a b), not a sum: PS_CarPaint
                // per pixel on the port's own maps gave each of the three terms to a level and their
                // total 10 levels under the port's sum on the lid at 05:45 - scene 0.3345 where the
                // parts add to 0.3576 = 0.0845 + 0.2731 - 0.0845 x 0.2731; work/lighting/
                // carpaint_px.py -terms. Diagnostics 101: the plain sum, as until 2026-10-14.)
                // (step 19g: + DL6.w, the Mirror power-up's weight - obj.fade_min.w, 0 but on the
                // player's car under Mirror; PS_CarPaint 111)
                let m6 = mix(dif, envm, w + obj.fade_min.w);
                lit = m6 + sp - clamp(m6 * sp, vec3<f32>(0.0), vec3<f32>(1.0));
                if (g.params.w > 100.5 && g.params.w < 101.5) { lit = m6 + sp; }
                if (amb6) {
                    // PSAmbientCarPaint (work/trafpaint/dis/amb_carpaint.txt; docs/LIGHTING.md "Traffic paint:
                    // the ambient cars' own program"): no flakes, no vinyl, no two-tone - the paint is the
                    // vertex program's sat(4 x C^2.2), handed in as obj.color (vehicles.rs upload_uniforms).
                    //   w    = sat(fresnel - [|grad(n.v)| >= 0.125]) x AO, fresnel = DL1.y + (DL1.z - DL1.y) sat(1 - n.v)^DL1.x
                    //   m    = lerp(paint x light, sharp map, w)               (the map not weighed by k)
                    //   spec = sat(DL0.y x AO x |soft|^(DL0.w - 1) x lerp(paint x lum(soft), soft, DL4.w) x k)
                    //   out  = screen(m, spec)
                    // (soft and sharp maps along the mirror ray, the fresnel and the edge's n.v as the vertex
                    // program worked them out at the vertices, interpolated - vs_body; MCLA_DEBUG_VIEW 700:
                    // per pixel, as until 2026-10-21)
                    let pv_a = !(g.params.w > 699.5 && g.params.w < 700.5);
                    let ndv_a = select(dot(n, v), in.neon_hl - 4.0, pv_a);
                    let edge_a = select(0.0, 1.0, sqrt(dpdx(ndv_a) * dpdx(ndv_a) + dpdy(ndv_a) * dpdy(ndv_a)) >= 0.125);
                    let fr_a = select(tb.y + (tb.z - tb.y) * pow(clamp(1.0 - ndv_a, 0.0, 1.0), tb.x), in.fill.w, pv_a);
                    let w_a = clamp(fr_a - edge_a, 0.0, 1.0) * ao_v;
                    let ray_a = select(reflect(-v, n), in.fill.xyz, pv_a);
                    let m_a = mix(dif, select(env, env_color(ray_a), pv_a), w_a);
                    let soft_a = env_soft(ray_a);
                    let sp_a = clamp(mix(base * dot(soft_a, vec3<f32>(0.3, 0.59, 0.11)), soft_a, tc.x) * (ta.y * ao_v * pow(max(length(soft_a), 0.00001), ta.w - 1.0) * kk), vec3<f32>(0.0), vec3<f32>(1.0));
                    lit = m_a + sp_a - clamp(m_a * sp_a, vec3<f32>(0.0), vec3<f32>(1.0));
                }
                // (the damage map's marks on the lit paint, PS_CarPaint: the soot towards (0.22,
                // 0.22, 0.17) of it, x (1 - rub), the bare metal - the soft surroundings three
                // quarters grey, to the power 1.5 - by m, x (1 - rub) again)
                if (in.dmg.x > 0.0 || in.dmg.y > 0.0) {
                    lit = mix(lit, lit * vec3<f32>(0.22, 0.22, 0.17), clamp(in.dmg.y, 0.0, 1.0));
                    lit = lit * (1.0 - rubbed);
                    let metal = pow(max(mix(soft, vec3<f32>(dot(soft, vec3<f32>(0.3, 0.59, 0.11))), 0.75), vec3<f32>(0.0)), vec3<f32>(1.5));
                    lit = mix(lit, metal, scraped);
                    lit = lit * (1.0 - rubbed);
                }
                // (diagnostics 26 / 27 / 28 / 29: the diffuse, the mirror, the soft specular alone; the soft map itself)
                if (g.params.w > 25.5 && g.params.w < 26.5) { lit = dif * (1.0 - w); }
                if (g.params.w > 26.5 && g.params.w < 27.5) { lit = envm * w; }
                if (g.params.w > 27.5 && g.params.w < 28.5) { lit = sp; }
                // (29: the soft map x 4 as a display value; 30: the sharp map x 4 along the mirror ray)
                if (g.params.w > 28.5 && g.params.w < 29.5) { return vec4<f32>(to_scene(soft * 4.0), 1.0); }
                if (g.params.w > 29.5 && g.params.w < 30.5) { return vec4<f32>(to_scene(env * 4.0), 1.0); }
                spec_k = 0.0;
            } else {
            let cell = floor(in.uv0 * tc.y * 220.0);
            let flake = fract(sin(vec3<f32>(dot(cell, vec2<f32>(127.1, 311.7)), dot(cell, vec2<f32>(269.5, 183.3)), dot(cell, vec2<f32>(419.2, 371.9)))) * 43758.5453) - vec3<f32>(0.5);
            // (kept faint: in the game's garage footage metallic paint reads as smooth, with only a fine sparkle)
            let nf = normalize(n + flake * tc.z * tc.z * 0.6 * (1.0 - scraped));
            let envp = env_color(reflect(-v, nf));
            let fm = (tb.y + (tb.z - tb.y) * pow(1.0 - ndv, tb.x) + obj.color.a * 0.02 * step(0.001, tb.z)) * (1.0 - 0.85 * scraped);
            // (against the game: its paint is deep - a lit panel shows about the swatch's own colour)
            var dif_l = (hemi * ao * (ta.z / 0.2) * 0.75 + g.sun_color.rgb * g.sun_dir.w * ndl * mix(0.6, 1.0, occ) * (ta.x / 0.225)) * 0.6;
            if (g.ambient_ground.w > 0.0) {
                // (display-space lighting, as for the world: paint in shade is deep)
                dif_l = dif_l * pow(clamp(dot(dif_l, vec3<f32>(0.2126, 0.7152, 0.0722)) / g.ambient_ground.w, 0.0, 1.4), 0.9);
            }
            // (sparkle's coarse flakes show as a grain in the colour itself)
            let dif = base * dif_l * (1.0 + flake.x * tc.z * tc.z * 0.9) * select(1.0, 1.25, g.ambient_ground.w < -0.5);
            let lum = dot(envp, vec3<f32>(0.3, 0.59, 0.11));
            let soft = mix(base * lum, envp, tc.x) * ta.y * pow(max(lum, 0.001), ta.w - 1.0) * 0.1 * (1.0 - scraped);
            lit = dif * (1.0 - fm * 0.5) + envp * fm * 0.8 + soft;
            // (the lamps' and the sun's highlight is the mirror's doing - sharp on gloss, none on
            // matte - with the soft specular's broad sheen about it on metallic and pearl)
            spec_k = clamp((tb.y + 0.3 * tb.z) * 4.0, 0.0, 1.2);
            // (the garage's lamps - ambient_ground.w below zero - are far brighter than the room they light)
            if (g.ambient_ground.w < -0.5) { spec_k = spec_k * 5.0; }
            lit = lit + g.sun_color.rgb * pow(ndh, 14.0) * ta.y * select(0.10, 0.4, g.ambient_ground.w < -0.5) * step(0.001, tb.z) * g.sun_dir.w * shadow * (1.0 - scraped);
            }
        } else if (mat.mode == 10u) {
            // rubber and matte black: no mirror in them at all
            lit = lit;
        } else if (mat.mode == 8u && g.dome.w > 0.5 && g.ambient_ground.w > -0.5 && !(g.params.w > 59.5 && g.params.w < 60.5)) {
            // PS_Light of carlight.fxc (cars/0xa4f50432: the lamps' reflectors and lit parts,
            // opaque), run in numpy with the captures' constants - work/lighting/lampprobe.py,
            // lampprobe2.py:
            //   dif  = (texEmuSpotMin + (Max - Min) |bump.x|^5) x B.rgb x (1 + illum.g) / 4 x light
            //   base = lerp(dif, surroundings, 0.025)
            //   out  = screen(base + screen(base, soft surroundings x B.rgb x 0.58125),
            //                 3 x illum.r x B.a x A.rgb x B.rgb)
            // light = screen(the car's two lights, ambient); screen(a, b) = a + b - sat(a b). The
            // illumination map is read as stored (no gamma). A (c128 + id) and B (c163 + id) are
            // rows of gCarConstantBuffer picked by the lamp id of the second UV set: B.rgb the
            // lamp's colour, B.a its state - recorded 0 by day; with the lights on 0.25 for brake
            // and tail lamps, 1 for signal (2), head (3), side (5) and fog (8) lamps; 0.77 on the
            // brake lamps and 0.69 on the third one in a braking frame (taken as fading to 1).
            // The rows are the car's own (render/car_lamps.rs: the LightTune of its set-up file,
            // bulb colour = A, cover colour = B; the Audi R8's where a car has none). The lamp's
            // normal map is not bound here: flat, so texEmuSpotMin 0.1875.
            // Traffic cars (lamp_a[0].w = 1) are drawn by PSAmbientCarLight of cars/0x1e0d744b
            // instead, read from its listing (work/lighting/carlamps_src.py):
            //   out = screen(map.rgb x cover x light, 3 x bulb x cover x state)
            // with bulb = c128 + id and cover = c148 + id of their type's LightTune, the state in
            // the cover row's w - the car's own (vehicles.rs `AmbLamps`: sub_82792FD8's eased
            // states at the entry's attachment; lamp_b[id].w here) - and no illumination map in
            // the lit term.
            // (MCLA_DEBUG_VIEW 60: the older reading; 61 the lamp ids; 62 the map as stored)
            let lid = i32(round(in.uv1.x));
            let lst = i32(round(obj.params.w)) % 16;
            let brk = clamp(obj.params.y * 4.0, 0.0, 1.0);
            let lrow = clamp(lid, 0, 18);
            let la = obj.lamp_a[lrow].rgb;
            let lb = obj.lamp_b[lrow].rgb;
            let amb8 = obj.lamp_a[0].w > 0.5;
            var l_on = 0.0;
            if (lid == 0) { l_on = max(0.25 * obj.params.x, brk); }
            else if (lid == 1) { l_on = 0.25 * obj.params.x; }
            else if (lid == 2) { l_on = select(obj.params.x, 0.0, amb8); }
            else if (lid == 3) { l_on = obj.params.x; }
            else if (lid == 5) { l_on = obj.params.x; }
            else if (lid == 6) { l_on = f32(lst / 4); }
            else if (lid == 8) { l_on = obj.params.x; }
            else if (lid == 12) { l_on = brk; }
            else if (lid == 13) { l_on = f32(lst % 4 == 1); }
            else if (lid == 14) { l_on = f32(lst % 4 == 2); }
            if (amb8) { l_on = obj.lamp_b[lrow].w; }
            let ix = clamp(albedo.rg, vec2<f32>(0.0), vec2<f32>(1.0));
            let illum = select(1.055 * pow(ix, vec2<f32>(1.0 / 2.4)) - vec2<f32>(0.055), ix * 12.92, ix <= vec2<f32>(0.0031308));
            if (g.params.w > 60.5 && g.params.w < 61.5) { return vec4<f32>(to_scene(vec3<f32>(f32(lid % 3) * 0.5, f32((lid / 3) % 3) * 0.5, f32(lid / 9) * 0.5) + vec3<f32>(0.05)), 1.0); }
            if (g.params.w > 61.5 && g.params.w < 62.5) { return vec4<f32>(to_scene(vec3<f32>(illum, 0.0)), 1.0); }
            let att8 = select(0.9, 1.0, g.params.w > 98.5 && g.params.w < 99.5);
            let lh8 = (g.sun_color.rgb * g.sun_dir.w * max(dot(n, g.sun_dir.xyz), 0.0) + car_kc() * max(dot(n, car_kd()), 0.0)) * att8;
            let ll8 = g.car_key_col.rgb * max(dot(n, g.car_key.xyz), 0.0) * att8;
            let l2 = lh8 + ll8 - clamp(lh8 * ll8, vec3<f32>(0.0), vec3<f32>(1.0));
            let lg8 = l2 + hemi - clamp(l2 * hemi, vec3<f32>(0.0), vec3<f32>(1.0));
            let b8 = mix(lb * lg8 * (0.1875 * 0.25 * (1.0 + illum.y)), env, 0.025);
            let sm8 = env_soft(r) * lb * 0.58125;
            var ub8 = b8 + (b8 + sm8 - clamp(b8 * sm8, vec3<f32>(0.0), vec3<f32>(1.0)));
            var em8 = la * lb * (3.0 * illum.x * l_on);
            if (amb8) {
                let ix3 = clamp(albedo.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
                let map3 = select(1.055 * pow(ix3, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055), ix3 * 12.92, ix3 <= vec3<f32>(0.0031308));
                ub8 = map3 * lb * lg8;
                em8 = la * lb * (3.0 * l_on);
            }
            lit = ub8 + em8 - clamp(ub8 * em8, vec3<f32>(0.0), vec3<f32>(1.0));
            // (the program has these three lights and no others; its emission is in the sum)
            emissive = vec3<f32>(0.0);
            base = vec3<f32>(0.0);
            spec_k = 0.0;
        } else if (mat.mode == 12u && in.uv2.x < 0.5 && g.dome.w > 0.5 && g.ambient_ground.w > -0.5 && !(g.params.w > 59.5 && g.params.w < 60.5)) {
            // PS_GlassFront of carglass.fxc (cars/0xddcdc14b, the windows) and PS_Light of
            // bumpedglass.fxc (cars/0x9bc3cca1, the lamps' lenses), run in numpy with the
            // captures' constants - work/lighting/cglassprobe.py, lensprobe.py. One rule:
            //   m     = sat(Fmin + (Fmax - Fmin) (1 - |n.v|)^Fexp + (lum(surroundings) - 1) / 4)
            //   rgb   = surroundings x m + tint.rgb x screen(the car's two lights, 2 x ambient)
            //   alpha = sat(tint.a + m)          (the program's alpha x 16: gInvColorExpBias)
            // windows 0.125 .. 0.85 by ^4.5, lenses 0.125 .. 0.875 by ^5; blended source alpha.
            // The tint is a row of TintColors / BumpedTintColors (c209 + the zone of the second
            // UV set), as recorded on the R8: (0, 0.0035, 0.005, 0.372) for zones 0 - 3 and 8,
            // alpha 0.392 for 6 and 7, 0 (windows) / 0.196 (lenses) for 4 and 5. Both faces alike.
            // The lenses are drawn twice where they face the eye (once unculled without depth
            // writes, once culled - registers 0x2205 / 0x2200 of the capture): a (2 - a) there.
            // (MCLA_DEBUG_VIEW 60: the older reading; 61 the zones)
            let lensg = in.uv2.y > 0.5;
            let zone = i32(round(in.uv1.x));
            var tint4 = vec4<f32>(0.0, 0.0035, 0.005, 0.372);
            if (zone == 4 || zone == 5) { tint4 = vec4<f32>(0.0, 0.0, 0.0, select(0.0, 0.1961, lensg)); }
            else if (zone == 6 || zone == 7) { tint4 = vec4<f32>(0.0, 0.0, 0.0, 0.3922); }
            // (the windows' rows are the car's own m_vWindowTintColorNew: the garage's tint
            // menu writes its darkness, 0.372 + 0.528 k, and colour slot 14 its colour)
            if (!lensg && obj.tint.w > 0.0 && (zone <= 3 || zone == 8)) { tint4 = obj.tint; }
            if (g.params.w > 60.5 && g.params.w < 61.5) { return vec4<f32>(to_scene(vec3<f32>(f32(zone % 3) * 0.5, f32((zone / 3) % 3) * 0.5, f32(zone / 9) * 0.5 + select(0.0, 0.25, lensg)) + vec3<f32>(0.05)), 1.0); }
            let nv12 = clamp(abs(dot(n, v)), 0.0, 1.0);
            let fr12 = select(0.125 + 0.725 * pow(1.0 - nv12, 4.5), 0.125 + 0.75 * pow(1.0 - nv12, 5.0), lensg);
            let m12 = clamp(fr12 + 0.25 * max(dot(env, vec3<f32>(0.3, 0.59, 0.11)) - 1.0, 0.0), 0.0, 1.0);
            let att12 = select(0.9, 1.0, g.params.w > 98.5 && g.params.w < 99.5);
            let lh12 = (g.sun_color.rgb * g.sun_dir.w * max(dot(n, g.sun_dir.xyz), 0.0) + car_kc() * max(dot(n, car_kd()), 0.0)) * att12;
            let ll12 = g.car_key_col.rgb * max(dot(n, g.car_key.xyz), 0.0) * att12;
            let l12 = lh12 + ll12 - clamp(lh12 * ll12, vec3<f32>(0.0), vec3<f32>(1.0));
            let lg12 = l12 + 2.0 * hemi - clamp(l12 * 2.0 * hemi, vec3<f32>(0.0), vec3<f32>(1.0));
            lit = tint4.rgb * lg12 + env * m12;
            var a12 = clamp(tint4.a + m12, 0.0, 1.0);
            // The driver's camera (2026-10-20, work/intglass; damage_map_rule.md Part C 6): the original's POV
            // camera sets G+122 of the car it follows (sub_822A9670), and that car's windows are drawn with
            // technique interiorwindowglass - VS_InteriorGlass + PS_InteriorGlass (recorded in the driver view,
            // cap/pov_noon), run in numpy against this (ps_check.py, 6e-6):
            //   sh   = 4/3 sat(collector - 0.25)
            //   D    = screen(sh x screen(12 L0, 12 L1), 12 ambient)     (L = colour x sat(n.l) x the 0.9)
            //   z    = (noise.g - 0.5) 0.15 + 0.5,  noise = 2dnoise4.dds at 11 uv0;  q = mask.g + 0.1
            //   rgb  = lerp(tint.rgb / 12, 0.5, z q / 11) x D + sh f^2 x L0 colour,  f = 4 sat(-v.l0 - 0.8)
            //   a    = z q / 1.1 + 0.5 sat(tint.a - 0.3)              (x gInvColorExpBias, as above)
            // (screen(a, b) = a + b - sat(a b); L0 = the car's first light slot, the one the glare takes;
            // mask = window_detail_mask.dds.) No surroundings, no cracks, no clip plane: WindowDmgVal is 0
            // while G+123 (sub_82357DB8), so k = 1 - sat(s - 1)(1 - mask.a) = 1 and nothing is cut.
            // Only the panes' inner side is drawn (cull mode 0x19806 against the body's 0x19802; the pipeline
            // `pipeline_glass_inner`, without depth writes), and twice in one frame with the same state - here
            // once with a (2 - a). MCLA_DEBUG_VIEW=730: the outside rule in the driver's view as before.
            let int12 = (u32(round(obj.params.w)) & 256u) != 0u && !lensg && in.uv2.y < -0.5 && !(g.params.w > 729.5 && g.params.w < 730.5);
            if (int12) {
                if (dot(in.normal, v) >= 0.0) { discard; }
                var fci = 127.0 / 255.0;
                if (g.tone.w < 0.5 && g.shadow_params.x > 0.0) {
                    fci = car_collector(in.wpos);
                } else if (g.tone.w > 0.5 && g.car_amb.w < 0.5) {
                    fci = 1.0;
                }
                let shi = 4.0 / 3.0 * clamp(fci * FC_LIT - 0.25, 0.0, 1.0);
                let ai = 12.0 * lh12;
                let bi = 12.0 * ll12;
                let oi = ai + bi - clamp(ai * bi, vec3<f32>(0.0), vec3<f32>(1.0));
                let li = shi * oi;
                let hi = 12.0 * hemi;
                let di = li + hi - clamp(li * hi, vec3<f32>(0.0), vec3<f32>(1.0));
                let nzi = (textureSampleGrad(t_third, s_diffuse, in.uv0 * 11.0, uv0_dx * 11.0, uv0_dy * 11.0).g - 0.5) * 0.15 + 0.5;
                let qi = textureSampleGrad(t_diffuse, s_diffuse, in.uv0, uv0_dx, uv0_dy).g + 0.1;
                let ti = tint4.rgb / 12.0;
                // (the glare: the first slot is the lamp when it came first - car_key_col.w - else the hour's light)
                let lamp0 = g.car_key_col.w > 0.5;
                let hn = g.car_key2.w > 0.5;
                let l0d = select(select(g.sun_dir.xyz, car_kd(), hn), g.car_key.xyz, lamp0);
                let l0c = select(select(g.sun_color.rgb * g.sun_dir.w, car_kc(), hn), g.car_key_col.rgb, lamp0) * att12;
                let fi = 4.0 * clamp(-dot(v, l0d) - 0.8, 0.0, 1.0);
                lit = (ti + (0.5 - ti) * (nzi * qi / 11.0)) * di + l0c * (shi * fi * fi);
                let ai1 = clamp(nzi * qi / 1.1 + 0.5 * clamp(tint4.a - 0.3, 0.0, 1.0), 0.0, 1.0);
                a12 = ai1 * (2.0 - ai1);
            }
            // Cracked and broken windows (2026-10-20; PS_GlassFront 9 - 87 with VS_Glass 38 - 43, run in
            // numpy, work/glassdmg/py g4.py / g5.py; damage_map_rule.md "The windows' look"). s =
            // WindowDmgVal[zone].x - 0 whole, 1 cracked, 2 broken (damagemap::CarDamage::window, zones 0
            // - 3 front, rear, left, right; the body's dents[0]); c = sat(s), b = sat(s - 1):
            //   crack  = glass_test.dds at 1.5 x uv0, base level: rgb the crack's normal in the pane's
            //            frame n' = normalize((2r-1) T + (2g-1) B + (2b-1) N), B = cross(T, N) x w; a = A
            //   k      = 1 - b (1 - detail mask alpha): a broken pane keeps its shards only
            //   rgb   += 1.6 k c A sh x soft(reflect at n') + 6 c A lum(sh x soft(..)) (0.296, 0.431, 0.510)
            //            + 0.01 c A,  sh = 0.25 + sat(collector - 0.25), lum = (0.3, 0.59, 0.11)
            //   alpha x= k
            // (T the pane's stored tangent, carried in the window vertices' colour, B from it and the stored
            // sign. MCLA_DEBUG_VIEW=697: no damage drawn; 699: T from the derivatives, as before)
            // The side windows (2, 3) are cut below the car's clip plane (VS_Glass 70 - 80 -> o6, PS_GlassFront
            // 8 kill if o6 < 0; the plane in the car body's dents[4], vehicles.rs): a rolled-down pane sinks
            // into the door.
            if ((zone == 2 || zone == 3) && !int12 && dot(obj.dents[4].xyz, in.wpos) + obj.dents[4].w < 0.0) { discard; }
            let wz12 = clamp(zone, 0, 3);
            let ws12 = select(0.0, obj.dents[0][wz12], zone >= 0 && zone <= 3 && in.uv2.y < -0.5 && !int12 && !(g.params.w > 696.5 && g.params.w < 697.5));
            if (ws12 > 0.0) {
                let c12 = clamp(ws12, 0.0, 1.0);
                let b12 = clamp(ws12 - 1.0, 0.0, 1.0);
                let k12 = 1.0 + b12 * (glass_mask - 1.0);
                let ck = textureSampleLevel(t_second, s_diffuse, in.uv0 * 1.5, 0.0);
                let q2 = cross(tb_dp2, n);
                let q1 = cross(n, tb_dp1);
                let tq = q2 * tb_duv1.x + q1 * tb_duv2.x;
                // (the pane's stored tangent where the window carries it - colour alpha 1, vehicles.rs - else
                // the one the derivatives give)
                let ts12 = (obj.model * vec4<f32>(in.color.rgb * 2.0 - 1.0, 0.0)).xyz;
                let tn = select(tq * inverseSqrt(max(dot(tq, tq), 1e-12)), ts12 * inverseSqrt(max(dot(ts12, ts12), 1e-12)), in.color.a > 0.5 && !(g.params.w > 698.5 && g.params.w < 699.5));
                let bn = cross(tn, n) * ((1.0 - 2.0 * in.bflip) * select(-1.0, 1.0, dot(cross(obj.model[0].xyz, obj.model[1].xyz), obj.model[2].xyz) >= 0.0));
                let cn = normalize((ck.r * 2.0 - 1.0) * tn + (ck.g * 2.0 - 1.0) * bn + (ck.b * 2.0 - 1.0) * n);
                var fc12 = 127.0 / 255.0;
                if (g.tone.w < 0.5 && g.shadow_params.x > 0.0) {
                    fc12 = car_collector(in.wpos);
                } else if (g.tone.w > 0.5 && g.car_amb.w < 0.5) {
                    fc12 = 1.0;
                }
                let sh12 = 0.25 + clamp(fc12 * FC_LIT - 0.25, 0.0, 1.0);
                let ss12 = env_soft(reflect(-v, cn)) * sh12;
                let ca12 = c12 * ck.a;
                lit = lit + ss12 * (1.6 * k12 * ca12) + vec3<f32>(0.29608, 0.43137, 0.5098) * (6.0 * ca12 * dot(ss12, vec3<f32>(0.3, 0.59, 0.11))) + vec3<f32>(0.01 * ca12);
                a12 = a12 * k12;
            }
            alpha = select(a12, a12 * (2.0 - a12), lensg && dot(in.normal, v) >= 0.0);
            base = vec3<f32>(0.0);
            spec_k = 0.0;
        } else if (mat.mode == 12u && in.uv2.y > 0.5 && g.dome.w > 0.5 && g.ambient_ground.w > -0.5) {
            // (the lens' mirror: reflectFresnelMin 0.125 to reflectFresnelMax 0.875 by
            // (1 - n.v)^5, as recorded with it)
            lit = lit + env * (0.125 + 0.75 * pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 5.0));
            spec_k = 0.0;
        } else if (mat.mode == 12u && in.uv2.y > 0.5) {
            // lamp lenses: a sheen, the colour stays
            lit = lit + env * (f * 0.9 + 0.03);
            alpha = clamp(alpha + f * 0.5, 0.0, 1.0);
        } else if (mat.mode == 12u) {
            if (in.uv2.x > 0.5 && cp_on) {
                // Glass of traffic cars (every AmbientCarGlass mesh): VSAmbientGlass + PSAmbientGlassFront
                // of cars/0xe059357e (work/lighting/aglass.py, work/trafglass/tg_px.py - the program on
                // numbers against this, exact):
                //   rgb   = lerp(tint.rgb x screen(the car's two lights, 2 x ambient), surroundings, f)
                //   f     = Fmin + (Fmax - Fmin) (1 - |n.v|)^Fexp = 0.025 + 0.825 (1 - |n.v|)^4.5125
                //   alpha = sat(a0 + x - sat(a0 x)), x = a0 + (1 - a0 + lum - sat(lum)) f,
                //           a0 = lerp(t, 1, f), lum = (0.3, 0.59, 0.11) of the surroundings
                // gCarAmbTintColor c127 (sub_827914B8) = (0, 0, 0, 0.25 + 1.5 sat(min(d / 30, 1) - 0.5)),
                // d = the camera's distance from the car's origin: see-through within 15 m, opaque
                // from 30 m (recorded 0.25 .. 1). t = c127.w on zones 0 - 2 (the windows), 0 on zones 3
                // and up (the lamp lenses); the zone is the vertex's uv1.x (the program's texcoord z:
                // one half4 fetch over uv0 and uv1). Blended source alpha, both faces drawn.
                // (the damage term, GlassDmgSampler.w x 8 sat(o2.z), is dead: the fetch writes 0 in that slot)
                // (c127.w is the car's own, set once from the game's camera - vehicles.rs `AmbLamps::tint`, in
                // fade_size.w - in every pass; the map's ray and f at the vertices, interpolated - vs_body.
                // MCLA_DEBUG_VIEW 700: ray and f per pixel; 701: the tint from this pass's camera)
                let d12a = min(distance(obj.model[3].xyz, g.cam_pos.xyz) / 30.0, 1.0);
                let c127w = select(obj.fade_size.w, 0.25 + 1.5 * clamp(d12a - 0.5, 0.0, 1.0), g.params.w > 700.5 && g.params.w < 701.5);
                let t12a = select(0.0, c127w, trunc(in.uv1.x) < 3.0);
                let pv12a = !(g.params.w > 699.5 && g.params.w < 700.5);
                let f12a = select(0.025 + 0.825 * pow(1.0 - clamp(abs(dot(n, v)), 0.0, 1.0), 4.5125), in.fill.w, pv12a);
                let env12a = select(env, env_color(in.fill.xyz), pv12a);
                let l12a = dot(env12a, vec3<f32>(0.3, 0.59, 0.11));
                let a12a = t12a + (1.0 - t12a) * f12a;
                let x12a = a12a + (1.0 - a12a + l12a - clamp(l12a, 0.0, 1.0)) * f12a;
                lit = env12a * f12a;
                alpha = clamp(a12a + x12a - clamp(a12a * x12a, 0.0, 1.0), 0.0, 1.0);
                base = vec3<f32>(0.0);
                spec_k = 0.0;
            } else if (in.uv2.x > 0.5) {
                // windows of ambient traffic cars (no cabin behind them): dark and opaque
                lit = lit * 0.6 + env * (f * 0.45 + 0.04);
                alpha = 1.0;
            } else if (dot(in.normal, v) < 0.0) {
                // looking out from the cabin: nearly clear
                lit = lit * 0.25;
                alpha = 0.08;
            } else {
                // (a mirror at a glancing angle, dark and clear face-on - the glass shader's
                // reflection comes with the same kind of fresnel term as the paint's)
                lit = lit + env * (f * 1.6 + 0.035);
                alpha = clamp(alpha + f * 0.6, 0.0, 1.0);
            }
        } else if (mat.mode == 19u) {
            lit = lit * 0.6 + env * (f * 0.45 + 0.04);
        } else if (mat.mode == 13u) {
            lit = lit + env * f * 1.2;
        } else if (mat.mode == 2u) {
            lit = lit + env * f * (1.0 + spec) * 0.5;
            alpha = clamp(alpha + f, 0.0, 1.0);
        } else if (mat.mode == 18u) {
            // bright zones mirror the surroundings, the dark barrel hardly at all
            let metal = step(0.3, base.r);
            if (rim_mirror < 0.0) {
                lit = lit * mix(1.0, 0.45, metal) + env * mix(0.03, 0.42, metal) * occ * mix(0.6, 1.0, clamp(f * 3.0, 0.0, 1.0));
            } else {
                lit = lit * mix(1.0, 0.45, clamp(rim_mirror / 0.42, 0.0, 1.0)) + env * rim_mirror * occ * mix(0.6, 1.0, clamp(f * 3.0, 0.0, 1.0));
            }
        } else if (mat.mode == 9u || mat.mode == 17u) {
            // matte trim (cloth, leather, plastic): only a faint sheen
            lit = lit + env * f * 0.10;
        } else {
            lit = lit + env * f * (1.0 + spec) * 0.5;
        }
        lit = lit + g.sun_color.rgb * s * spec_k;
    }
    // (the road at night under the weather's keys is the game's own night program: it carries
    // its lamps, the first two head lamps, the key light and the fade with height itself.
    // On a wet road - m_RoadReflection not small - nine tenths of a puddle are the game's
    // planar reflection of the street: with that picture there (collector.w, refl.wgsl) the
    // program draws wet nights too (stormy 21:00 capture: 4.7 against 5.6 whole frame, 3.7
    // against 10.5 on the road). Without the picture the cars' map in its place lit the
    // puddles up, and wet nights keep the older reading (MCLA_DEBUG_VIEW 63; 64: the picture,
    // but the older reading on wet nights).)
    var roadn = false;
    var roadn_lamps = vec2<i32>(-1, -1);
    var roadn_cp = -1;
    // (interior roads - tunnels, car parks: MAP_WORK.md pass 7 - keep the interior form set above;
    // the outdoor road programs gave them the sky's ambient and, outside the shadow map's reach, the sun.
    // Diagnostics 66: as before, the outdoor programs on them too.)
    let road_out0 = !interior_obj || (g.params.w > 65.5 && g.params.w < 66.5);
    // (not in the cars' map by day - moon_dir.w > 0.5: its road is PSCityRoadEnv, the plain rule set
    // in `light_in` above, where the main view's PSCityRoad has the collector, the highlight and
    // the darkening by wetness. Diagnostics 210: the main view's program there too, as until 2026-10-14)
    let road_out = road_out0 && !(g.moon_dir.w > 0.5 && g.tone.w < 0.5 && !(g.params.w > 209.5 && g.params.w < 210.5));
    if (road_out && mat.mode == 4u && g.tone.w > 0.5 && g.tone.w < 1.5 && g.dome.w > 0.5 && (g.key_dir.w < 0.3 || (g.collector.w > 0.5 && !(g.params.w > 63.5 && g.params.w < 64.5))) && g.ambient_ground.w > -0.5 && !(g.params.w > 32.5 && g.params.w < 33.5) && !(g.params.w > 19.5 && g.params.w < 23.5)) {
        roadn = true;
        roadn_lamps = head_lamps();
        roadn_cp = cp_light();
        lit = road_night_program(in.wpos, ng, in.color, in.color1, vec4<f32>(in.uv0, in.uv1), in.uv2);
    } else if (road_out && mat.mode == 4u && g.tone.w < 0.5 && g.dome.w > 0.5 && g.ambient_ground.w > -0.5 && !(g.params.w > 32.5 && g.params.w < 33.5)) {
        // The road by day under the weather's keys, dry or wet: the game's own programs as they
        // are (the older reading of them below stays for the map view and as diagnostics 33)
        lit = road_program(in.wpos, ng, in.color, in.color1.a, vec4<f32>(in.uv0, in.uv1), in.uv2, shadow);
    } else if (road_out && mat.mode == 4u && g.tone.w < 0.5) {
        // PSCityRoad's specular: (n.h)^SpecularExponent x sat(4 n.l), its strength running from
        // SpecularIntensityNear face-on (DecalSpecularIntensity on the decals, by their alpha
        // squared) to SpecularIntensityFar at a glancing angle ((1 - n.v)^FresnelExponent),
        // masked by the texture's green, coloured SpecularColorMixer of the way from the light's
        // colour to the road's, cut by the cube of occlusion x sun
        // (the materials' values: exponent 8, 0.4 / 0.33 .. 6, mixer 0.7, fresnel 10)
        let nl0 = max(dot(n, g.sun_dir.xyz), 0.0);
        let fr = pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 10.0);
        let near = mix(0.4, 0.33, second.a * second.a);
        let strength = (near + (6.0 - near) * fr) * albedo.g;
        let cut = occ * shadow;
        // (The program's colour reads light + (light - grey + (decal - grey) x decal alpha) x mixer,
        // i.e. away from the road's colour; tried 2026-10-12 - the road at noon came out far too
        // bright (frame mean 59 64 57 against 43 48 44), so some other factor of this term is
        // still misread. Until the term is measured on the running game the older mix stays:
        // it matches at noon.)
        // (diagnostics 33: without this term)
        lit = lit + mix(g.sun_color.rgb, base, 0.7) * pow(ndh, 8.0) * clamp(4.0 * nl0, 0.0, 1.0) * strength * g.sun_dir.w * cut * cut * cut * select(1.0, 0.0, g.params.w > 32.5 && g.params.w < 33.5);
        // the road's own darkening by wetness: lit x (1 + w (lit^2 - 1)), w = sat(puddle x r + r / 3)
        // with r = m_RoadReflection (0.0948 in dry weather; the puddle map taken at its middle)
        let w = clamp(0.0948 * 0.5 + 0.0948 / 3.0, 0.0, 1.0);
        lit = lit * (vec3<f32>(1.0) + w * (lit * lit - vec3<f32>(1.0)));
    }
    if (mat.mode == 3u) {
        // PSCityWindowUpper. The texture's alpha masks the frame; the pane (alpha 0) shows
        // 0.4 of the room behind - WindowSampler, on the second UV set, at the material's
        // WindowNight between its OnTime and OffTime (18:00 and 06:00 throughout) and its
        // WindowDay (next to nothing) otherwise - and 0.6 of the reflection, tinted by
        // WindowTintColor and rising from 0.4 face-on to 1.5 at a glancing angle.
        let glass = 1.0 - albedo.a;
        // The FAR city's window models (CityWindowLOD of city/sc/0xa6e77868 - the towers a kilometre off;
        // world::mark_window adds 8 to uv2.y) have programs of their own, run on a capture's constants
        // against the rules below (work/lighting/tw_rule.py, 4000 points: day 9e-9, night 4e-5):
        //   day,   PSCityWindowLOD:  lit = wall x (ambient + light x sat(n.l) x 2 x collector) + room x WindowDay x (1 - alpha)
        //                            out = lerp(lit, mini sky x WindowTintColor, 0.4 x (1 - alpha))
        //   night, PSMultiLight b41d9672db9dacbd:
        //          out = H x (wall x (ambient x night alpha + 1.25 |w| V + key + head lamps + four lamps)
        //                     + room x WindowTintColor x WindowNight x (1 - alpha))
        // - the pane keeps 0.6 of the lit wall by day and gets NO reflection picture at night, the room at its
        // whole WindowNight (the near windows' programs: 0.8 of it + 0.2 of ReflectSphereNight). They were drawn
        // with the near windows' rules. MCLA_DEBUG_VIEW=461: as before.
        // (far, 2026-10-13: world::mark_window also packs 16 x SpecularExponent - see `fc_e3` below)
        let fc_e3 = floor(in.uv2.y / 16.0);
        let fc_y3 = in.uv2.y - 16.0 * fc_e3;
        let wlod3 = fc_y3 > 7.5 && !(g.params.w > 460.5 && g.params.w < 461.5);
        let wy3 = fc_y3 - select(0.0, 8.0, fc_y3 > 7.5);
        let rv = reflect(-v, n);
        let ti = u32(round(in.uv2.x));
        let tint = vec3<f32>(f32(ti & 127u), f32((ti >> 7u) & 127u), f32((ti >> 14u) & 127u)) / 127.0;
        let always = wy3 >= 1.99;
        let hour = g.sun_color.w;
        let on = always || hour >= 18.0 || hour < 6.0;
        let room = second.rgb * select(0.001, wy3 - select(0.0, 2.0, always), on);
        let refl = tint * env_color(rv) * (1.5 - 1.1 * clamp(dot(n, v), 0.0, 1.0));
        if (night_prog && g.dome.w > 0.5 && wlod3) {
            lit = lit + glass * tint * second.rgb * select(0.0, wy3 - select(0.0, 2.0, always), on);
        } else if (g.dome.w > 0.5 && g.ambient_ground.w > -0.5 && wlod3) {
            let sky3 = mini_sky(2.0 * (dot(n, v) + v.y * in.wpos.y * 0.00666667) * n.xz - v.xz);
            lit = mix(lit + room * glass, sky3 * tint, 0.4 * glass);
        } else if (night_prog && g.dome.w > 0.5) {
            // The night program of the windows (PSMultiLight of effect city/0xeaf6f707, run on
            // numbers - work/lighting/wincheck.py): the pane keeps the lamp-lit texture and gets
            // on top the room at a FIFTH of WindowNight, tinted by WindowTintColor, and next to
            // nothing of the reflection (0.03 of it) - not the day program's 0.4 / 0.6 mix.
            // CORRECTED 2026-10-14 (work/lighting/l3_bandn.py: the night draw's own vertex program
            // from the capture, 300 points): the weights above were taken with the point 800 m up,
            // where the height fade H is its floor of 0.25 - and the port fades by H again below.
            // The program, exact:
            //   pane = (1 - diffuse alpha) x H x tint x (0.8 x WindowNight x room
            //          + 0.2 x (1.5 - 1.1 n.v) x ReflectSphereNight.dds at (0.5 - 0.9 R.x, 0.5 - 0.47 R.y))
            // - its ReflectionSampler at night is that 512 x 512 picture (the batch's fourth texture),
            // not the live map, and its alpha does nothing. MCLA_DEBUG_VIEW=125: as it stood.
            if (g.params.w > 124.5 && g.params.w < 125.5) {
                lit = lit + glass * tint * (second.rgb * 0.2 * select(0.0, wy3 - select(0.0, 2.0, always), on) + env_color(rv) * 0.03);
            } else {
                let npic = xgp(textureSampleLevel(t_fourth, s_diffuse, vec2<f32>(0.5 - 0.9 * rv.x, 0.5 - 0.47 * rv.y), 0.0).rgb);
                lit = lit + glass * tint * (second.rgb * 0.8 * select(0.0, wy3 - select(0.0, 2.0, always), on) + npic * (0.2 * (1.5 - 1.1 * clamp(dot(n, v), 0.0, 1.0))));
            }
            // (far, 2026-10-13: the same program - PSMultiLight ecc609ef0fc7384e behind its vertex program d7361c8aba83c642,
            // draw 1432 of capture np1_23.5, run on the file's vertices of sector 0x9e71606e batch 50 and on 3000 points
            // among the lamps, work/lighting/fc_winn.py, fc_winhl.py - ADDS on the OPAQUE texels (the wall between the
            // panes is this material too) the walls' highlight sum, unoccluded:
            //   alpha x H x lerp(S, luminance(S) x diffuse, SpecularColorMixer),  S = 0.3125 |w| V + the four lamps'
            //   highlights at exponent 2 x SpecularExponent
            // - no SpecularIntensity, no mask, not tinted: the shop panes' term (mode 22 below). The baked part is exact
            // on numbers (1.000); the lamps' part equals the grime walls' (median 0.994). Mixer 0.7 in every one of the
            // city's 2,960 CityWindowUpper materials. It is most of the light of a tower's wall high up, where the height
            // fade leaves a quarter: the tower 480 m off at Grand Ave. read 18 13 11 against 27 23 22.
            // MCLA_DEBUG_VIEW=643: without, as before; 644: the term alone.)
            if (fc_e3 > 0.5 && g.tone.w < 1.5 && !(g.params.w > 642.5 && g.params.w < 643.5)) {
                var fc_s3 = in.color1.rgb * (0.3125 * abs(g.ambient_ground.x)) + grid_lamps_spec(in.wpos, rv, 2.0 * fc_e3);
                // (645: the baked light's part alone; 646: the lamps' part alone)
                if (g.params.w > 644.5 && g.params.w < 645.5) { fc_s3 = in.color1.rgb * (0.3125 * abs(g.ambient_ground.x)); }
                if (g.params.w > 645.5 && g.params.w < 646.5) { fc_s3 = grid_lamps_spec(in.wpos, rv, 2.0 * fc_e3); }
                let fc_h3 = albedo.a * mix(fc_s3, dot(fc_s3, vec3<f32>(0.2125, 0.7154, 0.0721)) * base, 0.7);
                lit = lit + fc_h3;
                if (g.params.w > 643.5 && g.params.w < 644.5) { lit = fc_h3; }
            }
        } else if (g.dome.w > 0.5 && g.ambient_ground.w > -0.5) {
            // By day under the weather's keys, PSCityWindowUpper run on numbers
            // (work/lighting/winday.py): the pane is WindowTintColor x the reflection x
            // (0.36 - 0.262 n.v) - 0.098 face-on, 0.336 at a glancing angle; the 0.9 - 0.66 n.v
            // that stood here was two and a half times that - and the reflection is the fixed
            // picture ReflectSphereDay.dds (the batch's third texture) laid by its alpha over
            // the live map, at (0.5 - 0.9 R.x, 0.5 - 0.47 R.y) like the shop panes'.
            let wrs = textureSampleLevel(t_third, s_diffuse, vec2<f32>(0.5 - 0.9 * rv.x, 0.5 - 0.47 * rv.y), 0.0);
            let wr = vec4<f32>(xgp(wrs.rgb), wrs.a);
            // (2026-10-14: the live map under the picture is the MINI SKY - sampler 0, 128 x 128, the
            // dome alone - not the surroundings: with the cube of the surroundings a pane that
            // faces houses went black, the curved band at Selma / Highland 13 15 13 against 25 34
            // 28. Coordinates and weight from the program run on the capture's draw,
            // work/lighting/l3_band.py: 0.36 - 0.264 sat(n.v), back faces 0.36. 125: the cube.)
            var live_w = mini_sky(2.0 * (dot(n, v) + v.y * in.wpos.y * 0.00666667) * n.xz - v.xz);
            if (g.params.w > 124.5 && g.params.w < 125.5) { live_w = env_color(rv); }
            // (2026-10-13, coverage item 8 - the office block of Hollywood Blvd., sector 0x7dc489e4 batch 57, draw 2442 of
            // np6_12: PSCityWindowUpper 302948a16bb81a69 with its vertex program on the draw's constants against
            //   (1 - alpha) x (0.6 tint x env x (1.5 - 1.1 sat(n.v)) + 0.4 room) x max(occ x 2 collector, 0.4)
            // - work/lighting/dg/tower2.py, 2000 points, 2e-8 (instructions 18 - 19, 34 - 37: the screen collector's
            // sample doubled, times the vertex occlusion, floored at 0.4, times the whole pane). The 0.36 - 0.262 n.v
            // here is that with the factor AT its floor - right for a pane in shadow (the gate's fronts, where it was
            // measured with a zero collector), 2.5 times too dark for a pane the collector calls lit: a sunlit front,
            // and EVERY front beyond the shadows' reach (the collector fades to lit at 82 .. 160 m), turned to the sun
            // or not. The block 200 m off read 50 60 57 against 57 71 72. MCLA_DEBUG_VIEW=595: the floor, as before.)
            let f3 = select(max(occ * shadow, 0.4), 0.4, g.params.w > 594.5 && g.params.w < 595.5) * 2.5;
            let pane = tint * mix(live_w, wr.rgb, wr.a) * (0.36 - 0.262 * clamp(dot(n, v), 0.0, 1.0)) * f3;
            // (diagnostics 582 - 588, door round: the lit wall under the pane, the pane's term, the picture, its alpha /
            // the diffuse alpha / n.v, the mini sky, the tint, the diffuse as sampled - plain values)
            if (g.params.w > 581.5 && g.params.w < 588.5) {
                var d58 = lit;
                if (g.params.w > 582.5) { d58 = pane; }
                if (g.params.w > 583.5) { d58 = wr.rgb; }
                if (g.params.w > 584.5) { d58 = vec3<f32>(wr.a, albedo.a, clamp(dot(n, v), 0.0, 1.0)); }
                if (g.params.w > 585.5) { d58 = live_w; }
                if (g.params.w > 586.5) { d58 = tint; }
                if (g.params.w > 587.5) { d58 = base; }
                return vec4<f32>(to_scene(d58), 1.0);
            }
            lit = mix(lit, pane + room * 0.4 * select(f3 * 0.4, 1.0, g.params.w > 594.5 && g.params.w < 595.5), glass);
            // (589 - 594: the result / 2, the room, the glass share x 0.4, the diffuse alpha x 0.4, the tint x 0.3, shadow and occlusion x 0.4)
            if (g.params.w > 588.5 && g.params.w < 594.5) {
                var d59 = lit * 0.5;
                if (g.params.w > 589.5) { d59 = room; }
                if (g.params.w > 590.5) { d59 = vec3<f32>(glass * 0.4); }
                if (g.params.w > 591.5) { d59 = vec3<f32>(albedo.a * 0.4); }
                if (g.params.w > 592.5) { d59 = tint * 0.3; }
                if (g.params.w > 593.5) { d59 = vec3<f32>(shadow * 0.4, occ * 0.4, in.color1.a * 0.4); }
                return vec4<f32>(to_scene(d59), 1.0);
            }
        } else {
            lit = mix(lit, refl * 0.6 + room * 0.4, glass);
        }
    } else if (mat.mode == 22u) {
        // PSCityWindowLower: shop fronts. The reflection is faint - 0.1 face-on to 0.2 at a
        // glancing angle, plus 0.035 - and all there is of the pane, which lets 0.6 of the
        // shop behind through; the frame takes OpaqueReflectionIntensity of it.
        let rv = reflect(-v, n);
        // (under the weather's keys what the pane mirrors is the game's fixed picture
        // `__WindowReflection` - ReflectSphereDay.dds, the batch's second texture - laid by its
        // alpha over the live map, looked up with the mirrored view ray at (0.5 - 0.9 R.x,
        // 0.5 - 0.47 R.y): PSCityWindowLower's coordinates, work/lighting/glassuv.py)
        let sphere_uv = vec2<f32>(0.5 - 0.9 * rv.x, 0.5 - 0.47 * rv.y);
        var pane_env = env_color(rv);
        if (g.dome.w > 0.5 && g.ambient_ground.w > -0.5) {
            // (2026-10-15: the "live map" is the MINI SKY here as well - PSCityWindowLower fe6e412d62d0c967,
            // instructions 17 - 22: sampler 0, the 128 x 128 8888 picture of the dome alone (the same
            // texture the upper windows' program reads), at 0.5 + 0.47 R.xz, then lerp to the fixed
            // picture by its alpha. The cube of the surroundings stood here: under the storm's and the
            // fog's keys its horizon is far brighter than their dome, a veil over every shop pane -
            // +10.8 / +6.3 on the left pane at stormy / foggy noon, +1 on clear days.
            // MCLA_DEBUG_VIEW=401: the cube as before.)
            // (mini sky round: the program's coordinate is 0.5 + 0.47 R'.xz, R' = 2 (n.v + v.y x height / 15) n - v -
            // the normal's y raised by the WORLD height / 15 (instr 5: r4.x = pos.y x 0.0666667 + n.y; 14, 16 - 18),
            // as PSPropGlass has it and the upper windows with / 150. MCLA_DEBUG_VIEW=664: the plain mirrored ray.)
            var r22 = 2.0 * (dot(n, v) + v.y * in.wpos.y * 0.0666667) * n.xz - v.xz;
            if ((g.params.w > 663.5 && g.params.w < 664.5) || textureLoad(t_minisky, vec2<i32>(0, 0), 0).a < 0.5) { r22 = rv.xz; }
            if (!(g.params.w > 400.5 && g.params.w < 401.5)) { pane_env = mini_sky(r22); }
            let wrs = textureSampleLevel(t_second, s_diffuse, sphere_uv, 0.0);
            let wr = vec4<f32>(xgp(wrs.rgb), wrs.a);
            pane_env = mix(pane_env, wr.rgb, wr.a);
        }
        let refl = pane_env * (0.2 - 0.1 * clamp(dot(n, v), 0.0, 1.0)) + vec3<f32>(0.035);
        let lit0_glass = lit;
        lit = mix(lit, refl, mix(1.0, in.uv2.x, albedo.a));
        // The clear glass's highlight (2026-10-15; PSCityWindowLower fe6e412d62d0c967 by hand, instructions
        // 10 - 13, 28, 38 - 50): gLightColor x 0.4 x sat(V . (2 (n.l) n - l))^12 x sat(4 n.l), fading to the
        // frames' own highlight by the diffuse alpha - the mirrored light ray against the view ray, NOT
        // times the shadow collector. What stood here was 0.4 x (n.h)^12 x the shadow: a lobe about four
        // times as wide that the sun's shadow hid on clear days and that lay as a grey veil over every
        // shop pane once the storm's / the fog's keys left the fronts unshadowed (left pane +10.8 at
        // stormy noon, +6.3 foggy; +1 clear). MCLA_DEBUG_VIEW=402: as before.
        if (g.dome.w > 0.5 && g.ambient_ground.w > -0.5 && g.tone.w < 0.5 && !(g.params.w > 401.5 && g.params.w < 402.5)) {
            let nl22 = clamp(dot(n, g.sun_dir.xyz), 0.0, 1.0);
            let vr22 = clamp(dot(v, n * (2.0 * nl22) - g.sun_dir.xyz), 0.0, 1.0);
            lit = lit + g.sun_color.rgb * g.sun_dir.w * 0.4 * pow(vr22, 12.0) * clamp(4.0 * nl22, 0.0, 1.0) * (1.0 - albedo.a);
        } else {
            lit = lit + g.sun_color.rgb * pow(ndh, 12.0) * 0.4 * g.sun_dir.w * shadow * (1.0 - albedo.a);
        }
        alpha = max(albedo.a, 0.4);
        if (g.tone.w > 0.5 && g.dome.w > 0.5 && g.ambient_ground.w > -0.5) {
            // At night (PSMultiLight of city/0x5ad2ac01, work/lighting/glassnight.py): the pane
            // is 0.7 opaque at least - max(0.7, diffuse alpha); the program writes that over 16,
            // its c27.x, for a render target that scales what is written by 16 - and where the
            // texture is clear its colour is a fifth of the reflection map alone. Checked on the
            // running game: with the pane's alpha forced to zero the shops behind come out 3.3
            // times as bright (1 / 0.3).
            // (at night the picture is ReflectSphereNight.dds - the draw's first sampler, 512 x 512;
            // the batch's third texture - and nothing of the live map)
            // (2026-10-14, the program's instructions 76 - 78 and 115: the picture is weighed by
            // 0.2 - 0.1 n.v as by day - not a flat fifth - and the program adds its literal
            // 0.02 where the day program has 0.035. MCLA_DEBUG_VIEW=110: the flat fifth, no 0.02)
            var pane_n = xgp(textureSampleLevel(t_third, s_diffuse, sphere_uv, 0.0).rgb) * (0.2 - 0.1 * clamp(dot(n, v), 0.0, 1.0)) + vec3<f32>(0.02);
            // (2026-10-15: panes whose STORED normal is zero - a whole shop front, sector 0x49b61d44 batch 288,
            // the game's own vertex data: fetch word 0 on all 88 vertices. The program normalises the normal in
            // the pixel stage, n x rsq(n.n), and a zero times anything is zero on this GPU: n = 0, so
            // n.v = 0 and the picture is looked up along the view ray itself, at (0.5 + 0.9 v.x,
            // 0.5 + 0.47 v.y) - not mirrored about the 'up' this renderer puts in a missing normal's place,
            // which read the picture 0.03 lower, on its lit shop row. MCLA_DEBUG_VIEW=331: as before)
            // (world.rs hands a missing normal on as 'up': a shop pane whose normal is straight up is one of these)
            if ((!(length(in.normal) >= 0.01) || in.normal.y > 0.999) && !(g.params.w > 330.5 && g.params.w < 331.5)) {
                pane_n = xgp(textureSampleLevel(t_third, s_diffuse, vec2<f32>(0.5 + 0.9 * v.x, 0.5 + 0.47 * v.y), 0.0).rgb) * 0.2 + vec3<f32>(0.02);
            }
            if (g.params.w > 109.5 && g.params.w < 110.5) { pane_n = xgp(textureSampleLevel(t_third, s_diffuse, sphere_uv, 0.0).rgb) * 0.2; }
            lit = mix(lit0_glass, pane_n, mix(1.0, in.uv2.x, albedo.a));
            // (2026-10-15, signs: the night pane program d9fbe5f27ae394bc, instructions 22 - 23, 123 - 159 read from
            // capture sg_L23: over that it ADDS the walls' highlight sum S = 0.3125 |w| V + the four lamps'
            // highlights at exponent 2 x SpecularExponent, coloured lerp(S, luminance(S) x diffuse,
            // SpecularColorMixer) - no SpecularIntensity, no mask, no occlusion - by the diffuse alpha: nothing on
            // clear glass, all of it on the opaque things drawn with this material (the posters and plates on the
            // station wall, the shop's light pool on the pavement). world::mark_window: uv2.y = 8 x exponent +
            // mixer. MCLA_DEBUG_VIEW=380: without, as before.)
            let e22 = floor(in.uv2.y) / 8.0;
            if (e22 > 0.5 && g.tone.w < 1.5 && !(g.params.w > 379.5 && g.params.w < 380.5)) {
                let s22 = in.color1.rgb * (0.3125 * abs(g.ambient_ground.x)) + grid_lamps_spec(in.wpos, reflect(-v, n), 2.0 * e22);
                lit = lit + albedo.a * mix(s22, dot(s22, vec3<f32>(0.2125, 0.7154, 0.0721)) * base, fract(in.uv2.y));
            }
            alpha = max(albedo.a, 0.7);
        }
    } else if (mat.mode == 5u) {
        // neon signs and billboards (PSCityNeon, PSCityBillboard): the sign's own colours
        // times its neon map - not the neon map by itself, which washed billboards out to
        // white - while the sign is switched on (neon: twice NeonIntensity; billboards: once)
        // The neon signs' highlight of the sun under the day program (PSCityNeon, 2026-10-12):
        //   + o4.w x SpecularTint x lerp(gLightColor, diffuse, SpecularColorMixer)
        // o4.w from the vertex program (`neon_hl`, see vs_main), NOT times the shadow collector
        // nor the occlusion, whether the sign is on or off - what washes a poster out that
        // mirrors a low sun towards the camera, shadowed or not. SpecularTint is white on every
        // CityNeon material of the city; the mixer rides in the first colour word (hundredths).
        // Whole program against the closed form: work/lighting/neonhl_verify.py, 5e-14.
        // (Diagnostics 90: without it, as it stood.)
        if (in.uv2.x > 0.5 && (u32(round(in.uv2.y)) & 4096u) != 0u && g.tone.w < 0.5 && g.dome.w > 0.5 && g.ambient_ground.w > -0.5 && !(g.params.w > 89.5 && g.params.w < 90.5)) {
            let hl_col = select(g.key_col.rgb, g.sun_color.rgb * g.sun_dir.w, g.sun_dir.w > 0.0);
            lit = lit + mix(hl_col, albedo.rgb, round(in.color.b * 255.0) / 100.0) * max(in.neon_hl, 0.0);
        }
        if (in.uv2.x < 0.5) {
            // (not a city sign: no clock)
            lit = lit + albedo.rgb * second.rgb * (0.4 + 2.6 * night);
        } else if (gy_bill_n) {
            // (a billboard under the night programs - see `gy_bill_n`: the lit part is the night sum above; its own
            // light comes after the fade with height, like the neon signs')
            if (sign_on && !(g.params.w > 561.5 && g.params.w < 562.5)) {
                let gy_em = albedo.rgb * second.rgb * (vec3<f32>(0.3) + 0.7 * second.rgb) * sign_gain;
                if (g.tone.w > 1.5) { lit = lit + gy_em; } else { neon_em = gy_em; }
            }
        } else if ((u32(round(in.uv2.y)) & 4096u) == 0u && g.dome.w > 0.5) {
            // A billboard under the weather's keys, the whole of VSCityBillboard / PSCityBillboard:
            //   diffuse x occ x (ambient + light x n.l x sun)
            //   + lerp(light, diffuse, 0.5) x 0.5 x (v . (2 (n.l) n - l))^25 x sat(4 n.l)
            //   + diffuse x neon map while the sign is on
            // Round the clock - no night program, no lamps, no baked night light; the highlight
            // is not cut by the shadow or the occlusion (a poster facing a low sun washes out).
            let day_l = g.sun_dir.w > 0.0;
            let l_dir = select(g.key_dir.xyz, g.sun_dir.xyz, day_l);
            let l_col = select(g.key_col.rgb, g.sun_color.rgb * g.sun_dir.w, day_l);
            let nl_b = clamp(dot(ng, l_dir), 0.0, 1.0);
            let hl = 0.5 * pow(clamp(dot(v, ng * (2.0 * nl_b) - l_dir), 0.0, 1.0), 25.0) * clamp(4.0 * nl_b, 0.0, 1.0);
            lit = albedo.rgb * occ * (g.ambient_sky.rgb + l_col * nl_b * shadow) + mix(l_col, albedo.rgb, 0.5) * hl;
            if (sign_on) { lit = lit + albedo.rgb * second.rgb * sign_gain; }
            // (diagnostics 42: n.l, the shadow term it no longer uses, the highlight)
            if (g.params.w > 41.5 && g.params.w < 42.5) { return vec4<f32>(to_scene(vec3<f32>(nl_b, shadow, hl)), 1.0); }
        } else if (sign_on) {
            // (PSCityBillboard / PSCityNeon: diffuse x neon map x the strength, added as it is)
            if (g.tone.w > 0.5 && g.dome.w > 0.5 && g.ambient_ground.w > -0.5 && (u32(round(in.uv2.y)) & 4096u) != 0u && !(g.params.w > 46.5 && g.params.w < 47.5)) {
                // (diagnostics 47: the straight rule at night as well)
                // Neon under the night programs (PSMultiLight of city/0x25a6120c with the night
                // draw's own vertex program, work/lighting/neon2.py): not straight in the map -
                // diffuse x m x (0.6 + 2.8 m) x NeonIntensity a channel (m the neon map): 1.0 at
                // a half, 3.4 at full, where the day program's 2 m gives 1.0 and 2.0.
                // (2026-10-14, work/lighting/og_neon.py - the whole program against
                //   (diffuse x occ x (ambient + 1.25 |w| V + key + head lamps + THREE lamps)
                //    + tint x SpecularIntensity x lerp(1, diffuse, mixer) x (0.3125 |w| V + the three lamps' highlights)) x H
                //   + diffuse x m (0.6 + 2.8 m) x NeonIntensity x on
                // on 4000 points, 1e-7: the sign's own light is NOT faded with height - it
                // was, here, with the rest. Diagnostics 83: faded as it stood.)
                let em_n = albedo.rgb * second.rgb * (vec3<f32>(0.3) + 1.4 * second.rgb) * sign_gain;
                if (g.tone.w > 1.5 || (g.params.w > 82.5 && g.params.w < 83.5)) { lit = lit + em_n; } else { neon_em = em_n; }
            } else if (g.params.w > 47.5 && g.params.w < 48.5 && g.tone.w > 0.5 && (u32(round(in.uv2.y)) & 4096u) != 0u) {
                // (diagnostics 48: the same curve on the neon map as stored, not gamma-decoded)
                let mraw = pow(second.rgb, vec3<f32>(1.0 / 2.2));
                lit = lit + albedo.rgb * mraw * (vec3<f32>(0.3) + 1.4 * mraw) * sign_gain;
            } else {
                lit = lit + albedo.rgb * second.rgb * sign_gain;
            }
        }
    } else if (mat.mode == 24u) {
        // PSCityStoreSign: while it is on, the night texture times NightIntensity lies over
        // the sign by its alpha (and fills in the sign's own)
        var a = albedo.a;
        if (sign_on && sign_gain > 0.0) {
            // (2026-10-15, the shop signs' night program 066fa48bb96323ba of capture p2a_23.5, instructions 145 - 150:
            // the lit sign is faded with height FIRST - x H, OcclusionInfo - and the night picture x NightIntensity
            // laid over that by its alpha, unfaded. It was faded with the rest here: the FOOD MART sign, 6 m above
            // the camera, lost a sixth of its light and most of its bloom. MCLA_DEBUG_VIEW=384: as before.)
            if (night_prog && g.tone.w < 1.5 && !(g.params.w > 383.5 && g.params.w < 384.5)) {
                store_n = vec4<f32>(second.rgb * sign_gain, second.a);
            } else {
                lit = mix(lit, second.rgb * sign_gain, second.a);
            }
            a = mix(a, second.a, second.a);
        }
        // Lamp glass by day (PSPropLampGlassTint from OffTime to OnTime; the props' flag 8192 -
        // LIGHTING_WORK.md "Lamps, round 2"): no texture at all, the program's tint 0.5 times
        // its doubled sum = (ambient + light x n.l x sun) + 2 x light x (v . r)^8 x sat(4 n.l)
        // x sun - the sky's blue ambient on a glass that faces the ground. Diagnostics 117:
        // the glass texture as before.
        if (!sign_on && (u32(round(in.uv2.y)) & 8192u) != 0u && !(g.params.w > 116.5 && g.params.w < 117.5)) {
            let lg_day = g.sun_dir.w > 0.0;
            let lg_dir = select(g.key_dir.xyz, g.sun_dir.xyz, lg_day);
            let lg_col = select(g.key_col.rgb, g.sun_color.rgb * g.sun_dir.w, lg_day);
            let lg_nl = clamp(dot(ng, lg_dir), 0.0, 1.0);
            let lg_sp = pow(clamp(dot(v, ng * (2.0 * lg_nl) - lg_dir), 0.0, 1.0), 8.0) * clamp(4.0 * lg_nl, 0.0, 1.0);
            lit = g.ambient_sky.rgb + lg_col * shadow * (lg_nl + 2.0 * lg_sp);
            a = 1.0;
        }
        if (a < 0.3) { discard; }
    } else if (mat.mode == 25u) {
        // PSCityNeonTint, 18:00 to 06:00: NightColor times NightIntensity, strongest face-on
        let hour = g.sun_color.w;
        if (hour >= 18.0 || hour < 6.0) {
            let b = u32(round(in.uv2.y));
            let nc = vec3<f32>(f32(b & 63u), f32((b >> 6u) & 63u), f32((b >> 12u) & 63u)) / 63.0;
            lit = nc * (f32((b >> 18u) & 31u) / 4.0) * clamp(dot(n, v), 0.0, 1.0);
        }
    }
    // (signs light themselves: the baked lamp light is not laid over them as well)
    // (the baked lamp light of the night vertex colours is a display-space factor like the
    // rest of the game's lighting: 2.2 in linear terms - bright under the lamps, dark between)
    var baked_night = in.color1.rgb * 1.2;
    // (measured against the game at the same spot at night: lit walls and pavements stand at
    // about half their daytime value, warm from the sodium lamps)
    if (g.ambient_ground.w > 0.0) { baked_night = pow(in.color1.rgb, vec3<f32>(1.3)) * vec3<f32>(0.72, 0.58, 0.43); }
    // (the night programs carry the baked light in their own sum, above)
    // (so do the interior programs, at every hour)
    if (night_prog || int_wall || (interior_obj && int_new)) { baked_night = vec3<f32>(0.0); }
    lit = lit + emissive + base * baked_night * night * select(1.0, 0.0, (mat.mode >= 6u && mat.mode < 20u) || (mat.mode == 5u && (sign_on || g.dome.w > 0.5)));
    // local lights (street lamps, headlights)
    var local = vec3<f32>(0.0);
    let nl = min(lights.count.x, 48u);
    for (var i = 0u; i < nl; i = i + 1u) {
        let lp = lights.pos_range[i];
        let d = lp.xyz - in.wpos;
        let dist = length(d);
        if (dist < lp.w) {
            let l = d / max(dist, 0.001);
            let fall = 1.0 - dist / lp.w;
            var att = fall * fall;
            let lc = lights.color[i];
            if (roadn && (i32(i) == roadn_lamps.x || i32(i) == roadn_lamps.y || i32(i) == roadn_cp)) { continue; }
            if (lc.w > 2.5) {
                // (the checkpoint's light, the night programs' fourth dynamic light: colour x
                // sat(40 / d squared) x n.l, all round - its cone scale 0 and bias 1, c95.w)
                local = local + lc.rgb * (clamp(lights.dir[i].w / max(dist * dist, 0.0001), 0.0, 1.0) * max(dot(n, l), 0.0));
                continue;
            }
            if (lc.w > 1.5) {
                // the original's dynamic lights (the player's head lamps at night, PSMultiLight):
                // colour x sat(500 / d squared) x sat(cos x 4.4696 - 2.872) x n.l
                let cone = clamp(dot(-l, lights.dir[i].xyz) * 4.4696 - 2.872, 0.0, 1.0);
                local = local + lc.rgb * (clamp(500.0 / max(dist * dist, 0.0001), 0.0, 1.0) * cone * max(dot(n, l), 0.0));
                continue;
            }
            if (lc.w > -0.5) {
                // spot light
                let c = dot(-l, lights.dir[i].xyz);
                att = att * select(smoothstep(lc.w, mix(lc.w, 1.0, 0.5), c), pow(smoothstep(lc.w - 0.06, 1.0, c), 1.5), g.ambient_ground.w > 0.0);
            }
            // (the world's night vertex colours already hold the street lamps' light: there
            // the lamps only add a little; headlights and everything that moves get them in full)
            var baked = select(1.0, select(0.35, 0.28, g.ambient_ground.w > 0.0), lc.w < -0.5 && (mat.mode < 6u || mat.mode >= 20u));
            // (other cars' headlights rake a car's paint rather than flood it)
            if (mat.mode >= 6u && mat.mode < 20u) { baked = select(0.5, 0.4, lc.w > -0.5) * select(1.0, 0.3, g.ambient_ground.w > 0.0); }
            // (a headlight beam is thrown along the road: the game shows it on the asphalt far
            // ahead, where a point light's grazing angle would leave next to nothing)
            let facing = select(max(dot(n, l), 0.0) * 0.92 + 0.08, max(dot(n, l), 0.0) * 0.45 + 0.55 * step(-0.2, dot(n, l)), lc.w > -0.5 && g.ambient_ground.w > 0.0);
            local = local + lc.rgb * att * baked * facing;
        }
    }
    // several lamps at once (a starting grid's headlights) add up softly instead of bleaching
    if (g.ambient_ground.w > 0.0) {
        // (display-space light again: a beam that doubles the light on dark asphalt shows as
        // twice the texture's screen value - in linear terms the light to the power 2.2 -
        // which is what makes headlights wash the road ahead in the game's night footage)
        let ll = dot(local, vec3<f32>(0.2126, 0.7152, 0.0722));
        // (cars excepted: paint under several lamps went garish)
        if (mat.mode < 6u || mat.mode >= 20u) { local = local * (1.0 + 0.4 * min(ll, 3.0)); }
        local = local / (1.0 + 0.2 * dot(local, vec3<f32>(0.3, 0.5, 0.2)));
    } else if (!night_prog && !((plant_leaf_n || plant_card_n) && !plant_old)) {
        // (the original's night programs add their lights up as they are - the plants' too)
        local = local / (1.0 + 1.1 * dot(local, vec3<f32>(0.3, 0.5, 0.2)));
    }
    // (the trunk program has no dynamic lights)
    if (plant_trunk_n) { local = vec3<f32>(0.0); }
    // (the cars' paint and chrome under the weather's keys are the game's programs as they are:
    // PS_CarPaint has its two light slots, PS_Chrome no light at all - no lamps laid over them)
    if ((mat.mode == 6u || mat.mode == 7u) && g.dome.w > 0.5 && g.ambient_ground.w > -0.5) { local = vec3<f32>(0.0); }
    // (the tile form of the interior programs takes the head lamps, like all its light, unoccluded)
    // (so do the outdoor metal / tile form and the terrain's: their head lamps come unoccluded)
    lit = lit + base * local * select(1.0, occ, night_prog && !int_tile && !tile_n && !terr_n);
    // (diagnostics 41: the local lights alone - the player's head lamps under the night programs)
    if (g.params.w > 40.5 && g.params.w < 41.5) { lit = base * local; }
    if (night_prog) {
        // OcclusionInfo: what lies ABOVE x (the camera's height + 5 m) darkens over 1 / y = 20 m
        // down to z (m_OcclusionFades.y, 0.25 at night) - the upper storeys fade into the night,
        // the street keeps its light. (PSMultiLight: 1 + (z - 1) x sat((height - x) x y). It
        // stood the other way round here until 2026-10-12: the listing it was read from printed
        // "x - height", the disassembler having dropped the operand's negate flag.)
        // Neon signs (see the rule at `neon_em`): the baked light's highlight, 0.3125 |w| V x
        // SpecularIntensity x lerp(1, diffuse, SpecularColorMixer) - the "diffuse x baked x 0.49
        // that the occlusion does not cut" of the earlier probe. The two constants differ per
        // material (0.33 / 0.7, 0.5 / 0.5, 0.7 / 0.25, 0.7 / 0.5 in one frame); since 2026-10-12
        // the neon batches carry them in their first colour word (world::mark_window: exponent,
        // intensity and mixer in hundredths), and the lamps' highlights of the same program
        // are added beside the baked one (work/lighting/og_neon.py: s / (2E - (2E - 1) s) per
        // lamp as on the walls; the program takes the cell's first THREE lamps, `grid_lamps_spec`
        // all four - as the port's diffuse sum does for the signs). (83, 90: never.)
        if (mat.mode == 5u && g.tone.w < 1.5 && in.uv2.x > 0.5 && !(g.params.w > 82.5 && g.params.w < 83.5) && !(g.params.w > 89.5 && g.params.w < 90.5)) {
            if ((u32(round(in.uv2.y)) & 4096u) != 0u) {
                var hl_n = in.color1.rgb * (0.3125 * abs(g.ambient_ground.x)) + grid_lamps_spec(in.wpos, reflect(-v, n), 2.0 * max(round(in.color.r * 255.0), 1.0));
                // (the first three lamps' highlights, as the diffuse sum above; 333: four)
                if (!(g.params.w > 332.5 && g.params.w < 333.5)) {
                    hl_n = in.color1.rgb * (0.3125 * abs(g.ambient_ground.x)) + interior_lamps(in.wpos, n, reflect(-v, n), 2.0 * max(round(in.color.r * 255.0), 1.0), 3)[1] * g.ambient_ground.x;
                }
                if (!(g.params.w > 330.5 && g.params.w < 331.5)) { lit = lit + mix(vec3<f32>(1.0), albedo.rgb, round(in.color.b * 255.0) / 100.0) * (round(in.color.g * 255.0) / 100.0) * hl_n; }
            }
        }
        // (billboards under the night programs - see `gy_bill_n`: the baked light's and the four lamps' highlights)
        if (gy_bill_n && g.tone.w < 1.5 && !(g.params.w > 560.5 && g.params.w < 561.5)) {
            let gy_hl = in.color1.rgb * (0.3125 * abs(g.ambient_ground.x)) + grid_lamps_spec(in.wpos, reflect(-v, n), 50.0);
            lit = lit + mix(vec3<f32>(1.0), albedo.rgb, 0.5) * 0.5 * gy_hl;
        }
        let high = clamp((in.wpos.y - g.ambient_ground.y) * 0.05, 0.0, 1.0);
        // (the interior tile form has no such term: its program does not read OcclusionInfo)
        // (nor has the outdoor metal / tile form)
        if (!roadn && !int_tile && !tile_n) { lit = lit * (1.0 + high * (g.ambient_ground.z - 1.0)); }
        // (the neon signs' own light is added after the fade - see `neon_em`)
        lit = lit + neon_em;
        // (the shop signs' night picture is laid over after the fade - see `store_n`)
        lit = mix(lit, store_n.rgb, store_n.a);
    }
    // (props at night: the same fade with height, see `prop_n`)
    if (prop_n) { lit = lit * (1.0 + clamp((in.wpos.y - g.ambient_ground.y) * 0.05, 0.0, 1.0) * (g.ambient_ground.z - 1.0)); }
    // Walkers at night: the characters' VSPedNight + PSMultiLight (characters/0xbd0c6109) know no
    // lamp grid, no sun and no highlight - diffuse x (gLightAmbient + four lanes), a lane
    // colour x sat(sat(strength / d^2) x sat(dot(to light, axis) / d + bias) x sat(n.l)). The lanes
    // are the walker's own (figlight.rs: the cell's lamps eased in, the head lamps / key light laid
    // over), rows 0..11 of the figure's object, marked 2 in row 0 of the second table. 750: as before.
    if (mat.mode == 9u && in.uv1.y > 76.5 && in.uv1.y < 77.5 && obj.lamp_b[0].w > 1.5 && !(g.params.w > 749.5 && g.params.w < 750.5)) {
        var fl = vec3<f32>(0.0);
        for (var k = 0; k < 4; k = k + 1) {
            let lp = obj.lamp_a[3 * k];
            let lc = obj.lamp_a[3 * k + 1];
            let la = obj.lamp_a[3 * k + 2];
            let d = lp.xyz - in.wpos;
            let r = inverseSqrt(max(dot(d, d), 1e-12));
            let dn = d * r;
            fl = fl + lc.rgb * clamp(clamp(lp.w * r * r, 0.0, 1.0) * clamp(dot(dn, la.xyz) + la.w, 0.0, 1.0) * clamp(dot(n, dn), 0.0, 1.0), 0.0, 1.0);
        }
        lit = albedo.rgb * (hemi + fl);
    }
    if (mat.mode == 23u) {
        // PSCityStoreInterior: no lighting at all - a quarter of the texture by day, and from
        // 18:00 to 06:00 the lit-up texture times the vertex colour, four times over (here
        // scaled to this renderer's night exposure)
        // (VSCityStoreInterior: on = sgt(hour, OnTime) or sgt(OffTime, hour) - at 18:00 sharp the
        // shop is still dark, capture hl_L_18. MCLA_DEBUG_VIEW=129: >= as before)
        let hour = g.sun_color.w;
        if (hour > 18.0 || hour < 6.0 || (hour == 18.0 && g.params.w > 128.5 && g.params.w < 129.5)) {
            lit = second.rgb * in.color.rgb * 0.8;
            // Under the weather's keys the program as it is (VSCityStoreInterior and its pixel
            // program): the lit-up texture x the vertex colour x 4 - these meshes carry one
            // colour stream, the second (warm, e.g. 231 170 98) - seen through the shop pane,
            // which passes 0.3 of it at night (mode 22). Factor 4 checked on the running game
            // (hour constant overridden: 4.0 - 4.2 in light on the red channel).
            if (g.dome.w > 0.5 && g.ambient_ground.w > -0.5) { lit = second.rgb * in.color1.rgb * 4.0; }
            // (diagnostics 44 / 45: the night colour x 4, the day colour x 4)
            if (g.params.w > 43.5 && g.params.w < 44.5) { lit = second.rgb * in.color1.rgb * 4.0; }
            if (g.params.w > 44.5 && g.params.w < 45.5) { lit = second.rgb * in.color.rgb * 4.0; }
            // (46: the interior black - what the pane in front of it adds)
            if (g.params.w > 45.5 && g.params.w < 46.5) { lit = vec3<f32>(0.0); }
        } else {
            lit = albedo.rgb * 0.25;
        }
    }
    // The traffic signal heads (2026-10-13, p2day): VSPropTrafficSignal / PSPropTrafficSignal (78e0461d4a101d4a /
    // a1c0937069f23026, the same pair at every hour) know no light at all - the vertex has a place and one uv, the
    // pixel program is rgb = sat(tex x (DimIntensity 0.1 + 10 x dot(SignalColor, bands))), the texture's ALPHA naming
    // the lamp: below 0.25 the red one (stored 22 / 255), 0.25 .. 0.5 yellow (74), 0.5 .. 0.75 green (139), each
    // band's share = min(top of the band - a, 0.25); 1 = housing, visors, back: a tenth of the picture, near black
    // by day and by night. The pedestrian faces are bands of the same alpha (a hand at 22 with a walker at 139, and
    // the other way round). The texture has one level. Here stood a lit material (ambient + sun + a highlight): the
    // visors seen from below shone sky-blue. MCLA_DEBUG_VIEW=540: as before.
    if (mat.mode == 16u && !(g.params.w > 539.5 && g.params.w < 540.5)) {
        let sg_t = textureSampleLevel(t_diffuse, s_diffuse, in.uv0, 0.0);
        let sg_rgb = select(sg_t.rgb, xenos_gamma(sg_t.rgb), vec3<bool>(xenos_gamma_on()));
        let sg_state = i32(round(in.uv1.x));
        let sg_col = vec3<f32>(f32(sg_state == 0), f32(sg_state == 1), f32(sg_state == 2));
        let sg_a = sg_t.a;
        let sg_b = vec3<f32>(clamp(0.25 - sg_a, 0.0, 0.25), select(0.0, clamp(0.5 - sg_a, 0.0, 0.25), sg_a >= 0.25), select(0.0, clamp(0.75 - sg_a, 0.0, 0.25), sg_a >= 0.5));
        lit = clamp(sg_rgb * (0.1 + 10.0 * dot(sg_col, sg_b)), vec3<f32>(0.0), vec3<f32>(1.0));
        // (541: the texture's alpha as a grey)
        if (g.params.w > 540.5 && g.params.w < 541.5) { lit = vec3<f32>(sg_a); }
    }
    // The sea: the game's own programs (water_program above) at every hour, in the main view and the
    // cars' map. (MCLA_WATER_OLD=1, world.rs: the generic blended rule as before; diagnostics 751:
    // without the reflection picture - the stand-in.)
    if (mat.mode == 27u || mat.mode == 28u) {
        lit = water_program(in);
    }
    // (diagnostics 220 - 232, walls, run with --no-post - the terms of the night sum as plain numbers,
    // work/lighting/gw_*.py: 220 a calibration colour, 221 occlusion / grime alpha / night alpha, 222 the
    // cell's lamps / 4, 223 the bumped normal, 224 the whole before the fog, 225 the base colour, 226 the
    // mesh normal, 227 the lamps' highlight / 4, 228 the specular terms, 230 the baked light x 8,
    // 231 the fog's share, 232 the fog colour x 4)
    if ((g.params.w > 219.5 && g.params.w < 243.5) || (g.params.w > 299.5 && g.params.w < 302.5) || (g.params.w > 320.5 && g.params.w < 329.5)) {
        let dk = i32(g.params.w + 0.5);
        var dv = vec3<f32>(floor(in.clip.x / 50.0) / 32.0);
        if (dk == 221) { dv = vec3<f32>(occ, in.color.a, in.color1.a); }
        if (dk == 222) { dv = grid_lamps(in.wpos, n) * 0.25; }
        if (dk == 223) { dv = n * 0.5 + 0.5; }
        if (dk == 300) { dv = dbg_t * 0.5 + 0.5; }
        if (dk == 301) { dv = dbg_b * 0.5 + 0.5; }
        if (dk == 302) { dv = vec3<f32>(in.bflip * 0.4, select(0.0, 0.4, hs_k != 0u), select(0.0, 0.4, dot(cross(dpdx(in.wpos), dpdy(in.wpos)), ng) >= 0.0)); }
        if (dk == 224) { dv = lit; }
        // (shopw 321 - 328: world position / 4, / 64 behind the panes; pane alpha, uv2.x, occlusion; color1; lit-up texture; stored normal; mode)
        if (dk == 321) { dv = fract(in.wpos / 4.0); }
        if (dk == 322) { dv = fract(in.wpos / 64.0); }
        if (dk == 323) { dv = vec3<f32>(albedo.a, in.uv2.x, in.color.a); }
        if (dk == 324) { dv = in.color1.rgb; }
        if (dk == 325) { dv = second.rgb; }
        if (dk == 326) { dv = in.normal * 0.5 + 0.5; }
        if (dk == 327) { dv = vec3<f32>(f32(mat.mode) / 32.0, in.color1.a, albedo.a); }
        if (dk == 328) { dv = in.color.rgb; }
        if (dk == 225) { dv = base; }
        if (dk == 226) { dv = ng * 0.5 + 0.5; }
        if (dk == 227) { dv = grid_lamps_spec(in.wpos, reflect(-v, n), 2.0 * max(wall_spec.x, 1.0)) * 0.25; }
        if (dk == 228) { dv = vec3<f32>(wall_spec.x / 255.0, wall_spec.y, wall_spec.z); }
        if (dk == 229) { dv = vec3<f32>(hs.r, hs.a, hs.g); }
        if (dk == 230) { dv = in.color1.rgb * 8.0; }
        if (dk == 231) { let d231 = length(in.wpos - g.cam_pos.xyz); dv = vec3<f32>(clamp(1.0 - exp(-(d231 - g.params.x) / max(g.params.y - g.params.x, 1.0)), 0.0, 1.0) * g.fog_color.a, g.fog_color.a, 0.0); }
        if (dk == 232) { dv = (g.fog_color.rgb + (mix(g.fog_east.rgb, g.fog_west.rgb, 0.5) - g.fog_color.rgb) * g.fog_east.w) * 4.0; }
        if (dk == 233) { dv = fract(in.wpos / 4.0); }
        if (dk == 234) { dv = fract(in.wpos / 64.0); }
        if (dk == 235) { dv = vec3<f32>(f32(mat.mode) / 32.0, clamp(in.uv2.x / 8.0, 0.0, 1.0), clamp(in.uv1.y / 128.0, 0.0, 1.0)); }
        if (dk == 236) { dv = hemi * 4.0; }
        if (dk == 237) { dv = light_in * 0.25; }
        if (dk >= 240 && dk <= 243) { dv = grid_lamp_one(in.wpos, n, dk - 240) * 0.25; }
        // (0.4 of the value: the decode is exact below 0.45 - grey ramp of view 220, work/lighting/gw_read.py)
        if (dk != 220) { dv = dv * 0.4; }
        // (the tone map's saturation step undone beforehand, so that the shot holds dv itself)
        let gr = max(g.grade.rgb, vec3<f32>(0.001));
        let gv = clamp(dv, vec3<f32>(0.0), vec3<f32>(0.985)) / gr;
        let gy = dot(gv, vec3<f32>(0.2125, 0.7154, 0.0721));
        return vec4<f32>(to_scene((vec3<f32>(gy) + (gv - vec3<f32>(gy)) / max(g.tone.z, 0.01)) * gr), 1.0);
    }
    // Fog, as the original's full-screen pass lays it over the scene (PSBlendedHeightFogInFogNoPFog):
    // 1 - exp(-(d - near) / (far - near)), at most the fog colour's alpha; the colour leans a
    // quarter of the way to the horizon's - east or west by where one looks
    let to_px = in.wpos - g.cam_pos.xyz;
    let dist = length(to_px);
    let fg = clamp(1.0 - exp(-(dist - g.params.x) / max(g.params.y - g.params.x, 1.0)), 0.0, 1.0);
    let horizon = mix(g.fog_east.rgb, g.fog_west.rgb, 0.5 + 0.5 * to_px.x / max(dist, 0.001));
    let fog_rgb = g.fog_color.rgb + (horizon - g.fog_color.rgb) * g.fog_east.w;
    // The scene target's ceiling (2026-10-15): the original draws into a 2_10_10_10 FLOAT surface (7e3:
    // largest value 31.875) with an exponent bias of 4 - the programs' output x 16 - so nothing a
    // program writes is stored above 31.875 / 16 = 1.992 a channel; the fog pass and the bloom read
    // that. The port's 16-bit float target kept a lit sign's 5 .. 16 and bloomed it into a halo
    // (the wall lamps of the shop fronts, the poster). Diagnostics 311: no ceiling, as before.
    if (!(g.params.w > 310.5 && g.params.w < 311.5)) { lit = min(lit, vec3<f32>(1.9921875)); }
    // (754, water work: no fog at all - beside an override of the original's fog pass, alpha 0)
    let col = mix(lit, fog_rgb, select(clamp(fg * g.fog_color.a, 0.0, 1.0), 0.0, g.params.w > 753.5 && g.params.w < 754.5));
    if (mat.mode != 2u && mat.mode != 12u && mat.mode != 22u) { alpha = 1.0; }
    return vec4<f32>(tonemap(col), alpha);
}
