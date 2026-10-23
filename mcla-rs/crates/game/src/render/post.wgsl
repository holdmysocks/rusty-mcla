// Post-processing as the original's rage_postfx effect does it (read from its microcode,
// LIGHTING_WORK.md): the scene is HDR and linear; its luminance is averaged in log space and
// adapted over time, the bright pass and two 9-tap Gauss passes make the bloom at quarter size,
// and the tone map exposes, compresses, adds the bloom, desaturates and colour-corrects.
// (The tone map still holds the port's old speed-dependent radial blur; main.rs feeds it 0 since the original's
// motion blur - fs_dof's streaks - is ported.)

struct P {
    // blur passes: xy = one texel of the source
    // adapt: x = 1 - exp(-dt * Lambda), y = 1 on the first frame (no history); b.xy = one texel of the source
    // bright / tone map: x = middle grey, y = threshold, z = offset, w = white (squared)
    a: vec4<f32>,
    // bright / tone map: x = lower, y = upper limit of the adapted luminance, z = tone map white (squared), w = saturation
    b: vec4<f32>,
    // tone map: rgb = colour correction (the shader doubles it), w = radial blur amount
    c: vec4<f32>,
    // tone map: xy = blur centre in uv, z = aspect ratio, w = 1: skip bloom and blur (--no-post),
    // w = 2 + a luminance: that adapted luminance instead of the measured one (test hook)
    d: vec4<f32>,
    // tone map: x = 1: the depth of field's buffer (t4) is there, y = MCLA_DEBUG_VIEW 500 .. 519, zw = one
    // pixel of the original's 1280 x 720 frame in uv
    e: vec4<f32>,
    // tone map: x = 1: no bloom (OPTIONS > DISPLAY > BLOOM off)
    f: vec4<f32>,
};

@group(0) @binding(0) var t0: texture_2d<f32>;
@group(0) @binding(1) var t1: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> p: P;
@group(0) @binding(4) var t2: texture_2d<f32>;
// the lamps' glow pass (halo.wgsl): what the original adds to its back buffer after the tone map
@group(0) @binding(5) var t3: texture_2d<f32>;
// the depth of field's buffer (fs_dof): rgb = the scene blurred, a = its weight / 4
@group(0) @binding(6) var t4: texture_2d<f32>;

// fs_dof's own bindings
struct DofU {
    inv_view_proj: mat4x4<f32>,
    // xyz = the camera, w = MCLA_DEBUG_VIEW 500 .. 519
    cam: vec4<f32>,
    // m_DoFParams: x = where the blur begins (m), y = its ramp (m), z = scale
    dof: vec4<f32>,
    // xy = one pixel of the original's 1280 x 720 frame in uv; z = 1: the motion part on; w = 1: the depth of
    // field's weight on
    px: vec4<f32>,
    // Mc4MotionBlurVehicleMtxs: world -> the previous frame's clip space, per stencil id / 4 (render/mb.rs)
    mtx: array<mat4x4<f32>, 24>,
};
@group(0) @binding(10) var d_scene: texture_2d<f32>;
@group(0) @binding(11) var d_depth: texture_depth_2d;
@group(0) @binding(12) var d_samp: sampler;
@group(0) @binding(13) var<uniform> du: DofU;
// the stencil copy (R8Unorm = s / 255, as the original's 8888 copy; read bilinearly like its StencilSampler)
@group(0) @binding(14) var d_sten: texture_2d<f32>;
// PSInitialScaleBuffer's 640 x 360 copy (fs_copy)
@group(0) @binding(15) var d_copy: texture_2d<f32>;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_post(@builtin(vertex_index) vi: u32) -> VsOut {
    var o: VsOut;
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    o.clip = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, y);
    return o;
}

// A value as the console's 10-bit float colour format (7 bits of mantissa, 3 of exponent, the
// scene target's x 16) keeps it: steps of 1 / 8192 below 1 / 64. Round to nearest, as the
// reference's resolve packs it. (Debug views 503 / 504 only.)
fn q7e3(c: vec3<f32>) -> vec3<f32> {
    let s = clamp(c * 16.0, vec3<f32>(0.0), vec3<f32>(31.875));
    let st = exp2(floor(log2(max(s, vec3<f32>(0.25))))) / 128.0;
    return round(s / st) * st / 16.0;
}

