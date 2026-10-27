// The original's planar reflection picture for the wet road (ReflectionSampler, s4 of PSCityRoad
// and of the road effect's PSMultiLight): every frame the far city's sector models are drawn
// into a 256 x 256 target with the MAIN view's own matrix (c8 .. c11 of the pass's draws are
// those of the main view), mirrored in the vertex program, and the sky dome mirrored behind.
// Read from the recordings work/lighting/cap/rf12d / rf12w / rf21w (draws 27 .. 101):
//   - VSCityLODReflect / VSCityLODReflectNight (city/0xb6de4415; the window models' variant in
//     0xa226bcd4 does the same): the vertex's height y becomes  h - max(y - h, 0)  with h the
//     reflection plane's height (gReflectionParams.z, c43.z) held between the ground levels
//     packed in the far models' colour word (the lowest and the highest) - so each block is
//     mirrored about its own ground where that lies above the plane, and whatever lies under
//     the mirror is flattened onto it. The pixel program gets the normal and the place as they
//     were (unmirrored). Culling is turned round (register 0x2205 = 0x18006 against 0x18002).
//   - PSCityLOD by day: the day picture x (gLightAmbient + gLightColor x sat(n.l) x 2 x the
//     shadow collector's sample) - and the pass binds an 8 x 8 grey of 127 there: no shadows.
//   - PSMultiLight at night, the far city's (city.wgsl "The far city"): the day picture x
//     (gLightAmbient + the night picture x 1.25 x gPropNFLColor.w + the grid's lamps + the key
//     light) x the fade with height; the two head lamps are zeroed in this pass.
//   - vs_main + ps_main_cheap of the Skyhat (city/0x29901476) with the view mirrored (the dome
//     hangs under the horizon), its constants the main dome's.
//   - no fog (the game's fog is a pass over the finished frame), no blending, no alpha test.
// The target is 2:10:10:10 float with exponent bias 2, resolved to a 16:16:16:16 half-float
// texture: linear colours in the scene's own units (alpha comes out as 0.25, unused).
//   - PSCityWindowLOD (city/0xa226bcd4) by day for the four window models: see fs_refl_win.
// Not drawn here: the window models' night program (an unnamed vertex program + PSMultiLight of
// their effect - they get the walls' night rule without a night picture), the twelve additive
// light glow rectangles of the night pass (PSSeedRTNoZ; their places are vertex data the
// recordings do not hold), the 2 x multisampling.

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
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    sky_sunset: vec4<f32>,
    true_sun: vec4<f32>,
    moon_dir: vec4<f32>,
    light_vp: mat4x4<f32>,
    shadow_params: vec4<f32>,
    cloud_color: vec4<f32>,
    cloud_shade: vec4<f32>,
    tone: vec4<f32>,
    grade: vec4<f32>,
    fog_east: vec4<f32>,
    fog_west: vec4<f32>,
    dome: vec4<f32>,
    car_key: vec4<f32>,
    car_key_col: vec4<f32>,
    key_dir: vec4<f32>,
    key_col: vec4<f32>,
    collector: vec4<f32>,
};
@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(7) var t_lamp_grid: texture_2d<u32>;
@group(0) @binding(8) var t_lamp_data: texture_2d<f32>;
// a far model's batch: 0 the day picture (a window model's wall texture), 2 a window model's
// lit rooms (WindowSampler), 3 the night picture
@group(1) @binding(0) var t_a: texture_2d<f32>;
@group(0) @binding(16) var s_a: sampler;
@group(1) @binding(2) var t_b: texture_2d<f32>;
@group(1) @binding(3) var t_c: texture_2d<f32>;

struct Refl {
    plane: vec4<f32>,   // x = the reflection plane's height (c43.z), y = 1: the dome is there
};
struct Hat {
    vs: array<vec4<f32>, 12>,       // c64 c65 c66 c67 c68 c69 c72 c73 c75 c96 c97 c98
    ps: array<vec4<f32>, 24>,       // c64 .. c83, c100 .. c103 of the variant with the moon
    perlin: array<vec4<f32>, 10>,
    pad: array<vec4<f32>, 2>,
};
@group(2) @binding(0) var<uniform> rf: Refl;
@group(2) @binding(1) var<uniform> hat: Hat;
// the dome's Perlin pass target and its detail noise
@group(2) @binding(2) var t_perlin: texture_2d<f32>;
@group(2) @binding(3) var t_detail: texture_2d<f32>;
@group(2) @binding(4) var s_rep: sampler;

