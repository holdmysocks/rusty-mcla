// Step 19g: the power-up effects on the cars (pu_fx.rs), from the original's powerupicecube.fxc
// (cars/0x399eedf6): techniques icetechnique (VS_IceBlock / PS_IceBlock), shieldtechnique
// (VS_Shield / PS_Shield) and stealthtechnique (VS_Stealth / PS_Stealth); SAVE_WORK.md
// "Step 19g". Colours are written as the game's programs write them (true scale); their alpha
// is `x gInvColorExpBias.x x gLightAmbient.w`, which the scene target's exponent bias takes back
// to `x gLightAmbient.w` (1).

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
};
@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(5) var t_env: texture_cube<f32>;
@group(0) @binding(6) var s_env: sampler;

// dpFrontSampler, the car's map of its surroundings: the port's cube map of them, looked up
// along the mirror direction as the port's car paint does (city.wgsl env_color).
fn env(d: vec3<f32>) -> vec3<f32> {
    if (g.shadow_params.w > 0.5) {
        let real = textureSampleLevel(t_env, s_env, d, 0.0).rgb;
        if (g.dome.w > 0.5) { return real; }
        let sun = pow(max(dot(d, g.sun_dir.xyz), 0.0), 180.0) * g.sun_dir.w;
        return real * 1.15 + g.sun_color.rgb * sun * 2.5;
    }
    let up = clamp(d.y, -1.0, 1.0);
    let sky = mix(g.sky_zenith.rgb, g.sky_horizon.rgb, pow(1.0 - max(up, 0.0), 6.0)) * 1.25;
    return mix(g.ambient_ground.rgb * 0.3, sky, smoothstep(-0.08, 0.3, up));
}

// One effect drawn on one car: its world matrix (the car's, scaled and moved) and four
// parameter rows (see pu_fx.rs `Inst`).
struct Inst {
    m: mat4x4<f32>,
    a: vec4<f32>,
    b: vec4<f32>,
    c: vec4<f32>,
    d: vec4<f32>,
};
struct Fx {
    inst: array<Inst, 16>,
};
@group(1) @binding(0) var t_diff: texture_2d<f32>;
@group(1) @binding(1) var s_diff: sampler;
@group(1) @binding(2) var<uniform> fx: Fx;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) wpos: vec3<f32>,
    @location(1) n: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) ii: u32,
    // VS_Shield's o0: the CLIP position (x, y, depth) - see fs_shield
    @location(4) clipo: vec3<f32>,
};

@vertex
fn vs_fx(@builtin(instance_index) ii: u32, @location(0) pos: vec3<f32>, @location(1) nrm: vec3<f32>, @location(2) uv: vec2<f32>) -> VOut {
    let m = fx.inst[ii].m;
    let w = m * vec4<f32>(pos, 1.0);
    var o: VOut;
    o.clip = g.view_proj * w;
    o.wpos = w.xyz;
    // (both programs turn the normal by gWorld as it is - scale and all - and normalise)
    o.n = normalize((m * vec4<f32>(nrm, 0.0)).xyz);
    o.uv = uv;
    o.ii = ii;
    // (the original's clip z is the view depth less the near plane x f / (f - n): taken as w)
    o.clipo = vec3<f32>(o.clip.x, o.clip.y, o.clip.w);
    return o;
}

// The two passes of both techniques: CullMode CCW then CW, as the cars' windows - the inner
// faces first, then the outer ones (which side faces the camera, from the face's own plane,
// turned to the side of the vertex normal: the meshes are closed and convex).
fn outer_side(wpos: vec3<f32>, n: vec3<f32>) -> bool {
    let ng = cross(dpdx(wpos), dpdy(wpos));
    let o = select(-ng, ng, dot(ng, n) >= 0.0);
    return dot(o, g.cam_pos.xyz - wpos) > 0.0;
}

// PS_IceBlock: rgb = DiffuseTex(uv x 4.5).r x FX_TexColor.rgb x FX_TexColor.w + dpFront(R);
// a = gLightAmbient.w x (FX_FresnelMin + (FX_FresnelMax - FX_FresnelMin)(1 - |n.v|)^FX_FresnelExp).
// a = TexColor, b = (FresnelMin, FresnelMax, FresnelExp, -)
fn ice(in: VOut) -> vec4<f32> {
    let p = fx.inst[in.ii];
    let v = normalize(g.cam_pos.xyz - in.wpos);
    let n = normalize(in.n);
    let ndv = clamp(abs(dot(n, v)), 0.0, 1.0);
    let e = env(reflect(-v, n));
    let t = textureSample(t_diff, s_diff, in.uv * 4.5).r;
    let rgb = t * p.a.rgb * p.a.w + e;
    let a = p.b.x + (p.b.y - p.b.x) * pow(max(1.0 - ndv, 0.0), p.b.z);
    return vec4<f32>(rgb, clamp(a, 0.0, 1.0));
}