fn dof_texel(uv: vec2<f32>) -> vec2<i32> {
    let dims = vec2<i32>(textureDimensions(d_scene));
    return clamp(vec2<i32>(floor(uv * vec2<f32>(dims))), vec2<i32>(0), dims - vec2<i32>(1));
}

// PSStreakMotionBlur as it works out in a standing frame (rage_postfx a6b9b6b0fa692c54, run in
// numpy from its listing: work/lighting/df_run.py). The target is 640 x 360 beside the 1280 x 720
// scene; with `v` one scene pixel and the buffer's pixel centre on the corner of four of them:
//   rgb = (scene's pixel right below that corner (a point tap)
//          + 8 x the mean of the 4 x 4 scene pixels round the corner) / 9
//     - the eight taps of the 640 x 360 copy (PSInitialScaleBuffer: one bilinear tap a scene pixel
//     down-right of the corner = a 2 x 2 box) all fall on uv - half a texel of that copy while the
//     velocity is zero: a bilinear tap on the corner of four of its texels;
//   a   = sat(sat((d - DofParams.x) / DofParams.y) x DofParams.z), d the distance from the camera to
//     the point at that same scene pixel; 0 where the stencil is 1 - the sky and its ground dome
//     (read with a bilinear tap on the corner: two of the four pixels being sky are enough).
// Left out: the velocity's share of `a` and of the taps' spread (a moving camera's streaks), the
// taps' weights by stencil (other cars' outlines), blurEffectDesatScale (the abilities' effect).
// PSInitialScaleBuffer as the streak pass reads it: texel m = one bilinear tap a scene pixel down-right of its
// centre (the 2 x 2 box of scene pixels 2m+1, 2m+2; LIGHTING.md, depth of field (a)).
@fragment
fn fs_copy(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(max(textureSampleLevel(d_scene, d_samp, in.uv + du.px.xy, 0.0).rgb, vec3<f32>(0.0)), 1.0);
}

// PSStreakMotionBlur (rage_postfx a6b9b6b0fa692c54), the whole program - run in numpy from its listing and checked
// against this reading (work/ui-sharp/agent_rend/mb_check.py, mb_pair.py):
//   s   = the stencil copy at uv (bilinear, s / 255); sky = s in [1/512, 1.5/256) (stencil 1); k = trunc(trunc(256 s) / 4)
//   P   = this pixel's point (inverse view-projection, the depth at the point tap); v = uv - its place under
//         matrix k (the previous frame's, render/mb.rs), in uv units
//   M   = sat(38 x min(|v|^1.25, 0.02625));  B = M / 38 x v / |v|
//   seven taps of the 640 copy at uv + k B - half its texel, k = -0.5, 0.5, -0.375, -0.25, 0.25, -0.125, 0 (0 twice),
//   weighted by eight stencil taps at uv + j B / 8 - half a scene pixel (j = -4..-1, 1..4; tap j belongs to the copy
//   tap j / 8, except j = 1 and 3: the k = 0 ones, j = 2: 0.25, j = 4: 0.5): weight 0 where the tap's stencil is
//   1/64 or more above the pixel's (a vehicle drawn after this one), else 1
//   rgb = (the scene's point tap + sum of the weighted copy taps) / (1 + sum of the weights)
//   a   = sat(M + (sky ? 0 : A)), A the depth of field's weight below.
// Left out: blurEffectDesatScale / TintFade (c167 / c166: 0 / 1 in every capture; an effect blend towards the
// screen's centre, not traced).
fn copy_tap(p: vec2<f32>) -> vec3<f32> {
    return max(textureSampleLevel(d_copy, d_samp, p, 0.0).rgb, vec3<f32>(0.0));
}

fn sten_w(p: vec2<f32>, s: f32) -> f32 {
    return select(0.0, 1.0, textureSampleLevel(d_sten, d_samp, p, 0.0).r - s < 0.015625);
}

@fragment
fn fs_dof(in: VsOut) -> @location(0) vec4<f32> {
    let view = du.cam.w;
    if (view < 502.5 || view > 505.5) {
        return streak(in.uv);
    }
    return dof_static(in);
}

