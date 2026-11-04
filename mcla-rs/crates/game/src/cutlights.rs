//! The cutscene light list: the lamps a story character takes at night while a scene plays.
//!
//! The light manager's loader (F-57097) reads `tune/lighting/<name>_cutscene.lights`, a LightFileList, with <name>
//! "SC" when the content mask has bit 2 (the Complete Edition's 0x13F) and "LA" otherwise, and every file it lists
//! (`CutScene_Dyn_light`, `SC_CutScene_Dyn_lights`): their `<lights>` Items become 96-byte Light records in file order.
//! Records that are near each other are merged (F-32755), but the disc's two files have no such pair: the running
//! original holds 82 = 77 + 5 records. Path A's night branch (F-75930) scores them instead of the lamp grid when the
//! cine probe answers (F-22850: a cine script runs, or a hangout forces it) and the byte F-19428 sets is 1. No script
//! clears that byte. Checked against the 82 records read out of the running original (test below).

use crate::render::LampRec;
use glam::Vec3;
use std::path::Path;

/// One Light record (F-97843's fields as the XML fills them).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Light {
    pub pos: Vec3,
    pub color: Vec3,
    /// +32: (0, 0, -1) turned by the Direction angles (F-13367)
    pub dir: Vec3,
    pub intensity: f32,
    pub cone: f32,
    /// +72: a penumbra of 0 is taken as 1 (F-92648)
    pub penumbra: f32,
    /// +76 / +80: sat(cos x scale + bias)
    pub scale: f32,
    pub bias: f32,
    /// +88: 1 = spot
    pub spot: bool,
}

impl Light {
    /// As the score (F-60154) reads it.
    pub fn lamp(&self) -> LampRec {
        LampRec { pos: self.pos, color: self.color, strength: self.intensity, back: -self.dir, cone: [self.scale, self.bias] }
    }
}

/// F-13367: the angles in degrees (x 0.0174556, 0x820410C4) through the rotation F-28753, then (0, 0, -1) by its
/// rows: the third row negated (the four-lane sum adds +0, so a -0 comes out +0).
fn direction(deg: Vec3) -> Vec3 {
    let k = f32::from_bits(0x3c8e_fef4);
    let r = crate::physics::artic_build::euler_rows([deg.x * k, deg.y * k, deg.z * k]);
    Vec3::new(-r[2][0] + 0.0, -r[2][1] + 0.0, -r[2][2] + 0.0)
}

/// F-51633: o = cone x (1/720) x 6.28, q = penumbra x (1/720) x 6.28 (a negative penumbra widens o by q and turns
/// q round), s = 1 / (cos(o + q) - cos(o)) with the C runtime's cosine in double; scale = -s, bias = 1 + s cos(o).
fn cone_terms(cone: f32, pen: f32) -> (f32, f32) {
    let (k, tau) = (f32::from_bits(0x3ab6_0b61), f32::from_bits(0x40c8_f5c3));
    let mut o = (cone * k) * tau;
    let mut q = (pen * k) * tau;
    if pen < 0.0 {
        o = q + o;
        q = -q;
    }
    let cos = |x: f32| crate::physics::exact::guest_cos(x as f64) as f32;
    let s = 1.0 / (cos(q + o) - cos(o));
    let b = -(s * cos(o));
    (-s, 1.0 - b)
}

fn attr(item: &str, tag: &str, key: &str) -> Option<f32> {
    let i = item.find(&format!("<{tag}"))?;
    let rest = &item[i..i + item[i..].find('>')?];
    let k = rest.find(&format!("{key}=\"")).or_else(|| rest.find(&format!("{key} =\"")))?;
    let v = &rest[k..];
    let a = v.find('"')? + 1;
    let b = a + v[a..].find('"')?;
    v[a..b].trim().parse().ok()
}

