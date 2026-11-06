//! Additional inspection commands.

use anyhow::{Context, Result};
use mcla_formats::rage::scan::Drawable;
use mcla_formats::rsc::Resource;
use std::io::Write;

pub fn load_res(path: &str) -> Result<Resource> {
    let d = std::fs::read(path).with_context(|| format!("read {}", path))?;
    if d.len() > 16 && &d[0..4] == b"MCLR" {
        let rtype = u32::from_le_bytes(d[4..8].try_into().unwrap());
        let flags = u32::from_le_bytes(d[8..12].try_into().unwrap());
        let vs = u32::from_le_bytes(d[12..16].try_into().unwrap()) as usize;
        let data = d[16..].to_vec();
        let vs = vs.min(data.len());
        return Ok(Resource { rtype, flags, virtual_size: vs, physical_size: data.len() - vs, data });
    }
    Resource::parse(&d)
}

/// mcla-tool drawable <file.rNN> [main.r83] – list models/geometries/shaders, export OBJ next to it
pub fn cmd_drawable(a: &[String]) -> Result<()> {
    let res = load_res(&a[0])?;
    let d = Drawable::parse(&res)?;
    let main = match a.get(1) {
        Some(p) => Some(Drawable::parse(&load_res(p)?)?),
        None => None,
    };
    let shaders = if d.shader_group.shaders.is_empty() {
        main.as_ref().map(|m| &m.shader_group.shaders)
    } else {
        Some(&d.shader_group.shaders)
    };
    println!(
        "{}: {} models, {} tris, {} shaders, {} textures",
        a[0],
        d.models.len(),
        d.triangle_count(),
        d.shader_group.shaders.len(),
        d.shader_group.textures.len()
    );
    let mut obj = std::fs::File::create(format!("{}.obj", a[0]))?;
    let mut base = 1usize;
    for (mi, m) in d.models.iter().enumerate() {
        for (pi, (mesh, si)) in m.parts.iter().enumerate() {
            let (mn, mx) = mesh.bbox();
            let sh = shaders.and_then(|s| s.get(*si as usize));
            let sname = sh.map(|s| s.name.clone()).unwrap_or_default();
            let tex = sh.and_then(|s| s.diffuse().map(|t| t.to_string())).unwrap_or_default();
            println!(
                "  model {:2} @{:#08x} part {:2} shader {:2} {:<24} {:<28} verts={:5} tris={:5} stride={} bbox=({:.2},{:.2},{:.2})..({:.2},{:.2},{:.2})",
                mi,
                m.off,
                pi,
                si,
                sname,
                tex,
                mesh.positions.len(),
                mesh.indices.len() / 3,
                mesh.raw_stride,
                mn.x,
                mn.y,
                mn.z,
                mx.x,
                mx.y,
                mx.z
            );
            if std::env::var("MCLA_UV").is_ok() {
                let rng = |v: &Vec<glam::Vec2>| -> String {
                    if v.is_empty() {
                        return "-".into();
                    }
                    let mn = v.iter().fold(glam::Vec2::splat(1e9), |a, b| a.min(*b));
                    let mx = v.iter().fold(glam::Vec2::splat(-1e9), |a, b| a.max(*b));
                    format!("[{:.2},{:.2}]..[{:.2},{:.2}]", mn.x, mn.y, mx.x, mx.y)
                };
                println!("        uv0 {} uv1 {} uv2 {} colors {} colors1 {}", rng(&mesh.uv0), rng(&mesh.uv1), rng(&mesh.uv2), mesh.colors.len(), mesh.colors1.len());
            }
            if std::env::var("MCLA_UVDUMP").ok().as_deref() == Some(&format!("{},{}", mi, pi)) {
                let mut f = std::fs::File::create(format!("{}.uv.txt", a[0]))?;
                for (i, p) in mesh.positions.iter().enumerate() {
                    let u = mesh.uv0.get(i).copied().unwrap_or_default();
                    let n = mesh.normals.get(i).copied().unwrap_or_default();
                    let c = mesh.colors.get(i).copied().unwrap_or_default();
                    writeln!(f, "{} {} {} {} {} {} {} {} {} {} {} {}", p.x, p.y, p.z, u.x, u.y, n.x, n.y, n.z, c[0], c[1], c[2], c[3])?;
                }
            }
            writeln!(obj, "o m{}_p{}_{}", mi, pi, sname)?;
            for p in &mesh.positions {
                writeln!(obj, "v {} {} {}", p.x, p.y, p.z)?;
            }
            for t in mesh.indices.chunks_exact(3) {
                writeln!(obj, "f {} {} {}", base + t[0] as usize, base + t[1] as usize, base + t[2] as usize)?;
            }
            base += mesh.positions.len();
        }
    }
    Ok(())
}