struct GeoOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) wpos: vec3<f32>,
    @location(1) n: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) mark: f32,
    @location(4) dbg: vec4<f32>,
    @location(5) uv1: vec2<f32>,
    @location(6) tint: f32,
};

@vertex
fn vs_refl(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv0: vec2<f32>, @location(3) uv1: vec2<f32>, @location(4) color: vec4<f32>, @location(8) uv2: vec2<f32>) -> GeoOut {
    var o: GeoOut;
    // The colour word (memory bytes a b c d; here color = b c d a): b whole metres + 29, c
    // 255ths - the lowest ground level; d x 32 m and a x 32 m lie above it (instructions 9 - 13).
    // The mirror's height: the plane held between the lowest and the highest (14 - 20).
    let low = 255.0 * color.x + color.y - 29.0;
    let high = low + 32.0 * (color.z + color.w);
    let h = clamp(rf.plane.x, low, high);
    // (21 - 23)
    let y = h - max(pos.y - h, 0.0);
    o.clip = g.view_proj * vec4<f32>(pos.x, y, pos.z, 1.0);
    o.wpos = pos;
    // (normalised: the length carries the binormal's sign on bump-mapped walls, world::mark_binormal)
    o.n = normal * inverseSqrt(max(dot(normal, normal), 1e-12));
    o.uv = uv0;
    o.mark = uv2.y;
    o.uv1 = uv1;
    o.tint = uv2.x;
    o.dbg = vec4<f32>(low, high, h, pos.y);
    return o;
}

