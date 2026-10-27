// Camera-facing particle sprites (tyre smoke, nitrous flames, sparks) from the game's
// effect textures. One instance per particle.

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

// the dynamic lights (render/mod.rs `LightSet`): the lit sprites' four vertex lights
struct LightSet {
    count: vec4<u32>,
    pos_range: array<vec4<f32>, 48>,
    color: array<vec4<f32>, 48>,
    dir: array<vec4<f32>, 48>,
};
@group(0) @binding(1) var<uniform> lights: LightSet;
// the city lamp grid (render/mod.rs `set_lamps`): four lamp numbers per cell, the lamps' table
@group(0) @binding(7) var t_lamp_grid: texture_2d<u32>;
@group(0) @binding(8) var t_lamp_data: texture_2d<f32>;

// What the tone map (post.wgsl) will turn into this display colour: its colour correction
// undone, then its curve on the luminance, then its exposure.
fn to_scene(d: vec3<f32>) -> vec3<f32> {
    let v = d / max(g.grade.rgb, vec3<f32>(0.001));
    let y = clamp(dot(v, vec3<f32>(0.2125, 0.7154, 0.0721)), 0.0, 0.985);
    let w = max(g.tone.y, 0.001);
    let l = 0.5 * w * (sqrt((1.0 - y) * (1.0 - y) + 4.0 * y / w) - (1.0 - y));
    return v * (l / max(y, 0.0001)) / max(g.tone.x, 0.001);
}
@group(1) @binding(0) var t_sprite: texture_2d<f32>;
@group(0) @binding(16) var s_sprite: sampler;
// the projected arrows' four textures, checkpoint_arrows_anim_0..3 one above the other
// (particles.rs `arrow_atlas`; the effect sheets' second texture)
@group(1) @binding(2) var t_arrow: texture_2d<f32>;
// the scene's depth (reversed z), for PSCheckpointProj's soft fade
@group(2) @binding(0) var t_depth: texture_depth_2d;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    // (the quad's own y; the mode: 1 the light column, 2 the checkpoint smoke)
    @location(2) prof: vec2<f32>,
    // (the checkpoint smoke: ShadowColor, night boost)
    @location(3) extra: vec4<f32>,
    // (the checkpoints' low smoke, the projected arrows: the vertex's clip x, y, w; the arrows'
    // rectangle on the screen - xmin, xmax, ymin, ymax; the target's turns, texture, valid;
    // ArrowColorTint and the gain; ProjMapTile, CurrentTime x ProjMapRate)
    @location(4) pc: vec3<f32>,
    @location(5) @interpolate(flat) arect: vec4<f32>,
    @location(6) @interpolate(flat) ab: vec4<f32>,
    @location(7) @interpolate(flat) ac: vec4<f32>,
    @location(8) @interpolate(flat) ad: vec4<f32>,
    // (the sprite's frame A uv - `uv` is frame B - and the blend between them)
    @location(9) uva: vec3<f32>,
    @location(10) @interpolate(flat) soft: f32,
};

