//! The lamp colours of a car: the game's `LightTune`, nineteen entries in the order of the lamp
//! ids the lamp meshes carry in their second UV set (0 brake, 1 tail, 2 signal, 3 head, 4 high
//! beam, 5 side, 6 reverse, 8 fog, 12 third brake, 13 / 14 police ...). Each entry is a bulb
//! colour (`vLightBulbColor`: what the lamp shines with), a cover colour (`vLightCoverColor`:
//! its lens) and an attachment number. The game hands them to the lamp programs as rows of
//! `gCarConstantBuffer`, picked by the lamp id:
//!
//! * cars with a set-up (`mcCarConfig`, `resources/vehicle/configs/<set-up>.xcc`: the player's
//!   cars, rivals, police, delivery cars): nineteen records of 0x30 bytes at 0x230 - bulb
//!   (three big-endian floats and a filler word), cover (the same), attachment (u32 and three
//!   filler words). `PS_Light` of carlight.fxc reads bulb as c128 + id and cover as c163 + id.
//!   The brake and tail bulbs are stored four times over (4, 3.8, 3.4), as recorded.
//! * ambient traffic (`tune/ambients/traffic/<car>/mcAmbientRenderCarType.xml`, `LightTune`):
//!   the same nineteen entries as text. `PSAmbientCarLight` reads bulb as c128 + id and cover
//!   as c148 + id.
//!
//! Checked against the recorded frames (work/lighting/xcclamps.py, ambmatch.py): the Audi R8,
//! three delivery cars and five traffic types match their files to the last bit.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// lamp ids of a table
pub const IDS: usize = 19;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LampRows {
    /// bulb colour by lamp id
    pub bulb: [[f32; 3]; IDS],
    /// cover (lens) colour by lamp id
    pub cover: [[f32; 3]; IDS],
    /// a traffic car's table: its lamps are drawn by the ambient program
    pub ambient: bool,
    /// each entry's attachment (`iLightAttachment`): which of the car's lamp states lights it
    /// (traffic: sub_82792FD8's state table, `traffic_state`)
    pub att: [u8; IDS],
    /// a traffic type's `fGlowIntensityScale` (its glows' levels x this; 1 for set-ups)
    pub glow_scale: f32,
}

/// the attachments of the traffic tables on the disc (every `LightTune` has them in this order)
const ATT_TRAFFIC: [u8; IDS] = [0, 1, 6, 3, 4, 3, 11, 10, 5, 12, 12, 12, 2, 7, 7, 13, 14, 15, 16];

/// The Audi R8's table (`vp_aud_r8_08.xcc`): what a car without a table of its own is given.
pub const R8: LampRows = LampRows {
    bulb: [
        [3.804, 3.8, 3.4], [4.0, 3.8, 3.4], [0.9882, 0.6588, 0.0], [0.6157, 0.7961, 1.0], [1.0, 0.95, 0.85],
        [1.0, 0.95, 0.85], [1.0, 0.95, 0.85], [1.0, 0.95, 0.85], [1.0, 0.95, 0.85], [1.0, 0.95, 0.85],
        [1.0, 0.95, 0.85], [1.0, 0.95, 0.85], [1.0, 0.95, 0.85], [1.0, 0.95, 0.85], [1.0, 0.95, 0.85],
        [1.0, 0.95, 0.85], [1.0, 0.95, 0.85], [1.0, 0.95, 0.85], [1.0, 0.95, 0.85],
    ],
    cover: [
        [0.9804, 0.0314, 0.0], [0.9882, 0.0431, 0.0118], [0.5412, 0.1725, 0.0392], [1.0, 1.0, 1.0], [1.0, 1.0, 1.0],
        [1.0, 0.5031, 0.0], [0.9608, 0.9412, 0.9412], [0.6196, 0.6196, 0.6196], [1.0, 1.0, 1.0], [0.3137, 0.0118, 0.0],
        [0.781, 0.23, 0.0], [0.25, 0.25, 0.25], [0.9882, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0],
        [0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0],
    ],
    ambient: false,
    att: ATT_TRAFFIC,
    glow_scale: 1.0,
};