// The city's lamps at night (city.wgsl `grid_lamps_by`).
fn grid_lamps(wpos: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let uv = wpos.xz * vec2<f32>(1.0 / 6320.0, 1.0 / 4463.75) + vec2<f32>(0.54857594, 0.5282554);
    if (uv.x < 0.0 || uv.x >= 1.0 || uv.y < 0.0 || uv.y >= 1.0) {
        return vec3<f32>(0.0);
    }
    let cell = textureLoad(t_lamp_grid, vec2<i32>(uv * vec2<f32>(512.0, 640.0)), 0);
    var sum = vec3<f32>(0.0);
    for (var k = 0; k < lamp_count; k = k + 1) {
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
    return sum * 3.0 * g.ambient_ground.x;
}
// (how many of the cell's lamps a program takes: four, but the far city's PSMultiLight three)
var<private> lamp_count: i32 = 4;

// The night picture's decoding (city.wgsl `srgb_decode` / `pwl_decode`).
fn srgb_decode(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(pow((x + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)), x / 12.92, x <= vec3<f32>(0.04045));
}
fn pwl_decode(c: vec3<f32>) -> vec3<f32> {
    let e = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0;
    let k = select(select(select(vec3<f32>(1.0), vec3<f32>(2.0), e >= vec3<f32>(64.0)), vec3<f32>(4.0), e >= vec3<f32>(96.0)), vec3<f32>(8.0), e >= vec3<f32>(192.0));
    let off = select(select(select(vec3<f32>(0.0), vec3<f32>(-64.0), e >= vec3<f32>(64.0)), vec3<f32>(-256.0), e >= vec3<f32>(96.0)), vec3<f32>(-1024.0), e >= vec3<f32>(192.0));
    var l = e * k + off;
    l = l + trunc(l * k / 1024.0);
    return l / 1023.0;
}

@fragment
fn fs_refl(in: GeoOut) -> @location(0) vec4<f32> {
    var tex = textureSample(t_a, s_a, in.uv);
    let night_pic = textureSample(t_c, s_a, in.uv).rgb;
    // The far city's two pictures are decoded the console's piecewise way, not by the sRGB
    // curve (2026-10-14, work/lighting/og_lodfit.py against the original's dump of this very
    // picture at stormy 21:00: level 1.01 / 1.00 / 1.00 of the original's on 28,751 far city
    // pixels, 0.83 / 0.79 / 0.76 with sRGB; by day the original stood 1.4 - 1.8 times the port
    // in the dark half and level in the bright end - the same curve's mark). The day picture
    // comes sRGB-decoded from the sampler: encoded again, then decoded that way.
    // Diagnostics 84: as it stood (sRGB day picture, night picture ^ 2.2, four lamps).
    let old_lod = g.params.w > 83.5 && g.params.w < 84.5;
    let far_mark = (u32(round(in.mark)) & 2097152u) != 0u;
    if (far_mark && !old_lod && !(g.params.w > 85.5 && g.params.w < 86.5)) {
        let lin = clamp(tex.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
        tex = vec4<f32>(pwl_decode(select(1.055 * pow(lin, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055), lin * 12.92, lin <= vec3<f32>(0.0031308))), tex.a);
    }
    var light: vec3<f32>;
    if (g.tone.w > 0.5) {
        // PSMultiLight (the head lamps' columns of c84 .. c93 are zero in this pass)
        // (2026-10-14, work/lighting/og_lod.py: the same pixel program as the main view's far
        // city, the rule against it 2.7e-15 - the cell's first THREE lamps, |gPropNFLColor.w|,
        // both pictures gamma-flagged in their fetch constants. Diagnostics 88: the night
        // picture by the sRGB curve; 85 / 86 / 87: the night picture as stored, the day picture
        // as sampled (alpha: the fade with height), the light without the night picture's
        // share - for measuring against the original's dump.)
        if (far_mark && !old_lod) { lamp_count = 3; }
        light = g.ambient_sky.rgb + grid_lamps(in.wpos, in.n) + g.key_col.rgb * clamp(dot(in.n, g.key_dir.xyz), 0.0, 1.0);
        lamp_count = 4;
        if (far_mark && !(g.params.w > 86.5 && g.params.w < 87.5)) {
            var np = pwl_decode(night_pic);
            if (old_lod) { np = pow(night_pic, vec3<f32>(2.2)); }
            if (g.params.w > 87.5 && g.params.w < 88.5) { np = srgb_decode(night_pic); }
            light = light + np * (1.25 * abs(g.ambient_ground.x));
        }
        if (g.params.w > 84.5 && g.params.w < 85.5) { return vec4<f32>(night_pic, select(0.0, 1.0, far_mark)); }
        // (c56: OcclusionInfo - the fade with height above the camera)
        let h_fade = 1.0 + clamp((in.wpos.y - g.ambient_ground.y) * 0.05, 0.0, 1.0) * (g.ambient_ground.z - 1.0);
        light = light * h_fade;
        // (86: the fade in the alpha; 87: the light alone, no texture)
        if (g.params.w > 85.5 && g.params.w < 86.5) { return vec4<f32>(tex.rgb, h_fade); }
        if (g.params.w > 86.5 && g.params.w < 87.5) { return vec4<f32>(light, select(0.0, 1.0, far_mark)); }
    } else {
        // PSCityLOD (twice the 8 x 8 grey of 127 bound as the shadow collector)
        light = g.ambient_sky.rgb + g.sun_color.rgb * g.sun_dir.w * (clamp(dot(in.n, g.sun_dir.xyz), 0.0, 1.0) * (254.0 / 255.0));
    }
    // (diagnostics, MCLA_DEBUG_VIEW 65: the lowest ground level, the highest, the mirror's
    // height and the vertex's own, each less the plane's, in the picture's four channels)
    if (g.params.w > 64.5 && g.params.w < 65.5) {
        return in.dbg - vec4<f32>(rf.plane.x);
    }
    return vec4<f32>(max(tex.rgb * light, vec3<f32>(0.0)), 1.0);
}

// ---- the dome ----
struct XsVsOut {
    pos: vec4<f32>,
    o: array<vec4<f32>, 16>,
};
var<private> xs_c: array<vec4<f32>, 256>;
// (the main dome program's samplers - xs_sky.wgsl holds it too; not run here)
fn xs_s0(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(t_perlin, s_rep, uv, 0.0);
}
fn xs_s1(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(t_perlin, s_rep, uv, 0.0);
}
fn xs_s2(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(t_perlin, s_rep, uv, 0.0);
}
fn xs_s3(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(t_detail, s_rep, uv, 0.0);
}
// ps_main_cheap: PerlinNoiseSampler, HighDetailNoiseSampler
fn xs_cheap_s0(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(t_perlin, s_rep, uv, 0.0);
}
fn xs_cheap_s1(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(t_detail, s_rep, uv, 0.0);
}

// vs_main + ps_main_cheap on the dome mesh's point at `rho` of its rim's radius (0 the top, 1
// the rim) towards `flat_dir` (x, z): sky_hat.wgsl `hat_vs` from the mesh point on - half an
// ellipsoid 200 in radius and 163.67 high, its texture coordinates flat over it with the
// lowest five rings pulled out to the texture's edge.
fn dome(rho_in: f32, flat_dir: vec2<f32>) -> vec3<f32> {
    let rho = clamp(rho_in, 0.0, 1.0);
    let m = vec3<f32>(200.0 * rho * flat_dir.x, 163.67 * sqrt(max(1.0 - rho * rho, 0.0)), 200.0 * rho * flat_dir.y);
    var uvr = 0.888 * rho;
    if (rho > 0.8819) {
        var ring = array<vec2<f32>, 6>(vec2<f32>(0.8819, 0.7828), vec2<f32>(0.9239, 0.8289), vec2<f32>(0.9569, 0.8709), vec2<f32>(0.9808, 0.9116), vec2<f32>(0.9952, 0.9534), vec2<f32>(1.0, 1.0));
        for (var i = 0; i < 5; i = i + 1) {
            if (rho >= ring[i].x) {
                uvr = mix(ring[i].y, ring[i + 1].y, clamp((rho - ring[i].x) / (ring[i + 1].x - ring[i].x), 0.0, 1.0));
            }
        }
    }
    var vin: array<vec4<f32>, 16>;
    vin[0] = vec4<f32>(m, 1.0);
    vin[1] = vec4<f32>(0.5 + 0.5 * uvr * flat_dir.x, 0.5 - 0.5 * uvr * flat_dir.y, 0.0, 0.0);
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
    let vs = xs_sky_vs(vin);
    // ps_main_cheap's c64 .. c78 out of the constants of the variant with the moon (hat.ps:
    // c64 .. c83, c100 .. c103): SunCentre, SunDirection, SunColor, CloudColor (68), SunsetColor
    // (69), CloudThreshold, CloudBias, CloudFadeOut, TopCloudBias.. (73), TopCloudColor,
    // CloudShadowStrength (75), CloudThickness.. (77), SunSize (81), HDRExposure (102),
    // HDRSunExposure (103) - the same values in the recorded pass
    var slot = array<i32, 15>(0, 1, 2, 4, 5, 6, 7, 8, 9, 10, 11, 13, 17, 22, 23);
    for (var i = 0; i < 15; i = i + 1) {
        xs_c[64 + i] = hat.ps[slot[i]];
    }
    return max(xs_sky_cheap_ps(vs.o).rgb, vec3<f32>(0.0));
}

// The window models by day, PSCityWindowLOD:
//   lit = wall texture x (gLightAmbient + gLightColor x sat(n.l) x 2 x collector) + the lit
//         rooms (WindowSampler on the second coordinates) x WindowDay x (1 - the wall's alpha)
//   out = lit + 0.4 x (1 - alpha) x (SkyReflectionSampler x WindowTintColor - lit)
// The sky picture (a 128 x 128 map the game draws each frame with vs_MiniSky / ps_miniSky) is
// read at 0.5 + 0.47 x R.xz, R the view ray mirrored at the wall with the normal's y raised by
// the height / 150. That map is not made here: the dome's own program stands in, run on the
// mesh point whose flat texture coordinates lie that far out (the map taken to be the dome
// seen in its texture's layout - not read off the mini sky's programs).
// WindowDay is 0 on the recorded models (the rooms are dark by day): left out.
@fragment
fn fs_refl_win(in: GeoOut) -> @location(0) vec4<f32> {
    let tex = textureSample(t_a, s_a, in.uv);
    if (g.tone.w > 0.5) {
        var light = g.ambient_sky.rgb + grid_lamps(in.wpos, in.n) + g.key_col.rgb * clamp(dot(in.n, g.key_dir.xyz), 0.0, 1.0);
        light = light * (1.0 + clamp((in.wpos.y - g.ambient_ground.y) * 0.05, 0.0, 1.0) * (g.ambient_ground.z - 1.0));
        return vec4<f32>(max(tex.rgb * light, vec3<f32>(0.0)), 1.0);
    }
    let n = normalize(in.n);
    let lit = tex.rgb * (g.ambient_sky.rgb + g.sun_color.rgb * g.sun_dir.w * (clamp(dot(n, g.sun_dir.xyz), 0.0, 1.0) * (254.0 / 255.0)));
    var pane = lit;
    if (rf.plane.y > 0.5) {
        let v = normalize(g.cam_pos.xyz - in.wpos);
        // (instructions 11 - 14)
        let k = 2.0 * (dot(v, n) + v.y * in.wpos.y * 0.00666667);
        let r = k * n.xz - v.xz;
        let ti = u32(round(in.tint));
        let tint = vec3<f32>(f32(ti & 127u), f32((ti >> 7u) & 127u), f32((ti >> 14u) & 127u)) / 127.0;
        let rl = length(r);
        // (0.47 x |R.xz| in the map against the dome's flat coordinate 0.5 x uvr, uvr = 0.888 rho)
        let sky = dome(0.94 * rl / 0.888, r / max(rl, 0.0001));
        pane = sky * tint;
    }
    return vec4<f32>(max(mix(lit, pane, 0.4 * (1.0 - tex.a)), vec3<f32>(0.0)), 1.0);
}

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_refl_sky(@builtin(vertex_index) i: u32) -> SkyOut {
    var o: SkyOut;
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    o.clip = vec4<f32>(p, 0.0, 1.0);
    o.ndc = p;
    return o;
}

@fragment
fn fs_refl_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let p0 = g.inv_view_proj * vec4<f32>(in.ndc, 0.5, 1.0);
    var d = normalize(p0.xyz / p0.w - g.cam_pos.xyz);
    // the pass's view is the main one mirrored (its gViewInverse has the camera's up turned
    // round): a ray that goes down meets the dome where its mirror image goes up; what goes up
    // meets the dome's rim
    d.y = max(-d.y, 0.0);
    // the dome point the ray meets (sky_hat.wgsl `hat_vs`)
    let q = vec3<f32>(d.x * 0.2, d.y, d.z * 0.2);
    let m = q * inverseSqrt((q.x * q.x + q.z * q.z) / 40000.0 + q.y * q.y / 26787.9);
    return vec4<f32>(dome(length(m.xz) / 200.0, m.xz / max(length(m.xz), 0.0001)), 1.0);
}

// The rear-view mirror's sky (rearview.rs; the original's mirror pass draws the dome with vs_main +
// ps_main_cheap, recorded in work/lighting2/rear/cap/rv1, st1, nt1): the dome seen from the mirror's own
// camera, which the mirror's globals hold - not turned upside down as the road's picture is. A ray under
// the horizon meets the dome's rim (the city covers those pixels). Written at the far depth with the
// depth test off: it also clears the strip's depth before the city is drawn into it.
@fragment
fn fs_mirror_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let p0 = g.inv_view_proj * vec4<f32>(in.ndc, 0.5, 1.0);
    var d = normalize(p0.xyz / p0.w - g.cam_pos.xyz);
    d.y = max(d.y, 0.0);
    let q = vec3<f32>(d.x * 0.2, d.y, d.z * 0.2);
    let m = q * inverseSqrt((q.x * q.x + q.z * q.z) / 40000.0 + q.y * q.y / 26787.9);
    return vec4<f32>(dome(length(m.xz) / 200.0, m.xz / max(length(m.xz), 0.0001)), 1.0);
}