/// mcla-tool cartex <main.r83> <outdir> – dump a drawable's own texture dictionary to PNG
pub fn cmd_cartex(a: &[String]) -> Result<()> {
    use mcla_formats::rage::texture::Texture;
    use mcla_formats::rage::Rd;
    let res = load_res(&a[0])?;
    let d = Drawable::parse(&res)?;
    let out = std::path::PathBuf::from(&a[1]);
    std::fs::create_dir_all(&out)?;
    let rd = Rd::new(&res);
    let mut names: Vec<String> = d.shader_group.shaders.iter().flat_map(|s| s.textures().map(|(_, n)| n.to_string())).collect();
    names.sort();
    names.dedup();
    for (h, o) in &d.texture_offsets {
        match Texture::parse(&rd, *o) {
            Ok(t) => {
                let name = names.iter().find(|n| mcla_formats::hash::joaat(n) == *h).cloned().unwrap_or_else(|| format!("{:08x}", h));
                println!("{:08x} {}x{} {:?} levels={} {} means={:?}", h, t.width, t.height, t.format, t.levels.len(), name, t.level_means());
                let rgba = t.to_rgba8();
                let f = std::fs::File::create(out.join(format!("{}.png", name.replace(['/', ':'], "_"))))?;
                let mut enc = png::Encoder::new(std::io::BufWriter::new(f), t.width, t.height);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                enc.write_header()?.write_image_data(&rgba)?;
            }
            Err(e) => println!("{:08x} ERROR {:#}", h, e),
        }
    }
    Ok(())
}

/// mcla-tool shaders <drawable resource>: every shader with its parameters
pub fn cmd_shaders(a: &[String]) -> Result<()> {
    use mcla_formats::rage::drawable::ShaderParam;
    let res = load_res(&a[0])?;
    let d = Drawable::parse(&res)?;
    for (i, sh) in d.shader_group.shaders.iter().enumerate() {
        println!("{:2} {} [{}]", i, sh.name, sh.source_path.rsplit(|c| c == '/' || c == '\\').next().unwrap_or(""));
        for (h, v) in &sh.params {
            match v {
                ShaderParam::Texture { name } => println!("      {:08x} tex {}", h, name),
                ShaderParam::Vector(v) => println!("      {:08x} {:?}", h, v.iter().map(|x| format!("({:.3} {:.3} {:.3} {:.3})", x.x, x.y, x.z, x.w)).collect::<Vec<_>>().join(" ")),
                ShaderParam::Unknown => println!("      {:08x} -", h),
            }
        }
    }
    Ok(())
}

/// mcla-tool meshcol <part drawable> [main drawable]: per mesh, the shader and the ranges of
/// its vertex colours and second UV set
pub fn cmd_meshcol(a: &[String]) -> Result<()> {
    let d = Drawable::parse(&load_res(&a[0])?)?;
    let main = match a.get(1) {
        Some(p) => Some(Drawable::parse(&load_res(p)?)?),
        None => None,
    };
    let shaders = if d.shader_group.shaders.is_empty() { main.as_ref().map(|m| &m.shader_group.shaders) } else { Some(&d.shader_group.shaders) };
    for m in &d.models {
        for (mesh, si) in &m.parts {
            let sname = shaders.and_then(|s| s.get(*si as usize)).map(|s| s.name.clone()).unwrap_or_default();
            let stat = |v: &Vec<[u8; 4]>| -> String {
                if v.is_empty() {
                    return "-".into();
                }
                let mut mn = [255u8; 4];
                let mut mx = [0u8; 4];
                let mut sum = [0u64; 4];
                for c in v {
                    for k in 0..4 {
                        mn[k] = mn[k].min(c[k]);
                        mx[k] = mx[k].max(c[k]);
                        sum[k] += c[k] as u64;
                    }
                }
                format!("mean {:?} min {:?} max {:?}", sum.map(|x| x / v.len() as u64), mn, mx)
            };
            let u1: Vec<f32> = mesh.uv1.iter().map(|u| u.x).collect();
            let mut ids: Vec<i32> = u1.iter().map(|x| x.round() as i32).collect();
            ids.sort();
            ids.dedup();
            let nsum = mesh.normals.iter().fold(glam::Vec3::ZERO, |a, n| a + *n) / mesh.normals.len().max(1) as f32;
            let bad = mesh.normals.iter().filter(|n| !n.is_finite() || n.length_squared() < 0.25).count();
            println!("{:<22} verts {:5} normals {} (bad {}, mean {:.2} {:.2} {:.2}) stride {} col0 {} | col1 {} | uv1.x ids {:?}", sname, mesh.positions.len(), mesh.normals.len(), bad, nsum.x, nsum.y, nsum.z, mesh.raw_stride, stat(&mesh.colors), stat(&mesh.colors1), &ids[..ids.len().min(8)]);
        }
    }
    Ok(())
}