impl LampRows {
    /// The table of a car set-up (`.xcc`).
    pub fn from_xcc(d: &[u8]) -> Option<LampRows> {
        const AT: usize = 0x230;
        if d.len() < AT + IDS * 0x30 {
            return None;
        }
        let f = |o: usize| f32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]);
        let mut r = LampRows { bulb: [[0.0; 3]; IDS], cover: [[0.0; 3]; IDS], ambient: false, att: ATT_TRAFFIC, glow_scale: 1.0 };
        for i in 0..IDS {
            let o = AT + i * 0x30;
            for k in 0..3 {
                r.bulb[i][k] = f(o + 4 * k);
                r.cover[i][k] = f(o + 16 + 4 * k);
            }
            r.att[i] = u32::from_be_bytes([d[o + 32], d[o + 33], d[o + 34], d[o + 35]]).min(255) as u8;
            // (colours: nothing else is a table)
            if r.bulb[i].iter().chain(r.cover[i].iter()).any(|v| !v.is_finite() || *v < 0.0 || *v > 16.0) {
                return None;
            }
        }
        Some(r)
    }

    /// The table of a garage vehicle's record (the save's copy of its set-up, `lights`:
    /// nineteen (bulb, cover) rows); what the original draws the player's car with.
    pub fn from_record(rows: &[([f32; 3], [f32; 3])]) -> Option<LampRows> {
        if rows.len() != IDS {
            return None;
        }
        let mut r = LampRows { bulb: [[0.0; 3]; IDS], cover: [[0.0; 3]; IDS], ambient: false, att: ATT_TRAFFIC, glow_scale: 1.0 };
        for (i, (b, c)) in rows.iter().enumerate() {
            r.bulb[i] = *b;
            r.cover[i] = *c;
        }
        Some(r)
    }

    /// The table of a traffic car type (`mcAmbientRenderCarType.xml`).
    pub fn from_traffic_xml(s: &str) -> Option<LampRows> {
        fn vecs(s: &str, tag: &str) -> Vec<[f32; 3]> {
            let mut out = Vec::new();
            let mut rest = s;
            while let Some(p) = rest.find(tag) {
                rest = &rest[p + tag.len()..];
                let end = rest.find("/>").unwrap_or(rest.len());
                let el = &rest[..end];
                let mut v = [0.0f32; 3];
                let mut ok = true;
                for (k, key) in ["x=\"", "y=\"", "z=\""].iter().enumerate() {
                    match el.find(key).and_then(|a| el[a + 3..].split('"').next()).and_then(|t| t.trim().parse::<f32>().ok()) {
                        Some(x) => v[k] = x,
                        None => ok = false,
                    }
                }
                if ok {
                    out.push(v);
                }
            }
            out
        }
        let (b, c) = (vecs(s, "<vLightBulbColor"), vecs(s, "<vLightCoverColor"));
        if b.len() != IDS || c.len() != IDS {
            return None;
        }
        let mut r = LampRows { bulb: [[0.0; 3]; IDS], cover: [[0.0; 3]; IDS], ambient: true, att: ATT_TRAFFIC, glow_scale: 1.0 };
        r.bulb.copy_from_slice(&b);
        r.cover.copy_from_slice(&c);
        let att: Vec<u8> = s.split("<iLightAttachment value=\"").skip(1).filter_map(|t| t.split('"').next()?.trim().parse().ok()).collect();
        if att.len() == IDS {
            r.att.copy_from_slice(&att);
        }
        if let Some(v) = s.split("<fGlowIntensityScale value=\"").nth(1).and_then(|t| t.split('"').next()?.trim().parse::<f32>().ok()) {
            r.glow_scale = v;
        }
        Some(r)
    }

    /// The rows as the object uniform carries them: (bulb, cover); bulb[0].w = 1 on a traffic
    /// car's table.
    pub fn uniform(&self) -> ([[f32; 4]; IDS], [[f32; 4]; IDS]) {
        let mut a = [[0.0; 4]; IDS];
        let mut b = [[0.0; 4]; IDS];
        for i in 0..IDS {
            a[i] = [self.bulb[i][0], self.bulb[i][1], self.bulb[i][2], 0.0];
            b[i] = [self.cover[i][0], self.cover[i][1], self.cover[i][2], 0.0];
        }
        a[0][3] = if self.ambient { 1.0 } else { 0.0 };
        (a, b)
    }

    /// The colour of the car's two head-lamp lights in the night programs' dynamic lights
    /// (`gMultiLightDynamicLightData` c84..c86) and the lit particles' lane 3: three times the
    /// head lamp's (id 3) bulb through its cover. Recorded: the R8 (1.8471, 2.3882, 3.0) at
    /// 18:15, 20:00, 23:30 and 02:00; the Saleen S281 (2.1647, 2.0565, 1.858) at 01:06.
    pub fn head_light(&self) -> [f32; 3] {
        const HEAD: usize = 3;
        let (b, c) = (self.bulb[HEAD], self.cover[HEAD]);
        [3.0 * b[0] * c[0], 3.0 * b[1] * c[1], 3.0 * b[2] * c[2]]
    }

    /// The colour a lit lamp shines with (bulb through cover), brought to a full channel.
    pub fn lit(&self, id: usize) -> [f32; 3] {
        let i = id.min(IDS - 1);
        let c = [self.bulb[i][0] * self.cover[i][0], self.bulb[i][1] * self.cover[i][1], self.bulb[i][2] * self.cover[i][2]];
        let m = c[0].max(c[1]).max(c[2]);
        if m > 1e-4 { [c[0] / m, c[1] / m, c[2] / m] } else { [0.0; 3] }
    }
}

