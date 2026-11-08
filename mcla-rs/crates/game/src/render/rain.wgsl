// Rain as the original draws it (LIGHTING_WORK.md, "Rain as the original draws it"):
//   - the drops and their splashes: the effect `rain_system` (ParticleRenderVS / ParticleRenderPS
//     of shaders/effects/fxl_final/0x9e307cae), one quad a particle;
//   - the street lamps' light cones in the rain: `rage_spotlightfog` (vs_main_precal /
//     ps_mainFast), one cone a lamp.
// Both are blended source alpha / one minus source alpha over the finished scene, depth tested,
// no depth write.

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

// the effect's constants, under their own names
struct RainU {
    global_light: vec4<f32>,   // GlobalLightColor: m_KeyColor + m_KeyAmbLight + RainAmbient
    scale_offset: vec4<f32>,   // ScaleOffset: splash opacity, Rainstrength, SplashSizeScale, rain opacity
    tune: vec4<f32>,           // TuneParams: SplashOffset, Rainthreshold, Streakthickness, Streaklength
    spot_pos: vec4<f32>,       // SpotLightPos
    spot_dir: vec4<f32>,       // SpotLightDir
    spot_params: vec4<f32>,    // SpotLightParams: strength, height stretch
    spot_color: vec4<f32>,     // SpotLightColor: RainSpotLight
    fade_sphere: vec4<f32>,    // FadeOutSphere: centre, radius
    lights: array<vec4<f32>, 4>,      // PointLights: position, reach
    light_cols: array<vec4<f32>, 4>,  // PointLightColors
};
@group(1) @binding(0) var t_rain: texture_2d<f32>;
@group(1) @binding(1) var s_rain: sampler;
@group(1) @binding(2) var t_anim: texture_2d<f32>;
@group(1) @binding(3) var<uniform> r: RainU;

struct DropOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) light: vec3<f32>,
    @location(2) fade: vec2<f32>,   // alpha, 1 = splash
};

@vertex
fn vs_drop(
    @builtin(vertex_index) vi: u32,
    @location(0) pos_state: vec4<f32>,   // position; state: 1.25 falling, 0.75 .. 0 the splash's four frames, below 0 dead
    @location(1) vel: vec4<f32>,
) -> DropOut {
    var o: DropOut;
    let u = f32(vi & 1u);
    let v = f32((vi >> 1u) & 1u);
    let p = pos_state.xyz;
    let state = pos_state.w;
    let to_cam = g.cam_pos.xyz - p;
    let dist = max(length(to_cam), 1e-4);
    let d = to_cam / dist;
    var wp = p;
    var alpha = step(0.0, state);
    if (state < 1.0) {
        // a splash: a card facing the camera round the vertical, SplashSizeScale wide, half
        // as high (flatter the steeper the view), smaller nearer than 13 m
        let hd = max(length(to_cam.xz), 1e-4);
        let side = vec3<f32>(to_cam.z / hd, 0.0, -to_cam.x / hd);
        let k = min(dist * 0.0769230798, 1.0);
        let s = r.scale_offset.z;
        wp = p + (side * (u * s) + vec3<f32>(0.0, -0.5 * v * s * length(d.xz), 0.0)) * k;
        o.uv = vec2<f32>(0.25 * u + 0.75 - state, v);
        o.fade.y = 1.0;
        alpha = alpha * r.scale_offset.x;
    } else {
        // a streak: from the drop back along its velocity (x Streaklength), a constant width on
        // screen (Streakthickness x distance / 91)
        let c = cross(to_cam, vel.xyz);
        let cl = length(c);
        if (cl < 1e-6) {
            alpha = 0.0;
        }
        let side = c / max(cl, 1e-6);
        wp = p + side * (u * r.tune.z * 0.0109890122 * dist) - vel.xyz * (v * r.tune.w);
        o.uv = vec2<f32>(1.0 - u, 1.0 - v);
        o.fade.y = 0.0;
        alpha = alpha * r.scale_offset.w;
    }
    // the light on it: one colour for all of it, four point lights, one "spot"
    var light = r.global_light.rgb;
    for (var i = 0; i < 4; i = i + 1) {
        let l = r.lights[i].xyz - wp;
        let a = clamp(r.lights[i].w * inverseSqrt(max(dot(l, l), 1e-8)), 0.0, 1.0);
        light = light + r.light_cols[i].rgb * (a * a);
    }
    let sl = (r.spot_pos.xyz - wp) * vec3<f32>(1.0, r.spot_params.y, 1.0);
    let sll = max(length(sl), 1e-4);
    let sd = dot(sl / sll, r.spot_dir.xyz);
    if (sd > 0.0) {
        // (the ninth power as exp(9 log): far beyond the clamp for anything but a unit vector)
        let e = min(9.0 * log(max(sd, 1e-20)), 40.0);
        light = light + r.spot_color.rgb * clamp(exp(e) * r.spot_params.x / sll, 0.0, 2.0);
    }
    // nothing inside the sphere round the car (half as flat again in height)
    let fs = (wp - r.fade_sphere.xyz) * vec3<f32>(1.0, 0.5, 1.0);
    if (length(fs) < r.fade_sphere.w) {
        alpha = 0.0;
    }
    o.light = light;
    o.fade.x = alpha;
    o.clip = g.view_proj * vec4<f32>(wp, 1.0);
    return o;
}

