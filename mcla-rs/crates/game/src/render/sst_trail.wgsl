// The slipstream trail (sst_trail.rs), from the original's SSTTrail.fx (cars/0xc53f6c0e, technique "draw"):
// VS_Trail (gWorldViewProj c8, skinningData c100 + a0) and PS_Trail (DiffuseSampler, effectFade c64,
// gInvColorExpBias c27, gLightAmbient c26). Decode: work/slipstream/trail/trail_rule.md.
//
// VS_Trail, per vertex (the mesh's x, y; u = its ring 0..31, v around it):
//   i = trunc(u); j = i == 0 ? 1 : i - 1; s = i == 0 ? -1 : 1; P = point i, Q = point j
//   n = normalize(P - Q); world = (P.x + s n.z / |n.xz| x, P.y + y, P.z - s n.x / |n.xz| x)
//   uv = (u / 32, v + P.w); along = i == 0 ? 0 : 1 - u / 32
// PS_Trail: rgb = tex.rgb; a = tex.a x 0.08 x effectFade x gInvColorExpBias.x x gLightAmbient.w x along
// (the scene target's exponent bias takes gInvColorExpBias back - the emulator scales all four channels;
// gLightAmbient.w 1, recorded), blended SRCALPHA / ONE.
// The original's fog pass after it lerps the target towards the fog colour by the fog of the scene's depth (not
// the trail's, which writes none), except where the stencil is 1 (the sky, the ground dome): the trail keeps
// 1 - that share. The port's materials fog themselves (city.wgsl, the same fog), so the trail's alpha takes it.

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
};
@group(0) @binding(0) var<uniform> g: Globals;
@group(2) @binding(0) var t_depth: texture_depth_2d;
@group(2) @binding(1) var t_stencil: texture_2d<f32>;

struct Car {
    p: array<vec4<f32>, 32>,
    // x: effectFade (the faded value, mcCarModel +7980)
    fade: vec4<f32>,
};
struct Cars {
    car: array<Car, 8>,
};
@group(1) @binding(0) var t_diff: texture_2d<f32>;
@group(1) @binding(1) var s_diff: sampler;
@group(1) @binding(2) var<uniform> cars: Cars;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) along: f32,
    @location(2) @interpolate(flat) fade: f32,
};

@vertex
fn vs_trail(@builtin(instance_index) ii: u32, @location(0) pos: vec3<f32>, @location(1) uv: vec2<f32>) -> VOut {
    let i = clamp(i32(trunc(uv.x)), 0, 31);
    let first = i == 0;
    let j = select(i - 1, 1, first);
    let s = select(1.0, -1.0, first);
    let p = cars.car[ii].p[i];
    let q = cars.car[ii].p[j];
    let d = p.xyz - q.xyz;
    let h2 = d.x * d.x + d.z * d.z;
    var side = vec2<f32>(0.0, 0.0);
    if (h2 > 0.0) {
        side = vec2<f32>(d.z, -d.x) * (s * inverseSqrt(h2));
    }
    let w = vec3<f32>(p.x + side.x * pos.x, p.y + pos.y, p.z + side.y * pos.x);
    var o: VOut;
    o.clip = g.view_proj * vec4<f32>(w, 1.0);
    o.uv = vec2<f32>(uv.x * 0.03125, uv.y + p.w);
    o.along = select(1.0 - uv.x * 0.03125, 0.0, first);
    o.fade = cars.car[ii].fade.x;
    return o;
}

// The fog pass's share at this pixel (city.wgsl's fog: 1 - exp(-(d - start) / (end - start)), at most the fog
// colour's alpha), d = the distance to the scene's point under the pixel; none on stencil 1.
fn fog_share(px: vec2<f32>) -> f32 {
    let ip = vec2<i32>(px);
    let s = textureLoad(t_stencil, ip, 0).r * 255.0;
    if (abs(s - 1.0) < 0.5) {
        return 0.0;
    }
    let dim = vec2<f32>(textureDimensions(t_depth));
    let d = textureLoad(t_depth, ip, 0);
    let ndc = vec2<f32>(px.x / dim.x * 2.0 - 1.0, 1.0 - px.y / dim.y * 2.0);
    let h = g.inv_view_proj * vec4<f32>(ndc, d, 1.0);
    var dist = 1e9;
    if (abs(h.w) > 1e-12) {
        dist = length(h.xyz / h.w - g.cam_pos.xyz);
    }
    let fg = clamp(1.0 - exp(-(dist - g.params.x) / max(g.params.y - g.params.x, 1.0)), 0.0, 1.0);
    return clamp(fg * g.fog_color.a, 0.0, 1.0);
}

@fragment
fn fs_trail(in: VOut) -> @location(0) vec4<f32> {
    let t = textureSample(t_diff, s_diff, in.uv);
    let a = t.a * 0.08 * in.fade * in.along;
    // (AlphaTestEnable, AlphaFunc GREATER, AlphaRef 0 - on the program's alpha, before the fog)
    if (!(a > 0.0)) {
        discard;
    }
    return vec4<f32>(t.rgb, a * (1.0 - fog_share(in.clip.xy)));
}