/// mcla-tool secshaders <sector.r1> [name filter]: the sector's shaders with their parameters
pub fn cmd_secshaders(a: &[String]) -> Result<()> {
    use mcla_formats::rage::drawable::ShaderParam;
    let res = load_res(&a[0])?;
    let sec = mcla_formats::rage::sector::Sector::parse(&res)?;
    let filter = a.get(1).map(|s| s.to_ascii_lowercase());
    let mut seen = std::collections::HashSet::new();
    for sh in &sec.shaders {
        if filter.as_ref().map_or(false, |f| !sh.name.to_ascii_lowercase().contains(f.as_str())) || !seen.insert(sh.source_path.clone()) {
            continue;
        }
        println!("{} [{}]", sh.name, sh.source_path.rsplit(|c| c == '/' || c == '\\').next().unwrap_or(""));
        for (h, v) in &sh.params {
            match v {
                ShaderParam::Texture { name } => println!("      {:08x} tex {}", h, name),
                ShaderParam::Vector(v) => println!("      {:08x} {}", h, v.iter().take(4).map(|x| format!("({:.3} {:.3} {:.3} {:.3})", x.x, x.y, x.z, x.w)).collect::<Vec<_>>().join(" ")),
                ShaderParam::Unknown => println!("      {:08x} -", h),
            }
        }
    }
    Ok(())
}

/// `uimesh <file.r102>`: a UI mesh (`resources/ui/meshes`) with everything the
/// menu panels need - vertices with their bone, texture coordinates, the root
/// object's words (to find the skeleton) and the triangle list.
pub fn cmd_uimesh(a: &[String]) -> Result<()> {
    let res = load_res(&a[0])?;
    let rd = mcla_formats::rage::Rd::new(&res);
    let d = Drawable::parse(&res)?;
    println!("virtual {:#x} bytes", res.virtual_size);
    for o in (0..0xc0usize).step_by(4) {
        let w = rd.u32(o);
        let p = rd.ptr(o);
        println!("  root+{:02x}: {:08x} {}", o, w, p.map(|p| format!("-> {:#x}", p)).unwrap_or_else(|| format!("{:.4}", rd.f32(o))));
    }
    for (mi, m) in d.models.iter().enumerate() {
        for (pi, (mesh, si)) in m.parts.iter().enumerate() {
            println!("model {} part {} shader {} bone {} palette {:?}", mi, pi, si, m.bone, mesh.bone_palette);
            for i in 0..mesh.positions.len() {
                let p = mesh.positions[i];
                let uv = mesh.uv0.get(i).copied().unwrap_or_default();
                let bi = mesh.blend_indices.get(i).copied().unwrap_or_default();
                let bw = mesh.blend_weights.get(i).copied().unwrap_or_default();
                let n = mesh.normals.get(i).copied().unwrap_or_default();
                let c = mesh.colors.get(i).copied().unwrap_or_default();
                println!("  v{:3} pos {:8.4} {:8.4} {:8.4} uv {:.4} {:.4} n {:.2} {:.2} {:.2} col {:?} bi {:?} bw {:.2} {:.2} {:.2} {:.2}", i, p.x, p.y, p.z, uv.x, uv.y, n.x, n.y, n.z, c, bi, bw.x, bw.y, bw.z, bw.w);
            }
            if a.get(1).map(|s| s.as_str()) == Some("tris") {
                for t in mesh.indices.chunks(3) {
                    println!("  t {} {} {}", t[0], t[1], t[2]);
                }
            }
        }
    }
    Ok(())
}