@vertex
fn vs_particle(
    @builtin(vertex_index) vi: u32,
    @location(0) pos_size: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv_rect: vec4<f32>,   // u0, v0, du, dv
    @location(3) misc: vec4<f32>,      // rotation, stretch x, stretch y, lit (1 = shaded by daylight; 2 column, 3 checkpoint smoke)
    @location(4) extra: vec4<f32>,
    @location(5) arrow_a: vec4<f32>,
    @location(6) arrow_b: vec4<f32>,
    @location(7) arrow_c: vec4<f32>,
    @location(8) arrow_d: vec4<f32>,
    @location(9) uv2: vec4<f32>,       // frame A's u0, v0; the blend A -> B
    @location(10) quad_u: vec4<f32>,   // DrawDirectional: the quad's half extents (w = 1)
    @location(11) quad_v: vec4<f32>,
    @location(12) base_a: vec4<f32>,   // BaseLight: BaseLightColor, gFallOff
    @location(13) base_b: vec4<f32>,   // BaseLightDir, the base's height (w)
) -> VsOut {
    var o: VsOut;
    let corner = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u)) - vec2<f32>(1.0);
    let to_cam = normalize(g.cam_pos.xyz - pos_size.xyz);
    let right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), to_cam) + vec3<f32>(1e-5, 0.0, 0.0));
    // misc.w 2: the checkpoints' light column (mc4_rmptfx_column's VS_Transform) - turned
    // about the up axis only
    let column = misc.w > 1.5 && misc.w < 2.5;
    let smoke = misc.w > 2.5;
    let up = select(cross(to_cam, right), vec3<f32>(0.0, 1.0, 0.0), column);
    let cs = cos(misc.x);
    let sn = sin(misc.x);
    let c = vec2<f32>(corner.x * misc.y, corner.y * misc.z);
    let r = vec2<f32>(c.x * cs - c.y * sn, c.x * sn + c.y * cs);
    var wp = pos_size.xyz + (right * r.x + up * r.y) * pos_size.w;
    // the lit programs of mc4_rmptfx_mainlist (modes 4, 6) place the corner in the camera's plane
    // (gViewInverse c12, c13), not facing the camera from the particle; the camera's rows from
    // view_proj: x and y rows / their scale, c14 (towards the viewer) = -(w row)
    let lit_ml = (misc.w > 3.5 && misc.w < 4.5) || misc.w > 5.5;
    let cr = normalize(vec3<f32>(g.view_proj[0][0], g.view_proj[1][0], g.view_proj[2][0]));
    let cu = normalize(vec3<f32>(g.view_proj[0][1], g.view_proj[1][1], g.view_proj[2][1]));
    let cb = -normalize(vec3<f32>(g.view_proj[0][3], g.view_proj[1][3], g.view_proj[2][3]));
    var off = (cr * r.x + cu * r.y) * pos_size.w;
    if (lit_ml) {
        wp = pos_size.xyz + off;
    }
    if (quad_u.w > 0.5) {
        // DrawDirectional (`sub_824BBB20`): the four corners worked out on the CPU, no corner
        // offset in the vertex (so none in the lit programs' normal either)
        wp = pos_size.xyz + quad_v.xyz * corner.x + quad_u.xyz * corner.y;
        off = vec3<f32>(0.0);
    }
    o.clip = g.view_proj * vec4<f32>(wp, 1.0);
    o.uv = uv_rect.xy + (corner * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5)) * uv_rect.zw;
    o.uva = vec3<f32>(uv2.xy + (corner * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5)) * uv_rect.zw, uv2.z);
    o.soft = uv2.w;
    // smoke takes the light of the scene, flames and sparks glow on their own
    let light = g.ambient_sky.rgb * 0.9 + g.sun_color.rgb * g.sun_dir.w * 0.35;
    o.color = vec4<f32>(color.rgb * mix(vec3<f32>(1.0), light, select(misc.w, 0.0, misc.w > 1.5)), color.a);
    if ((misc.w > 3.5 && misc.w < 4.5) || misc.w > 5.5) {
        // misc.w 4: mc4_rmptfx_mainlist's vs_main_SoftPart_VertLit - the vertex's normal is its
        // turned corner in the camera's plane plus (gNormalArc + 0.1) towards the camera; misc.w 6:
        // vs_main_SoftPart_TexBlend_VertLit (and _Rescale, the same microcode), the same light
        // (the normal: the corner's offset in metres + (gNormalArc + 0.1) x c14)
        let nrm = normalize(off + cb * (arrow_a.x + 0.1));
        var lit = vert_lit(wp, nrm, extra, arrow_b.xyz, arrow_c.xyz, arrow_b.w);
        if (base_b.w != 0.0 || base_a.w != 0.0) {
            // the _BaseLight programs: + 0.55 x BaseLightColor x sat(gFallOff / (y - h)^2) x
            // sat(n . BaseLightDir x (1 - gTranslucency) + gTranslucency), not scaled by gDiffuse
            let hy = wp.y - base_b.w;
            lit = lit + 0.55 * base_a.rgb * clamp(base_a.w / max(hy * hy, 1e-12), 0.0, 1.0) * clamp(dot(nrm, base_b.xyz) * (1.0 - extra.w) + extra.w, 0.0, 1.0);
        }
        o.color = vec4<f32>(color.rgb * ptx_fog(lit, wp, arrow_d, arrow_c.w), color.a);
    }
    o.prof = vec2<f32>(corner.y, select(select(select(0.0, 1.0, column), 2.0, smoke), misc.w, misc.w > 3.5));
    o.extra = extra;
    // VSCheckpointProj: EffectPos + ProjMapOffset projected, the rectangle ProjMapBounds about
    // it scaled as the billboards are (gAspectRatio = the projection's x and y scales, set from
    // the viewport's matrix +448 / +468 by sub_82478D80)
    o.pc = vec3<f32>(o.clip.x, o.clip.y, o.clip.w);
    let ce = g.view_proj * vec4<f32>(arrow_a.xyz, 1.0);
    let p00 = length(vec3<f32>(g.view_proj[0][0], g.view_proj[1][0], g.view_proj[2][0]));
    let p11 = length(vec3<f32>(g.view_proj[0][1], g.view_proj[1][1], g.view_proj[2][1]));
    let hb = arrow_b.xy * 0.5;
    let cw = select(1.0, ce.w, abs(ce.w) > 1e-6);
    o.arect = vec4<f32>(ce.x - hb.x * p00, ce.x + hb.x * p00, ce.y - hb.y * p11, ce.y + hb.y * p11) / cw;
    o.ab = vec4<f32>(arrow_b.z, arrow_b.w, arrow_d.x, select(0.0, 1.0, ce.w > 0.0));
    // (PSCheckpointProj: the map's gain = EffectPos.w + night x gNightArrowBoost)
    o.ac = vec4<f32>(arrow_c.xyz, arrow_a.w + arrow_d.w * arrow_c.w);
    o.ad = vec4<f32>(arrow_d.y, arrow_d.z, 0.0, 0.0);
    return o;
}

