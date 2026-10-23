// The original's "fast shadow collector" (VS_ShadowFastBlend / PS_ShadowFastBlend, PS_Blur,
// PS_Copy of city/0xb6de4415; listings in work/lighting/re, LIGHTING_WORK.md "The collector pass
// decoded"): a 256 x 256 map round the camera, 1 m a texel, kept from frame to frame. Each frame
// the far city's sector models are drawn into it from straight above, the highest face in front:
//   - a "roof" - a level face (normal.y > 0.99) more than 3 m above the ground under it - gets
//     `fade` alone,
//   - every other face `fade + (1 - fade) x lit`, lit the plain on / off sun shadow at the face
//     (red) and 8 m above it (green),
//   - fade = sat((distance - 82) / 78), the distance that of the tested point from the pass's own
//     eye, 100 m above the camera: 0.23 on the ground under it, 1 from 125 m out, less on a roof
//     the higher it is. That is the grey of the blocks in the game's map.
// Then the whole map is blurred with four taps into the map the scene reads, and that is copied
// back: what no draw covers is blurred again every frame.

struct Fc {
    view_proj: mat4x4<f32>,  // the map's view from above
    light_vp: mat4x4<f32>,   // world -> the sun shadow map
    eye: vec4<f32>,          // the pass's eye (c15: the map's centre, 100 m above the camera); w = the reflection plane's height (c43.z)
    to_light: vec4<f32>,     // unit vector towards the sun (-c16)
    blur: vec4<f32>,         // xy = the blur's tap shift in texels (diagnostics), z = 1: the first reading (roofs and walls not drawn)
    top: vec4<f32>,          // the cascades' rectangle: its long axis as a plane through the camera,
    right: vec4<f32>,        // its short one,
    rect: vec4<f32>,         // and its reach: x .. y along the long axis, +-z along the short; w = 1: in use
    hist: vec4<f32>,         // xy = how many texels the map's centre moved since the last frame, z = the new map's weight of 255 (26; 0: no history)
};
@group(0) @binding(0) var<uniform> fc: Fc;
@group(0) @binding(1) var t_shadow: texture_depth_2d;
@group(0) @binding(2) var s_shadow: sampler_comparison;
@group(0) @binding(3) var t_src: texture_2d<f32>;
@group(0) @binding(4) var s_src: sampler;

// The scene's shadow cascades (city.wgsl `Casc`; render/mod.rs `Gpu::set_cascades`): what the
// original's pass tests against. PS_ShadowFastBlend 10 - 39 (work/lighting/re): the planes c82 ..
// c87 count the tiles that hold the tested point, the matrix is gLightMatrixArr[4 - count] - the
// SMALLEST tile holding it, as in the scene - and the two tests, the point and the point 8 m up,
// are one tap each of that tile: lit where the map's depth >= z x w of the point there. No tile
// holds the point: the index runs past the array - not lit.
struct Casc {
    m: array<mat4x4<f32>, 4>,
    top: vec4<f32>,
    right: vec4<f32>,
    top_off: vec4<f32>,
    top_half: vec4<f32>,
    right_off: vec4<f32>,
    right_half: vec4<f32>,
    params: vec4<f32>,
};
@group(0) @binding(5) var<uniform> casc: Casc;
@group(0) @binding(6) var t_casc: texture_depth_2d;
// last frame's collector (PS_Copy's sampler 13)
@group(0) @binding(7) var t_prev: texture_2d<f32>;

fn casc_depth(uv: vec2<f32>) -> f32 {
    let size = vec2<i32>(textureDimensions(t_casc));
    let p = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * vec2<f32>(size) - vec2<f32>(0.5);
    let i = floor(p);
    let f = p - i;
    let a = vec2<i32>(i);
    let lo = clamp(a, vec2<i32>(0), size - vec2<i32>(1));
    let hi = clamp(a + vec2<i32>(1), vec2<i32>(0), size - vec2<i32>(1));
    let d00 = textureLoad(t_casc, lo, 0);
    let d10 = textureLoad(t_casc, vec2<i32>(hi.x, lo.y), 0);
    let d01 = textureLoad(t_casc, vec2<i32>(lo.x, hi.y), 0);
    let d11 = textureLoad(t_casc, hi, 0);
    return mix(mix(d00, d10, f.x), mix(d01, d11, f.x), f.y);
}

// (x: the point, y: the point 8 m up)
fn casc_lit(p: vec3<f32>) -> vec2<f32> {
    let t = vec4<f32>(dot(p, casc.top.xyz)) + casc.top_off;
    let r = vec4<f32>(dot(p, casc.right.xyz)) + casc.right_off;
    let held = vec4<f32>(casc.top_half > abs(t)) * vec4<f32>(casc.right_half > abs(r));
    let tile = i32(4.0 - dot(held, held));
    if (tile > 3) {
        return vec2<f32>(0.0);
    }
    let q0 = casc.m[tile] * vec4<f32>(p, 1.0);
    let q1 = casc.m[tile] * vec4<f32>(p + vec3<f32>(0.0, 8.0, 0.0), 1.0);
    return vec2<f32>(step(q0.z * q0.w, casc_depth(q0.xy)), step(q1.z * q1.w, casc_depth(q1.xy)));
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) p0: vec3<f32>,
    // x = the roof flag (o0.w), y = fade (o1.y), z = normal.y
    @location(1) v: vec3<f32>,
};

