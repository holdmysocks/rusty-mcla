// Sky dome: fullscreen triangle shaded from the game's sky keyframes
// (zenith / azimuth / sunset colours, sun vector) with the original star field and moon.

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
    sky_zenith: vec4<f32>,   // rgb, a = day factor
    sky_horizon: vec4<f32>,  // rgb, a = star brightness
    sky_sunset: vec4<f32>,   // rgb, a = sunset strength
    true_sun: vec4<f32>,     // xyz towards the real sun, w = unused
    moon_dir: vec4<f32>,
    light_vp: mat4x4<f32>,
    shadow_params: vec4<f32>,
    cloud_color: vec4<f32>,  // lit colour, a = coverage
    cloud_shade: vec4<f32>,  // shaded colour, a = drift
    tone: vec4<f32>,
    grade: vec4<f32>,
    fog_east: vec4<f32>,
    fog_west: vec4<f32>,
    dome: vec4<f32>,         // x = AzimuthStrength, y = AzimuthHeight, z = HDRExposure, w = 1: the dome's own keys
};
@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var t_stars: texture_2d<f32>;
@group(0) @binding(16) var s_lin: sampler;
@group(1) @binding(2) var t_moon: texture_2d<f32>;
@group(1) @binding(3) var t_noise: texture_2d<f32>;

// What the tone map (post.wgsl) will turn into this display colour: its colour correction
// undone, then its curve on the luminance, then its exposure.
fn to_scene(d: vec3<f32>) -> vec3<f32> {
    let v = d / max(g.grade.rgb, vec3<f32>(0.001));
    let y = clamp(dot(v, vec3<f32>(0.2125, 0.7154, 0.0721)), 0.0, 0.985);
    let w = max(g.tone.y, 0.001);
    let l = 0.5 * w * (sqrt((1.0 - y) * (1.0 - y) + 4.0 * y / w) - (1.0 - y));
    return v * (l / max(y, 0.0001)) / max(g.tone.x, 0.001);
}

fn cloud_noise(uv: vec2<f32>) -> f32 {
    // three octaves of the dome's noise map, drifting at different rates
    let dr = g.cloud_shade.a;
    let a = textureSampleLevel(t_noise, s_lin, uv * 0.11 + vec2<f32>(dr, dr * 0.35), 0.0).g;
    let b = textureSampleLevel(t_noise, s_lin, uv * 0.31 + vec2<f32>(-dr * 0.6, dr * 1.4), 0.0).g;
    let c = textureSampleLevel(t_noise, s_lin, uv * 0.97 + vec2<f32>(dr * 2.0, -dr), 0.0).g;
    return a * 0.58 + b * 0.30 + c * 0.12;
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) i: u32) -> VsOut {
    var o: VsOut;
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    o.clip = vec4<f32>(p, 0.0, 1.0); // reverse-Z: 0 = far plane
    o.ndc = p;
    return o;
}

fn sky_color(d: vec3<f32>) -> vec3<f32> {
    let up = clamp(d.y, -1.0, 1.0);
    let sun = normalize(g.true_sun.xyz);
    let h = pow(1.0 - max(up, 0.0), 6.0);
    var c = mix(g.sky_zenith.rgb, g.sky_horizon.rgb, h);
    // sunset glow around the sun's azimuth, strongest at the horizon
    let flat_d = normalize(vec3<f32>(d.x, 0.0, d.z) + vec3<f32>(0.0, 0.0, 1e-5));
    let flat_s = normalize(vec3<f32>(sun.x, 0.0, sun.z) + vec3<f32>(0.0, 0.0, 1e-5));
    // the horizon on the sun's side has its own colour (smog glow, sunset)
    // (a: 0.55 with the sun high, 1 at sunrise / sunset, when the glow is wide and tall)
    let low = clamp((g.sky_sunset.a - 0.55) / 0.45, 0.0, 1.0);
    let toward = pow(max(dot(flat_d, flat_s), 0.0), mix(3.0, 1.2, low)) * mix(0.45, 1.0, low);
    let h2 = pow(1.0 - max(up, 0.0), mix(7.0, 2.5, low));
    c = mix(c, g.sky_sunset.rgb, toward * h2);
    // below the horizon: darken towards the ground haze
    c = mix(c, g.sky_horizon.rgb * 0.55, smoothstep(0.0, -0.25, up));
    return c;
}

// The dome's colour as the game's Skyhat vertex program has it (keys as they are, linear):
// the sky colour, giving way to the azimuth colour - west to east by the direction's x - by
// AzimuthStrength x (1 - sat(height x AzimuthHeight)); times HDRExposure.
fn dome_color(d: vec3<f32>) -> vec3<f32> {
    let az = mix(g.sky_sunset.rgb, g.sky_horizon.rgb, d.x * 0.5 + 0.5);
    let k = g.dome.x * (1.0 - clamp(d.y * g.dome.y, 0.0, 1.0));
    return (g.sky_zenith.rgb + (az - g.sky_zenith.rgb) * k) * g.dome.z;
}