// vs_main_SoftPart_VertLit's light (mc4_rmptfx_mainlist; v = (gDiffuse, gAmbient,
// gAmbientDayControl, gTranslucency) of the rule's shader variables):
//   gGlobalPtxBrightnessScale (0.55) x (gLightAmbient x lerp(gAmbientDayControl, gAmbient, N)
//     + gDiffuse x (gLightColor x sat(n.l + T) + the four dynamic lights)),
// N = sat(4 (|hour / 12 - 1| - 0.25)) (1 at night); a dynamic light: colour x sat(strength / d^2)
// x sat(dot(to light, axis) / d + cone bias) x sat(n.to light / d + T).
// The four lights (the effect's render setup sub_82264968 -> the light picker sub_822F5D98, at the
// effect's cull-sphere centre `ec`): by day none; at night the city lamp grid's four lamps of the
// cell under it (sub_82303900 - the same table as the city's night programs, render/mod.rs
// `set_lamps`; full colour, the table holds half), the player's first head lamp in lane 3.
// gLightAmbient there is the scene's plus the night fill light at the effect (`fill`, particles.rs).
fn lamp_term(i: i32, wp: vec3<f32>, n: vec3<f32>, tr: f32) -> vec3<f32> {
    let at = vec2<i32>((i % 256) * 4, i / 256);
    let p = textureLoad(t_lamp_data, at, 0);
    let c = textureLoad(t_lamp_data, at + vec2<i32>(1, 0), 0);
    let axis = textureLoad(t_lamp_data, at + vec2<i32>(2, 0), 0);
    let bias = textureLoad(t_lamp_data, at + vec2<i32>(3, 0), 0).x;
    let d = p.xyz - wp;
    let inv = inverseSqrt(max(dot(d, d), 1e-6));
    let dn = d * inv;
    return c.rgb * 2.0 * clamp(p.w * inv * inv, 0.0, 1.0) * clamp(dot(dn, axis.xyz) * axis.w + bias, 0.0, 1.0) * clamp(dot(n, dn) * (1.0 - tr) + tr, 0.0, 1.0);
}

