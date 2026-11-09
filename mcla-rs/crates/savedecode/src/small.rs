//! The small entries: Stats, EditorData, UserVinylData, RubberbandMgr, RaceFavorites, enableGps (port of `tools/mc4save/small.py`).
//! Only what the importer carries into the port's save is produced (the Python decoder also builds descriptive texts for a reference dump).

use crate::tables::VEHICLES;
use crate::util::*;
use serde_json::{json, Map, Value};

/// The statistics table of the original (`Stats`): records of 16 bytes.
pub struct StatRecord {
    pub id: String,
    pub sub: u32,
    /// 0 float, 1 int, 2 unsigned (3..7 are unnamed and read as unsigned).
    pub ty: u8,
    pub value: StatValue,
}

#[derive(Clone, Copy)]
pub enum StatValue {
    F(f64),
    I(i64),
    U(u64),
}

impl StatValue {
    pub fn json(&self) -> Value {
        match *self {
            StatValue::F(f) => num(f),
            StatValue::I(i) => Value::from(i),
            StatValue::U(u) => Value::from(u),
        }
    }
    pub fn truthy(&self) -> bool {
        match *self {
            StatValue::F(f) => f != 0.0,
            StatValue::I(i) => i != 0,
            StatValue::U(u) => u != 0,
        }
    }
}

pub struct Stats {
    pub summary: Value,
    pub records: Vec<StatRecord>,
}

/// The vehicle table the statistics arrays index (64 places used of 100).
const NVEH: usize = 64;

pub fn decode_stats(b: &[u8]) -> Result<Stats, crate::Error> {
    let bb = B(b);
    let f = |o: usize| round_to(bb.f32(o), 6);
    let u = |o: usize| bb.u32(o);
    let veh_dist: Map<String, Value> = (0..NVEH).filter(|i| f(304 + 4 * i) != 0.0).map(|i| (VEHICLES[i].to_string(), num(f(304 + 4 * i)))).collect();
    let veh_wins: Map<String, Value> = (0..NVEH).filter(|i| bb.u16(704 + 2 * i) != 0).map(|i| (VEHICLES[i].to_string(), Value::from(bb.u16(704 + 2 * i)))).collect();
    let summary = json!({
        "tag": cstr(bb.slice(0, 4)),
        "career_percent": u(4),
        "win_percent": f(8),
        "cash_earned": u(12),
        "current_vehicle_hash": hex32(u(16)),
        "hours_played": f(20), "distance": f(24), "arrests": u(28), "cops_lost": u(32),
        "average_restarts": f(36), "max_jump_distance": f(40), "damaged_out": u(44),
        "list_3dfc8": (0..32).map(|i| u(48 + 4 * i)).collect::<Vec<_>>(),
        "wins": u(176), "zero_180": u(180), "hours_cruising": f(184), "top_speed": f(188),
        "nitrous": u(192), "slipstream": u(196), "zone": u(200), "roar": u(204), "agro": u(208), "agro_hits": u(212),
        "two_wheel_distance": f(216), "wheelie_distance": f(220), "max_jump_distance_2": f(224), "jump_time": f(228), "most_air_time": f(232),
        "wipeouts": u(236), "damaged_out_2": u(240), "collectables": u(244), "races_completed": u(248), "wins_x": u(252),
        "career_wins": u(256), "hookmen_beaten": u(260), "most_consecutive_wins": u(264), "agro_x": u(268), "career_percent_2": u(272),
        "lead_changes": u(276), "wager_winnings": u(280), "career_win_streak": u(284), "career_races_completed": u(288),
        "burnouts": u(292), "opponents_hit": u(296), "props_hit": u(300),
        "vehicle_distance": Value::Object(veh_dist),
        "vehicle_wins": Value::Object(veh_wins),
    });
    let count = u(904) as usize;
    if 908 + 16 * count > b.len() {
        return Err(crate::Error::new("layout", format!("statistics table claims {count} records")));
    }
    let mut records = Vec::with_capacity(count);
    for i in 0..count {
        let o = 908 + 16 * i;
        let ty = bb.u8(o + 8) & 7;
        let value = match ty {
            // (the f32 exactly, widened: the port adds float statistics in f32 - mc4::stat_type)
            0 => StatValue::F(bb.f32(o + 12)),
            1 => StatValue::I(bb.i32(o + 12) as i64),
            _ => StatValue::U(bb.u32(o + 12) as u64),
        };
        records.push(StatRecord { id: latin1(bb.slice(o, 4)), sub: bb.u32(o + 4), ty, value });
    }
    Ok(Stats { summary, records })
}

// ------------------------------------------------------------------ EditorData

const SLOT: usize = 1056;
const CHECKPOINTS: usize = 31;