/// The records of one LightList file's `<lights>` (F-97843 defaults: penumbra 0, cone 360, type 0).
fn parse_list(x: &str) -> Vec<Light> {
    let Some(body) = x.split("<lights>").nth(1).and_then(|b| b.split("</lights>").next()) else { return Vec::new() };
    body.split("<Item>")
        .skip(1)
        .filter_map(|it| {
            let v3 = |t: &str| Some(Vec3::new(attr(it, t, "x")?, attr(it, t, "y")?, attr(it, t, "z")?));
            let spot = attr(it, "Type", "value").unwrap_or(0.0) == 1.0;
            let cone = attr(it, "ConeAngle", "value").unwrap_or(360.0);
            let pen = attr(it, "Penumbra", "value").unwrap_or(0.0);
            let penumbra = if pen == 0.0 { 1.0 } else { pen };
            let (scale, bias) = if spot { cone_terms(cone, penumbra) } else { (0.0, 1.0) };
            Some(Light {
                pos: v3("Position")?,
                color: v3("Colour")?,
                dir: direction(v3("Direction").unwrap_or(Vec3::ZERO)),
                intensity: attr(it, "Intensity", "value").unwrap_or(0.0),
                cone,
                penumbra,
                scale,
                bias,
                spot,
            })
        })
        .collect()
}

/// A tune/lighting file by its name: as stored, or under the hash of its lower-cased name.
fn read(data_dir: &str, file: &str) -> Option<String> {
    let dir = Path::new(data_dir).join("tune/lighting");
    std::fs::read_to_string(dir.join(file))
        .or_else(|_| std::fs::read_to_string(dir.join(format!("0x{:08x}", mcla_formats::hash::joaat(&file.to_ascii_lowercase())))))
        .ok()
}

/// F-57097's cutscene part: the file list, then each listed file's lights in order.
pub fn load(data_dir: &str) -> Vec<Light> {
    let name = if crate::mc4::CONTENT_MASK_START[0] & 4 != 0 { "SC" } else { "LA" };
    let Some(list) = read(data_dir, &format!("{name}_cutscene.lights")) else { return Vec::new() };
    let mut out = Vec::new();
    for n in list.split("<Name").skip(1).filter_map(|s| Some(s[s.find('>')? + 1..s.find("</Name>")?].trim().to_string())) {
        if let Some(x) = read(data_dir, &format!("{n}.xml")) {
            out.extend(parse_list(&x));
        }
    }
    out
}

/// The list as loaded at start (`init`), for the characters' night lights.
pub static LIST: std::sync::OnceLock<Vec<Light>> = std::sync::OnceLock::new();

pub fn init(data_dir: &str) {
    let _ = LIST.set(load(data_dir));
}

