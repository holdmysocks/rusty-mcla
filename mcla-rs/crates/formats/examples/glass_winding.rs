//! glass_winding <car dir>: for every mesh drawn with a glass shader (by the .r83's shader names),
//! how the triangles' winding (b - a) x (c - a) lies against the stored normals, per window zone (uv1.x)
use mcla_formats::rage::scan::Drawable;
use mcla_formats::rsc::Resource;

fn load(p: &std::path::Path) -> Option<Resource> {
    let d = std::fs::read(p).ok()?;
    if &d[0..4] == b"MCLR" {
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u(12) as usize).min(data.len());
        return Some(Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data });
    }
    Resource::parse(&d).ok()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dir = std::path::Path::new(&a[1]);
    let files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    let r83 = files.iter().find(|p| p.extension().map_or(false, |e| e == "r83")).unwrap();
    let names: Vec<String> = Drawable::parse(&load(r83).unwrap()).unwrap().shader_group.shaders.iter().map(|s| s.name.clone()).collect();
    for p in &files {
        if p.extension().map_or(true, |e| e != "r63") {
            continue;
        }
        let Some(dr) = load(p).and_then(|r| Drawable::parse(&r).ok()) else { continue };
        for m in &dr.models {
            for (mesh, si) in &m.parts {
                let name = names.get(*si as usize).cloned().unwrap_or_default();
                if !name.to_ascii_lowercase().contains("glass") {
                    continue;
                }
                // zone -> (agree, disagree, area)
                let mut z: std::collections::BTreeMap<i32, (u32, u32, f32)> = Default::default();
                for t in mesh.indices.chunks_exact(3) {
                    let (i, j, k) = (t[0] as usize, t[1] as usize, t[2] as usize);
                    let f = (mesh.positions[j] - mesh.positions[i]).cross(mesh.positions[k] - mesh.positions[i]);
                    let n = mesh.normals.get(i).copied().unwrap_or_default() + mesh.normals.get(j).copied().unwrap_or_default() + mesh.normals.get(k).copied().unwrap_or_default();
                    let zone = mesh.uv1.get(i).map_or(-1, |u| u.x.round() as i32);
                    let e = z.entry(zone).or_default();
                    if f.dot(n) >= 0.0 { e.0 += 1 } else { e.1 += 1 }
                    e.2 += f.length() * 0.5;
                }
                println!("{} model {:x} shader {} {}: tris {} zones {:?}", p.file_name().unwrap().to_string_lossy(), m.off, si, name, mesh.indices.len() / 3, z);
            }
        }
    }
}
