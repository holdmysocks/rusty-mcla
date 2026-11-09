// The cockpit readouts (readouts.rs), from the original's FontDraw.fx (cars/0x8146788c): VS_Font / VS_Roller give the
// cell's uv (computed on the CPU here, readouts.rs `cell_uv`), PS_Font:
//   rgb = font.rgb x fontColor.rgb x (emIntColor + ambYPos + the two cabin point lights x the shadow)
//   a = font.a x fontColor.w (x gInvColorExpBias.x x gLightAmbient.w, 1)
// *(approx.)* the ambient term and the point lights (c16 / c17, c39 / c40) are the scene's sky ambient here.
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
@group(1) @binding(0) var t_font: texture_2d<f32>;
@group(1) @binding(1) var s_font: sampler;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) emissive: vec3<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>, @location(3) emissive: vec3<f32>, @location(4) depth: f32) -> VOut {
    var o: VOut;
    o.clip = g.view_proj * vec4<f32>(pos, 1.0);
    // (render state 12, the in-car view's depth offset - readouts.rs set_depth_offset)
    o.clip.z = o.clip.z + depth * o.clip.w;
    o.uv = uv;
    o.color = color;
    o.emissive = emissive;
    return o;
}

@fragment
fn fs_main(i: VOut) -> @location(0) vec4<f32> {
    let f = textureSample(t_font, s_font, i.uv);
    let light = i.emissive + g.ambient_sky.rgb;
    return vec4<f32>(f.rgb * i.color.rgb * light, f.a * i.color.a);
}