fn streak(uv: vec2<f32>) -> vec4<f32> {
    let v = du.px.xy;
    let view = du.cam.w;
    let sharp = uv + 0.5 * v;
    let s = textureSampleLevel(d_sten, d_samp, uv, 0.0).r;
    let sky = s >= 0.001953125 && s < 0.005859375;
    let k = min(u32(trunc(trunc(s * 256.0) * 0.25)), 23u);
    let z = textureLoad(d_depth, dof_texel(sharp), 0);
    let ndc = vec4<f32>(sharp.x * 2.0 - 1.0, 1.0 - sharp.y * 2.0, z, 1.0);
    let w = du.inv_view_proj * vec4<f32>(ndc.xy, max(z, 1e-9), 1.0);
    // the motion: the point under the previous frame's matrix (homogeneous: the sky's far points too)
    var m = 0.0;
    var b = vec2<f32>(0.0);
    if (du.px.z > 0.5) {
        let pc = du.mtx[k] * du.inv_view_proj * ndc;
        if (pc.w > 1e-12) {
            let prev = vec2<f32>(pc.x / pc.w * 0.5 + 0.5, 0.5 - pc.y / pc.w * 0.5);
            let vel = sharp - prev;
            let l = length(vel);
            if (l > 0.0) {
                m = clamp(38.0 * min(pow(l, 1.25), 0.02625), 0.0, 1.0);
                b = (m / 38.0) * vel / l;
            }
        }
    }
    let h = v;
    let q_m50 = copy_tap(uv - 0.5 * b - h);
    let q_p50 = copy_tap(uv + 0.5 * b - h);
    let q_m375 = copy_tap(uv - 0.375 * b - h);
    let q_m25 = copy_tap(uv - 0.25 * b - h);
    let q_p25 = copy_tap(uv + 0.25 * b - h);
    let q_m125 = copy_tap(uv - 0.125 * b - h);
    let q_0 = copy_tap(uv - h);
    let st = uv - 0.5 * v;
    let w_m4 = sten_w(st - 0.5 * b, s);
    let w_m3 = sten_w(st - 0.375 * b, s);
    let w_m2 = sten_w(st - 0.25 * b, s);
    let w_m1 = sten_w(st - 0.125 * b, s);
    let w_p1 = sten_w(st + 0.125 * b, s);
    let w_p2 = sten_w(st + 0.25 * b, s);
    let w_p3 = sten_w(st + 0.375 * b, s);
    let w_p4 = sten_w(st + 0.5 * b, s);
    let f = max(textureLoad(d_scene, dof_texel(sharp), 0).rgb, vec3<f32>(0.0));
    let sum = f + w_m4 * q_m50 + w_m3 * q_m375 + w_m2 * q_m25 + w_m1 * q_m125 + (w_p1 + w_p3) * q_0 + w_p2 * q_p25 + w_p4 * q_p50;
    var rgb = sum / (1.0 + w_m4 + w_m3 + w_m2 + w_m1 + w_p1 + w_p2 + w_p3 + w_p4);
    if (view > 506.5 && view < 507.5) {
        rgb = q_0;
    }
    // the depth of field's weight (DofParams = m_DoFParams)
    let d = length(w.xyz / w.w - du.cam.xyz);
    var a = clamp((d - du.dof.x) / max(du.dof.y, 1e-3), 0.0, 1.0) * du.dof.z;
    if (du.px.w < 0.5 || (sky && !(view > 505.5 && view < 506.5))) {
        a = 0.0;
    }
    return vec4<f32>(rgb, clamp(m + a, 0.0, 1.0));
}

