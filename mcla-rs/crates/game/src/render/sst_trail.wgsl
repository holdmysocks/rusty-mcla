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
// The original's fog pass after it (PSBlendedHeightFogInFogNoPFog, SRCALPHA / INVSRCALPHA, stencil NOT EQUAL 1) lerps
// the target towards the fog colour by the fog of the scene's depth (not the trail's, which writes none): the
// trail keeps 1 - that share. The port's materials fog themselves (city.wgsl, the same share), so the trail's alpha
// takes it - computed as that program computes it (`fog_share`, sst_trail.rs `fog_share`).

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

// The fog pass's share at this pixel, as the original's program PSBlendedHeightFogInFogNoPFog (xrage_postfx)
// computes it from the scene's depth under the pixel (microcode, work/slipstream/trail3/fogps.wgsl):
//   d = |the scene's point - the camera| (WorldViewProjInverse of the pixel and the depth; gViewInverse row 3),
//   0 where the depth is the cleared one (1 - depth < 1 fails: nothing drawn there)
//   share = sat(SuperFogMaxColor.w x sat(1 - 2^(-1.4426950 (d x c65.z + c65.w)))), output alpha x
//   gInvColorExpBias.x, which the target's exponent bias takes back
// with SuperFogParams2 c65.z = 1 / (far - near), c65.w = -(near / (far - near)) (F-88930 makes them from the fog's
// near / far so; recorded c65 = (0.001, 0, 1/450, -50/450) at near 50, far 500). None where the stencil is 1 (the
// sky, the ground dome: the pass's stencil test NOT EQUAL 1).
// (rgb: the fog's colour there as the materials take it - city.wgsl fs_main: leaning towards the horizon's colour,
// east or west by where one looks; a: the share)
fn fog_at(px: vec2<f32>) -> vec4<f32> {
    let ip = vec2<i32>(px);
    let s = textureLoad(t_stencil, ip, 0).r * 255.0;
    if (abs(s - 1.0) < 0.5) {
        return vec4<f32>(g.fog_color.rgb, 0.0);
    }
    let dim = vec2<f32>(textureDimensions(t_depth));
    let dz = textureLoad(t_depth, ip, 0);
    var dist = 0.0;
    var dir_x = 0.0;
    if (dz > 0.0) {
        let ndc = vec2<f32>(px.x / dim.x * 2.0 - 1.0, 1.0 - px.y / dim.y * 2.0);
        let h = g.inv_view_proj * vec4<f32>(ndc, dz, 1.0);
        let to_px = h.xyz / h.w - g.cam_pos.xyz;
        dist = length(to_px);
        dir_x = to_px.x / max(dist, 0.001);
    }
    let span = g.params.y - g.params.x;
    let k = 1.0 / span;
    let k0 = -(g.params.x / span);
    let e = exp2(-1.4426950216293335 * (dist * k + k0));
    let horizon = mix(g.fog_east.rgb, g.fog_west.rgb, 0.5 + 0.5 * dir_x);
    let fog_rgb = g.fog_color.rgb + (horizon - g.fog_color.rgb) * g.fog_east.w;
    return vec4<f32>(fog_rgb, clamp(g.fog_color.a * clamp(1.0 - e, 0.0, 1.0), 0.0, 1.0));
}

fn fog_share(px: vec2<f32>) -> f32 {
    return fog_at(px).a;
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

// The ceiling of the original's scene target under the trail (sst_trail.rs `compose`): the original adds the trail
// into its 2_10_10_10_FLOAT target (7e3, exponent bias 4: nothing above 31.875 / 16 = 1.9921875 a channel is kept;
// each blend's result is clamped there) BEFORE its fog pass lerps the target to the fog colour F by the share f.
// So the original shows min(S + T, M)(1 - f) + F f with S the scene unfogged, M = 1.9921875; the port's scene
// already holds S(1 - f) + F f, adds T(1 - f) in fs_trail, and then this pass (blend MIN, the same pixels: the trail's
// coverage with its alpha > 0) keeps each channel at most M(1 - f) + F f - the same value, in any order of layers
// and trails (all the adds are positive).
@fragment
fn fs_trail_cap(in: VOut) -> @location(0) vec4<f32> {
    let t = textureSample(t_diff, s_diff, in.uv);
    let a = t.a * 0.08 * in.fade * in.along;
    if (!(a > 0.0)) {
        discard;
    }
    let fog = fog_at(in.clip.xy);
    return vec4<f32>(vec3<f32>(1.9921875) * (1.0 - fog.a) + fog.rgb * fog.a, 0.0);
}