@fragment
fn fs_ice_in(in: VOut) -> @location(0) vec4<f32> {
    let outer = outer_side(in.wpos, in.n);
    let c = ice(in);
    if (outer) { discard; }
    return c;
}

@fragment
fn fs_ice_out(in: VOut) -> @location(0) vec4<f32> {
    let outer = outer_side(in.wpos, in.n);
    let c = ice(in);
    if (!outer) { discard; }
    return c;
}

// PS_Shield (blend source alpha, one). Its view direction is taken from VS_Shield's o0, which is
// the clip position (VS_Shield 23: o0 = (clip x, y, z)), not the world position:
// v = normalize(camera - clip.xyz). With T1 = FX_Timer1.xy, T2 = FX_Timer2.x, T3 = FX_Timer3.x:
//   A = DiffuseTex(125 uv + (-sin T2, cos T2)),  B = DiffuseTex((T3 + 0.2) uv + (sin 10.5 T2, -sin 2.5 T2))
//   wa = A.a FuzzColor.a T1.x,  wb = B.a TexColor.a T1.y
//   rgb = A.rgb FuzzColor.rgb wa + B.rgb TexColor.rgb wb + dpFront(R) EnvMapAlpha
//   a   = gLightAmbient.w ((Min + (Max - Min)(1 - |n.v|)^Exp) EnvMapAlpha + wa + wb)
// a = FuzzColor, b = TexColor, c = (T1.x, T1.y, T2, T3), d = (Min, Max, Exp, EnvMapAlpha)
// (FuzzColor.w, TexColor.w and EnvMapAlpha come already times the effect's fade)
fn shield(in: VOut) -> vec4<f32> {
    let p = fx.inst[in.ii];
    let v = normalize(g.cam_pos.xyz - in.clipo);
    let n = normalize(in.n);
    let ndv = clamp(abs(dot(n, v)), 0.0, 1.0);
    let e = env(reflect(-v, n));
    let t2 = p.c.z;
    let k = p.c.w + 0.2;
    let ua = in.uv * 125.0 + vec2<f32>(-sin(t2), cos(t2));
    let ub = in.uv * k + vec2<f32>(sin(10.5 * t2), -sin(2.5 * t2));
    let ta = textureSample(t_diff, s_diff, ua);
    let tb = textureSample(t_diff, s_diff, ub);
    let wa = ta.a * p.a.w * p.c.x;
    let wb = tb.a * p.b.w * p.c.y;
    let rgb = ta.rgb * p.a.rgb * wa + tb.rgb * p.b.rgb * wb + e * p.d.w;
    let fr = p.d.x + (p.d.y - p.d.x) * pow(max(1.0 - ndv, 0.0), p.d.z);
    return vec4<f32>(rgb, clamp(fr * p.d.w + wa + wb, 0.0, 1.0));
}

@fragment
fn fs_shield_in(in: VOut) -> @location(0) vec4<f32> {
    let outer = outer_side(in.wpos, in.n);
    let c = shield(in);
    if (outer) { discard; }
    return c;
}

@fragment
fn fs_shield_out(in: VOut) -> @location(0) vec4<f32> {
    let outer = outer_side(in.wpos, in.n);
    let c = shield(in);
    if (!outer) { discard; }
    return c;
}

// The car under Stealth: its own meshes with PS_Stealth (blend source alpha, one):
//   rgb = dpFront(R),  a = gLightAmbient.w (0.125 + 0.875 (1 - |n.v|)^4.5)
struct Obj {
    model: mat4x4<f32>,
};
@group(2) @binding(0) var<uniform> obj: Obj;

struct SOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) wpos: vec3<f32>,
    @location(1) n: vec3<f32>,
};

@vertex
fn vs_stealth(@location(0) pos: vec3<f32>, @location(1) nrm: vec3<f32>) -> SOut {
    let w = obj.model * vec4<f32>(pos, 1.0);
    var o: SOut;
    o.clip = g.view_proj * w;
    o.wpos = w.xyz;
    o.n = (obj.model * vec4<f32>(nrm, 0.0)).xyz;
    return o;
}

