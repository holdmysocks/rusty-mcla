// The lamps' glow pass, drawn by the original AFTER its tone map straight into the back buffer
// (LIGHTING_WORK.md "The lamps' glow pass after the tone map"): per light one "seed" quad
// (VS_GlowPassColorThrough + PSGlowSeedRT) and three "star" quads (VS_PassColorStarGlow +
// PSSeedRT1, textures/effects/star_glow.dds), blend one / one, no depth test - the programs
// read the depth buffer themselves. Translated from the microcode; the quads' corners are
// built here as the game's CPU side builds them (sub_822FC0E8 / sub_822FB7A8).
// The target is the halo picture the tone map adds to its encoded value (display terms).

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
};
@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var star: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
@group(1) @binding(2) var depth: texture_depth_2d;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // rgb = the light's colour, a = the quad's strength (seed or star)
    @location(1) @interpolate(flat) col: vec4<f32>,
    // xy = the light's place on screen (uv), z = its depth, w = the seed's tap distance
    @location(2) @interpolate(flat) centre: vec4<f32>,
    // 0 the seed, 1 a star quad
    @location(3) @interpolate(flat) kind: f32,
};

// The star quads' half diagonals in the camera's right / up plane: the table the game builds once
// (F-18836, called only from the tune's set-up F-15091, before GlowTuning.xml is read) from the
// constructor's StreakAngle defaults 1.83, 0.08, 1.39 (F-80296): A = (sin, cos), B = (-cos, sin) of each
// angle in f32. The file's StreakAngle (0.829, 2.293, 0) never reaches it. A carries the short one
// (uv 0,0 -> 1,1: the picture's bright line), B the long one.
fn star_a(q: u32) -> vec2<f32> {
    if (q == 0u) { return vec2<f32>(0.9665944, -0.2563109); }
    if (q == 1u) { return vec2<f32>(0.0799147, 0.9968017); }
    return vec2<f32>(0.9837008, 0.1798130);
}
fn star_b(q: u32) -> vec2<f32> {
    if (q == 0u) { return vec2<f32>(0.2563109, 0.9665944); }
    if (q == 1u) { return vec2<f32>(-0.9968017, 0.0799147); }
    return vec2<f32>(-0.1798130, 0.9837008);
}

fn to_uv(c: vec4<f32>) -> vec2<f32> {
    return vec2<f32>(c.x / c.w * 0.5 + 0.5, 0.5 - c.y / c.w * 0.5);
}

// 24 vertices an instance: the seed's two triangles, then three star quads.
// pos_a: the light's place (already pulled towards the camera), a = the seed's strength
// col_s: its colour, a = the star's strength
// size: xy = the seed's half size (m), z / w = the star's long / short half diagonal (m)
@vertex
fn vs_halo(@builtin(vertex_index) vi: u32, @location(0) pos_a: vec4<f32>, @location(1) col_s: vec4<f32>, @location(2) size: vec4<f32>) -> VsOut {
    var o: VsOut;
    let quad = vi / 6u;
    let k = vi % 6u;
    // (0,0) (1,0) (1,1) (1,1) (0,1) (0,0)
    let u = select(0.0, 1.0, k == 1u || k == 2u || k == 3u);
    let v = select(0.0, 1.0, k == 2u || k == 3u || k == 4u);
    // the camera's right and up (the original's c12 / c13: rows of the inverse view)
    let right = normalize(vec3<f32>(g.view_proj[0][0], g.view_proj[1][0], g.view_proj[2][0]));
    let up = normalize(vec3<f32>(g.view_proj[0][1], g.view_proj[1][1], g.view_proj[2][1]));
    let c = g.view_proj * vec4<f32>(pos_a.xyz, 1.0);
    // a light whose own place is off the screen has no quads at all (both vertex programs)
    let on = select(0.0, 1.0, c.w > 0.0 && abs(c.x) <= c.w && abs(c.y) <= c.w);
    var off = vec2<f32>(0.0);
    if (quad == 0u) {
        off = (vec2<f32>(u, v) * 2.0 - vec2<f32>(1.0)) * size.xy;
        o.col = vec4<f32>(col_s.rgb, pos_a.w);
        o.kind = 0.0;
    } else {
        off = star_a(quad - 1u) * size.w * (1.0 - u - v) + star_b(quad - 1u) * size.z * (u - v);
        o.col = vec4<f32>(col_s.rgb, col_s.a);
        o.kind = 1.0;
    }
    let wp = pos_a.xyz + (right * off.x + up * off.y) * on;
    o.clip = g.view_proj * vec4<f32>(wp, 1.0);
    o.uv = vec2<f32>(u, v);
    let cw = max(c.w, 1e-6);
    let cuv = vec2<f32>(c.x / cw * 0.5 + 0.5, 0.5 - c.y / cw * 0.5);
    // the seed's four outer depth taps stand 0.05 x the quad's half diagonal (in uv) off the middle
    let c1 = g.view_proj * vec4<f32>(pos_a.xyz + right * size.x + up * size.y, 1.0);
    let d = 0.05 * length(to_uv(vec4<f32>(c1.xyz, max(c1.w, 1e-6))) - cuv);
    o.centre = vec4<f32>(cuv, c.z / cw, d);
    return o;
}

fn scene_depth(uv: vec2<f32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(depth));
    let p = clamp(vec2<i32>(uv * vec2<f32>(dims)), vec2<i32>(0), dims - vec2<i32>(1));
    return textureLoad(depth, p, 0);
}

// 1 where the light's place lies before the scene at that pixel (the depth buffer is reversed)
fn seen(z: f32, uv: vec2<f32>) -> f32 {
    return select(0.0, 1.0, z > scene_depth(uv));
}

@fragment
fn fs_halo(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(star, samp, in.uv).rgb;
    let c = in.centre.xy;
    let z = in.centre.z;
    let d = in.centre.w;
    if (in.kind > 0.5) {
        // PSSeedRT1: one depth tap at the light's place, all or nothing
        if (seen(z, c) < 0.5) {
            discard;
        }
        return vec4<f32>(clamp(t * in.col.rgb * in.col.a, vec3<f32>(0.0), vec3<f32>(1.0)), 0.0);
    }
    // PSGlowSeedRT: the light's visibility from five depth taps (0.4 the middle, 0.15 each of
    // four round it), then (visibility - distance from the quad's middle)^2
    let vis = 0.4 * seen(z, c)
        + 0.15 * (seen(z, c + vec2<f32>(d, 0.5 * d)) + seen(z, c + vec2<f32>(-0.5 * d, d)) + seen(z, c + vec2<f32>(-d, -0.5 * d)) + seen(z, c + vec2<f32>(0.5 * d, -d)));
    let q = abs(in.uv - vec2<f32>(0.5)) * 2.0;
    let r = clamp(sqrt(q.x * q.x + q.y * q.y), 0.0, 1.0);
    let f = clamp(vis - r, 0.0, 1.0);
    return vec4<f32>(in.col.rgb * (f * f * in.col.a), 0.0);
}
