// The skid marks (skidmarks.rs), from the original's SkidMark.fx (cars/0x53bcb23c, technique "draw"), appended to
// sst_trail.wgsl (its globals, the scene's depth / stencil and `fog_at`):
//   VS_SkidMark: oPos = position x gWorldViewProj (the identity world: the rings are in the world), o0 = the
//   texture coordinates, o1 = the vertex colour
//   PS_SkidMark: t = DiffuseSampler(o0); rgb = t.rgb x o1.rgb; a = gLightAmbient.w x t.a x gInvColorExpBias.x x o1.a
// (gLightAmbient.w 1, recorded; the scene target's exponent bias takes gInvColorExpBias back), blended
// SRCALPHA / INVSRCALPHA. The original's fog pass after it lerps the target T to the fog colour F by the share f
// of the scene's depth under the pixel: (S (1 - a) + M a)(1 - f) + F f. The port's scene already holds
// S (1 - f) + F f, so the mark gives M (1 - f) + F f with the alpha a - the same value.

// (the texture and its sampler: sst_trail.wgsl's group 1 bindings 0 and 1, `t_diff` / `s_diff`)

struct SkidOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) colour: vec4<f32>,
};

@vertex
fn vs_skid(@location(0) pos: vec3<f32>, @location(1) bgra: vec4<f32>, @location(2) uv: vec2<f32>) -> SkidOut {
    var o: SkidOut;
    o.clip = g.view_proj * vec4<f32>(pos, 1.0);
    o.uv = uv;
    o.colour = bgra.zyxw;
    return o;
}

@fragment
fn fs_skid(in: SkidOut) -> @location(0) vec4<f32> {
    let t = textureSample(t_diff, s_diff, in.uv);
    let m = t.rgb * in.colour.rgb;
    let a = t.a * in.colour.a;
    let fog = fog_at(in.clip.xy);
    return vec4<f32>(m + (fog.rgb - m) * fog.a, a);
}