@fragment
fn fs_stealth(in: SOut) -> @location(0) vec4<f32> {
    let v = normalize(g.cam_pos.xyz - in.wpos);
    let n = normalize(in.n);
    let ndv = clamp(abs(dot(n, v)), 0.0, 1.0);
    let e = env(reflect(-v, n));
    return vec4<f32>(e, 0.125 + 0.875 * pow(max(1.0 - ndv, 0.0), 4.5));
}

// The lightning (lightning.fxc, effects/0x1d16b44b "draw"): rgb = LightningTexture.rgb x the
// vertex colour, a = its alpha x the vertex alpha (x gInvColorExpBias, taken back by the
// target); blend source alpha, one; the vertices are in world space.
struct LOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) col: vec4<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vs_bolt(@location(0) pos: vec3<f32>, @location(1) col: vec4<f32>, @location(2) uv: vec2<f32>) -> LOut {
    var o: LOut;
    o.clip = g.view_proj * vec4<f32>(pos, 1.0);
    o.col = col;
    o.uv = uv;
    return o;
}

@fragment
fn fs_bolt(in: LOut) -> @location(0) vec4<f32> {
    let t = textureSample(t_diff, s_diff, in.uv);
    return vec4<f32>(t.rgb * in.col.rgb, t.a * in.col.a);
}

// The shot's laser fan (PowerupTargetEffect.fxc, cars/0x8028c322 "draw": VS_Laser / PS_Laser;
// blend one, one; no depth writes; no culling). The vertex program puts each vertex at
// x = v (u ScaleParams.x + ScaleParams.z), z = 0 / -30 in the shooter's frame (pu_fx.rs does that
// and hands over the world position, the local one and the layer's factor
// k = 4 FX_Color.rgb x ScaleParams.w x ((alphaWide - alphaNarrow)(ScaleParams.x / 10)^0.2 + alphaNarrow)).
// PS_Laser, d = |local position|:
//   rgb = k (1 - d / 30)^6 (DiffuseTex(world.xz / 7).rgb + 0.15) sat(8 (d / 30 - 0.08))
struct ZOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) wxz: vec2<f32>,
    @location(1) local: vec3<f32>,
    @location(2) k: vec3<f32>,
};

@vertex
fn vs_laser(@location(0) pos: vec3<f32>, @location(1) local: vec3<f32>, @location(2) k: vec3<f32>) -> ZOut {
    var o: ZOut;
    o.clip = g.view_proj * vec4<f32>(pos, 1.0);
    o.wxz = pos.xz / 7.0;
    o.local = local;
    o.k = k;
    return o;
}

@fragment
fn fs_laser(in: ZOut) -> @location(0) vec4<f32> {
    let d = length(in.local) / 30.0;
    let t = textureSample(t_diff, s_diff, in.wxz).rgb;
    let a = 1.0 - d;
    let f = a * a * a * a * a * a;
    let rgb = in.k * f * (t + vec3<f32>(0.15)) * clamp(8.0 * (d - 0.08), 0.0, 1.0);
    return vec4<f32>(rgb, 1.0);
}

// The shot's ribbon beam (PowerupShootEffect.fxc, cars/0xdf4fb56c: "draw" VS_Beam / PS_Beam and "drawRibbon" VS_Ribbon /
// PS_Ribbon; the passes' states are the laser's: one, one; no depth writes; no culling). pu_fx.rs places the strips in the
// world and hands over (s, t) and k = 4 colour.rgb x the vertex's alpha (alpha x AlphaFadeParams.z / .w x fade).
// PS_Beam / PS_Ribbon: rgb = DiffuseTex(s, t).rgb x 4 x colour.rgb x alpha.
struct BOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) st: vec2<f32>,
    @location(1) k: vec3<f32>,
};

@vertex
fn vs_beam(@location(0) pos: vec3<f32>, @location(1) st: vec3<f32>, @location(2) k: vec3<f32>) -> BOut {
    var o: BOut;
    o.clip = g.view_proj * vec4<f32>(pos, 1.0);
    o.st = st.xy;
    o.k = k;
    return o;
}

@fragment
fn fs_beam(in: BOut) -> @location(0) vec4<f32> {
    let t = textureSample(t_diff, s_diff, in.st).rgb;
    return vec4<f32>(t * in.k, 1.0);
}
