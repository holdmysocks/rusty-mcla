// The GPS map's icons and MAP MODE road overlay in the 3D frame (hudmap_gpu.rs): the original's
// "forced_zbuffer" technique of ui effect 0x62ca36ca - VS_TransformAndTextureAlpha +
// PS_TexturedAlternateDepth (and RippleTarget's PS_RippleAltDepth for the ripple's rings).

struct Globals {
    vp: mat4x4<f32>,
    // x = gXrayTerm, y = k = far / (far - near) of the original's map camera, z = its near,
    // w = the port's near (the port's depth is reversed and infinite: near / view depth)
    xray: vec4<f32>,
    // the ripple: x = clock, y = frequency, z = speed, w = falloff exponent
    ripple: vec4<f32>,
    // x = the ripple's amplitude
    ripple2: vec4<f32>,
};

struct DrawRec {
    world: mat4x4<f32>,
    // gSimpleHue
    colour: vec4<f32>,
    // x: 0 textured, 1 the ripple's rings, 2 flat colour
    mode: vec4<u32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<storage, read> recs: array<DrawRec>;
@group(0) @binding(2) var scene_depth: texture_depth_2d;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) colour: vec4<f32>,
    @location(2) @interpolate(flat) mode: u32,
};

@vertex
fn vs_main(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @builtin(instance_index) inst: u32) -> VOut {
    let r = recs[inst];
    var o: VOut;
    o.pos = g.vp * (r.world * vec4<f32>(p, 1.0));
    o.uv = uv;
    o.colour = r.colour;
    o.mode = r.mode.x;
    return o;
}

// the original's z/w (D3D, 0 near .. 1 far) at the distance the port's depth value stands for
fn orig_z(z_port: f32) -> f32 {
    // (w 0: the open map's projection has the original's planes - hudmap_gpu::port_near)
    if g.xray.w == 0.0 {
        return 1.0 - z_port;
    }
    return g.xray.y * (1.0 - g.xray.z * z_port / g.xray.w);
}

// PS_RippleAltDepth's rings (hudmap::Ripple::alpha)
fn ripple_alpha(uv: vec2<f32>) -> f32 {
    let q = uv * 2.0 - 1.0;
    let r = sqrt(dot(q, q));
    let ph = (g.ripple.x * g.ripple.z - r * g.ripple.y) * 0.159155 + 0.5;
    let w = sin(fract(ph) * 6.2831855 - 3.1415927);
    var fall = 0.0;
    if r < 1.0 {
        fall = pow(1.0 - r, g.ripple.w);
    }
    return clamp(w * g.ripple2.x, 0.0, 1.0) * fall;
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, in.uv);
    let z = orig_z(in.pos.z);
    // (outside the original camera's near / far planes its clipper drops the pixel)
    if z < 0.0 || z > 1.0 {
        discard;
    }
    var c = t * in.colour;
    if in.mode == 1u {
        c = vec4<f32>(in.colour.rgb, in.colour.a * ripple_alpha(in.uv));
    }
    // the scene's depth at this pixel (DepthMap, point sampled); behind it: alpha x gXrayTerm
    let dims = textureDimensions(scene_depth);
    let px = min(vec2<u32>(in.pos.xy), dims - vec2<u32>(1u, 1u));
    let zs = orig_z(textureLoad(scene_depth, px, 0));
    if z - 7.5e-6 > zs {
        c.a = c.a * g.xray.x;
    }
    // (the 8-bit target clamps; alpha test "greater than 0")
    c = clamp(c, vec4<f32>(0.0), vec4<f32>(1.0));
    if c.a <= 0.0 {
        discard;
    }
    return c;
}
