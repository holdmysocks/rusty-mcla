//! car_shader_dump <.r83> [name filter]: a car drawable's shaders, their parameter hashes and values
use mcla_formats::rage::drawable::ShaderParam;
use mcla_formats::rage::scan::Drawable;
use mcla_formats::rsc::Resource;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let d = std::fs::read(&a[1]).unwrap();
    let res = if &d[0..4] == b"MCLR" {
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u(12) as usize).min(data.len());
        Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data }
    } else { Resource::parse(&d).unwrap() };
    let dr = Drawable::parse(&res).unwrap();
    let want = a.get(2).map(|s| s.to_ascii_lowercase());
    for (i, sh) in dr.shader_group.shaders.iter().enumerate() {
        if want.as_ref().map_or(false, |w| !sh.name.to_ascii_lowercase().contains(w.as_str()) && !sh.source_path.to_ascii_lowercase().contains(w.as_str())) { continue }
        println!("{i} {} {} 0x{:08x}", sh.name, sh.source_path, sh.name_hash);
        for (h, p) in &sh.params {
            match p {
                ShaderParam::Texture { name } => println!("   0x{h:08x} tex {name}"),
                ShaderParam::Vector(v) => println!("   0x{h:08x} {:?}", v.iter().map(|x| x.to_array()).collect::<Vec<_>>()),
                ShaderParam::Unknown => println!("   0x{h:08x} ?"),
            }
        }
    }
}
