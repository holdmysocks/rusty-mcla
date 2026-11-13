// The cockpit readouts (readouts.rs), from the original's FontDraw.fx (cars/0x8146788c): VS_Font / VS_Roller give the
// cell's uv (computed on the CPU here, readouts.rs `cell_uv`), PS_Font:
//   rgb = font.rgb x fontColor.rgb x (emIntColor + ambYPos + 1.33333 sat(collector - 0.25) x o1)
//   o1  = screen(screen(L1, L0), ambYPos) (VS_Font), Lk = gLightColor[k].rgb x sat(n.lk) x sat(1 - min(dk / range k, 1))
//         x -gLightColor[k].w, n = the strip matrix's z axis normalised, lk / dk the direction / distance to gLightPosDir[k]
//   a = font.a x fontColor.w (x gInvColorExpBias.x x gLightAmbient.w, 1)
// Exact against both programs translated from the recorded microcode (work/carparts2/fontprobe.py, work/intglass/cap/
// pov_night draws 824 - 826: 1e-9, and 300 cases with random lights round the strip 6.6e-9). The two lights and ambYPos
// are the car programs' own (city.wgsl: the hour's slot = the sun + car_kc, the lamp slot car_lc, both x 0.9 for the
// 100 m of the original's point lights; ambYPos = the player car's ambient, `hemi`), the collector the cars' (day / night).
struct Globals {
    view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    ambient_sky: vec4<f32>,
    ambient_ground: vec4<f32>,
    fog_color: vec4<f32>,    // a = the fog's most
    params: vec4<f32>,       // x = fog start, y = fog end
    inv_view_proj: mat4x4<f32>,
    // (the rest of city.wgsl's Globals up to the fog's horizon colours, read by `fog_at`)
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
    fog_east: vec4<f32>,     // rgb, w = lean towards the horizon colours
    fog_west: vec4<f32>,
    dome: vec4<f32>,
    car_key: vec4<f32>,
    car_key_col: vec4<f32>,
    key_dir: vec4<f32>,
    key_col: vec4<f32>,
    collector: vec4<f32>,
    car_key2: vec4<f32>,
    car_key2_col: vec4<f32>,
    car_amb: vec4<f32>,
};
@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(2) var t_shadow: texture_depth_2d;
@group(0) @binding(3) var s_shadow: sampler_comparison;
@group(0) @binding(6) var s_env: sampler;
@group(0) @binding(10) var t_fc: texture_2d<f32>;
@group(1) @binding(0) var t_font: texture_2d<f32>;
@group(1) @binding(1) var s_font: sampler;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) emissive: vec3<f32>,
    @location(3) wpos: vec3<f32>,
    @location(4) normal: vec3<f32>,
    @location(5) fill: vec4<f32>,
};

// (city.wgsl's sun_tap / fast_collector_at / night_collector, the cars' collector)
fn sun_tap(wpos: vec3<f32>) -> f32 {
    let p = g.light_vp * vec4<f32>(wpos, 1.0);
    let uv = p.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || p.z < 0.0 || p.z > 1.0) {
        return 1.0;
    }
    let bias = 0.05 * length(vec3<f32>(g.light_vp[0].z, g.light_vp[1].z, g.light_vp[2].z));
    return textureSampleCompareLevel(t_shadow, s_shadow, uv, p.z - bias);
}
fn car_collector(p: vec3<f32>) -> f32 {
    if ((g.params.w > 50.5 && g.params.w < 51.5) || g.collector.z < 0.5 || g.collector.z > 1.5) {
        return 0.25 * (sun_tap(p + vec3<f32>(-1.5, 0.0, -1.5)) + sun_tap(p + vec3<f32>(-1.5, 0.0, 0.5)) + sun_tap(p + vec3<f32>(0.5, 0.0, -1.5)) + sun_tap(p + vec3<f32>(0.5, 0.0, 0.5)));
    }
    let uv = (p.xz - g.collector.xy) * (1.0 / 256.0) + vec2<f32>(0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return 1.0;
    }
    return textureSampleLevel(t_fc, s_env, uv, 0.0).r;
}
fn night_collector(p: vec3<f32>) -> f32 {
    if (g.collector.z < 1.5 || (g.params.w > 774.5 && g.params.w < 775.5)) {
        return 1.0;
    }
    let uv = (p.xz - g.collector.xy) * (1.0 / 256.0) + vec2<f32>(0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return 1.0;
    }
    return textureSampleLevel(t_fc, s_env, uv, 0.0).r;
}
fn scr3(a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    return a + b - clamp(a * b, vec3<f32>(0.0), vec3<f32>(1.0));
}

@vertex
fn vs_main(@location(0) pos: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>, @location(3) emissive: vec3<f32>, @location(4) depth: f32, @location(5) normal: vec3<f32>, @location(6) fill: vec4<f32>) -> VOut {
    var o: VOut;
    o.clip = g.view_proj * vec4<f32>(pos, 1.0);
    // (render state 12, the in-car view's depth offset - readouts.rs set_depth_offset)
    o.clip.z = o.clip.z + depth * o.clip.w;
    o.uv = uv;
    o.color = color;
    o.emissive = emissive;
    o.wpos = pos;
    o.normal = normal;
    o.fill = fill;
    return o;
}

@fragment
fn fs_main(i: VOut) -> @location(0) vec4<f32> {
    let f = textureSample(t_font, s_font, i.uv);
    // ambYPos: the player car's (city.wgsl `hemi`: the sky's ambient + the fill map's, x the recorded 0.4; the garage's own)
    var amb = (g.ambient_sky.rgb + i.fill.rgb) * select(1.0, i.fill.w, i.fill.w > 0.0);
    if (g.car_amb.w > 0.5) { amb = g.car_amb.rgb; }
    // the two lights as the car programs take them (city.wgsl car_kd / car_kc / car_ld / car_lc for the player's car)
    let n = normalize(i.normal);
    var kd = select(g.key_dir.xyz, g.car_key2.xyz, g.car_key2.w > 0.5);
    var kc = select(g.key_col.rgb, g.car_key2_col.rgb, g.car_key2.w > 0.5);
    if (g.car_key2.w > 1.5) {
        kd = normalize(g.car_key2.xyz - i.wpos);
        kc = g.car_key2_col.rgb * (clamp(1.0 - distance(g.car_key2.xyz, i.wpos) * 0.001, 0.0, 1.0) / 0.9);
    }
    let att = select(0.9, 1.0, g.params.w > 98.5 && g.params.w < 99.5);
    let lh = (g.sun_color.rgb * (g.sun_dir.w * max(dot(n, g.sun_dir.xyz), 0.0)) + kc * max(dot(n, kd), 0.0)) * att;
    let ll = g.car_key_col.rgb * (max(dot(n, g.car_key.xyz), 0.0) * att);
    let o1 = scr3(scr3(ll, lh), amb);
    var fc = 127.0 / 255.0;
    if (g.tone.w < 0.5 && g.shadow_params.x > 0.0) {
        fc = car_collector(i.wpos);
    } else if (g.tone.w > 0.5 && g.car_amb.w < 0.5) {
        fc = night_collector(i.wpos);
    }
    let sc = clamp(fc - 0.25, 0.0, 1.0) * 1.33333;
    var light = i.emissive + amb + o1 * sc;
    // (MCLA_DEBUG_VIEW=837: the sky's ambient as before)
    if (g.params.w > 836.5 && g.params.w < 837.5) { light = i.emissive + g.ambient_sky.rgb; }
    return vec4<f32>(f.rgb * i.color.rgb * light, f.a * i.color.a);
}
