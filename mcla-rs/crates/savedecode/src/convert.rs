//! The importer proper (port of `tools/import_mc4_save.py`): carries what the port can use out of an original save into a save of the port,
//! and puts everything else, by the original's names, into the `mc4` block that `crates/game/src/mc4.rs` reads.

use crate::container::Container;
use crate::tables::VEHICLES;
use crate::util::*;
use crate::{career, options, small, vehicle, Error, Result};
use serde_json::{json, Map, Value};

/// A garage record: 8176 bytes, the vehicle's name 8109 bytes in, the thirteen performance stages at +28.
const RECORD: usize = 8176;
const NAME_AT: usize = 8109;
const PERF_AT: usize = 28;
/// The fitted parts' bytes in a record (the port's names), in the importer's order.
const PART_BYTES: &[(&str, usize)] = &[
    ("hood", 0x862), ("bumper_r", 0x863), ("bumper_f", 0x865), ("skirts", 0x868), ("spoiler", 0x869), ("exhaust", 0x877), ("seats", 0x88d), ("body", 0x89b),
    ("headlight", 0x86b), ("taillight", 0x86c), ("steer_whl", 0x894), ("post_gauge", 0x895), ("scissor_door", 0x89a), ("blower", 0x870),
    ("side_exhaust", 0x878), ("intercooler", 0x89d), ("bike_tail", 0x871), ("bike_tank", 0x872), ("bike_cowl", 0x873), ("carbon_hood", 0x898),
];
const CFG_SLOTS: &[&str] = &["hood", "bumper_r", "bumper_f", "skirts", "spoiler", "exhaust", "seats", "body"];
/// Layout notes and constant set-up data of the decoders' output (the importer's `NOTES`).
const NOTES: &[&str] = &[
    "_layout", "_residue", "_unknown", "_pad_unexpected", "_not_in_this_entry", "off", "entry", "vtable", "class_key", "base_unused_8", "dirty_flags",
    "dirty_flags_all", "carcfgint", "vehicle_record_off", "vehicle_record_len", "slot29_raw_hex", "_version",
];
const GARAGE_DROP: &[&str] = &[
    "_layout", "_residue", "_unknown", "_pad_unexpected", "_not_in_this_entry", "off", "entry", "vtable", "class_key", "base_unused_8", "dirty_flags",
    "dirty_flags_all", "carcfgint", "vehicle_record_off", "vehicle_record_len", "slot29_raw_hex", "_version", "vinyl", "vinyl_spare_layers_224_287",
    "vinyl_zone_group", "PreDmg", "stages_raw", "brand_raw",
];

/// What the conversion found, for the confirmation text and the log.
#[derive(Debug, Clone)]
pub struct Summary {
    /// The save's container version: 14 = Xbox 360, 15 = PS3.
    pub container_version: u32,
    pub statistics: usize,
    pub vehicles: Vec<String>,
    pub current_vehicle: String,
    pub missions: usize,
    pub missions_complete: usize,
    pub money: u32,
    pub reputation: Option<u32>,
    pub collectables: usize,
    pub user_races: usize,
    pub favourite_races: usize,
}

pub struct Converted {
    pub save: Map<String, Value>,
    pub summary: Summary,
}

fn py_isspace(c: char) -> bool {
    matches!(c as u32, 9..=13 | 28..=32 | 0x85 | 0xa0)
}