// The standing frame's reading as it was (MCLA_DEBUG_VIEW 503 - 505 only: the 7e3 tests and the taps without the
// quads' half-pixel shift).
fn dof_static(in: VsOut) -> vec4<f32> {
    let v = du.px.xy;
    let view = du.cam.w;
    let uv = in.uv;
    var sharp = uv + 0.5 * v;
    var q = vec3<f32>(0.0);
    if (view > 504.5 && view < 505.5) {
        // (505: the quads' coordinates taken without their half-pixel shift - the point taps fall on
        // the pixel up-left of the corner's block, the blurred part is the 2 x 2 box up-left of it)
        sharp = uv - 0.5 * v;
        q = textureSampleLevel(d_scene, d_samp, uv - 2.0 * v, 0.0).rgb;
    } else if (view > 503.5 && view < 504.5) {
        // (504: every scene pixel through the 7e3 format first - exact only in a 1280 x 720 window)
        let t0i = dof_texel(uv - 1.5 * v);
        let dims = vec2<i32>(textureDimensions(d_scene));
        for (var y = 0; y < 4; y = y + 1) {
            for (var x = 0; x < 4; x = x + 1) {
                q = q + q7e3(max(textureLoad(d_scene, clamp(t0i + vec2<i32>(x, y), vec2<i32>(0), dims - vec2<i32>(1)), 0).rgb, vec3<f32>(0.0)));
            }
        }
        q = q / 16.0;
    } else {
        q = textureSampleLevel(d_scene, d_samp, uv + vec2<f32>(-v.x, -v.y), 0.0).rgb + textureSampleLevel(d_scene, d_samp, uv + vec2<f32>(v.x, -v.y), 0.0).rgb
            + textureSampleLevel(d_scene, d_samp, uv + vec2<f32>(-v.x, v.y), 0.0).rgb + textureSampleLevel(d_scene, d_samp, uv + vec2<f32>(v.x, v.y), 0.0).rgb;
        q = q * 0.25;
    }
    q = max(q, vec3<f32>(0.0));
    var f = max(textureLoad(d_scene, dof_texel(sharp), 0).rgb, vec3<f32>(0.0));
    if (view > 502.5 && view < 504.5) {
        // (503 / 504: the 640 x 360 copy is a 7e3 target of its own)
        q = q7e3(q);
    }
    if (view > 503.5 && view < 504.5) {
        f = q7e3(f);
    }
    var rgb = (f + 8.0 * q) / 9.0;
    if (view > 506.5 && view < 507.5) {
        rgb = q;
    }
    let z = textureLoad(d_depth, dof_texel(sharp), 0);
    let w = du.inv_view_proj * vec4<f32>(sharp.x * 2.0 - 1.0, 1.0 - sharp.y * 2.0, max(z, 1e-9), 1.0);
    let d = length(w.xyz / w.w - du.cam.xyz);
    var a = clamp((d - du.dof.x) / max(du.dof.y, 1e-3), 0.0, 1.0) * du.dof.z;
    // the sky: nothing drawn into the depth buffer (it is cleared to 0, reversed)
    var sky = 0.0;
    for (var y = 0; y < 2; y = y + 1) {
        for (var x = 0; x < 2; x = x + 1) {
            sky = sky + select(0.0, 1.0, textureLoad(d_depth, dof_texel(uv + (vec2<f32>(f32(x), f32(y)) - vec2<f32>(0.5)) * v), 0) <= 0.0);
        }
    }
    if (sky > 1.5 && !(view > 505.5 && view < 506.5)) {
        a = 0.0;
    }
    return vec4<f32>(rgb, clamp(a, 0.0, 1.0));
}

// PSInitialScaleBuffer / PSScaleBuffer: half and quarter size copies (a 2x2 box each: the
// bilinear tap at the centre of four texels)
@fragment
fn fs_scale(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(max(textureSampleLevel(t0, samp, in.uv, 0.0).rgb, vec3<f32>(0.0)), 1.0);
}

const LUMINANCE: vec3<f32> = vec3<f32>(0.2125, 0.7154, 0.0721);

// PSSampleAverageLuminance: the mean of log(luminance + 0.0001) over 3x3 taps
@fragment
fn fs_lum_log(in: VsOut) -> @location(0) vec4<f32> {
    var s = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let c = textureSampleLevel(t0, samp, in.uv + vec2<f32>(f32(x), f32(y)) * p.a.xy, 0.0).rgb;
            s = s + log(dot(max(c, vec3<f32>(0.0)), LUMINANCE) + 0.0001);
        }
    }
    return vec4<f32>(s / 9.0, 0.0, 0.0, 1.0);
}

// PSReSampleAverageLuminance: 4x4 taps averaged
fn lum_16(uv: vec2<f32>, texel: vec2<f32>) -> f32 {
    var s = 0.0;
    for (var y = 0; y < 4; y = y + 1) {
        for (var x = 0; x < 4; x = x + 1) {
            s = s + textureSampleLevel(t0, samp, uv + (vec2<f32>(f32(x), f32(y)) - vec2<f32>(1.5)) * texel, 0.0).r;
        }
    }
    return s / 16.0;
}

@fragment
fn fs_lum_down(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(lum_16(in.uv, p.a.xy), 0.0, 0.0, 1.0);
}

// PSReSampleAverageLuminanceExp followed by PSCalculateAdaptedLuminance: the scene's
// luminance of this frame, and the adapted one moving towards it
@fragment
fn fs_lum_adapt(in: VsOut) -> @location(0) vec4<f32> {
    let now = exp(lum_16(in.uv, p.b.xy));
    let before = textureSampleLevel(t1, samp, vec2<f32>(0.5), 0.0).r;
    let v = mix(before + (now - before) * p.a.x, now, p.a.y);
    return vec4<f32>(clamp(v, 0.0, 64.0), 0.0, 0.0, 1.0);
}