/// mcla-tool fvfscan <dir>...: every geometry of every resource under the folders, one line each:
/// shader name hash, shader name, the vertex buffer's fvf channel mask, the file (for checking the effects' `dcl` masks)
pub fn cmd_fvfscan(a: &[String]) -> Result<()> {
    use mcla_formats::rage::{drawable::Shader, scan, vt, Rd};
    fn walk(p: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                let q = e.path();
                if q.is_dir() {
                    walk(&q, out);
                } else if q.extension().and_then(|x| x.to_str()).map_or(false, |x| x.len() > 1 && x.starts_with('r') && x[1..].bytes().all(|b| b.is_ascii_digit())) {
                    out.push(q);
                }
            }
        }
    }
    let mut files = Vec::new();
    for d in a {
        walk(std::path::Path::new(d), &mut files);
    }
    let so = std::io::stdout();
    let mut o = so.lock();
    // (a fragment's models index the shader group of the folder's main drawable)
    let mut folder: std::collections::HashMap<std::path::PathBuf, Vec<(String, u32)>> = std::collections::HashMap::new();
    let mut group_of = |dir: &std::path::Path| -> Vec<(String, u32)> {
        folder
            .entry(dir.to_path_buf())
            .or_insert_with(|| {
                for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                    let q = e.path();
                    if q.extension().and_then(|x| x.to_str()) == Some("r83") {
                        if let Ok(d) = load_res(&q.to_string_lossy()).and_then(|r| Ok(Drawable::parse(&r)?)) {
                            if !d.shader_group.shaders.is_empty() {
                                return d.shader_group.shaders.iter().map(|s| (s.name.escape_default().to_string(), s.name_hash)).collect();
                            }
                        }
                    }
                }
                Vec::new()
            })
            .clone()
    };
    for f in files {
        let Ok(res) = load_res(&f.to_string_lossy()) else { continue };
        let Ok(d) = Drawable::parse(&res) else { continue };
        let rd = Rd::new(&res);
        let mut done = std::collections::HashSet::new();
        // sector batches: root +18 {shader, geometry} pairs, +24 count
        let n = rd.u16(0x24) as usize;
        if let Some(arr) = rd.ptr(0x18) {
            if n > 0 && n < 100_000 && rd.has(arr, n * 8) {
                for i in 0..n {
                    let (Some(sp), Some(gp)) = (rd.res.ptr(rd.u32(arr + i * 8) & !0xF), rd.res.ptr(rd.u32(arr + i * 8 + 4) & !0xF)) else { continue };
                    if rd.vtable(sp) != vt::SHADER || rd.vtable(gp) != vt::GEOMETRY {
                        continue;
                    }
                    let (Ok(sh), Some(gr)) = (Shader::parse(&rd, sp), scan::geometry_at(&rd, gp)) else { continue };
                    done.insert(gp);
                    writeln!(o, "{:08x}	{}	{}	{}", sh.name_hash, sh.name.escape_default(), gr.vb.fvf.mask, f.display())?;
                }
            }
        }
        let own: Vec<(String, u32)> = d.shader_group.shaders.iter().map(|s| (s.name.escape_default().to_string(), s.name_hash)).collect();
        let shaders = if own.is_empty() { f.parent().map(|p| group_of(p)).unwrap_or_default() } else { own };
        for m in scan::scan_models(&rd) {
            for (i, &g) in m.geometries.iter().enumerate() {
                if done.contains(&g) {
                    continue;
                }
                if let Some(gr) = scan::geometry_at(&rd, g) {
                    let si = m.shader_index.get(i).copied().unwrap_or(0) as usize;
                    let (name, h) = shaders.get(si).cloned().unwrap_or_default();
                    writeln!(o, "{:08x}	{}	{}	{}", h, name, gr.vb.fvf.mask, f.display())?;
                }
            }
        }
    }
    Ok(())
}
