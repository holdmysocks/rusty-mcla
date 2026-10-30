// The occlusion blobs under the cars (LIGHTING_WORK.md "The car's shadow model, the blobs under
// it, its tangent frame", car round 13): the original's cars/0xf9cd0545 `quadao_draw` -
// VS_DrawSingle + PS_CalcShadowsSingle once per wheel of the player's car, VS_DrawQuad +
// PS_CalcShadowsQuad once per car - drawn after the cars' glass and before the fog pass, black
// over the frame with blend source alpha / one minus source alpha, depth test without writes,
// and ONLY where the stencil is 0 (cars write 4 / 5 ...: the blobs darken the world, never a car).
//
// Here: the scene target's ALPHA is the working value (nothing reads it after the scene pass):
// fs_one sets it to 1, every blob multiplies it by (1 - its alpha) - what blending black five
// times does to the colour - the cars are drawn over it with 1 again (the stencil's part), and
// fs_apply multiplies the colour by it.
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
};
@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var depth: texture_depth_2d;

struct Object {
    model: mat4x4<f32>,
};
@group(3) @binding(0) var<uniform> obj: Object;

struct BlobIn {
    @location(0) c: vec4<f32>,   // the volume's middle, w = kind (0 a wheel, 1 a body)
    @location(1) ax: vec4<f32>,  // its three half axes (world), ax.w = strength
    @location(2) ay: vec4<f32>,
    @location(3) az: vec4<f32>,
    @location(4) a: vec4<f32>,   // wheel: its middle, w = radius | body: the first of four sphere middles, w = radius squared
    @location(5) b: vec4<f32>,   // wheel: the axle's direction, w = half length | body: the step to the next middle
};

struct BlobOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) c: vec4<f32>,
    @location(1) @interpolate(flat) ax: vec4<f32>,
    @location(2) @interpolate(flat) ay: vec4<f32>,
    @location(3) @interpolate(flat) az: vec4<f32>,
    @location(4) @interpolate(flat) a: vec4<f32>,
    @location(5) @interpolate(flat) b: vec4<f32>,
};

// A screen rectangle round the volume's box (the whole screen when a corner is behind the eye).
@vertex
fn vs_blob(@builtin(vertex_index) vi: u32, in: BlobIn) -> BlobOut {
    var lo = vec2<f32>(1e9);
    var hi = vec2<f32>(-1e9);
    var whole = false;
    for (var i = 0u; i < 8u; i = i + 1u) {
        let s = vec3<f32>(f32(i & 1u), f32((i >> 1u) & 1u), f32((i >> 2u) & 1u)) * 2.0 - vec3<f32>(1.0);
        let p = g.view_proj * vec4<f32>(in.c.xyz + in.ax.xyz * s.x + in.ay.xyz * s.y + in.az.xyz * s.z, 1.0);
        if (p.w < 0.05) {
            whole = true;
        } else {
            lo = min(lo, p.xy / p.w);
            hi = max(hi, p.xy / p.w);
        }
    }
    if (whole) {
        lo = vec2<f32>(-1.0);
        hi = vec2<f32>(1.0);
    }
    lo = clamp(lo, vec2<f32>(-1.0), vec2<f32>(1.0));
    hi = clamp(hi, vec2<f32>(-1.0), vec2<f32>(1.0));
    let k = array<vec2<f32>, 6>(vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0), vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0));
    let t = k[vi % 6u];
    var o: BlobOut;
    o.clip = vec4<f32>(mix(lo, hi, t), 0.5, 1.0);
    o.c = in.c;
    o.ax = in.ax;
    o.ay = in.ay;
    o.az = in.az;
    o.a = in.a;
    o.b = in.b;
    return o;
}

fn unproject(ndc: vec2<f32>, z: f32) -> vec3<f32> {
    let p = g.inv_view_proj * vec4<f32>(ndc, z, 1.0);
    return p.xyz / p.w;
}