// The same program's fog (RMPTFX_FogParams c79 = (height H, -, near, far), FogParams2 c78 = (1 / H,
// 0, 1 / (far - near), -near / (far - near)), FogMaxColor c80 = the fog pass's colour and alpha,
// FogMinColor c81 = `mn`): the share f of the ray from the vertex up to the camera below H, the
// heights of its two ends seen through it, the colour between the min (at H) and the max
// (on the ground) by them, its alpha x sat((d - near) / (far - near)) the amount towards it.
fn ptx_fog(lit: vec3<f32>, wp: vec3<f32>, mn: vec4<f32>, hgt: f32) -> vec3<f32> {
    let dy = wp.y - g.cam_pos.y;
    let dh = wp.xz - g.cam_pos.xz;
    let d = sqrt(dy * dy + dot(dh, dh));
    var r2 = vec2<f32>(1.0, 0.0);
    if (wp.y < hgt) {
        let f = clamp((hgt - wp.y) / (g.cam_pos.y - wp.y), 0.0, 1.0);
        r2 = vec2<f32>(1.0 - f, f);
    }
    let hw = 0.5 * r2.y;
    let h1 = clamp((dy * r2.x + wp.y) / hgt, 0.0, 1.0);
    let h2 = clamp((dy * r2.y + wp.y) / hgt, 0.0, 1.0);
    let a = hw * (h1 + h2) + r2.x;
    let b = hw * ((1.0 - h1) + (1.0 - h2));
    let fc = mn * a + g.fog_color * b;
    let amount = clamp(fc.a * clamp((d - g.params.x) / max(g.params.y - g.params.x, 0.001), 0.0, 1.0), 0.0, 1.0);
    return mix(lit, fc.rgb, amount);
}

fn vert_lit(wp: vec3<f32>, n: vec3<f32>, v: vec4<f32>, ec: vec3<f32>, fill: vec3<f32>, sun_way: f32) -> vec3<f32> {
    let nf = clamp(4.0 * (abs(g.sun_color.w / 12.0 - 1.0) - 0.25), 0.0, 1.0);
    let amb = (g.ambient_sky.rgb + fill) * mix(v.z, v.y, nf);
    // (sat(n.l x (1 - gTranslucency) + gTranslucency): r13.x = adds_prev(1) after maxs(-gTranslucency))
    // (gLightPosDir c16: towards the sun for LightingMode 2 and up, else (0, -1, 0) - `sun_way`)
    let l0 = select(vec3<f32>(0.0, -1.0, 0.0), g.sun_dir.xyz, sun_way > 0.5);
    // (night - after 18:00 or before 06:00 - `sub_822F5D98` sets light 0's colour to zero: no key term,
    // as recorded (c39 = 0 on every particle draw of the night burnout))
    let day = g.sun_color.w >= 6.0 && g.sun_color.w <= 18.0;
    var dl = select(vec3<f32>(0.0), g.sun_color.rgb * g.sun_dir.w * clamp(dot(n, l0) * (1.0 - v.w) + v.w, 0.0, 1.0), day);
    // (night: the light picker's own test - after 18:00 or before 06:00 (obj +14568 / +14564, read
    // live: 18, 6); its third case, the interior flag the garage scenes set (`sub_82304648`),
    // has no lit particle to apply to)
    if (g.sun_color.w > 18.0 || g.sun_color.w < 6.0) {
        // (the player's first head lamp: strength 500, the cone sat(4.4696 cos - 2.872))
        var hl = -1;
        for (var i = 0; i < i32(min(lights.count.x, 48u)); i = i + 1) {
            if (hl < 0 && lights.color[i].w > 1.5 && lights.color[i].w < 2.5) {
                hl = i;
            }
        }
        let uv = ec.xz * vec2<f32>(1.0 / 6320.0, 1.0 / 4463.75) + vec2<f32>(0.54857594, 0.5282554);
        if (uv.x >= 0.0 && uv.x < 1.0 && uv.y >= 0.0 && uv.y < 1.0) {
            let cell = textureLoad(t_lamp_grid, vec2<i32>(uv * vec2<f32>(512.0, 640.0)), 0);
            for (var k = 0; k < select(4, 3, hl >= 0); k = k + 1) {
                let i = i32(cell[k]);
                if (i != 0) {
                    dl = dl + lamp_term(i, wp, n, v.w);
                }
            }
        }
        if (hl >= 0) {
            let d = lights.pos_range[hl].xyz - wp;
            let inv = inverseSqrt(max(dot(d, d), 1e-6));
            let cone = clamp(dot(d, -lights.dir[hl].xyz * 4.4696) * inv - 2.872, 0.0, 1.0);
            dl = dl + lights.color[hl].rgb * clamp(500.0 * inv * inv, 0.0, 1.0) * cone * clamp(dot(n, d) * inv * (1.0 - v.w) + v.w, 0.0, 1.0);
        }
    }
    return 0.55 * (amb + v.x * dl);
}