fn editor_slot(b: &[u8], index: usize) -> Value {
    let bb = B(b);
    let count_word = bb.u32(0);
    let city = b[1043];
    // the game's rule (sub_822502D8): city byte 0xCD (never written) -> the word, else the byte
    let mut count = if city == 0xCD { count_word as i64 } else { b[1042] as i64 };
    let mut legacy = false;
    if count == 0 && (city == 0 || city == 0xCD) && (2..=30).contains(&count_word) {
        // a slot written by the other console's build: the count in the word only
        count = count_word as i64;
        legacy = true;
    }
    let count = count.min(30) as usize;
    let pts: Vec<(Vec<f64>, u8, u8, &[u8])> = (0..CHECKPOINTS)
        .map(|i| {
            let c = &b[48 + 32 * i..80 + 32 * i];
            let cb = B(c);
            (vec![cb.f32(0), cb.f32(4), cb.f32(8)], c[16], c[17], c)
        })
        .collect();
    let mut out = Map::new();
    out.insert("slot".into(), json!(index));
    out.insert("name".into(), json!(if b[4] != 0xCD { cstr(&b[4..36]) } else { String::new() }));
    out.insert("in_use".into(), json!(count >= 2));
    out.insert("count".into(), json!(count));
    out.insert("city".into(), if city == 0xCD { Value::Null } else { json!(city) });
    out.insert("grid_variant".into(), if b[1040] == 0xCD { Value::Null } else { json!(b[1040] as i8) });
    out.insert("flag_1041".into(), if b[1041] == 0xCD { Value::Null } else { json!(b[1041]) });
    out.insert("count_word".into(), json!(count_word));
    out.insert("count_byte".into(), if b[1042] == 0xCD { Value::Null } else { json!(b[1042]) });
    out.insert("checkpoints".into(), Value::Array(pts[..count].iter().map(|p| json!({"pos": nums(&p.0), "activation_height": p.1, "activation_radius": p.2})).collect()));
    if legacy {
        out.insert("count_from_word".into(), json!(true));
    }
    let mut unused = vec![0u8; 16];
    unused.extend_from_slice(&[0x14, 0x14]);
    let stale: Vec<Value> = (count..CHECKPOINTS)
        .filter(|i| pts[*i].3[..18] != unused[..])
        .map(|i| json!({"index": i, "pos": nums(&pts[i].0), "activation_height": pts[i].1, "activation_radius": pts[i].2}))
        .collect();
    if !stale.is_empty() {
        out.insert("stale_checkpoints".into(), Value::Array(stale));
    }
    let mut pad: Vec<u8> = b[36..48].to_vec();
    for p in &pts {
        pad.extend_from_slice(&p.3[18..]);
    }
    pad.extend_from_slice(&b[1044..1056]);
    if pad.iter().any(|v| *v != 0xCD) {
        out.insert("padding_not_cd".into(), json!(true));
    }
    Value::Object(out)
}

pub fn decode_editor(b: &[u8]) -> Value {
    let mine: Vec<Value> = (0..20).map(|i| editor_slot(&b[i * SLOT..(i + 1) * SLOT], i)).collect();
    let recv: Vec<Value> = (0..20).map(|i| editor_slot(&b[(20 + i) * SLOT..(21 + i) * SLOT], i)).collect();
    let received: Vec<Value> = recv.into_iter().filter(|s| truthy(&s["name"]) || truthy(&s["count"])).collect();
    json!({"slots": mine, "received": received, "received_cursor": B(b).u32(42240)})
}

// ------------------------------------------------------------------ UserVinylData

pub const ZONES: [(&str, usize); 5] = [("top", 64), ("left", 64), ("right", 64), ("front", 16), ("back", 16)];
const VINYL_SLOT: usize = 5764;

/// A vinyl layer as the port's `vinyl::Livery` has it (blend: `AlphaCutout`/`NegativeCutout` as words, else `Normal`).
fn port_layer(r: &[u8]) -> Value {
    let b = B(r);
    let blend = match r[18] {
        0 => json!("Normal"),
        1 => json!("AlphaCutout"),
        2 => json!("NegativeCutout"),
        other => json!(other),
    };
    json!({"shape": b.u16(16), "scale": nums(&[b.half(0), b.half(2)]), "pos": nums(&[b.half(4), b.half(6)]), "rot": num(b.half(12 + 2)), "skew": num(b.half(12)),
        "color": [r[9], r[10], r[11], r[8]], "blend": blend})
}

/// The three user vinyl packages as `{slot, used, livery: {zones: [top, left, right, front, back]}}` (hidden layers left out).
pub fn decode_user_vinyls(b: &[u8]) -> Vec<Value> {
    (0..3)
        .map(|s| {
            let v = &b[s * VINYL_SLOT..(s + 1) * VINYL_SLOT];
            let mut o = 4;
            let mut zones = Vec::new();
            for (_, cap) in ZONES {
                let mut zl = Vec::new();
                for _ in 0..cap {
                    let r = &v[o..o + 20];
                    o += 20;
                    // (the layer's blend byte is read as a word by the port; a layer with a name unknown to it falls to Normal as the importer did)
                    if r[16..18] != [0xff, 0xff] && r[19] == 0 {
                        zl.push(port_layer(r));
                    }
                }
                zones.push(Value::Array(zl));
            }
            json!({"slot": s, "used": v[0] != 0, "livery": {"zones": zones}})
        })
        .collect()
}

pub struct Rubber {
    pub points: i32,
    pub step_in_use: i32,
    pub level: i32,
}

pub fn decode_rubberband(b: &[u8]) -> Rubber {
    let bb = B(b);
    Rubber { points: bb.i32(0), step_in_use: bb.i32(4), level: bb.i32(8) }
}

/// (favourite race paths in slot order, the removed ones as `{slot, race_without_first_letter}`)
pub fn decode_favorites(b: &[u8]) -> (Vec<String>, Vec<Value>) {
    let (mut favs, mut removed) = (Vec::new(), Vec::new());
    for i in 0..100 {
        let r = &b[i * 64..(i + 1) * 64];
        if r[0] != 0 {
            favs.push(cstr(r));
        } else if r[1..].iter().any(|c| *c != 0 && *c != 0xCD) {
            // removal zeroes the first byte only
            removed.push(json!({"slot": i, "race_without_first_letter": cstr(&r[1..])}));
        }
    }
    (favs, removed)
}