@fragment
fn fs_blob(in: BlobOut) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(depth));
    let px = clamp(vec2<i32>(in.clip.xy), vec2<i32>(0), dims - vec2<i32>(1));
    let z = textureLoad(depth, px, 0);
    if (z <= 0.0) {
        discard;
    }
    let ndc = vec2<f32>((f32(px.x) + 0.5) / f32(dims.x) * 2.0 - 1.0, 1.0 - (f32(px.y) + 0.5) / f32(dims.y) * 2.0);
    let p = unproject(ndc, z);
    // the original draws the volume's FRONT faces with the depth test: the pixel takes the blob when its ray
    // enters the volume before it reaches the scene (and the eye is outside). In the volume's own measure
    // (wheel: the unit sphere; body: the cylinder x2 + y2 <= 1, |z| <= 1 - VS_DrawQuad's mesh is +-0.5 long).
    let eye = g.cam_pos.xyz - in.c.xyz;
    let ray = p - g.cam_pos.xyz;
    let inv = vec3<f32>(1.0 / dot(in.ax.xyz, in.ax.xyz), 1.0 / dot(in.ay.xyz, in.ay.xyz), 1.0 / dot(in.az.xyz, in.az.xyz));
    let uo = vec3<f32>(dot(eye, in.ax.xyz), dot(eye, in.ay.xyz), dot(eye, in.az.xyz)) * inv;
    let ud = vec3<f32>(dot(ray, in.ax.xyz), dot(ray, in.ay.xyz), dot(ray, in.az.xyz)) * inv;
    var alpha = 0.0;
    if (in.c.w < 0.5) {
        let qa = dot(ud, ud);
        let qb = dot(uo, ud);
        let qc = dot(uo, uo) - 1.0;
        let disc = qb * qb - qa * qc;
        if (qc <= 0.0 || disc < 0.0 || qa <= 0.0) {
            discard;
        }
        let t0 = (-qb - sqrt(disc)) / qa;
        if (t0 < 0.0 || t0 > 1.0) {
            discard;
        }
        // PS_CalcShadowsSingle 15 - 38: the wheel as a segment (the axle, half length h) with a radius;
        // 1 - (1 - sat(|along| - h)) (1 - sat(off the axle - radius)), gone at 0.3 (c255.w = 1 / 0.3),
        // squared, x 0.85 (c255.z)
        let rel = p - in.a.xyz;
        let along = dot(rel, in.b.xyz);
        let off = length(cross(rel, in.b.xyz));
        let f = 1.0 - (1.0 - clamp(abs(along) - in.b.w, 0.0, 1.0)) * (1.0 - clamp(off - in.a.w, 0.0, 1.0));
        let x = 1.0 - clamp(3.3333333 * f, 0.0, 1.0);
        alpha = 0.85 * x * x;
    } else {
        let qa = dot(ud.xy, ud.xy);
        let qb = dot(uo.xy, ud.xy);
        let qc = dot(uo.xy, uo.xy) - 1.0;
        if (qc <= 0.0 && abs(uo.z) <= 1.0) {
            discard;
        }
        var t_in = 0.0;
        var t_out = 1e9;
        if (qa > 1e-12) {
            let disc = qb * qb - qa * qc;
            if (disc < 0.0) {
                discard;
            }
            let sq = sqrt(disc);
            t_in = (-qb - sq) / qa;
            t_out = (-qb + sq) / qa;
        } else if (qc > 0.0) {
            discard;
        }
        if (abs(ud.z) > 1e-12) {
            let za = (-1.0 - uo.z) / ud.z;
            let zb = (1.0 - uo.z) / ud.z;
            t_in = max(t_in, min(za, zb));
            t_out = min(t_out, max(za, zb));
        } else if (abs(uo.z) > 1.0) {
            discard;
        }
        if (t_in > t_out || t_in < 0.0 || t_in > 1.0) {
            discard;
        }
        // PS_CalcShadowsQuad 16 - 34: four spheres along the car, s = sat(d2 / r2) each;
        // 1 - the product of (1 - (1 - s)^2), x the car's strength (c64.x)
        var q = 1.0;
        for (var i = 0; i < 4; i = i + 1) {
            let d = p - (in.a.xyz + in.b.xyz * f32(i));
            let s = clamp(dot(d, d) / in.a.w, 0.0, 1.0);
            q = q * (1.0 - (1.0 - s) * (1.0 - s));
        }
        alpha = 1.0 - q;
    }
    return vec4<f32>(0.0, 0.0, 0.0, clamp(alpha * in.ax.w, 0.0, 1.0));
}

struct FullOut {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> FullOut {
    var o: FullOut;
    o.clip = vec4<f32>(vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u)) * 2.0 - vec2<f32>(1.0), 0.5, 1.0);
    return o;
}

// alpha = 1 (before the blobs, and again at the end)
@fragment
fn fs_one(in: FullOut) -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}

// colour x the target's alpha (the blend state does it)
@fragment
fn fs_apply(in: FullOut) -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 0.0, 0.0, 0.0);
}

// the cars over the blobs' value: 1 again (the original's stencil test). The body's vertices as
// vs_body places them, without the dents (a pressed-in panel lies behind this one: it passes).
@vertex
fn vs_mask(@location(0) pos: vec3<f32>) -> FullOut {
    var o: FullOut;
    o.clip = g.view_proj * (obj.model * vec4<f32>(pos, 1.0));
    return o;
}

// The head-lamp cone: VS_DrawCone + PS_CalcShadowsCone (F-68081's record, drawn by F-35120).
struct ConeOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) pos: vec4<f32>,
    @location(1) @interpolate(flat) dir: vec4<f32>,
    @location(2) @interpolate(flat) col: vec4<f32>,
};

@vertex
fn vs_cone(@location(0) v: vec3<f32>, @location(1) x: vec4<f32>, @location(2) y: vec4<f32>, @location(3) z: vec4<f32>, @location(4) w: vec4<f32>, @location(5) pos: vec4<f32>, @location(6) dir: vec4<f32>, @location(7) col: vec4<f32>) -> ConeOut {
    var o: ConeOut;
    o.clip = g.view_proj * vec4<f32>(w.xyz + x.xyz * v.x + y.xyz * v.y + z.xyz * v.z, 1.0);
    o.pos = pos;
    o.dir = dir;
    o.col = col;
    return o;
}

@fragment
fn fs_cone(in: ConeOut) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(depth));
    let px = clamp(vec2<i32>(in.clip.xy), vec2<i32>(0), dims - vec2<i32>(1));
    let z = textureLoad(depth, px, 0);
    let ndc = vec2<f32>((f32(px.x) + 0.5) / f32(dims.x) * 2.0 - 1.0, 1.0 - (f32(px.y) + 0.5) / f32(dims.y) * 2.0);
    let d = unproject(ndc, z) - in.pos.xyz;
    let d2 = dot(d, d);
    let along = clamp(dot(d / sqrt(d2), in.dir.xyz) - 0.5, 0.0, 1.0) * 2.0;
    let k = (1.0 - clamp(d2 / in.pos.w, 0.0, 1.0)) * along;
    let k2 = k * k;
    let s = clamp(k2 * k2, 0.0, 1.0) * 8.0;
    return vec4<f32>(vec3<f32>(1.0) + in.col.rgb * s, 1.0);
}
