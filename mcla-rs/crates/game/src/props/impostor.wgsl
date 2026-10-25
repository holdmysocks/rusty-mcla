// The generation pass of a plant impostor (work/map/render_rule.md 3c): the type's model seen by
// the tile camera, into the type's 128 x 128 colour picture (target 0) and normal picture (target 1).
//   day    leaves  VSPropFoliageGenImpostorDay + PSGenerateImposterDay:
//                  texture x 0.75 x occlusion x (gLightAmbient + gLightColor x (0.1 + 0.9 sat(n.l)))
//          trunk   PSGenerateImposter: texture x occlusion x (gLightAmbient + gLightColor x n.l)
//   night  PSGenerateImposterNight: texture x sat(occlusion + 0.3) (trunk: x occlusion); no light
// alpha 1, the pixel killed where the texture's alpha is under 0.2; the normal picture holds
// 0.5 n + 0.5. The sun's shadow is not in the picture: the card's pixel program applies it.

struct Tile {
    view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,   // xyz towards the light, w = 1: night
    sun_col: vec4<f32>,   // gLightColor x its strength, w = OcclusionInfo.w (the port's lift of the occlusion)
    ambient: vec4<f32>,   // gLightAmbient
};
@group(0) @binding(0) var<uniform> tile: Tile;
@group(1) @binding(0) var t_diffuse: texture_2d<f32>;
@group(2) @binding(0) var s_diffuse: sampler;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv0: vec2<f32>,
    @location(5) color1: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv0: vec2<f32>,
    @location(2) occ: f32,
};

@vertex
fn vs_tile(in: VsIn) -> VsOut {
    var o: VsOut;
    o.clip = tile.view_proj * vec4<f32>(in.pos, 1.0);
    o.normal = in.normal;
    o.uv0 = in.uv0;
    o.occ = mix(in.color1.a, 1.0, tile.sun_col.w);
    return o;
}

struct FsOut {
    @location(0) color: vec4<f32>,
    @location(1) normal: vec4<f32>,
};

// The shadow impostor's mask (render_rule.md 3d; VSPropFoliageGenImpostorShadow +
// PSGenerateImposterShadowDepth): 1 where the texture's alpha is at least 2 / 3 on the leaves,
// 0.2 on the trunk; the surface is multisampled, so the resolved edge is a fraction.
@fragment
fn fs_mask_leaf(in: VsOut) -> @location(0) vec4<f32> {
    if (textureSample(t_diffuse, s_diffuse, in.uv0).a < 0.66667) {
        discard;
    }
    return vec4<f32>(1.0);
}

@fragment
fn fs_mask_trunk(in: VsOut) -> @location(0) vec4<f32> {
    if (textureSample(t_diffuse, s_diffuse, in.uv0).a < 0.2) {
        discard;
    }
    return vec4<f32>(1.0);
}

// (two pipelines: the leaves' program and the trunk's)
@fragment
fn fs_leaf(in: VsOut) -> FsOut {
    return tile_pixel(in, false);
}

@fragment
fn fs_trunk(in: VsOut) -> FsOut {
    return tile_pixel(in, true);
}

// The console's decoding of a "gamma" texture - four straight pieces on the stored value (the
// same as city.wgsl `xenos_gamma`; the sample arrives sRGB-decoded and is encoded again first).
fn xenos_gamma(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let e = select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055), x * 12.92, x <= vec3<f32>(0.0031308)) * 255.0;
    let k = select(select(select(vec3<f32>(1.0), vec3<f32>(2.0), e >= vec3<f32>(64.0)), vec3<f32>(4.0), e >= vec3<f32>(96.0)), vec3<f32>(8.0), e >= vec3<f32>(192.0));
    let off = select(select(select(vec3<f32>(0.0), vec3<f32>(-64.0), e >= vec3<f32>(64.0)), vec3<f32>(-256.0), e >= vec3<f32>(96.0)), vec3<f32>(-1024.0), e >= vec3<f32>(192.0));
    var l = e * k + off;
    l = l + trunc(l * k / 1024.0);
    return l / 1023.0;
}

fn tile_pixel(in: VsOut, trunk: bool) -> FsOut {
    var t = textureSample(t_diffuse, s_diffuse, in.uv0);
    if (t.a < 0.2) {
        discard;
    }
    // (2026-10-14: the generation programs' sampler 0 carries the gamma flag - fetch word
    // 0x810000fe on PSGenerateImposterDay, PSGenerateImposter and PSGenerateImposterNight - so
    // the texture is decoded the console's piecewise way here, and the card's own picture is then
    // read as it is: its fetch word 0x81024802 has no flag, city.wgsl leaves it alone.)
    let old_pic = tile.sun_dir.w > 1.5;
    let night_pic = (tile.sun_dir.w > 0.5 && tile.sun_dir.w < 1.5) || tile.sun_dir.w > 2.5;
    if (!old_pic) { t = vec4<f32>(xenos_gamma(t.rgb), t.a); }
    let n = normalize(select(in.normal, vec3<f32>(0.0, 1.0, 0.0), dot(in.normal, in.normal) < 1e-6));
    var light: vec3<f32>;
    if (night_pic) {
        light = vec3<f32>(select(clamp(in.occ + 0.3, 0.0, 1.0), in.occ, trunk));
    } else {
        let nl = max(dot(n, tile.sun_dir.xyz), 0.0);
        if (trunk) {
            light = in.occ * (tile.ambient.rgb + tile.sun_col.rgb * nl);
        } else {
            light = 0.75 * in.occ * (tile.ambient.rgb + tile.sun_col.rgb * (0.1 + 0.9 * nl));
        }
    }
    var o: FsOut;
    o.color = vec4<f32>(t.rgb * light, 1.0);
    // (the original's picture is an 8888 surface holding LINEAR values - by day 1 .. 4 of 255
    // on most leaves, read out of the running game: work/lighting/carddump.py - so every sample
    // is one of those steps: the dark undersides fall to 0, which is what gives the far bushes
    // their black depths)
    if (!old_pic) { o.color = vec4<f32>(floor(clamp(o.color.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0 + vec3<f32>(0.5)) / 255.0, 1.0); }
    o.normal = vec4<f32>(n * 0.5 + vec3<f32>(0.5), 1.0);
    return o;
}