@fragment
fn fs_sky(in: VsOut) -> @location(0) vec4<f32> {
    let p0 = g.inv_view_proj * vec4<f32>(in.ndc, 0.5, 1.0);
    let d = normalize(p0.xyz / p0.w - g.cam_pos.xyz);
    if (g.dome.w > 0.5) {
        var h = dome_color(d);
        // (clouds, sun and stars are still this port's own, laid over the game's dome)
        let sun0 = normalize(g.true_sun.xyz);
        if (d.y > 0.015 && g.cloud_color.a > 0.01) {
            let uv = d.xz / (d.y + 0.16) * vec2<f32>(0.55, 1.0);
            let n = cloud_noise(uv);
            let cover = g.cloud_color.a;
            let thr = 0.672 - 0.49 * cover;
            let dens = smoothstep(thr - 0.03, thr + mix(0.13, 0.26, cover), n);
            let n2 = cloud_noise(uv + normalize(sun0.xz + vec2<f32>(1e-4, 0.0)) * 0.22);
            let shade = clamp((n2 - n) * 3.5 + 0.35 * cover, 0.0, 1.0);
            let col = mix(g.cloud_color.rgb, g.cloud_shade.rgb, shade * 0.85) * g.dome.z;
            h = mix(h, col, dens * smoothstep(0.015, 0.16, d.y) * mix(0.82, 1.0, cover) * 0.6);
        }
        let sd0 = max(dot(d, sun0), 0.0);
        let above0 = smoothstep(-0.03, 0.02, sun0.y);
        h = h + g.sun_color.rgb * (pow(sd0, 1800.0) * 40.0 + pow(sd0, 90.0) * 0.25) * above0;
        let night0 = 1.0 - g.sky_zenith.a;
        if (night0 > 0.01 && d.y > -0.05) {
            let uv = d.xz / (abs(d.y) + 0.35) * 2.2;
            let st = textureSampleLevel(t_stars, s_lin, uv, 0.0);
            h = h + st.rgb * 0.6 * night0 * g.sky_horizon.a * smoothstep(0.02, 0.3, d.y);
        }
        return vec4<f32>(h, 1.0);
    }
    var c = sky_color(d);
    let sun = normalize(g.true_sun.xyz);
    let day = g.sky_zenith.a;
    // cloud deck: a flat layer high above, thinning out towards the horizon
    var cloud_a = 0.0;
    if (d.y > 0.015 && g.cloud_color.a > 0.01) {
        // stretched along the wind: long wisps rather than puffs
        let uv = d.xz / (d.y + 0.16) * vec2<f32>(0.55, 1.0);
        let n = cloud_noise(uv);
        let cover = g.cloud_color.a;
        // (the noise mix has its median at 0.44; 30 % of it lies above 0.525, 90 % above 0.23)
        let thr = 0.672 - 0.49 * cover;
        let dens = smoothstep(thr - 0.03, thr + mix(0.13, 0.26, cover), n);
        // lit from the sun's side: compare with the noise a little towards the sun
        let n2 = cloud_noise(uv + normalize(sun.xz + vec2<f32>(1e-4, 0.0)) * 0.22);
        let shade = clamp((n2 - n) * 3.5 + 0.35 * cover, 0.0, 1.0);
        let col = mix(g.cloud_color.rgb, g.cloud_shade.rgb, shade * 0.85);
        cloud_a = dens * smoothstep(0.015, 0.16, d.y) * mix(0.82, 1.0, cover);
        c = mix(c, col, cloud_a);
    }
    // sun disc + glow
    let sd = max(dot(d, sun), 0.0);
    let above = smoothstep(-0.03, 0.02, sun.y);
    c = c + g.sun_color.rgb * (pow(sd, 1800.0) * 14.0 + pow(sd, 90.0) * 0.55 + pow(sd, 8.0) * 0.12) * above;
    // stars (original star field, tiled over the dome) and the moon at night
    let night = 1.0 - day;
    if (night > 0.01 && d.y > -0.05) {
        // planar projection of the dome, tiled like the original (StarFieldRepeat)
        let uv = d.xz / (abs(d.y) + 0.35) * 2.2;
        let st = textureSampleLevel(t_stars, s_lin, uv, 0.0);
        c = c + st.rgb * 6.0 * night * g.sky_horizon.a * smoothstep(0.02, 0.3, d.y) * (1.0 - cloud_a);
        let m = normalize(g.moon_dir.xyz);
        let md = dot(d, m);
        if (md > 0.995) {
            let right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), m));
            let upv = cross(m, right);
            let q = vec2<f32>(dot(d, right), dot(d, upv)) / 0.06;
            if (abs(q.x) < 1.0 && abs(q.y) < 1.0) {
                let mc = textureSampleLevel(t_moon, s_lin, q * 0.5 + 0.5, 0.0);
                c = mix(c, mc.rgb * 1.6, mc.a * night);
            }
        }
        c = c + vec3<f32>(0.55, 0.6, 0.7) * pow(max(md, 0.0), 300.0) * 0.25 * night;
    }
    let x = c * g.params.z * 0.7;
    // (this dome is still composed in display terms; the scene target wants what tone-maps to it)
    return vec4<f32>(to_scene(clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
}
