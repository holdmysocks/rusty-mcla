// Depth-only pass for the sun shadow map (city, vehicles, instanced props).

struct ShadowGlobals {
    light_vp: mat4x4<f32>,
    // the cascades' passes (fs_shadow_casc): x = the slope bias (BiasSlope of the tile), y = the
    // constant bias in depth units, z = the cut-outs' alpha threshold
    bias: vec4<f32>,
};
@group(0) @binding(0) var<uniform> sg: ShadowGlobals;

@group(1) @binding(0) var t_diffuse: texture_2d<f32>;
@group(0) @binding(1) var s_diffuse: sampler;

struct Material {
    mode: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};
@group(2) @binding(0) var<uniform> mat: Material;

struct Object {
    model: mat4x4<f32>,
    color: vec4<f32>,
    params: vec4<f32>,
};
@group(3) @binding(0) var<uniform> obj: Object;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv0: vec2<f32>,
    // 1: a plant's shadow impostor (cut by its mask's red)
    @location(1) card: f32,
    // the light's depth before any bias (fs_shadow_casc)
    @location(2) z: f32,
    // +1: the mesh as stored, -1: drawn through a mirroring matrix (a car's wheels of one side) - its winding is
    // turned over on the screen (fs_shadow_casc_cull)
    @location(3) @interpolate(flat) face: f32,
};

@vertex
fn vs_shadow(@location(0) pos: vec3<f32>, @location(2) uv0: vec2<f32>) -> VsOut {
    var o: VsOut;
    o.clip = sg.light_vp * (obj.model * vec4<f32>(pos, 1.0));
    o.uv0 = uv0;
    o.card = 0.0;
    o.z = o.clip.z;
    let m3 = mat3x3<f32>(obj.model[0].xyz, obj.model[1].xyz, obj.model[2].xyz);
    o.face = select(1.0, -1.0, determinant(m3) < 0.0);
    return o;
}

fn quat_rot(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

@vertex
fn vs_shadow_inst(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv0: vec2<f32>, @location(3) uv1: vec2<f32>, @location(6) pos_scale: vec4<f32>, @location(7) rot: vec4<f32>) -> VsOut {
    var o: VsOut;
    // (the quaternion's squared length = a plant's height scale over its width scale, see vs_inst)
    let tall = dot(rot, rot);
    let iq = rot * inverseSqrt(max(tall, 1e-8));
    var wp = quat_rot(iq, pos * vec3<f32>(pos_scale.w, pos_scale.w * tall, pos_scale.w)) + pos_scale.xyz;
    // Pass 9: a plant's shadow impostor (uv1.y = 105; props/impostor.rs) -
    // VS_ShadowImpostorInstanceDepthAlpha: the impostor card's quad (city.wgsl vs_inst, kind 4)
    // with the light's direction where the camera's is: pos = (side sign, top sign), normal =
    // (half the box's width, CenterY, HeightY); the axis c = -L, L the way clip depth grows -
    // the same numbers the mask was drawn with (Impostors::update_shadow).
    let card = uv1.y > 104.5 && uv1.y < 105.5;
    if (card) {
        let cb = -normalize(vec3<f32>(sg.light_vp[0][2], sg.light_vp[1][2], sg.light_vp[2][2]));
        let s0 = vec3<f32>(cb.z, 0.0, -cb.x);
        let side = select(vec3<f32>(1.0, 0.0, 0.0), s0 * inverseSqrt(max(dot(s0, s0), 1e-12)), dot(s0, s0) > 1e-10);
        let tu = cross(cb, side);
        let up = quat_rot(iq, vec3<f32>(0.0, 1.0, 0.0)) * (pos_scale.w * tall);
        let hw = normal.x * pos_scale.w;
        wp = pos_scale.xyz + up * (normal.y + normal.z * pos.y) + side * (pos.x * hw) + vec3<f32>(tu.x, 0.0, tu.z) * (pos.y * hw);
    }
    o.clip = sg.light_vp * vec4<f32>(wp, 1.0);
    // (the original clamps the card to the near plane: oPos.z = max(z, 0) x w)
    if (card) { o.clip.z = max(o.clip.z, 0.0); }
    o.uv0 = uv0;
    o.card = select(0.0, 1.0, card);
    o.z = o.clip.z;
    o.face = 1.0;
    return o;
}

@fragment
fn fs_shadow(in: VsOut) {
    // cut-out materials (foliage, fences, grills) cast cut-out shadows
    if (mat.mode == 1u || mat.mode == 13u || mat.mode == 15u) {
        // (a shadow impostor: PS_ShadowImpostorDepthAlpha - killed where the mask is under 10 / 255)
        let t = textureSample(t_diffuse, s_diffuse, in.uv0);
        if (select(t.a < 0.5, t.r < 0.039216, in.card > 0.5)) {
            discard;
        }
    }
}

// The original's cascades (render/mod.rs `Gpu::set_cascades`): the same cut-outs, and the depth
// written with the tile's own bias as its depth draws have it (registers 0x2380 .. 0x2383:
// D3D's SlopeScaleDepthBias x the larger of the depth's two slopes per texel + DepthBias; the
// numbers are tune/360CascadedShadowMaps.xml's BiasSlope and DepthBias).
struct CascOut {
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_shadow_casc(in: VsOut) -> CascOut {
    let slope = max(abs(dpdx(in.z)), abs(dpdy(in.z)));
    if (mat.mode == 1u || mat.mode == 13u || mat.mode == 15u) {
        let t = textureSample(t_diffuse, s_diffuse, in.uv0);
        // (PS_ShadowDepthAlpha: killed where the texture's alpha is under 10 / 255 - bias.z)
        if (select(t.a < sg.bias.z, t.r < 0.039216, in.card > 0.5)) {
            discard;
        }
    }
    var o: CascOut;
    o.depth = clamp(in.z + sg.bias.x * slope + sg.bias.y, 0.0, 1.0);
    return o;
}

// The same with the original's culling (casc2, 2026-10-15). Its depth draws carry the cull word of the mesh's own
// colour draw with the front-face bit turned over (register 0x2205: 0x19806 against 0x18002 - the light's
// projection is of the other handedness, so the SAME faces stay: those turned to the pass's eye, here the light);
// a mesh drawn through a mirroring matrix has the bit turned back (0x19802: the wheels of one side). Both sides
// stay where the colour draw keeps both: the plants (PSPropFoliage, 0x19800) and their shadow impostors, the cars'
// glass and lamps (PS_GlassFront, PS_Light: 0x18000). The pipeline's front face is the clockwise one, as there.
@fragment
fn fs_shadow_casc_cull(in: VsOut, @builtin(front_facing) ff: bool) -> CascOut {
    let slope = max(abs(dpdx(in.z)), abs(dpdy(in.z)));
    let two = in.card > 0.5 || mat.mode == 15u || mat.mode == 12u || mat.mode == 8u;
    var kill = !two && (ff != (in.face > 0.0));
    if (mat.mode == 1u || mat.mode == 13u || mat.mode == 15u) {
        let t = textureSample(t_diffuse, s_diffuse, in.uv0);
        if (select(t.a < sg.bias.z, t.r < 0.039216, in.card > 0.5)) {
            kill = true;
        }
    }
    if (kill) {
        discard;
    }
    var o: CascOut;
    o.depth = clamp(in.z + sg.bias.x * slope + sg.bias.y, 0.0, 1.0);
    return o;
}