// The arrows' 128 x 128 target (sub_82262960), worked out per texel instead of drawn: a camera at
// (0, 0, 10) looking down -z, 85 degrees, square, 0.1 .. 20 m, cleared black, and one quad - 12 m
// each way, scaled x 1.1 / x 0.4, turned about z by `z` and then about x by `x` - textured with
// u -0.4 .. 1.4 across, v 1.2 .. -0.2 up, clamped. For the texel centre `px` (pixels from the top
// left): the texture's u, v and 1 where the quad covers it (z < 0: it does not).
const ARROW_TAN: f32 = 0.91633117;
fn arrow_quad(px: vec2<f32>, cz: f32, sz: f32, cx: f32, sx: f32) -> vec3<f32> {
    let ndc = vec2<f32>(px.x / 64.0 - 1.0, 1.0 - px.y / 64.0);
    let dir = vec3<f32>(ndc.x * ARROW_TAN, ndc.y * ARROW_TAN, -1.0);
    // (the quad's plane after the turns: through the origin, normal (0, -sin x, cos x))
    let den = -dir.y * sx + dir.z * cx;
    if (abs(den) < 1e-6) {
        return vec3<f32>(0.0, 0.0, -1.0);
    }
    let t = -(10.0 * cx) / den;
    if (t < 0.1 || t > 20.0) {
        return vec3<f32>(0.0, 0.0, -1.0);
    }
    let p = vec3<f32>(0.0, 0.0, 10.0) + dir * t;
    // (the x turn undone, then the z turn, then the scale)
    let v2 = vec2<f32>(p.x, p.y * cx + p.z * sx);
    let v1 = vec2<f32>(v2.x * cz + v2.y * sz, -v2.x * sz + v2.y * cz);
    let q = vec2<f32>(v1.x / 1.1, v1.y / 0.4);
    if (abs(q.x) > 12.0 || abs(q.y) > 12.0) {
        return vec3<f32>(0.0, 0.0, -1.0);
    }
    return vec3<f32>(-0.4 + 1.8 * (q.x + 12.0) / 24.0, 1.2 - 1.4 * (q.y + 12.0) / 24.0, 1.0);
}

// One texel of the target: the arrow texture (frame `f` of the four) where the quad covers it,
// filtered as the target's draw samples it - linear, mips 0..4, 4:1 anisotropic (fetch dword 3
// 0x06a80d10): `n` taps spread along the footprint's long axis `ax` (in u, v), at level `lod`.
fn arrow_tap(u: vec2<f32>, f: f32, lod: f32) -> vec3<f32> {
    let h = max(64.0 / exp2(lod), 1.0);
    let v = clamp(clamp(u.y, 0.0, 1.0), 0.5 / h, 1.0 - 0.5 / h);
    return textureSampleLevel(t_arrow, s_sprite, vec2<f32>(clamp(u.x, 0.0, 1.0), (f + v) * 0.25), lod).rgb;
}
fn arrow_texel(q: vec3<f32>, f: f32, lod: f32, ax: vec2<f32>, n: f32) -> vec3<f32> {
    if (q.z < 0.0) {
        return vec3<f32>(0.0);
    }
    var c = vec3<f32>(0.0);
    for (var i = 0; i < i32(n); i++) {
        c += arrow_tap(q.xy + ax * ((f32(i) + 0.5) / n - 0.5), f, lod);
    }
    return c / n;
}

