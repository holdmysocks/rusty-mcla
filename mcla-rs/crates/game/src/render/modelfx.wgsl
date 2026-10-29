// The effect list's model particles (modelfx.rs): `mc4_rmptfx_model`'s VSRrmptfxModel and
// PSRrmptfxModel (the recorded draws run VSRrmptfxModelBump + PSRrmptfxModelWithNormalMap, whose
// light is the same formula - it samples no normal map; extracted/cache/shaders/effects/fxl_final/0x5c44658b; read in
// work/map2/model/model_particle_rule.md section 4), per pixel:
//   n     = normalize(normal x face sign)                        (two-sided)
//   point = sum of the 4 lights: colour x sat(c87 / d^2) x sat(dot(l, axis) / |l| + c95) x sat(n.l / |l|)
//   light = sat(dot(n, -c16.xyz)) x c39 + vcol.a x c26 + 0.01 x c26 + point
//   light = lerp(light, c130.rgb, c130.w)                        (gLightOverride)
//   out   = tex.rgb x gForcedColor.rgb x light, tex.a x gForcedColor.a x gInvColorExpBias.x
// No fog in the program. The output is the scene's linear value (as the lit sprites').

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

// the dynamic lights (render/mod.rs `LightSet`): the player's head lamp of the four lights
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

@group(1) @binding(0) var t_diffuse: texture_2d<f32>;
@group(0) @binding(16) var s_diffuse: sampler;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    // gWorld's columns
    @location(4) w0: vec4<f32>,
    @location(5) w1: vec4<f32>,
    @location(6) w2: vec4<f32>,
    @location(7) w3: vec4<f32>,
    // gForcedColor
    @location(8) forced: vec4<f32>,
    // the light picker's place, the rule's LightingMode
    @location(9) at: vec4<f32>,
    // the night fill light at it (added to gLightAmbient)
    @location(10) fill: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) wp: vec3<f32>,
    @location(1) n: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) vcol: vec4<f32>,
    @location(4) forced: vec4<f32>,
    @location(5) at: vec4<f32>,
    @location(6) fill: vec3<f32>,
};

@vertex
fn vs_model(in: VsIn) -> VsOut {
    let w = mat4x4<f32>(in.w0, in.w1, in.w2, in.w3);
    let wp = w * vec4<f32>(in.pos, 1.0);
    var o: VsOut;
    o.clip = g.view_proj * wp;
    o.wp = wp.xyz;
    // (the normal by gWorld's 3 x 3, not normalised in the vertex program)
    o.n = mat3x3<f32>(in.w0.xyz, in.w1.xyz, in.w2.xyz) * in.normal;
    o.uv = in.uv;
    o.vcol = in.color;
    o.forced = in.forced;
    o.at = in.at;
    o.fill = in.fill.xyz;
    return o;
}

// One lamp of the city lamp grid (the same table as the lit sprites', particle.wgsl `lamp_term`,
// here with no translucency term: sat(n.l / |l|)).
fn lamp_term(i: i32, wp: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let at = vec2<i32>((i % 256) * 4, i / 256);
    let p = textureLoad(t_lamp_data, at, 0);
    let c = textureLoad(t_lamp_data, at + vec2<i32>(1, 0), 0);
    let axis = textureLoad(t_lamp_data, at + vec2<i32>(2, 0), 0);
    let bias = textureLoad(t_lamp_data, at + vec2<i32>(3, 0), 0).x;
    let d = p.xyz - wp;
    let inv = inverseSqrt(max(dot(d, d), 1e-6));
    let dn = d * inv;
    return c.rgb * 2.0 * clamp(p.w * inv * inv, 0.0, 1.0) * clamp(dot(dn, axis.xyz) * axis.w + bias, 0.0, 1.0) * clamp(dot(n, dn), 0.0, 1.0);
}

// The four lights of the effect's render setup (sub_82264968 -> the light picker sub_822F5D98 at
// the effect's cull-sphere centre `ec`): by day none; at night the lamp grid's four lamps of the
// cell under it, the player's first head lamp in lane 3 (as particle.wgsl `vert_lit`).
fn four_lights(wp: vec3<f32>, n: vec3<f32>, ec: vec3<f32>) -> vec3<f32> {
    var dl = vec3<f32>(0.0);
    if (g.sun_color.w > 18.0 || g.sun_color.w < 6.0) {
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
                    dl = dl + lamp_term(i, wp, n);
                }
            }
        }
        if (hl >= 0) {
            let d = lights.pos_range[hl].xyz - wp;
            let inv = inverseSqrt(max(dot(d, d), 1e-6));
            // (while HEADLIGHTS is held - dir.w, the flash target - F-43427: strength x 2, cone x 1.2)
            let hk = select(vec3<f32>(500.0, 4.4696, -2.872), vec3<f32>(1000.0, 3.9948, -2.2326), lights.dir[hl].w > 0.5);
            let cone = clamp(dot(d, -lights.dir[hl].xyz * hk.y) * inv + hk.z, 0.0, 1.0);
            dl = dl + lights.color[hl].rgb * clamp(hk.x * inv * inv, 0.0, 1.0) * cone * clamp(dot(n, d) * inv, 0.0, 1.0);
        }
    }
    return dl;
}

@fragment
fn fs_model(in: VsOut, @builtin(front_facing) ff: bool) -> @location(0) vec4<f32> {
    let t = textureSample(t_diffuse, s_diffuse, in.uv);
    // (the face register's sign: the back of a sheet takes the normal turned round)
    let n = normalize(in.n * select(-1.0, 1.0, ff));
    // gLightPosDir c16: the light group's default way (0, -1, 0, 1000) in every model draw of the
    // noon recording (84 draws, wind_debris_hw at Sunset / Rexford; work/map2/litter/litter_port.md) -
    // although the rules' LightingMode is 2, the sun point (sub_822F5D98's +16) is not sent here:
    // the key light is sat(n.y) x c39, light from straight above
    let c16 = vec3<f32>(0.0, -1.0, 0.0);
    // gLightColor c39, gLightAmbient c26 (the scene's plus the night fill light at the effect)
    let c39 = g.sun_color.rgb * g.sun_dir.w;
    let c26 = g.ambient_sky.rgb + in.fill;
    var light = clamp(dot(n, -c16), 0.0, 1.0) * c39 + in.vcol.a * c26 + 0.01 * c26 + four_lights(in.wp, n, in.at.xyz);
    // (gLightOverride c130: recorded (1, 1, 1, 0) - w 0, no override; gInvColorExpBias.x 0.0625 x the
    // target's exponent bias 4 = 1 on the alpha)
    return vec4<f32>(t.rgb * in.forced.rgb * light, t.a * in.forced.a);
}