@fragment
fn fs_drop(in: DropOut) -> @location(0) vec4<f32> {
    let a = textureSample(t_rain, s_rain, in.uv).r;
    let b = textureSample(t_anim, s_rain, in.uv).r;
    return vec4<f32>(in.light, in.fade.x * mix(a, b, step(0.5, in.fade.y)));
}

// ---- the lamps' cones ------------------------------------------------------------------
// rage_spotlightfog's vs_main_precal and ps_mainFast as their microcode reads (work/lighting/re/rain/spotlightfog_fx.txt;
// LIGHTING.md "The lamps' cones"). The mesh is the original's unit cone (rain.rs cone_mesh): (cos a, y, sin a).

struct ConeOut {
    @builtin(position) clip: vec4<f32>,
    // o0: the four samples' cone terms, o1: their distances to the lamp, o3.x: the fade
    @location(0) cone: vec4<f32>,
    @location(1) dist: vec4<f32>,
    @location(2) fade: f32,
    @location(3) @interpolate(flat) colour: vec4<f32>,
    @location(4) @interpolate(flat) volume_x: f32,
};

@vertex
fn vs_cone(
    @location(0) p: vec3<f32>,
    @location(1) l0: vec4<f32>, @location(2) l1: vec4<f32>, @location(3) l2: vec4<f32>, @location(4) l3: vec4<f32>,
    @location(5) m0: vec4<f32>, @location(6) m1: vec4<f32>, @location(7) m2: vec4<f32>, @location(8) m3: vec4<f32>,
    @location(9) lpos: vec4<f32>,     // LightPosition (w: the edge's offset)
    @location(10) ldir: vec4<f32>,    // LightDirection (w: the edge's factor)
    @location(11) lvol: vec4<f32>,    // LightVolume
    @location(12) lcol: vec4<f32>,    // LightColor
    @location(13) misc: vec4<f32>,    // LightFallOff, gInvColorExpBias.x
) -> ConeOut {
    var o: ConeOut;
    let cam = g.cam_pos.xyz;
    // the camera in the light's frame (c15 x LightInvMatrix)
    let e = cam.z * m2.xyz + m3.xyz + cam.y * m1.xyz + cam.x * m0.xyz;
    // the vertex: (y + LightVolume.z) x (x row0 + z row1 + row2) + row3 (gWorld the identity)
    let s = lvol.z + p.y;
    let w = (s * p.x) * l0.xyz + ((s * p.z) * l1.xyz + (s * l2.xyz + l3.xyz));
    o.clip = g.view_proj * vec4<f32>(w, 1.0);
    // the view ray to it, in the world and in the light's frame
    let dw = (w - cam) * inverseSqrt(dot(w - cam, w - cam));
    let vl = dw.x * m0.xyz + (dw.y * m1.xyz + dw.z * m2.xyz);
    let u = vl * inverseSqrt(dot(vl, vl));
    // the ray e + t u against the cone x^2 + y^2 = z^2: a t^2 + b t + c
    let a = (u.x * u.x + u.y * u.y) - u.z * u.z;
    let b = (u.x * (e.x + e.x) + u.y * (e.y + e.y)) - u.z * (e.z + e.z);
    let c = (e.x * e.x + e.y * e.y) - e.z * e.z;
    let ac4 = (a * 4.0) * c;
    var len = 0.0;
    if (!(ac4 >= b * b) && !(a == 0.0)) {
        let r = sqrt(abs(b * b - ac4));
        let k = 1.0 / (a + a);
        let t1 = (r - b) * k;
        let t2 = (-b - r) * k;
        let tmin = min(t1, t2);
        let tmax = max(t1, t2);
        let za = u.z * tmin + e.z;
        let zb = u.z * tmax + e.z;
        // where the ray crosses the cone's end (z = 1)
        let t_end = (1.0 - e.z) * (1.0 / u.z);
        // in: the nearer crossing, the end beyond it, the camera itself when that crossing is on the mirrored cone
        var ta = select(tmin, t_end, za > 1.0);
        if (!(za > 0.0)) {
            ta = 0.0;
        }
        // out: the farther crossing, the end when that is beyond it or on the mirrored cone
        let tb = select(t_end, select(tmax, t_end, zb > 1.0), zb > 0.0);
        let pa = vec3<f32>(u.x * ta + e.x, u.y * ta + e.y, u.z * ta + e.z);
        let pb = vec3<f32>(u.x * tb + e.x, u.y * tb + e.y, u.z * tb + e.z);
        let wa = pa.x * l0.xyz + (pa.y * l1.xyz + (pa.z * l2.xyz + l3.xyz));
        let wb = pb.x * l0.xyz + (pb.y * l1.xyz + (pb.z * l2.xyz + l3.xyz));
        len = sqrt(abs(dot(wb - wa, wb - wa)));
    }
    // four samples from the vertex along the view ray, at 1/8 .. 7/8 of the stretch
    let ks = len * vec4<f32>(0.125, 0.375, 0.625, 0.875);
    var dist: vec4<f32>;
    var cone: vec4<f32>;
    for (var i = 0; i < 4; i++) {
        let q = lpos.xyz - (ks[i] * dw + w);
        let d2 = dot(q, q);
        dist[i] = sqrt(abs(d2));
        let cs = (inverseSqrt(abs(d2)) * ldir.w) * dot(q, -ldir.xyz) + lpos.w;
        cone[i] = exp2(log2(abs(cs)) * misc.x);
    }
    o.cone = cone;
    o.dist = dist;
    o.fade = misc.y * (lvol.y * clamp(8.0 * (1.0 - p.y), 0.0, 1.0));
    o.colour = lcol;
    o.volume_x = lvol.x;
    return o;
}

@fragment
fn fs_cone(in: ConeOut) -> @location(0) vec4<f32> {
    // s = min(sat(cone) x sat(1 / d^2) x LightColor.w, 0.3) x LightVolume.x; alpha = sat((s0 + q (s1 + q (s2 + q s3))) x fade)
    let r = 1.0 / in.dist;
    let near = clamp(r * r, vec4<f32>(0.0), vec4<f32>(1.0)) * in.colour.w;
    let s = min(near * clamp(in.cone, vec4<f32>(0.0), vec4<f32>(1.0)), vec4<f32>(0.3)) * in.volume_x;
    let q = 1.0 - in.volume_x;
    let t = s.x + q * (s.y + q * (s.z + q * s.w));
    return vec4<f32>(in.colour.rgb, clamp(t * in.fade, 0.0, 1.0));
}