// ProjMapSampler at `uv` (bilinear over the target's texels).
fn arrow_map(uv: vec2<f32>, ab: vec4<f32>) -> vec3<f32> {
    let cz = cos(ab.x);
    let sz = sin(ab.x);
    let cx = cos(ab.y);
    let sx = sin(ab.y);
    let st = uv * 128.0 - vec2<f32>(0.5);
    let i0 = floor(st);
    let fr = st - i0;
    let a = clamp(i0, vec2<f32>(0.0), vec2<f32>(127.0)) + vec2<f32>(0.5);
    let b = clamp(i0 + vec2<f32>(1.0), vec2<f32>(0.0), vec2<f32>(127.0)) + vec2<f32>(0.5);
    let q00 = arrow_quad(a, cz, sz, cx, sx);
    let q10 = arrow_quad(vec2<f32>(b.x, a.y), cz, sz, cx, sx);
    let q01 = arrow_quad(vec2<f32>(a.x, b.y), cz, sz, cx, sx);
    let q11 = arrow_quad(b, cz, sz, cx, sx);
    // (the level the target's own draw would take, 4:1 anisotropic: the texture's step from one
    // texel to the next along both axes - n = min(ceil(long / short), 4) taps along the long one,
    // the level of long / n)
    var lod = 0.0;
    var ax = vec2<f32>(0.0);
    var n = 1.0;
    if (q00.z > 0.0 && q10.z > 0.0 && q01.z > 0.0) {
        let dx = q10.xy - q00.xy;
        let dy = q01.xy - q00.xy;
        let lx = length(dx * vec2<f32>(128.0, 64.0));
        let ly = length(dy * vec2<f32>(128.0, 64.0));
        let pmax = max(max(lx, ly), 1e-4);
        let pmin = max(min(lx, ly), 1e-4);
        n = clamp(ceil(pmax / pmin), 1.0, 4.0);
        lod = clamp(log2(pmax / n), 0.0, 4.0);
        ax = select(dy, dx, lx >= ly);
    }
    let c = mix(mix(arrow_texel(q00, ab.z, lod, ax, n), arrow_texel(q10, ab.z, lod, ax, n), fr.x), mix(arrow_texel(q01, ab.z, lod, ax, n), arrow_texel(q11, ab.z, lod, ax, n), fr.x), fr.y);
    return c;
}

// PSCheckpointProj's addition: inside the rectangle, the map x gain x ArrowColorTint.
fn arrow_add(in: VsOut) -> vec3<f32> {
    if (in.ab.w < 0.5 || in.ac.w <= 0.0 || abs(in.pc.z) < 1e-6) {
        return vec3<f32>(0.0);
    }
    let ndc = in.pc.xy / in.pc.z;
    let r = in.arect;
    if (!(ndc.x > r.x && ndc.y > r.z && r.y > ndc.x && r.w > ndc.y)) {
        return vec3<f32>(0.0);
    }
    let s = (ndc.x - r.x) / (r.y - r.x);
    let t = (ndc.y - r.z) / (r.w - r.z);
    // (ProjMapTile x ProjMapTile frames run at CurrentTime x ProjMapRate - 1 x 1 in the data)
    let tile = max(trunc(in.ad.x), 1.0);
    let n = floor(tile * tile);
    let fnum = trunc(in.ad.y);
    let f = trunc(fnum - n * trunc(fnum / n));
    let col = trunc(f - tile * trunc(f / tile));
    let row = trunc(f / tile);
    let uv = vec2<f32>((s + col) / tile, (1.0 - t + row) / tile);
    return max(arrow_map(uv, in.ab) * in.ac.w * in.ac.xyz, vec3<f32>(0.0));
}

// PSCheckpointProj's soft fade: sat(0.1 d^2), d = the scene's view depth at the pixel (its depth
// map through NearFarPlane) - the particle's clip z (the original's projection: near 0.2, far
// 6000, clip z = (view depth - 0.2) x f / (f - n)).
fn soft_fade(in: VsOut) -> f32 {
    if (in.soft <= 0.0) {
        return 1.0;
    }
    let px = vec2<i32>(in.clip.xy);
    let d = textureLoad(t_depth, px, 0);
    let ndc = vec2<f32>(in.clip.x / f32(textureDimensions(t_depth).x) * 2.0 - 1.0, 1.0 - in.clip.y / f32(textureDimensions(t_depth).y) * 2.0);
    // (the scene's view depth = the clip w of the point under the pixel: 1 / w of its unprojection)
    let h = g.inv_view_proj * vec4<f32>(ndc, d, 1.0);
    let zs = select(1e6, 1.0 / h.w, abs(h.w) > 1e-9);
    let zp = (in.pc.z - 0.2) * (6000.0 / 5999.8);
    let dd = zs - zp;
    return clamp(in.soft * dd * dd, 0.0, 1.0);
}

