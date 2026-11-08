// The sun probe's count (sunglow.rs): the samples the probe sphere marked, summed - the original's ZPass sample
// count of its occlusion query (F-63954). MARK_TYPE is the marks' texture type and SAMPLES their sample count
// (textureNumSamples(marks) when multisampled, else 1u: textureLoad's last argument is then the level, 0).
@group(0) @binding(0) var marks: MARK_TYPE;
@group(0) @binding(1) var<storage, read_write> total: atomic<u32>;

@compute @workgroup_size(8, 8)
fn cs_count(@builtin(global_invocation_id) id: vec3<u32>) {
    let d = textureDimensions(marks);
    if (id.x >= d.x || id.y >= d.y) { return; }
    var c = 0u;
    for (var s = 0u; s < SAMPLES; s++) {
        if (textureLoad(marks, vec2<i32>(id.xy), i32(s)).x > 0.5) { c++; }
    }
    if (c > 0u) { atomicAdd(&total, c); }
}