@vertex
fn vs_fc(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(4) color: vec4<f32>) -> VsOut {
    var o: VsOut;
    o.clip = fc.view_proj * vec4<f32>(pos, 1.0);
    // The ground's height under the vertex, packed in the far models' colour word (memory bytes
    // 0 hi lo 0: whole metres + 29, then 255ths); the other two bytes x 32 m lie above it where
    // the ground has several levels, and the level taken is the one at the reflection plane's
    // height (instructions 12 - 28).
    let g1 = 255.0 * color.x + color.y - 29.0;
    let ground = clamp(fc.eye.w, g1, g1 + 32.0 * (color.z + color.w));
    // (the normal's length carries the binormal's sign on bump-mapped walls: world::mark_binormal)
    let ny = normal.y * inverseSqrt(max(dot(normal, normal), 1e-12));
    let roof = ny > 0.99 && (pos.y - ground > 3.0);
    // the tested point: the vertex moved towards the light by a metre and 0.25 m up - on a roof 1.75 m down
    o.p0 = pos + fc.to_light.xyz + vec3<f32>(0.0, select(0.25, -1.75, roof), 0.0);
    let fade = clamp((distance(o.p0, fc.eye.xyz) - 82.0) / 78.0, 0.0, 1.0);
    o.v = vec3<f32>(select(0.0, 1.0, roof), fade, ny);
    return o;
}

// One tap of the shadow map: is this point in the sun (1) or not (0)? (city.wgsl `sun_tap`)
// (Outside the game's cascades its test finds no map and answers 0: only inside their rectangle
// is anything lit - see fs_fc.)
fn lit(wpos: vec3<f32>) -> f32 {
    let p = fc.light_vp * vec4<f32>(wpos, 1.0);
    let uv = p.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    let bias = 0.05 * length(vec3<f32>(fc.light_vp[0].z, fc.light_vp[1].z, fc.light_vp[2].z));
    let s = textureSampleCompareLevel(t_shadow, s_shadow, uv, p.z - bias);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || p.z < 0.0 || p.z > 1.0) {
        return 1.0;
    }
    return s;
}

@fragment
fn fs_fc(in: VsOut) -> @location(0) vec4<f32> {
    // (the cascade is picked once, by the tested point itself, for both heights)
    let along = dot(in.p0, fc.top.xyz) + fc.top.w;
    let across = dot(in.p0, fc.right.xyz) + fc.right.w;
    let inside = fc.rect.w < 0.5 || (along >= fc.rect.x && along <= fc.rect.y && abs(across) <= fc.rect.z);
    var a = select(0.0, lit(in.p0), inside);
    var b = select(0.0, lit(in.p0 + vec3<f32>(0.0, 8.0, 0.0)), inside);
    // (the cascades where they are drawn - the original's test; to_light.w = 1, MCLA_FC_SINGLE: the
    // single map inside the largest tile's rectangle as until 2026-10-14)
    if (casc.params.x > 0.5 && fc.to_light.w < 0.5) {
        let c2 = casc_lit(in.p0);
        a = c2.x;
        b = c2.y;
    }
    // (the draws cull back faces, register 0x2205 = 0x18002: what looks down is not drawn)
    if (in.v.z < 0.0) {
        discard;
    }
    let roof = in.v.x > 0.999;
    if (fc.blur.z > 0.5) {
        // the first reading of the pass: only level ground is written
        if (roof || in.v.z <= 0.99) {
            discard;
        }
    }
    let fade = in.v.y;
    if (roof) {
        return vec4<f32>(fade, fade, 1.0, 1.0);
    }
    return vec4<f32>(fade + (1.0 - fade) * a, fade + (1.0 - fade) * b, 1.0, 1.0);
}

struct BlurOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_blur(@builtin(vertex_index) i: u32) -> BlurOut {
    var o: BlurOut;
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    o.clip = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    return o;
}

// PS_Blur: four taps at -1.5 / +0.5 texels, a quarter each.
@fragment
fn fs_blur(in: BlurOut) -> @location(0) vec4<f32> {
    // (one texel: 1 / 256 at the original's size - SHADOW RESOLUTION, render/mod.rs fc_size)
    let n = vec2<i32>(textureDimensions(t_prev));
    let t = 1.0 / f32(n.x);
    let uv = in.uv + fc.blur.xy * t;
    let b = 0.25 * (textureSampleLevel(t_src, s_src, uv + vec2<f32>(-1.5, -1.5) * t, 0.0)
        + textureSampleLevel(t_src, s_src, uv + vec2<f32>(-1.5, 0.5) * t, 0.0)
        + textureSampleLevel(t_src, s_src, uv + vec2<f32>(0.5, -1.5) * t, 0.0)
        + textureSampleLevel(t_src, s_src, uv + vec2<f32>(0.5, 0.5) * t, 0.0));
    if (fc.hist.z <= 0.0) {
        return b;
    }
    // PS_Copy over PS_Blur (the trace's draws 1974 / 1975, blend 0x07060706 with the quad's alpha 229 / 255;
    // LIGHTING_WORK.md "The collector's two lit levels"): the blurred fresh map is stored in 8 bits, last
    // frame's collector - read where the same ground lay, white beyond the map - is laid over it with
    // 229 / 255 and the sum stored in 8 bits again: new = round((229 old + 26 fresh) / 255). A step of less
    // than half a level is lost, so a texel stops 4 levels short of its goal from either side (a lit texel
    // that was dark once reads 251, not 255).
    let px = vec2<i32>(in.clip.xy) + vec2<i32>(fc.hist.xy);
    var prev = vec4<f32>(1.0);
    if (px.x >= 0 && px.y >= 0 && px.x < n.x && px.y < n.y) {
        prev = textureLoad(t_prev, px, 0);
    }
    let fresh = floor(b * 255.0 + vec4<f32>(0.5));
    let old = floor(prev * 255.0 + vec4<f32>(0.5));
    let w = fc.hist.z;
    return floor((old * (255.0 - w) + fresh * w) / 255.0 + vec4<f32>(0.5)) / 255.0;
}
