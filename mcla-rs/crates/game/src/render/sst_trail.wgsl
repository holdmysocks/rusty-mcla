// The slipstream trail (sst_trail.rs), from the original's SSTTrail.fx (cars/0xc53f6c0e, technique "draw"):
// VS_Trail (gWorldViewProj c8, skinningData c100 + a0) and PS_Trail (DiffuseSampler, effectFade c64,
// gInvColorExpBias c27, gLightAmbient c26). Decode: work/slipstream/trail/trail_rule.md.
//
// VS_Trail, per vertex (the mesh's x, y; u = its ring 0..31, v around it):
//   i = trunc(u); j = i == 0 ? 1 : i - 1; s = i == 0 ? -1 : 1; P = point i, Q = point j
//   n = normalize(P - Q); world = (P.x + s n.z / |n.xz| x, P.y + y, P.z - s n.x / |n.xz| x)
//   uv = (u / 32, v + P.w); along = i == 0 ? 0 : 1 - u / 32
// PS_Trail: rgb = tex.rgb; a = tex.a x 0.08 x effectFade x gInvColorExpBias.x x gLightAmbient.w x along
// (the scene target's exponent bias takes gInvColorExpBias back; gLightAmbient.w 1), blended SRCALPHA / ONE.

struct Globals {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> g: Globals;

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

@fragment
fn fs_trail(in: VOut) -> @location(0) vec4<f32> {
    let t = textureSample(t_diff, s_diff, in.uv);
    let a = t.a * 0.08 * in.fade * in.along;
    // (AlphaTestEnable, AlphaFunc GREATER, AlphaRef 0)
    if (!(a > 0.0)) {
        discard;
    }
    return vec4<f32>(t.rgb, a);
}