// The game's RGB <-> XYZ (its literals)
fn to_xyz(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(c, vec3<f32>(0.5141364, 0.3238786, 0.16036376)), dot(c, vec3<f32>(0.265068, 0.67023426, 0.06409157)), dot(c, vec3<f32>(0.0241188, 0.1228178, 0.8444266)));
}

fn from_xyz(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(c, vec3<f32>(2.5651, -1.1665, -0.3986)), dot(c, vec3<f32>(-1.0217, 1.9777, 0.0439)), dot(c, vec3<f32>(0.0753, -0.2543, 1.1892)));
}

// the adapted luminance as both passes use it: clamped between the two limits
// (p.d.w >= 2: a test value forced from outside, in its fraction x 100 ... see post.rs)
fn adapted() -> f32 {
    if (p.d.w >= 2.0) {
        return clamp(p.d.w - 2.0, p.b.x, p.b.y);
    }
    return clamp(textureSampleLevel(t1, samp, vec2<f32>(0.5), 0.0).r, p.b.x, p.b.y);
}

// PSBrightPass
@fragment
fn fs_bright(in: VsOut) -> @location(0) vec4<f32> {
    let c = max(textureSampleLevel(t0, samp, in.uv, 0.0).rgb, vec3<f32>(0.0));
    let xyz = to_xyz(c);
    // (the original puts 0.001 where the sum or y is 0 - work/ui-sharp/agent_tone)
    let sum = select(xyz.x + xyz.y + xyz.z, 0.001, xyz.x + xyz.y + xyz.z == 0.0);
    let l = xyz.y * p.a.x / (adapted() + 0.001);
    let over = max(l * (1.0 + l / p.a.w) - p.a.y, 0.0);
    let ld = over / (p.a.z + over);
    let xy = xyz.xy / sum;
    let yy = select(xy.y, 0.001, xy.y == 0.0);
    let big = vec3<f32>(ld * xy.x / yy, ld, ld * (1.0 - xy.x - xy.y) / yy);
    return vec4<f32>(clamp(from_xyz(big), vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

// PSGaussX / PSGaussY: nine taps a texel apart
@fragment
fn fs_gauss(in: VsOut) -> @location(0) vec4<f32> {
    let d = p.a.xy;
    var c = textureSampleLevel(t0, samp, in.uv, 0.0).rgb * 0.25;
    c = c + (textureSampleLevel(t0, samp, in.uv + d, 0.0).rgb + textureSampleLevel(t0, samp, in.uv - d, 0.0).rgb) * 0.15;
    c = c + (textureSampleLevel(t0, samp, in.uv + d * 2.0, 0.0).rgb + textureSampleLevel(t0, samp, in.uv - d * 2.0, 0.0).rgb) * 0.10;
    c = c + (textureSampleLevel(t0, samp, in.uv + d * 3.0, 0.0).rgb + textureSampleLevel(t0, samp, in.uv - d * 3.0, 0.0).rgb) * 0.08;
    c = c + (textureSampleLevel(t0, samp, in.uv + d * 4.0, 0.0).rgb + textureSampleLevel(t0, samp, in.uv - d * 4.0, 0.0).rgb) * 0.05;
    return vec4<f32>(c, 1.0);
}

// PSToneMap. t0 = the scene, t1 = adapted luminance, t2 = bloom. The target is an sRGB surface:
// what is written here is the linear value the original then encodes itself.
@fragment
fn fs_tonemap(in: VsOut) -> @location(0) vec4<f32> {
    let dir = in.uv - p.d.xy;
    let r = length(dir * vec2<f32>(p.d.z, 1.0));
    let amt = p.c.w * smoothstep(0.10, 0.75, r);
    var c = vec3<f32>(0.0);
    let plain = p.d.w > 0.5 && p.d.w < 1.5;
    if (amt > 0.0005 && !plain) {
        for (var i = 0; i < 10; i = i + 1) {
            c = c + textureSampleLevel(t0, samp, in.uv - dir * amt * (f32(i) / 9.0), 0.0).rgb;
        }
        c = c * 0.1;
    } else {
        c = textureSampleLevel(t0, samp, in.uv, 0.0).rgb;
    }
    c = max(c, vec3<f32>(0.0));
    // PSToneMap instr. 6 - 9: the scene lerped towards the depth of field's buffer (640 x 360, one
    // bilinear tap) by sat(4 x its alpha) - from 15 m beyond m_DoFParams.x the buffer alone. The bloom
    // and the luminance were taken from the scene before this.
    if (p.e.x > 0.5) {
        // The tap stands half a scene pixel up-left of the pixel's centre (the tone map's quad has its
        // coordinates on the pixels' corners; the buffer's own passes have theirs on the centres). NOT
        // read from the quads' vertices (no capture holds them): of the eight ways the three passes can
        // stand, this one gives the far tower's gradients as the screenshots have them (df_phase.py,
        // df_px2.py: x 0.97 across, x 0.92 down; with the tap on the centre x 0.82 / x 0.80).
        // MCLA_DEBUG_VIEW=508: on the centre.
        var duv = in.uv - 0.5 * p.e.zw;
        if (p.e.y > 507.5 && p.e.y < 508.5) {
            duv = in.uv;
        }
        if (p.e.y > 503.5 && p.e.y < 504.5) {
            c = q7e3(c);
        }
        let far = textureSampleLevel(t4, samp, duv, 0.0);
        var wd = clamp(far.a * 4.0, 0.0, 1.0);
        if (p.e.y > 501.5 && p.e.y < 502.5) {
            wd = 1.0;
        }
        c = mix(c, max(far.rgb, vec3<f32>(0.0)), wd);
        if (p.e.y > 500.5 && p.e.y < 501.5) {
            c = vec3<f32>(0.002 + 0.1 * wd);
        }
    }
    let xyz = to_xyz(c);
    let sum = select(xyz.x + xyz.y + xyz.z, 0.001, xyz.x + xyz.y + xyz.z == 0.0);
    var o = vec3<f32>(0.0);
    {
        let l = xyz.y * p.a.x / (adapted() + 0.001);
        let ld = l * (1.0 + l / p.b.z) / (1.0 + l);
        let xy = xyz.xy / sum;
        let yy = select(xy.y, 0.001, xy.y == 0.0);
        o = from_xyz(vec3<f32>(ld * xy.x / yy, ld, ld * (1.0 - xy.x - xy.y) / yy));
    }
    if (!plain && p.f.x < 0.5) {
        o = o + textureSampleLevel(t2, samp, in.uv, 0.0).rgb;
    }
    let luma = dot(o, LUMINANCE);
    o = mix(vec3<f32>(luma), o, p.b.w);
    o = clamp(o * 2.0 * p.c.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    // The display ramp. The game writes sRGB-encoded values and the console sends every
    // presented pixel through a 1024-entry table the game builds itself (sub_82426B68, at
    // guest 0x828CDA48; read from the running game: 64 -> 48, 128 -> 115, 192 -> 185 of 255):
    // it re-encodes sRGB as BT.709, the display gamma the kernel reports. Shadows come out
    // deeper than the tone map alone leaves them. What is written here is the linear value
    // whose sRGB encoding (the surface's) is that BT.709 signal.
    // The game's own encode is NOT the exact sRGB curve: PSToneMap ends in
    // `x <= 0.00304 ? 12.92 x : 1.055 x^Gamma - 0.055` with Gamma (c70) = 0.4545 where sRGB has
    // 1 / 2.4 (work/lighting/tm_cmp.py runs the program: grey 0.02 at adapted 0.2 gives 42,
    // the sRGB curve 50). Bring `o` to the linear value whose sRGB encoding the game writes.
    let enc0 = select(1.055 * pow(o, vec3<f32>(0.45455)) - vec3<f32>(0.055), o * 12.92, o <= vec3<f32>(0.00304));
    // (the lamps' glow pass lands here: added to the encoded frame, before the display ramp)
    let enc = clamp(enc0 + textureSampleLevel(t3, samp, in.uv, 0.0).rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    o = select(pow((enc + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)), enc / 12.92, enc <= vec3<f32>(0.04045));
    let sig = select(1.099 * pow(o, vec3<f32>(0.45)) - vec3<f32>(0.099), o * 4.5, o < vec3<f32>(0.018));
    o = select(pow((sig + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)), sig / 12.92, sig <= vec3<f32>(0.04045));
    return vec4<f32>(clamp(o, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