fn cached(key: &str, load: impl FnOnce(&std::path::Path) -> Option<LampRows>) -> Option<Arc<LampRows>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<LampRows>>>>> = OnceLock::new();
    let mut c = CACHE.get_or_init(Default::default).lock().ok()?;
    if let Some(r) = c.get(key) {
        return r.clone();
    }
    // (nothing is remembered until the data directory is known)
    let root = super::DATA_DIR.get()?;
    let r = load(std::path::Path::new(root)).map(Arc::new);
    c.insert(key.to_string(), r.clone());
    r
}

/// The table of a set-up by name ("GH_06_0_vp_dsn_280z_75", "vp_aud_r8_08").
pub fn config(name: &str) -> Option<Arc<LampRows>> {
    if name.is_empty() || name.contains(['/', '\\', ':']) {
        return None;
    }
    cached(name, |root| LampRows::from_xcc(&std::fs::read(setup_file(root, name)?).ok()?))
}

/// The file of a set-up (`resources/vehicle/configs/<name>.xcc`), found by its archive name
/// hash where the extraction named it otherwise: seven set-ups have no name in the install's
/// name list (`0x2aa070fd`, the police `vp_se_*` ones) and two were given a colliding string
/// (`vp_sln_s7_06.xcc` as "BoundingSphere 976.132.xtd", `vp_lam_murcielago_06_DUB.xcc` as
/// "0.5486715436.wav": the same joaat). The archive itself only knows the hash.
pub fn setup_file(data_dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    if name.is_empty() || name.contains(['/', '\\', ':']) {
        return None;
    }
    let dir = data_dir.join("resources/vehicle/configs");
    let plain = dir.join(format!("{name}.xcc"));
    if plain.is_file() {
        return Some(plain);
    }
    static BY_HASH: OnceLock<Mutex<HashMap<std::path::PathBuf, HashMap<u32, std::path::PathBuf>>>> = OnceLock::new();
    let mut all = BY_HASH.get_or_init(Default::default).lock().ok()?;
    let table = all.entry(dir.clone()).or_insert_with(|| {
        let mut t = HashMap::new();
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let f = e.file_name().to_string_lossy().to_string();
            let h = match f.strip_prefix("0x").and_then(|x| u32::from_str_radix(x, 16).ok()) {
                Some(h) if f.len() == 10 => h,
                _ => mcla_formats::hash::joaat(&f),
            };
            t.insert(h, e.path());
        }
        t
    });
    table.get(&mcla_formats::hash::joaat(&format!("{name}.xcc"))).cloned()
}

