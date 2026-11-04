// The sun's glow: the one additive quad the original lays over the finished scene right before
// its fog pass (VS_TransformGlow + PSGlowNoTaps of effects/0xa5ea667b, technique draw_glow_notap;
// LIGHTING_WORK.md "The low sun's veil"). CPU side sub_822DD720 -> sub_822DD328 -> sub_822FA7B0:
//   centre  = camera + 700 x the sky's sun direction (also below the horizon),
//   corners = centre -+ 200 x the camera's right -+ 200 x the camera's up (uv 0 .. 1),
//   colour  = the sky file's SunColorKF at the hour (sub_822DC520's settings + 192; nothing at
//             night or in the storm) as bytes (x 255, cut off), alpha byte = 45 x visible x
//             2.55 (cut off); "visible" is an occlusion query's pixel count / 1250, which
//             reads 1000 = 0.8 in the reference whatever stands before the sun.
// Programs: colour x alpha in the vertex program, rgb = 10 x (1 - sat(|2 uv - 1|))^2 x alpha^2 x
// that; blend one / one, no depth test. The fog pass comes after it: the glow fades with the
// fog's share on things (none on the sky, whose depth the fog pass leaves alone).
// MCLA_DEBUG_VIEW=128: not drawn.

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
@group(1) @binding(0) var depth: texture_depth_2d;
struct Consts {
    sun: vec4<f32>,          // rgb = SunColor of the hour, w = visible
    glow: vec4<f32>,         // cloudspeed.xml: distance (700), half size (200), intensity (45)
};
@group(1) @binding(1) var<uniform> k: Consts;


struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
    @location(1) @interpolate(flat) right: vec3<f32>,
    @location(2) @interpolate(flat) up: vec3<f32>,
    @location(3) @interpolate(flat) fwd: vec3<f32>,
};

fn unproject(ndc: vec2<f32>, z: f32) -> vec3<f32> {
    let p = g.inv_view_proj * vec4<f32>(ndc, z, 1.0);
    return p.xyz / p.w;
}

@vertex
fn vs_sun_glow(@builtin(vertex_index) vi: u32) -> VsOut {
    var o: VsOut;
    let xy = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u)) * 2.0 - vec2<f32>(1.0);
    o.clip = vec4<f32>(xy, 0.5, 1.0);
    o.ndc = xy;
    o.right = normalize(unproject(vec2<f32>(1.0, 0.0), 0.5) - unproject(vec2<f32>(-1.0, 0.0), 0.5));
    o.up = normalize(unproject(vec2<f32>(0.0, 1.0), 0.5) - unproject(vec2<f32>(0.0, -1.0), 0.5));
    o.fwd = normalize(unproject(vec2<f32>(0.0, 0.0), 0.5) - g.cam_pos.xyz);
    return o;
}

@fragment
fn fs_sun_glow(in: VsOut) -> @location(0) vec4<f32> {
    // (only under the weather's keys, not in the garage; 128: off)
    if (g.dome.w < 0.5 || g.ambient_ground.w < -0.5 || (g.params.w > 127.5 && g.params.w < 128.5)) { discard; }
    let d = normalize(g.true_sun.xyz);
    let DISTANCE = k.glow.x;
    let depth_c = DISTANCE * dot(d, in.fwd);
    if (depth_c <= 0.1) { discard; }
    let ray = normalize(unproject(in.ndc, 0.5) - g.cam_pos.xyz);
    let t = depth_c / max(dot(ray, in.fwd), 1e-4);
    let off = ray * t - d * DISTANCE;
    let uv = vec2<f32>(dot(off, in.right), dot(off, in.up)) / k.glow.y;
    if (abs(uv.x) > 1.0 || abs(uv.y) > 1.0) { discard; }
    let fall = 1.0 - clamp(length(uv), 0.0, 1.0);
    let a = floor(k.glow.z * k.sun.w * 2.55) / 255.0;
    let col = floor(clamp(k.sun.rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0) / 255.0;
    var glow = col * (10.0 * fall * fall * a * a * a);
    // the fog pass over it (city.wgsl's rule; the sky's cleared depth takes none)
    let dims = vec2<i32>(textureDimensions(depth));
    let z = textureLoad(depth, clamp(vec2<i32>(in.clip.xy), vec2<i32>(0), dims - vec2<i32>(1)), 0);
    if (z > 0.0) {
        let dist = length(unproject(in.ndc, z) - g.cam_pos.xyz);
        let fg = clamp(1.0 - exp(-(dist - g.params.x) / max(g.params.y - g.params.x, 1.0)), 0.0, 1.0);
        glow *= 1.0 - clamp(fg * g.fog_color.a, 0.0, 1.0);
    }
    return vec4<f32>(glow, 0.0);
}