fn tonemap(c: vec3<f32>) -> vec3<f32> {
    let x = c * g.params.z;
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

// alpha blended (smoke, dust)
@fragment
fn fs_particle(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(t_sprite, s_sprite, in.uv);
    let t_a = textureSample(t_sprite, s_sprite, in.uva.xy);
    // (the column's PS_Draw: alpha x (1 - 2 |v - 0.5|) squared)
    if (in.prof.y > 1.5 && in.prof.y < 2.5) {
        // (the checkpoint smoke, mc4_rmptfx_projected's PSCheckpoint: the texture's alpha from
        // frames A and B by the blend, lerp(ShadowColor, colour x boost, it), alpha = sat(2 x it x
        // colour alpha); the projected arrows added on top, unclamped. The output is the scene's
        // own linear value - into the HDR target as the original writes it, not a display colour)
        let ta = mix(t_a.a, t.a, clamp(in.uva.z, 0.0, 1.0));
        let c = clamp(mix(in.extra.rgb, in.color.rgb * in.extra.w, ta), vec3<f32>(0.0), vec3<f32>(1.0)) + arrow_add(in);
        return vec4<f32>(c, clamp(2.0 * ta * in.color.a, 0.0, 1.0) * soft_fade(in));
    }
    if (in.prof.y > 3.5 && (in.prof.y < 4.5 || in.prof.y > 5.5)) {
        // (modes 4 and 6 alpha-blended, ps_main_SoftPart_VertLit / ps_main_SoftPart_TexBlend_VertLit:
        // rgb = the texture (6: frames A and B by the blend) x the lit vertex colour, alpha = its
        // alpha x the colour's x sat(4 d^2); the scene's linear value)
        let tb = select(t, mix(t_a, t, clamp(in.uva.z, 0.0, 1.0)), in.prof.y > 5.5);
        return vec4<f32>(tb.rgb * in.color.rgb, tb.a * in.color.a * soft_fade(in));
    }
    if (in.prof.y > 4.5) {
        // (mode 5, mc4_rmptfx_default's PS_Draw: frames A and B by the blend, times the colour;
        // alpha x the soft fade; the scene's linear value)
        let tb = mix(t_a, t, clamp(in.uva.z, 0.0, 1.0));
        return vec4<f32>(tb.rgb * in.color.rgb, tb.a * in.color.a * soft_fade(in));
    }
    let pr = 1.0 - abs(in.prof.x);
    let a = t.a * in.color.a * mix(1.0, pr * pr, in.prof.y);
    if (in.prof.y > 0.5) {
        // (the column's PS_Draw takes only the texture's alpha; its colour is the particle's -
        // the scene's linear value)
        return vec4<f32>(in.color.rgb, a);
    }
    return vec4<f32>(to_scene(tonemap(t.rgb * in.color.rgb)), a);
}

// additive (flames, sparks, glows)
@fragment
fn fs_particle_add(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(t_sprite, s_sprite, in.uv);
    let t_a = textureSample(t_sprite, s_sprite, in.uva.xy);
    if (in.prof.y > 3.5) {
        // (modes 4 and 5 add the scene's linear value, as SRC_ALPHA / ONE: the lit sprite's
        // ps_main_SoftPart_VertLit takes one frame, mc4_rmptfx_default's PS_Draw blends A and B)
        let tb = select(t, mix(t_a, t, clamp(in.uva.z, 0.0, 1.0)), in.prof.y > 4.5);
        return vec4<f32>(tb.rgb * in.color.rgb * (tb.a * in.color.a * soft_fade(in)), 1.0);
    }
    return vec4<f32>(to_scene(min(t.rgb * in.color.rgb * t.a * in.color.a, vec3<f32>(0.98))), 1.0);
}