/// F-65001 on the list and the cut that follows it in F-75930: every record scored at `p` (F-60154), the best four
/// by score (a later equal score after the earlier), those above 1e-6 kept. Empty: the caller takes the lamp grid
/// (byte 0x8287E269 is 0).
pub fn best(lights: &[Light], p: Vec3) -> Vec<(f32, LampRec)> {
    let mut top: Vec<(f32, LampRec)> = Vec::with_capacity(5);
    for l in lights {
        let r = l.lamp();
        let s = crate::figlight::score(&r, p);
        let at = top.iter().position(|t| t.0 < s).unwrap_or(top.len());
        if at < 4 {
            top.insert(at, (s, r));
            top.truncate(4);
        }
    }
    top.retain(|t| t.0 > f32::from_bits(0x3586_37bd));
    top
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every field of the 82 records the running original holds (work/lighting-trails/cutl.py, read 2026-11-04) is
    /// equal bit for bit to the records the port builds from the disc's files.
    #[test]
    fn the_list_matches_the_original() {
        let dir = "D:/MCLA/extracted/cache";
        if !Path::new(dir).join("tune/lighting").exists() {
            return;
        }
        let ours = load(dir);
        let want = include_str!("../testdata/cutscene_lights_ref.txt");
        let rows: Vec<&str> = want.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(ours.len(), rows.len());
        let mut bad = 0;
        for (l, row) in ours.iter().zip(&rows) {
            let w: Vec<u32> = row.split_whitespace().map(|h| u32::from_str_radix(h, 16).unwrap()).collect();
            let got = [l.pos.x, l.pos.y, l.pos.z, l.color.x, l.color.y, l.color.z, l.dir.x, l.dir.y, l.dir.z, l.intensity, l.cone, l.penumbra, l.scale, l.bias]
                .map(f32::to_bits);
            if got[..] != w[..14] || (l.spot as u32) != w[14] {
                bad += 1;
                eprintln!("{got:08x?} {:?}\n{w:08x?}", l.spot);
            }
        }
        assert_eq!(bad, 0, "records differ");
    }

    /// GH_06 recorded at 23:00 (work/lighting-trails/night/cap/g.24 .. g.43, 1,430 cast draws, all within 1.3e-7 of the
    /// rule - check.py there): two draws' three lights from the cutscene list and the key light, and ambYPos (c57) =
    /// the fill map's cube at the draw's place for every distinct place of the recordings.
    #[test]
    fn the_night_scene_draws_match() {
        let dir = "D:/MCLA/extracted/cache";
        if !Path::new(dir).join("tune/lighting").exists() || !Path::new(dir).join("tune/city/sc").exists() {
            return;
        }
        init(dir);
        crate::fillmap::init(dir);
        let key = Vec3::new(-67559.0, -73716.5, -1286.73);
        let cl = crate::skin::CharLight {
            night: true,
            night_path: true,
            cine: true,
            dynamic: Some(crate::figlight::Dynamic { heads: [None; 2], key: Some((Vec3::splat(0.05098), key / 1.0e5)), rec3: None }),
            ..Default::default()
        };
        let draws: [(Vec3, [[f32; 4]; 6]); 2] = [
            (
                Vec3::new(-179.411, 18.178, -751.707),
                [
                    [-180.261, 19.6579, -753.99, 1000.0],
                    [-176.271, 20.6403, -749.831, 1000.0],
                    [-176.244, 20.713, -752.164, 1000.0],
                    [0.23076, 0.22192, 0.17007, -1.0],
                    [1.0, 0.95076, 0.721, -1.0],
                    [1.0, 0.93664, 0.641, -1.0],
                ],
            ),
            (
                Vec3::new(-175.82, 18.216, -756.775),
                [
                    [-176.271, 20.6403, -749.831, 1000.0],
                    [-180.261, 19.6579, -753.99, 1000.0],
                    [-67559.0, -73716.5, -1286.73, 1.0e6],
                    [1.0, 0.95076, 0.721, -0.36833],
                    [0.23076, 0.22192, 0.17007, -0.33833],
                    [0.05098, 0.05098, 0.05098, -0.05098],
                ],
            ),
        ];
        for (p, want) in draws {
            for (r, w) in cl.night_slots(p, None).iter().zip(want.iter()) {
                for k in 0..4 {
                    assert!((r[k] - w[k]).abs() <= 2e-4 * w[k].abs().max(1.0), "{p}: {r:?} vs {w:?}");
                }
            }
        }
        // (c57 of the recordings, rounded to 6 digits: map / 255 x 0.25)
        for (x, z, c57) in [
            (-179.411, -751.707, [0.07451, 0.07451, 0.058824]),
            (-175.82, -756.775, [0.058824, 0.05098, 0.043137]),
            (-195.376, -748.058, [0.117647, 0.098039, 0.069608]),
            (-176.197, -751.846, [0.043137, 0.043137, 0.035294]),
        ] {
            let c = crate::fillmap::cube_at(x, z);
            assert!((c - Vec3::from_array(c57)).abs().max_element() < 2e-6, "({x}, {z}): {c} vs {c57:?}");
        }
    }

    /// F-65001's order and cut: the best four by score, a later equal score after the earlier, nothing at or under 1e-6.
    #[test]
    fn best_four_in_the_original_order() {
        let at = |x: f32, i: f32| Light { pos: Vec3::new(x, 2.0, 0.0), color: Vec3::ONE, intensity: i, bias: 1.0, ..Default::default() };
        let p = Vec3::ZERO;
        let l = [at(1.0, 0.5), at(3.0, 1e-6), at(-1.0, 0.5), at(2.0, 100.0), at(5.0, 2.0), at(9.0, 4.0), at(0.5, 0.0)];
        let b = best(&l, p);
        let xs: Vec<f32> = b.iter().map(|t| t.1.pos.x).collect();
        assert_eq!(xs, vec![2.0, 1.0, -1.0, 5.0]);
        assert!(best(&[at(3.0, 1e-6)], p).is_empty());
    }
}