/// The game's race table (`tune/race/0x0f1f4d8d` of the data folder) as the career decoder reads it: every word after `Id` and white space.
/// The race hashes of the save and the race history are named from it.
pub fn race_table(file: &[u8]) -> Vec<String> {
    let t: Vec<char> = file.iter().map(|b| *b as char).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < t.len() {
        if t[i] == 'I' && t[i + 1] == 'd' {
            let mut j = i + 2;
            while j < t.len() && py_isspace(t[j]) {
                j += 1;
            }
            let k = j;
            while j < t.len() && !py_isspace(t[j]) {
                j += 1;
            }
            if k > i + 2 && j > k {
                out.push(t[k..j].iter().collect());
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

struct Garage {
    cars: Vec<String>,
    perf: Map<String, Value>,
    perf_raw: Vec<(String, Vec<u8>)>,
    dims: Vec<(String, Vec<u8>)>,
    plates: Map<String, Value>,
    paint: Option<([f64; 3], u32, u32)>,
    wheels: Vec<(String, [u8; 3])>,
    mats: Option<Vec<i64>>,
    paints: Vec<(String, ([f64; 3], u32, u32))>,
    parts: Vec<(String, Vec<(&'static str, Value)>)>,
    finishes: Map<String, Value>,
    liveries: Map<String, Value>,
}

/// `vpd?_[a-z0-9_]+\0` found first in the file's first 0x40000 bytes.
fn first_name(d: &[u8]) -> Option<usize> {
    let end = d.len().min(0x40000);
    let d = &d[..end];
    let is_class = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_';
    (0..d.len()).find(|&i| {
        if d[i] != b'v' || d.get(i + 1) != Some(&b'p') {
            return false;
        }
        let mut j = i + 2;
        if d.get(j) == Some(&b'd') {
            j += 1;
        }
        if d.get(j) != Some(&b'_') {
            return false;
        }
        j += 1;
        let k = j;
        while j < d.len() && is_class(d[j]) {
            j += 1;
        }
        j > k && d.get(j) == Some(&0)
    })
}

/// `vpd?_[A-Za-z0-9_]{4,36}` as a full match.
fn is_vehicle_name(n: &[u8]) -> bool {
    let rest = n.strip_prefix(b"vpd_").or_else(|| n.strip_prefix(b"vp_"));
    match rest {
        Some(r) => (4..=36).contains(&r.len()) && r.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_'),
        None => false,
    }
}

fn tri(b: B, o: usize) -> [f64; 3] {
    [b.f32(o), b.f32(o + 4), b.f32(o + 8)]
}

fn in_unit(c: &[f64]) -> bool {
    c.iter().all(|v| (0.0..=1.0).contains(v))
}

/// The port's garage key of a slot: a model's first slot by its name, the next ones `<name>#2`, `#3` (the
/// game's `catalog::copy_key`; the original's garage may hold a model more than once, F-91320), so that each
/// copy's paint, parts and plate stay its own. `seen` counts the model's slots so far.
fn copy_key(seen: &mut std::collections::HashMap<String, usize>, name: &str) -> String {
    let n = seen.entry(name.to_string()).or_insert(0);
    *n += 1;
    if *n > 1 { format!("{name}#{n}") } else { name.to_string() }
}

fn garage(d: &[u8]) -> Result<Garage> {
    let b = B(d);
    let mut seen = std::collections::HashMap::new();
    let mut g = Garage {
        cars: vec![], perf: Map::new(), perf_raw: vec![], dims: vec![], plates: Map::new(), paint: None, wheels: vec![], mats: None, paints: vec![],
        parts: vec![], finishes: Map::new(), liveries: Map::new(),
    };
    let mut first = first_name(d);
    while let Some(f) = first {
        if f + 40 >= d.len() {
            break;
        }
        let name_raw: Vec<u8> = d[f..f + 40].iter().copied().take_while(|c| *c != 0).collect();
        if !is_vehicle_name(&name_raw) {
            break;
        }
        let name = copy_key(&mut seen, &latin1(&name_raw).to_lowercase());
        let rec = f.checked_sub(NAME_AT).ok_or_else(|| Error::new("layout", "a vehicle name before the garage"))?;
        if rec + RECORD > d.len() {
            return Err(Error::new("layout", "the garage runs past the end of the file"));
        }
        g.cars.push(name.clone());
        let st = b.slice(rec + PERF_AT, 13).to_vec();
        g.perf.insert(name.clone(), json!(st));
        g.perf_raw.push((name.clone(), st));
        g.dims.push((name.clone(), b.slice(rec + 0x83a, 8).to_vec()));
        let plate = b.slice(rec + 0x847, 7);
        if plate.iter().all(|c| (32..127).contains(c)) {
            g.plates.insert(name.clone(), json!(latin1(plate)));
        }
        g.wheels.push((name.clone(), [b.u8(rec + 0x844), b.u8(rec + 0x845), b.u8(rec + 0x846)]));
        let mut parts: Vec<(&'static str, Value)> = PART_BYTES.iter().map(|(k, o)| (*k, json!(b.u8(rec + o)))).collect();
        // (the port's rim finish: 0 painted, 1 chrome, 2 brushed metal, 3 carbon fibre - the record's 0, 1, 3, 2)
        let own_mats: Vec<i64> = (0..3).map(|i| match b.u32(rec + 0x178 + 4 * i) { 0 => 0, 1 => 1, 3 => 2, 2 => 3, _ => 1 }).collect();
        if g.mats.is_none() {
            g.mats = Some(own_mats.clone());
        }
        // The vehicle's other colours: interior neon +0x60, gauge colour A +0x70, B +0x80, rim spoke A +0x90, spoke B +0xa0, lip +0xb0, interior
        // colour A +0x5c0, B +0x5d0, C +0x5e0, in the port's order of slots (neon, gauges, interior, rims).
        let mut slots = Vec::new();
        for (k, off) in [0x60usize, 0x70, 0x80, 0x5c0, 0x5d0, 0x5e0, 0x90, 0xa0, 0xb0].into_iter().enumerate() {
            let c = tri(b, rec + off);
            if !in_unit(&c) {
                slots.push(Value::Null);
                continue;
            }
            let h = rgb_to_hsv(c[0], c[1], c[2]);
            // (a black cabin and grey gauge lights are the stock look, which the port draws its own way)
            let stock = ((3..=5).contains(&k) && h.2 < 0.08) || ((1..=2).contains(&k) && h.1 < 0.15);
            slots.push(if stock { Value::Null } else { json!([num(round_to(h.0, 4)), num(round_to(h.1, 4)), num(round_to(h.2, 4))]) });
        }
        g.finishes.insert(name.clone(), json!([slots, own_mats]));
        // The vehicle's vinyls: the set-up file's layer records (20 bytes from +0x904, the zones top, left, right, front, back).
        let mut zones = Vec::new();
        let mut o = rec + 0x904;
        let mut any = false;
        for cap in [64usize, 64, 64, 16, 16] {
            let mut layers = Vec::new();
            for _ in 0..cap {
                let r = b.slice(o, 20);
                o += 20;
                let shape = B(r).u16(16);
                if shape == 0xffff || r[19] != 0 {
                    continue;
                }
                let rb = B(r);
                let blend = match r[18] {
                    1 => "AlphaCutout",
                    2 => "NegativeCutout",
                    _ => "Normal",
                };
                layers.push(json!({
                    "shape": shape, "scale": [num(rb.half(0)), num(rb.half(2))], "pos": [num(rb.half(4)), num(rb.half(6))], "rot": num(rb.half(14)),
                    "skew": num(rb.half(12)), "color": [r[9], r[10], r[11], r[8]], "blend": blend,
                }));
                any = true;
            }
            zones.push(Value::Array(layers));
        }
        if any {
            g.liveries.insert(name.clone(), json!({"zones": zones}));
        }
        let e = rec + 0x660;
        let rgb = tri(b, e);
        let (kind, fade) = (b.u32(e + 0x20), b.u32(e + 0x24));
        if in_unit(&rgb) && (1..=5).contains(&kind) {
            g.paints.retain(|(k, _)| *k != name);
            g.paints.push((name.clone(), (rgb, kind, fade)));
            // (the six entries are the paint shop's PAINT AREAs in its order - main body, doors, hood, front bumper, rear bumper, sideskirts)
            let mut own = Map::new();
            for (n, key) in ["doors", "hood", "bumper_f", "bumper_r", "skirts"].iter().enumerate() {
                let n = n + 1;
                let h = tri(b, e + 0x30 * n);
                if in_unit(&h) && h.iter().zip(rgb.iter()).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max) > 0.02 {
                    own.insert((*key).into(), json!(h.iter().map(|c| round_int(c * 255.0)).collect::<Vec<_>>()));
                }
            }
            if !own.is_empty() {
                parts.push(("_part_paint", Value::Object(own)));
            }
            if g.paint.is_none() {
                g.paint = Some((rgb, kind, fade));
            }
        }
        g.parts.retain(|(k, _)| *k != name);
        g.parts.push((name, parts));
        first = Some(f + RECORD);
    }
    Ok(g)
}

fn lookup<'a, T>(v: &'a [(String, T)], k: &str) -> Option<&'a T> {
    v.iter().rev().find(|(n, _)| n == k).map(|(_, x)| x)
}

/// `dict.setdefault(key, {})` that also replaces a value which is not an object.
fn obj<'a>(m: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    let e = m.entry(key.to_string()).or_insert_with(|| Value::Object(Map::new()));
    if !e.is_object() {
        *e = Value::Object(Map::new());
    }
    match e {
        Value::Object(o) => o,
        _ => unreachable!(),
    }
}

fn hsv4(rgb: [f64; 3]) -> Value {
    let h = rgb_to_hsv(rgb[0], rgb[1], rgb[2]);
    json!([num(round_to(h.0, 4)), num(round_to(h.1, 4)), num(round_to(h.2, 4))])
}

fn linear(rgb: [f64; 3]) -> [f64; 3] {
    [rgb[0].powf(2.2), rgb[1].powf(2.2), rgb[2].powf(2.2)]
}

/// Converts a save of the original game.
///
/// * `sav`: the bytes of an Xbox 360 `mc4.sav` or a decrypted PS3 `RAGE.SAV`.
/// * `base`: the port's current save (its members that the original has no counterpart of are kept), or an empty map.
/// * `races`: the game's race table, see [`race_table`]. The race names of the career's unlocked races and race history come from it.
pub fn convert(sav: &[u8], base: Map<String, Value>, races: &[String]) -> Result<Converted> {
    let c = Container::parse(sav)?;
    let stats = small::decode_stats(c.data("Stats"))?;
    let editor = small::decode_editor(c.data("EditorData"));
    let vinyls = small::decode_user_vinyls(c.data("UserVinylData"));
    let rubber = small::decode_rubberband(c.data("RubberbandMgr"));
    let (favs, favs_removed) = small::decode_favorites(c.data("RaceFavorites"));
    let enable_gps = c.data("enableGps")[0] != 0;
    // (ColorData: the 40 colour slots every colour strip of the garage shows - the executable's table at first, a slot
    // edited in the colour picker 0xFFRRGGBB - big-endian words; docs/UI.md "ColorData")
    let color_data: Vec<u32> = c.data("ColorData").chunks_exact(4).take(40).map(|w| u32::from_be_bytes([w[0], w[1], w[2], w[3]])).collect();
    let lo = c.data("LocalOptions");
    let opt = options::decode(lo);
    let veh = vehicle::decode(lo);
    let car = career::decode(c.data("Career"), c.version, races)?;
    let d = sav;

    let mut save = base;
    let career_map = obj(&mut save, "career");
    // the statistics: every record to career.ostats, whole numbers with sub id 0 also to career.stats under their id
    let mut ost = Map::new();
    let mut st_updates = Vec::new();
    for r in &stats.records {
        let first = r.id.chars().next().unwrap_or(' ');
        let key = if first == 'v' && (r.sub as usize) < VEHICLES.len() { format!("{}/{}", r.id, VEHICLES[r.sub as usize]) } else { format!("{}/{}", r.id, r.sub) };
        let is_float = r.ty == 0;
        ost.insert(
            key,
            match r.value {
                // (the record's f32 exactly, as the shortest decimal that gives it back: the port goes on adding to it in
                // f32, and a short number reads back the same through any JSON parser)
                small::StatValue::F(f) => num(f32_shortest(f)),
                other => other.json(),
            },
        );
        if !is_float && r.sub == 0 {
            st_updates.push((r.id.clone(), r.value.json()));
        }
    }
    let st = obj(career_map, "stats");
    for (k, v) in st_updates {
        st.insert(k, v);
    }
    career_map.insert("ostats".into(), Value::Object(ost));

    let g = garage(d)?;
    // (the vehicle in use: the garage's current slot, LocalOptions + 1344 - not always slot 0)
    let cur_slot = opt.current_slot;
    let cur_car: String = if cur_slot >= 0 && (cur_slot as usize) < g.cars.len() { g.cars[cur_slot as usize].clone() } else { g.cars.first().cloned().unwrap_or_default() };
    let mut seen = std::collections::HashMap::new();
    let slots_by_name: Vec<(String, &vehicle::Slot)> = veh.iter().filter(|s| !s.empty).map(|s| (copy_key(&mut seen, &s.type_name.to_lowercase()), s)).collect();
    if !g.cars.is_empty() {
        save.insert("owned".into(), json!(g.cars));
        save.insert("car".into(), json!(cur_car));
        let mut paint = lookup(&g.paints, &cur_car).copied().or(g.paint);
        let mut brands = Map::new();
        for (k, s) in &slots_by_name {
            brands.insert(k.clone(), json!(s.brand_raw));
        }
        save.insert("perf_brand".into(), Value::Object(brands));
        let pp = obj(&mut save, "perf_parts");
        for (k, v) in &g.perf_raw {
            if v.iter().all(|x| *x <= 2) {
                pp.insert(k.clone(), json!(v));
            }
        }
        let dm = obj(&mut save, "dims");
        for (k, v) in &g.dims {
            if (12..=28).contains(&v[4]) && (12..=28).contains(&v[5]) {
                dm.insert(k.clone(), json!(v));
            }
        }
        let pl = obj(&mut save, "plates");
        for (k, v) in &g.plates {
            pl.insert(k.clone(), v.clone());
        }
        let mut rim_ids = Map::new();
        for (k, w) in &g.wheels {
            rim_ids.insert(k.clone(), json!(w[0]));
        }
        save.insert("rim_ids".into(), Value::Object(rim_ids));
        let xp = obj(&mut save, "xparts");
        for (k, w) in &g.wheels {
            // (0x845 is the tires; 0x846 IS the brake model: an index into the `brakes` list of tune/vehicle/wheel_models.list; the BRAKES menu
            // offers rows 1..4, the other values are the cars' own stock models, which the port draws without a value)
            let e = obj(xp, k);
            e.insert("tires".into(), json!(w[1]));
            if (1..=4).contains(&w[2]) {
                e.insert("brakes".into(), json!(w[2]));
            } else {
                e.remove("brakes");
            }
        }
        if let Some(m) = &g.mats {
            save.insert("rim_mats".into(), json!(m));
        }
        // the port's `cfg` of the vehicle in use, `cfgs` of the others
        let mut cfgs = Map::new();
        let mut xp_updates: Vec<(String, Vec<(&str, Value)>)> = Vec::new();
        for (k, parts) in &g.parts {
            let mut cfg = Map::new();
            cfg.insert("paint".into(), json!(0));
            cfg.insert("rim".into(), json!(0));
            let mut rest = Vec::new();
            for (slot, v) in parts {
                if CFG_SLOTS.contains(slot) {
                    cfg.insert((*slot).into(), v.clone());
                } else if *slot == "_part_paint" {
                    cfg.insert("part_paint".into(), v.clone());
                } else {
                    rest.push((*slot, v.clone()));
                }
            }
            cfgs.insert(k.clone(), Value::Object(cfg));
            xp_updates.push((k.clone(), rest));
        }
        let xp = obj(&mut save, "xparts");
        for (k, rest) in xp_updates {
            let e = obj(xp, &k);
            for (slot, v) in rest {
                e.insert(slot.into(), v);
            }
        }
        save.insert("finishes".into(), Value::Object(g.finishes.clone()));
        if !g.liveries.is_empty() {
            let lv = obj(&mut save, "liveries");
            for (k, v) in &g.liveries {
                lv.insert(k.clone(), v.clone());
            }
            let vy = obj(&mut save, "vinyls");
            for k in g.liveries.keys() {
                vy.insert(k.clone(), json!("custom"));
            }
        }
        // (the vehicle in use: its own slots)
        let cur = save.get("car").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
        if let Some(f) = g.finishes.get(&cur) {
            save.insert("slot_hsv".into(), f[0].clone());
            save.insert("rim_mats".into(), f[1].clone());
        }
        let mut curcfg = cfgs.get(&cur_car).cloned().unwrap_or(Value::Object(Map::new()));
        let rim = save.get("cfg").and_then(|c| c.get("rim")).cloned().unwrap_or(json!(0));
        if let Value::Object(m) = &mut curcfg {
            m.insert("rim".into(), rim);
        }
        save.insert("cfg".into(), curcfg);
        save.insert("cfgs".into(), Value::Object(cfgs));
        // (hydraulics, air bags, window tint, neons, second paint colour, special ability, plate background, brake model outside the menu's rows and
        // the stored damage: each vehicle's own, in its record of the mc4 block - the port takes them from there when a vehicle comes into use)
        // parts owned, from the per-model purchase records: the port's own tags "<section>:<index>"
        let po = obj(&mut save, "parts_owned");
        for r in &opt.records {
            if let (true, Some(vehicle)) = (r.in_use, &r.vehicle) {
                let mut tags = Vec::new();
                for (cat, v) in &r.owned {
                    let name = if cat == "rims" { "rim" } else { cat.as_str() };
                    for i in v {
                        if *i != 0 || matches!(cat.as_str(), "rims" | "tires" | "performance" | "paint" | "vinyl") {
                            tags.push(format!("{name}:{i}"));
                        }
                    }
                }
                if !tags.is_empty() {
                    let e = po.entry(vehicle.to_lowercase()).or_insert_with(|| json!([]));
                    if !e.is_array() {
                        *e = json!([]);
                    }
                    if let Value::Array(have) = e {
                        for t in tags {
                            if !have.iter().any(|x| x.as_str() == Some(&t)) {
                                have.push(json!(t));
                            }
                        }
                    }
                }
            }
        }
        // every vehicle's paint: mixed colour, swatch, type, fade - the port's `paints`
        let mut paints = Map::new();
        for (k, (rgb, kind, fade)) in &g.paints {
            paints.insert(k.clone(), json!([hsv4(linear(*rgb)), 0, kind, if *fade <= 4 { *fade } else { 0 }]));
        }
        save.insert("paints".into(), Value::Object(paints));
        if let Some((rgb, kind, fade)) = paint.take() {
            // (the port's paint is a linear colour; the record's is the screen's)
            save.insert("paint_hsv".into(), hsv4(linear(rgb)));
            save.insert("paint_type".into(), json!(kind));
            save.insert("paint_fade".into(), json!(if fade <= 4 { fade } else { 0 }));
        }
    }

    // The missions: state 0 INACTIVE (not in the port's save), 1 CHECKING_PREREQUISITES (Armed), 2 READY_TO_START, 3 CHECKING_OBJECTIVES,
    // 4 READY_TO_COMPLETE (Active), 5 COMPLETE; the tallies of the objectives in their order.
    let career_map = obj(&mut save, "career");
    let old = match career_map.get("missions") {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    let mut new = Map::new();
    let mut complete = 0;
    for m in &car.missions {
        let stage = match m.state_id {
            1 => "Armed",
            2 | 3 | 4 => "Active",
            5 => "Complete",
            _ => continue,
        };
        let mut e = match old.get(&m.name) {
            Some(Value::Object(o)) => o.clone(),
            _ => {
                let mut o = Map::new();
                o.insert("progress".into(), json!([]));
                o.insert("won".into(), json!([]));
                o.insert("pts".into(), json!({}));
                o
            }
        };
        e.insert("stage".into(), json!(stage));
        // (a mission under way: its tallies. A complete one keeps what the port had)
        if stage != "Complete" {
            let mut prog: Vec<Value> = match e.get("progress") {
                Some(Value::Array(a)) => a.clone(),
                _ => vec![],
            };
            for (k, o) in m.objectives.iter().enumerate() {
                while prog.len() < k + 1 {
                    prog.push(json!(0));
                }
                if let Some(t) = o {
                    if let Some(count) = t.count {
                        prog[k] = json!((count as i64).max(0));
                    } else if let Some(value) = t.value {
                        prog[k] = json!((value as i64 - t.start_value as i64).max(0));
                    }
                }
            }
            e.insert("progress".into(), Value::Array(prog));
        } else {
            complete += 1;
        }
        new.insert(m.name.clone(), Value::Object(e));
    }
    let n_missions = new.len();
    career_map.insert("missions".into(), Value::Object(new));
    career_map.insert("order".into(), json!(car.active_order));
    // (the reputation is the Progression system's amount, the money the Career entry's own word - the statistic tPUR mirrors it)
    save.insert("rep".into(), car.reputation.map(Value::from).unwrap_or(Value::Null));
    save.insert("money".into(), json!(car.money));

    // The options: volumes 0..1 -> the port's 0..10; the camera shake is one half of the "cinematics" value; steering sensitivity -1..1 in 21
    // steps -> the port's 0..10; the genre: the original's 7 (all) is the port's 0, its 0..6 the port's 1..7 (8, the custom playlist: all).
    let o = &opt.options;
    let f = |k: &str| o[k].as_f64().unwrap_or(0.0);
    let po = obj(&mut save, "opts");
    po.insert("master".into(), json!(round_int(f("volume_effects") * 10.0)));
    po.insert("music".into(), json!(round_int(f("volume_music") * 10.0)));
    po.insert("voice".into(), json!(round_int(f("volume_voice") * 10.0)));
    po.insert("vehicle".into(), json!(round_int(f("volume_player_vehicle") * 10.0)));
    po.insert("cutscene".into(), json!(round_int(f("volume_cutscene") * 10.0)));
    po.insert("subtitles".into(), json!(o["subtitles"].as_bool().unwrap_or(false)));
    po.insert("shake".into(), json!(o["camera_shake"].as_bool().unwrap_or(false)));
    po.insert("sens".into(), json!(round_int((f("steering_sensitivity") + 1.0) * 5.0).clamp(0, 10)));
    let genre = o["music_genre"].as_u64().unwrap_or(7);
    save.insert("music_genre".into(), json!(if genre <= 6 { genre + 1 } else { 0 }));
    // The collectables: one flag byte each, by the index the Collectables script hands to LocalOptions_SaveCollectables.
    save.insert("collected".into(), json!(opt.collectables_found));
    // The race editor's own races (their checkpoints in order).
    let mine: Vec<&Value> = editor["slots"].as_array().map(|a| a.iter().filter(|s| s["in_use"] == json!(true)).collect()).unwrap_or_default();
    let user_races: Vec<Value> = mine.iter().map(|s| Value::Array(s["checkpoints"].as_array().map(|c| c.iter().map(|p| p["pos"].clone()).collect()).unwrap_or_default())).collect();
    let n_user = user_races.len();
    save.insert("user_races".into(), Value::Array(user_races));
    // The favourite races ("Game/Race/<District>/<Type>/<name>"; the port's ids are the same paths without "game/race/", compared without case).
    let race_favs: Vec<String> = favs
        .iter()
        .map(|f| {
            let l = f.to_lowercase();
            l.strip_prefix("game/race/").map(str::to_string).unwrap_or(l)
        })
        .collect();
    let n_favs = race_favs.len();
    save.insert("race_favs".into(), json!(race_favs));
    // The rubber band manager: level and points.
    save.insert("adaptive".into(), json!({"level": rubber.level, "points": rubber.points}));

    // ---- the mc4 block ----
    let garage_json: Vec<Value> = veh
        .iter()
        .filter(|s| !s.empty)
        .map(|s| {
            let mut v = strip(&s.json, GARAGE_DROP);
            let slot = s.json["slot"].as_u64().unwrap_or(0) as usize;
            if let Value::Object(m) = &mut v {
                m.insert("vehicle".into(), json!(s.type_name.to_lowercase()));
                let (dmg, zones) = opt.damage.get(slot).copied().unwrap_or((0.0, [0.0; 4]));
                m.insert("damage".into(), json!({"damage": num(dmg), "zones": nums(&zones)}));
                m.insert("name_hash".into(), json!(opt.name_hashes.get(slot)));
                if !s.flagged.is_empty() {
                    m.insert("vinyl_layers_flagged".into(), Value::Array(s.flagged.clone()));
                }
            }
            v
        })
        .collect();
    let in_use_recs: Vec<Value> = opt.records.iter().filter(|r| r.in_use).map(|r| r.json.clone()).collect();
    let mut career_json = car.json.clone();
    career_json.remove("money");
    career_json.remove("reputation");
    let editor_races: Vec<Value> = mine.iter().map(|s| (*s).clone()).collect();
    let received: Vec<Value> = editor["received"].as_array().map(|a| a.iter().filter(|s| s["in_use"] == json!(true)).cloned().collect()).unwrap_or_default();
    let packages: Vec<Value> = vinyls.iter().map(|p| json!({"slot": p["slot"], "used": p["used"], "livery": p["livery"]})).collect();
    let mut achievements: Vec<&str> = stats.records.iter().filter(|r| r.id.starts_with('a') && r.value.truthy()).map(|r| r.id.as_str()).collect();
    achievements.sort();
    let mut tutorials: Vec<&str> = stats.records.iter().filter(|r| r.id.starts_with("tT") && r.id != "tTTA" && r.value.truthy()).map(|r| r.id.as_str()).collect();
    tutorials.sort();
    let mc4 = json!({
        "container_version": c.version,
        "options": strip(&opt.options, NOTES),
        "options_unknown": opt.unknown,
        "enable_gps": enable_gps,
        "color_data": color_data,
        "garage_current_slot": opt.current_slot,
        "garage": garage_json,
        "vehicle_records": strip(&Value::Array(in_use_recs), NOTES),
        "ability_levels": opt.ability_levels,
        "career": strip(&Value::Object(career_json), NOTES),
        "editor": {
            "races": strip(&Value::Array(editor_races), NOTES),
            "received": strip(&Value::Array(received), NOTES),
            "received_cursor": editor["received_cursor"],
        },
        "vinyl_packages": packages,
        "race_favorites_removed": favs_removed,
        "rubberband_step_in_use": rubber.step_in_use,
        "achievements": achievements,
        "tutorials_shown": tutorials,
        "stats_summary": strip(&stats.summary, NOTES),
    });
    save.insert("mc4".into(), mc4);

    let summary = Summary {
        container_version: c.version,
        statistics: stats.records.len(),
        vehicles: g.cars.clone(),
        current_vehicle: cur_car,
        missions: n_missions,
        missions_complete: complete,
        money: car.money,
        reputation: car.reputation,
        collectables: opt.collectables_found.len(),
        user_races: n_user,
        favourite_races: n_favs,
    };
    Ok(Converted { save, summary })
}

#[cfg(test)]
mod copy_key_tests {
    /// A model twice in the garage: the second slot is `<name>#2`, as the game keys it.
    #[test]
    fn a_model_twice_gets_its_copy_key() {
        let mut seen = std::collections::HashMap::new();
        let keys: Vec<String> = ["vp_aud_r8_08", "vp_golf", "vp_aud_r8_08", "vp_aud_r8_08"].iter().map(|n| super::copy_key(&mut seen, n)).collect();
        assert_eq!(keys, ["vp_aud_r8_08", "vp_golf", "vp_aud_r8_08#2", "vp_aud_r8_08#3"]);
    }
}