/// The table of a traffic car type by model name ("va_taxi_96").
pub fn traffic(model: &str) -> Option<Arc<LampRows>> {
    if !model.starts_with("va_") || model.contains(['/', '\\', ':']) {
        return None;
    }
    cached(model, |root| LampRows::from_traffic_xml(&std::fs::read_to_string(root.join("tune/ambients/traffic").join(model).join("mcAmbientRenderCarType.xml")).ok()?))
}

/// The table a car is drawn with: its set-up's where it wears one, else its traffic type's,
/// else the set-up named after the model (a car as it is sold). None: no table on disc.
pub fn for_car(model: &str, setup: &str) -> Option<Arc<LampRows>> {
    // (a set-up is named after its car: a livery borrowed from another model brings no lamps)
    let own = !model.is_empty() && setup.len() >= model.len() && setup.to_ascii_lowercase().contains(&model.to_ascii_lowercase());
    (if own { config(setup) } else { None }).or_else(|| traffic(model)).or_else(|| config(model))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traffic_xml_rows() {
        let mut s = String::from("<mcAmbientRenderCarType><LightTune>");
        for i in 0..IDS {
            s += &format!("<Item><vLightBulbColor x=\"0.5\" y=\"{}\" z=\"0.25\"/>\n<vLightCoverColor x=\"1.000000\" y=\"0.8\" z=\"0\"/><iLightAttachment value=\"3\"/></Item>", i);
        }
        s += "</LightTune></mcAmbientRenderCarType>";
        let r = LampRows::from_traffic_xml(&s).unwrap();
        assert!(r.ambient);
        assert_eq!(r.bulb[7], [0.5, 7.0, 0.25]);
        assert_eq!(r.cover[18], [1.0, 0.8, 0.0]);
        assert!(LampRows::from_traffic_xml("<a/>").is_none());
    }

    #[test]
    fn xcc_rows() {
        let mut d = vec![0u8; 0x230 + IDS * 0x30];
        for i in 0..IDS {
            let o = 0x230 + i * 0x30;
            d[o..o + 4].copy_from_slice(&(i as f32 * 0.125).to_be_bytes());
            d[o + 16 + 8..o + 16 + 12].copy_from_slice(&0.5f32.to_be_bytes());
            d[o + 12..o + 16].copy_from_slice(&[0x7f, 0x80, 0, 1]);
        }
        let r = LampRows::from_xcc(&d).unwrap();
        assert_eq!(r.bulb[4][0], 0.5);
        assert_eq!(r.cover[18], [0.0, 0.0, 0.5]);
        assert!(!r.ambient);
        assert!(LampRows::from_xcc(&d[..0x400]).is_none());
        assert_eq!(R8.lit(3), [0.6157, 0.7961, 1.0]);
    }

    /// The head-lamp lights' colour against the recordings of the original (c84..c86 of the
    /// night programs): the R8 at 23:30 (`cap/car_n`), the Saleen S281 of the race save at 01:06
    /// (`work/checkpoints2/cap/f4`, `vp_sln_s281_06.xcc` head row: bulb (1, 0.95, 0.85), cover
    /// (0.72157, 0.72157, 0.72862)).
    #[test]
    fn head_light_colour() {
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 2e-4);
        assert!(close(R8.head_light(), [1.8471, 2.3882, 3.0]), "{:?}", R8.head_light());
        let mut s = R8;
        s.bulb[3] = [1.0, 0.95, 0.85];
        s.cover[3] = [0.7215686, 0.7215686, 0.7286152];
        assert!(close(s.head_light(), [2.1647, 2.0565, 1.858]), "{:?}", s.head_light());
    }
}
