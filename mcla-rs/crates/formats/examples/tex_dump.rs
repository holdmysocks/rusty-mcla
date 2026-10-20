//! tex_dump <.r83> <name part> <out prefix>: every texture of a drawable whose name holds `name part`, level 0
//! decoded to RGBA8, written as <out prefix>_<name>.rgba (raw, width x height x 4) with its size on stdout.
use mcla_formats::rage::texture::Texture;
use mcla_formats::rage::{scan::Drawable, Rd};
use mcla_formats::rsc::Resource;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let d = std::fs::read(&a[1]).expect("read");
    assert!(&d[0..4] == b"MCLR");
    let rtype = u32::from_le_bytes(d[4..8].try_into().unwrap());
    let flags = u32::from_le_bytes(d[8..12].try_into().unwrap());
    let vs = (u32::from_le_bytes(d[12..16].try_into().unwrap()) as usize).min(d.len() - 16);
    let data = d[16..].to_vec();
    let res = Resource { rtype, flags, virtual_size: vs, physical_size: data.len() - vs, data };
    let dr = Drawable::parse(&res).expect("drawable");
    let rd = Rd::new(&res);
    // ("@<hex offset>": the texture object at that offset of the resource's data instead, e.g. one outside the dictionary)
    let offs: Vec<usize> = match a[2].strip_prefix("@") {
        Some(h) => vec![usize::from_str_radix(h.trim_start_matches("0x"), 16).unwrap()],
        None => dr.texture_offsets.iter().map(|(_, o)| *o).collect(),
    };
    for o in &offs {
        let Ok(t) = Texture::parse(&rd, *o) else { continue };
        if !a[2].starts_with('@') && !t.name.to_lowercase().contains(&a[2].to_lowercase()) {
            continue;
        }
        let px = t.to_rgba8();
        let name = t.name.replace(|c: char| matches!(c, '/' | '\\' | ':'), "_");
        std::fs::write(format!("{}_{}.rgba", a[3], name), &px).unwrap();
        println!("{} {} {} {:?} fetch {:08x?}", t.name, t.width, t.height, t.format, t.fetch);
    }
}
