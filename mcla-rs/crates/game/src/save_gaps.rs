//! Save gaps closed in 2026-10-28 (docs/reference/save/triage_save_gaps.md, section A): rules of the original's
//! save code that the port had not got, each read from the recompiled code and compared with it offline where it
//! can run there (`tools/save_gaps_reference.py`, the tests at the end).
//!
//! 1. The career's pink slip profiles by location (mcCareer +8768: one profile per district, the hangout's own car,
//!    not saved) next to the lot of held cars (+50128, saved: `mc4::PinkSlips`), and the queries the wager hangouts'
//!    scripts make by location.

use crate::mc4::{Car, PinkSlip, PinkSlips};

// ---------------------------------------------------------------------------------------------------------------
// 1. pink slip profiles by location
// ---------------------------------------------------------------------------------------------------------------

/// The career's locations and their profile index (mcCareer_RegisterLocation(id, key, desc, index) of the career
/// script `game/career/0x1529a945`, in its order: the map at mcCareer +8748, the index at the value's +260). The
/// index is the district: the hangouts, garages, delivery and payback places and the photo spots of a district
/// share its profile.
pub const PINK_LOCATIONS: [(u32, usize); 30] = [
    (500, 0), (501, 0), (502, 0), (510, 1), (520, 2), (530, 3), (540, 4), (503, 0), (511, 1), (521, 2), (531, 3), (541, 4),
    (506, 0), (505, 0), (512, 1), (543, 4), (504, 0), (542, 4), (513, 1), (600, 0), (601, 3), (602, 3), (603, 1), (604, 1),
    (605, 0), (606, 1), (607, 1), (608, 0), (609, 3), (610, 4),
];

/// The profile index of a location (F-16395: the map's lookup), None for an id the script did not register.
pub fn pink_location(id: i32) -> Option<usize> {
    PINK_LOCATIONS.iter().find(|p| p.0 as i32 == id).map(|p| p.1)
}

/// A location's own profile (8272 bytes at mcCareer +8768 + 8272 * index): +0 the kind, +4 the name (64 bytes),
/// +80 the record (`record` is there when its +8104 word is), +8256 the id.
#[derive(Clone, Debug, PartialEq)]
pub struct PinkLocal {
    pub kind: i32,
    pub name: String,
    pub record: Option<Car>,
    pub uid: i32,
}

impl Default for PinkLocal {
    /// As F-47738 leaves an entry: kind -1, no name, the record reset, id -1.
    fn default() -> Self {
        PinkLocal { kind: -1, name: String::new(), record: None, uid: -1 }
    }
}

/// The five locations' own profiles (not in the save: a new career's, and gone at every load).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PinkLocals {
    pub at: [PinkLocal; 5],
}

/// What a profile query finds: the location's own profile or a slot of the lot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PinkAt {
    Local(usize),
    Lot(usize),
}

/// The 64-byte name field as strncpy(.., 64) fills it.
fn name64(s: &str) -> String {
    s.chars().take(64).collect()
}

fn lot_empty(k: usize) -> PinkSlip {
    PinkSlip { slot: k as u32, kind: -1, empty: true, name: String::new(), uid: -1, extra: Default::default() }
}

fn lot_ensure(lot: &mut PinkSlips) {
    while lot.slots.len() < crate::mc4::PINK_SLOTS {
        let k = lot.slots.len();
        lot.slots.push(lot_empty(k));
    }
}

/// The lot's branch of Add / Load (kind != 0): the slots 3..0 that hold a car move one down (a slot is copied, not
/// cleared: the next one up overwrites it), slot 0 is reset and takes the kind, the record, the name and a new id.
fn lot_put(lot: &mut PinkSlips, kind: i32, name: &str, car: &Car) -> i32 {
    lot_ensure(lot);
    for k in (0..crate::mc4::PINK_SLOTS - 1).rev() {
        if !lot.slots[k].empty {
            let mut s = lot.slots[k].clone();
            s.slot = (k + 1) as u32;
            lot.slots[k + 1] = s;
        }
    }
    let mut s = lot_empty(0);
    s.kind = kind;
    s.empty = false;
    s.name = name64(name);
    s.uid = lot.next_uid();
    s.extra.insert("vehicle_name".into(), serde_json::Value::String(car.vehicle.clone()));
    s.extra.insert("car".into(), serde_json::to_value(car).unwrap_or(serde_json::Value::Null));
    let uid = s.uid;
    lot.slots[0] = s;
    uid
}

impl PinkLocals {
    /// mcCareer_AddPinkSlipProfile(location, record, kind) (F-58746): kind 0 puts the record into the location's
    /// own profile (kind 0, the record's name +8109, a new id - whatever it held before), any other kind into the
    /// lot. -1 for an unknown location (nothing changes, not even the id counter).
    pub fn add(&mut self, lot: &mut PinkSlips, loc: i32, car: &Car, kind: i32) -> i32 {
        let Some(ix) = pink_location(loc) else { return -1 };
        if kind != 0 {
            return lot_put(lot, kind, &car.TypeName, car);
        }
        let e = &mut self.at[ix];
        e.kind = 0;
        e.record = Some(car.clone());
        e.name = name64(&car.TypeName);
        e.uid = lot.next_uid();
        e.uid
    }

    /// mcCareer_LoadPinkSlipProfile(location, set-up, kind) (F-53651): as Add, with the record made from the set-up
    /// file and the set-up's name as the profile's name.
    pub fn load(&mut self, lot: &mut PinkSlips, loc: i32, setup: &str, car: &Car, kind: i32) -> i32 {
        let Some(ix) = pink_location(loc) else { return -1 };
        if kind != 0 {
            return lot_put(lot, kind, setup, car);
        }
        let e = &mut self.at[ix];
        e.kind = 0;
        e.record = Some(car.clone());
        e.name = name64(setup);
        e.uid = lot.next_uid();
        e.uid
    }

    /// mcCareer_GetNumPinkSlipProfiles(location) (F-54647): the location's own profile when it has a record, plus
    /// every slot of the lot that has one (the lot is the same at every location); 0 for an unknown location.
    pub fn num(&self, lot: &PinkSlips, loc: i32) -> i32 {
        let Some(ix) = pink_location(loc) else { return 0 };
        self.at[ix].record.is_some() as i32 + lot.slots.iter().take(crate::mc4::PINK_SLOTS).filter(|s| !s.empty).count() as i32
    }

    /// mcCareer_GetPinkSlipUID(location, i) (F-50697): the i-th profile's id in GetNum's order (the location's own
    /// first), -1 past the end or for an unknown location.
    pub fn uid(&self, lot: &PinkSlips, loc: i32, i: i32) -> i32 {
        let Some(ix) = pink_location(loc) else { return -1 };
        let own = self.at[ix].record.is_some();
        if i == 0 && own {
            return self.at[ix].uid;
        }
        let mut n = own as i32;
        for s in lot.slots.iter().take(crate::mc4::PINK_SLOTS) {
            if !s.empty {
                if n == i {
                    return s.uid;
                }
                n += 1;
            }
        }
        -1
    }

    /// Get / GetType / GetName's search (F-93331, F-87077, F-42349): the location's own profile when it has a record and the id,
    /// else the first slot of the lot with a record and the id.
    pub fn find(&self, lot: &PinkSlips, loc: i32, uid: i32) -> Option<PinkAt> {
        let ix = pink_location(loc)?;
        if self.at[ix].record.is_some() && self.at[ix].uid == uid {
            return Some(PinkAt::Local(ix));
        }
        lot.slots.iter().take(crate::mc4::PINK_SLOTS).position(|s| !s.empty && s.uid == uid).map(PinkAt::Lot)
    }

    /// mcCareer_GetPinkSlipProfileType(location, id): the profile's kind, -1 when none has the id.
    pub fn kind(&self, lot: &PinkSlips, loc: i32, uid: i32) -> i32 {
        match self.find(lot, loc, uid) {
            Some(PinkAt::Local(ix)) => self.at[ix].kind,
            Some(PinkAt::Lot(k)) => lot.slots[k].kind,
            None => -1,
        }
    }

    /// mcCareer_GetPinkSlipProfileName(location, id): the profile's name.
    pub fn name<'a>(&'a self, lot: &'a PinkSlips, loc: i32, uid: i32) -> Option<&'a str> {
        match self.find(lot, loc, uid)? {
            PinkAt::Local(ix) => Some(self.at[ix].name.as_str()),
            PinkAt::Lot(k) => Some(lot.slots[k].name.as_str()),
        }
    }

    /// mcCareer_RemovePinkSlipProfile(location, id) (F-85350): the location's own profile is reset (F-47738) when
    /// it is the one, else the lot's slot with the id is cleared and the later slots that hold a car move up.
    pub fn remove(&mut self, lot: &mut PinkSlips, loc: i32, uid: i32) {
        let Some(ix) = pink_location(loc) else { return };
        if self.at[ix].record.is_some() && self.at[ix].uid == uid {
            self.at[ix] = PinkLocal::default();
            return;
        }
        lot_ensure(lot);
        let Some(r) = lot.slots.iter().position(|s| !s.empty && s.uid == uid) else { return };
        lot.slots[r] = lot_empty(r);
        for j in r..crate::mc4::PINK_SLOTS {
            if !lot.slots[j].empty && j > 0 {
                let mut s = lot.slots[j].clone();
                s.slot = (j - 1) as u32;
                lot.slots[j - 1] = s;
                lot.slots[j] = lot_empty(j);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// 2. the vehicle record as its constructor leaves it
// ---------------------------------------------------------------------------------------------------------------

/// Byte writes in the big-endian record.
struct W<'a>(&'a mut [u8]);

impl W<'_> {
    fn u8(&mut self, o: usize, v: u8) {
        self.0[o] = v;
    }
    fn u16(&mut self, o: usize, v: u16) {
        self.0[o..o + 2].copy_from_slice(&v.to_be_bytes());
    }
    fn u32(&mut self, o: usize, v: u32) {
        self.0[o..o + 4].copy_from_slice(&v.to_be_bytes());
    }
    fn f(&mut self, o: usize, v: f32) {
        self.u32(o, v.to_bits());
    }
    fn f3(&mut self, o: usize, v: [f32; 3]) {
        for (k, x) in v.iter().enumerate() {
            self.f(o + 4 * k, *x);
        }
    }
    fn v4(&mut self, o: usize, v: [f32; 4]) {
        for (k, x) in v.iter().enumerate() {
            self.f(o + 4 * k, *x);
        }
    }
    fn get(&self, o: usize) -> f32 {
        f32::from_bits(u32::from_be_bytes(self.0[o..o + 4].try_into().unwrap()))
    }
    /// A vector times a scalar in place (lvx / vspltw / vmulfp / stvx): all four lanes, the fourth whatever the
    /// memory held.
    fn scale4(&mut self, o: usize, s: f32) {
        for k in 0..4 {
            let v = self.get(o + 4 * k) * s;
            self.f(o + 4 * k, v);
        }
    }
}

/// The half float the constructors make from a float's bits: the sign, the exponent + 16 cut to 5 bits and the
/// top 10 bits of the mantissa (no rounding, no range check).
pub fn half_of(f: f32) -> u16 {
    let b = f.to_bits();
    let se = b >> 23;
    ((((se + 16) << 10) & 0xFFFF_FC00) | ((b >> 13) & 0x3FF) | ((b >> 16) & 0x8000)) as u16
}

/// The window tint, headlight and bumped headlight glass colour of the constructor (the vector at 0x827E9A40).
const TINT: [f32; 4] = [0.0, 0.0035, 0.005, 0.372];
/// The vector the taillight glass colours are the tint times (0x820512B0).
const TAIL_MASK: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
/// The plate's characters (0x820512C0, 43 bytes with the end): 26 letters, the signs, 0..9 from 32.
pub const PLATE_CHARS: &[u8; 42] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ *@&+ 0123456789";

/// The constructor's random plate (F-92553): four letters and three digits from the game's generator (the first
/// letter draws twice), byte 7 = 0, LicenseBG (+8) = 4.
fn plate(w: &mut W, o: usize, rng: &mut crate::vehicles::parked::Mwc) {
    w.u8(o + 7, 0);
    for i in 0..7 {
        if i == 0 {
            rng.next();
        }
        let x = rng.next() & 0x7FFF_FFFF;
        let k = if i < 4 { x % 26 } else { x % 10 + 32 };
        w.u8(o + i, PLATE_CHARS[k as usize]);
    }
    w.u8(o + 8, 4);
}

/// The vinyl layer records of the look: 20 bytes, +0/+2 the scale and +4/+6 the position (halves), +8 the
/// colour, +12 / +14 the skew and the rotation (halves), +16 the shape (0xFFFF none), +18 / +19.
fn layer(w: &mut W, q: usize, scale: u16) {
    w.u16(q + 16, 0xFFFF);
    w.u16(q + 14, 0);
    w.u8(q + 19, 0);
    w.u32(q + 8, 0xFFFF_FFFF);
    w.u16(q, scale);
    w.u16(q + 2, scale);
    w.u16(q + 4, 0);
    w.u16(q + 6, 0);
    w.u16(q + 12, 0);
    w.u8(q + 18, 0);
}

/// The colour block of the look (F-46953, at look + 16 = record + 80): neon, gauges, rims, brushed metal, brake
/// callipers, the glass and tint colours, the neon intensities and the rim materials.
fn look_colours(w: &mut W, x: usize) {
    w.0[x..x + 320].fill(0);
    let (neon, gauge, metal) = ([0.52, 0.52, 1.0], [0.8, 0.5, 0.2], [0.75; 3]);
    w.f3(x, neon);
    w.f(x + 288, 1.0);
    w.f3(x + 16, neon);
    w.f(x + 292, 1.0);
    w.f3(x + 32, gauge);
    w.f3(x + 48, gauge);
    for (k, o) in [64, 80, 96, 112, 128].iter().enumerate() {
        w.f3(x + o, [1.0; 3]);
        // (the rim materials of rims A, C, D, E: 1; B keeps the 0)
        if k != 1 {
            w.u32(x + 296 + 4 * k, 1);
        }
    }
    w.f3(x + 144, metal);
    w.f3(x + 160, metal);
    w.f3(x + 176, metal);
    w.f3(x + 192, [0.25; 3]);
    let tail = [0, 1, 2, 3].map(|k| TINT[k] * TAIL_MASK[k]);
    w.v4(x + 272, TINT);
    w.v4(x + 208, TINT);
    w.v4(x + 224, tail);
    w.v4(x + 240, TINT);
    w.v4(x + 256, tail);
}

/// The lamps of the look (F-52861, 19 records of 48 bytes at look + 496): the bulb colour (1, 0.95, 0.85; four
/// times that for the first two and the thirteenth), the cover colour, the kind.
const LAMPS: [([f32; 3], u32, bool); 19] = [
    ([0.25, 0.0, 0.0], 0, true), ([0.25, 0.0, 0.0], 1, true), ([1.0, 1.0, 0.0], 6, false), ([1.0, 1.0, 0.75], 3, false),
    ([1.0, 1.0, 1.0], 4, false), ([1.0, 0.8, 0.0], 3, false), ([0.5, 0.5, 0.5], 11, false), ([0.7, 0.7, 0.2], 10, false),
    ([1.0, 0.85, 0.5], 5, false), ([0.25, 0.0, 0.0], 12, false), ([0.25, 0.25, 0.0], 12, false), ([0.25, 0.25, 0.25], 12, false),
    ([0.25, 0.0, 0.0], 2, true), ([1.0, 0.0, 0.0], 7, false), ([0.0, 0.0, 1.0], 7, false), ([0.0, 0.0, 1.0], 13, false),
    ([0.0, 0.0, 1.0], 14, false), ([0.0, 0.0, 1.0], 15, false), ([0.0, 0.0, 1.0], 16, false),
];

/// The look's second default block (F-52861 with F-90968): the vinyl redraw request -1 / -1 / -1 and its flag,
/// the gauge light places, the lamps, the five zone layers' scale 1.
fn look_defaults(w: &mut W, e: usize) {
    w.u32(e + 2068, 0xFFFF_FFFF);
    w.u32(e + 2072, 0xFFFF_FFFF);
    w.u32(e + 2076, 0xFFFF_FFFF);
    w.u8(e + 8004, 0);
    w.f3(e + 1984, [0.0; 3]);
    w.f(e + 4, 1.0);
    w.f3(e + 2000, [0.0; 3]);
    w.f3(e + 2016, [0.0, 0.0, -1.0]);
    w.f(e + 8, 1.0);
    w.f(e + 12, 1.0);
    for (k, (cover, kind, bright)) in LAMPS.iter().enumerate() {
        let o = e + 496 + 48 * k;
        w.f3(o + 16, *cover);
        w.f3(o, [1.0, 0.95, 0.85]);
        if *bright {
            w.scale4(o, 4.0);
        }
        w.u32(o + 32, *kind);
    }
    for k in 0..5 {
        let q = e + 2144 + 20 * k;
        w.u16(q + 14, 0);
        w.u16(q + 4, 0);
        w.u16(q + 6, 0);
        w.u16(q, half_of(1.0));
        w.u16(q + 2, half_of(1.0));
    }
}

/// The look of a vehicle record (CarCfgExt, F-35798, at record + 64).
fn look(w: &mut W, e: usize, rng: &mut crate::vehicles::parked::Mwc) {
    w.u32(e, 0x8205_1378);
    look_colours(w, e + 16);
    // (the five gauge texts: white, skew 0, font 0)
    for k in 0..5 {
        let o = e + 336 + 32 * k;
        w.v4(o, [1.0; 4]);
        w.f(o + 16, 0.0);
        w.u32(o + 20, 0);
    }
    // (the ten base colours)
    for k in 0..10 {
        w.f3(e + 1408 + 16 * k, [0.3; 3]);
    }
    for k in 0..288 {
        layer(w, e + 2244 + 20 * k, half_of(0.12));
    }
    for k in 0..5 {
        layer(w, e + 2144 + 20 * k, half_of(0.12));
    }
    // (the six paint areas: colour A, colour B black, type 3, fade 0, the flag 0)
    for k in 0..6 {
        let o = e + 1568 + 48 * k;
        w.f3(o, [0.23, 0.26, 0.28]);
        w.f3(o + 16, [0.0; 3]);
        w.u32(o + 32, 3);
        w.u32(o + 36, 0);
        w.u8(o + 40, 0);
    }
    for o in [1856, 1888, 1860, 1864, 1872, 1892, 1896, 1904, 1920, 1924, 1928, 1936, 1952, 1956, 1960, 1968] {
        w.f(e + o, 0.0);
    }
    // (camber -2.5 / -1, tyre offset 0, ride height 3, tyre width 4, rim size 17, tyre profile 5, the rim, tyre and
    // brake models 0, the plate)
    for o in [2032, 2036, 2040, 2044, 2048] {
        w.u32(e + o, 0);
    }
    w.f(e + 2032, -2.5);
    w.f(e + 2036, -1.0);
    for (o, v) in [(2046, 17), (2047, 17), (2048, 5), (2049, 5), (2042, 3), (2043, 3), (2044, 4), (2045, 4), (2052, 0), (2053, 0), (2054, 0)] {
        w.u8(e + o, v);
    }
    plate(w, e + 2055, rng);
    w.0[e + 2080..e + 2144].fill(0);
    look_defaults(w, e);
}

/// A vehicle record (8176 bytes, the set-up file's layout) as the record constructor F-13631 leaves it: the
/// vtables, the class key 0xFD813, the performance part (stages 0, the three percent bytes 100, two -1 bytes),
/// the look (above) with a random plate from `rng`, the name "blank", ability -1. `fill`: what the memory held
/// (the bytes the constructor does not write keep it; the original's allocation leaves whatever was there).
pub fn record_ctor_bytes(rng: &mut crate::vehicles::parked::Mwc, fill: u8) -> Vec<u8> {
    let mut b = vec![fill; 8176];
    let mut w = W(&mut b);
    w.u32(0, 0x8204_F3D4);
    // (the performance part at +20: its vtable, 28 zero bytes from +28, +59 / +60 = -1, +61..63 = 100)
    w.u32(20, 0x8204_F3BC);
    w.0[28..56].fill(0);
    for o in [61, 62, 63] {
        w.u8(o, 100);
    }
    w.u8(60, 0xFF);
    w.u8(59, 0xFF);
    look(&mut w, 64, rng);
    w.u32(8080, 0x8204_F3A4);
    w.0[8109..8115].copy_from_slice(b"blank\0");
    let mut w = W(&mut b);
    w.u8(8141, 0);
    w.u32(8100, 0);
    w.u32(8096, 0);
    w.u8(8108, 0xFF);
    w.u32(8088, 0xFFFF_FFFF);
    w.u32(8104, 0);
    w.u32(4, 0x000F_D813);
    w.u8(8175, 0);
    b
}

// ---------------------------------------------------------------------------------------------------------------
// 3. the per-model records before a save: the refund value
// ---------------------------------------------------------------------------------------------------------------

/// What the vehicle list (`tune/vehicle`: Vehicle { .. }) gives a model for the refund: (ContentDownloadFlags,
/// Price). The list's entries start at Price 12345, ContentFlags 1, and ContentDownloadFlags is ContentFlags when
/// the entry has none (the list's constructor F-83010 and parser F-87336).
pub fn download_and_price(data_dir: &str, car: &str) -> (u32, i64) {
    static TABLE: std::sync::OnceLock<std::collections::HashMap<String, (u32, i64)>> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut out = std::collections::HashMap::new();
        for f in ["0xe94bde63", "0xccb5daee", "0x3cb91343"] {
            let Ok(s) = std::fs::read(std::path::Path::new(data_dir).join("tune/vehicle").join(f)) else { continue };
            let s = String::from_utf8_lossy(&s);
            for block in s.split("Vehicle").skip(1) {
                let field = |k: &str| block.lines().map(|l| l.trim()).find_map(|l| l.strip_prefix(k).filter(|r| r.starts_with(|c: char| c.is_whitespace())).map(|r| r.trim().to_string()));
                let Some(name) = field("Name") else { continue };
                let content = field("ContentFlags").and_then(|v| v.parse::<u32>().ok()).unwrap_or(1);
                let download = field("ContentDownloadFlags").and_then(|v| v.parse::<u32>().ok()).unwrap_or(content);
                let price = field("Price").and_then(|v| v.parse::<i64>().ok()).unwrap_or(12345);
                out.insert(name.to_ascii_lowercase(), (download, price));
            }
        }
        out
    });
    t.get(&car.to_ascii_lowercase()).copied().unwrap_or((1, 12345))
}

/// Before the options block is saved (F-80786): every model's refund value is its parts sum word (+82) - 1, and
/// every car of the garage of a downloaded model (ContentDownloadFlags above 3) adds the model's price to its
/// model's.
pub fn refund_values(m: &mut crate::mc4::Mc4, info: impl Fn(&str) -> (u32, i64)) {
    for r in m.vehicle_records.iter_mut() {
        r.refund_value = (r.spent_plus_1 & 0xFFFF) as i32 - 1;
    }
    let cars: Vec<String> = m.garage.iter().map(|c| c.vehicle.to_ascii_lowercase()).collect();
    for car in cars {
        let (download, price) = info(&car);
        if download > 3 {
            let r = m.record_mut(&car);
            r.refund_value = r.refund_value.wrapping_add(price as i32);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// 4. a car's profile name when it comes into the garage
// ---------------------------------------------------------------------------------------------------------------

/// The profile name (+8141, 34 bytes) F-80615 gives slot `k` as the car comes in (F-91320): the string of
/// `<TypeName>_shrt`, followed by " N" when N other cars of the garage are the same model (snprintf of 34: at most
/// 33 characters). None when the string table has no such key (what the lookup F-12531 then returns was not read).
pub fn profile_name(garage: &[Car], k: usize, string: impl Fn(&str) -> Option<String>) -> Option<String> {
    let me = garage.get(k)?;
    let others = garage.iter().enumerate().filter(|(j, c)| *j != k && c.vehicle_index == me.vehicle_index).count();
    let short = string(&format!("{}_shrt", me.TypeName.to_ascii_lowercase()))?;
    let s = if others > 0 { format!("{short} {others}") } else { short };
    Some(s.chars().take(33).collect())
}

// ---------------------------------------------------------------------------------------------------------------
// 5. the free part of a Part reward
// ---------------------------------------------------------------------------------------------------------------

/// The free part F-39082 (the Part reward's apply F-48610; no reward of the disc's tables or missions is of that
/// kind): one draw of the game's generator (its low 31 bits mod 4), then the part category of reward category
/// `cat` - 0: bike 2, car 11 / 12 / 13 / 17; 1: bike 4, car 45 / 45 / 46 / 43; 2: bike 3, car 18 / 18 / 19 / 19;
/// 3: car 10 / 10 / 14 / 14; 4: car 21 - -1 for a "_DUB" car, a bike in 3 or 4, another category. A part whose
/// first upgrade (bit 1) the model owns gives -1; else the bit is bought for nothing (F-66800 with price 0) and the
/// category returned. `owned(part)` asks F-10180 (part, 1); `buy(part)` is F-66800 (part, 1, 0).
pub fn free_part(rng: &mut crate::vehicles::parked::Mwc, cat: u32, bike: bool, type_name: &str, owned: impl Fn(i32) -> bool, mut buy: impl FnMut(i32)) -> i32 {
    let r = rng.next() & 0x7FFF_FFFF & 3;
    if type_name.contains("_DUB") {
        return -1;
    }
    let part = match cat {
        0 if bike => 2,
        0 => [11, 12, 13, 17][r as usize],
        1 if bike => 4,
        1 => [45, 45, 46, 43][r as usize],
        2 if bike => 3,
        2 => [18, 18, 19, 19][r as usize],
        3 | 4 if bike => return -1,
        3 => [10, 10, 14, 14][r as usize],
        4 => 21,
        _ => -1,
    };
    if owned(part) {
        return -1;
    }
    buy(part);
    part
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The free part against the original's (save_gaps_reference.py `part` -> testdata/save_gaps_part_ref.txt:
    /// 3,000 random generator states, categories 0..5 and -1, bikes, DUB names, owned masks): the owned test
    /// and the purchase with their arguments, the result, the generator after it.
    #[test]
    fn free_part_matches_the_original() {
        let mut n = 0;
        for line in include_str!("testdata/save_gaps_part_ref.txt").lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let f: Vec<&str> = input.split_whitespace().collect();
            let h = |i: usize| f[i].parse::<u32>().unwrap();
            let mut rng = crate::vehicles::parked::Mwc { lo: h(0), carry: h(1) };
            let owned_mask = h(4);
            let calls = std::cell::RefCell::new(String::new());
            let owned = |p: i32| {
                calls.borrow_mut().push_str(&format!(" O{p},1"));
                (p >= 0 && p < 32 && owned_mask >> p & 1 != 0) || (p >= 32 && owned_mask >> 31 & 1 != 0)
            };
            let got = free_part(&mut rng, h(2), h(3) != 0, f[5], owned, |p| calls.borrow_mut().push_str(&format!(" M{p},1,0")));
            let line_got = format!("{} = {} {} {}", calls.borrow(), got, rng.lo, rng.carry);
            assert_eq!(line_got.trim(), want.trim(), "{input}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    #[test]
    fn refund_and_profile_name() {
        let mut m = crate::mc4::Mc4::default();
        let car = |v: &str| Car { TypeName: v.to_string(), vehicle: v.to_string(), vehicle_index: crate::mc4::vehicle_index(v), ..Default::default() };
        m.garage = vec![car("vp_frd_gt_06"), car("vp_aud_r8_08"), car("vp_aud_r8_08")];
        m.record_mut("vp_frd_gt_06").spent_plus_1 = 1201;
        refund_values(&mut m, |v| if v == "vp_aud_r8_08" { (60, 40000) } else { (1, 9000) });
        assert_eq!(m.record("vp_frd_gt_06").map(|r| r.refund_value), Some(1200));
        assert_eq!(m.record("vp_aud_r8_08").map(|r| r.refund_value), Some(80000));
        let s = |k: &str| (k == "vp_aud_r8_08_shrt").then(|| "DL CAR".to_string());
        assert_eq!(profile_name(&m.garage, 1, s), Some("DL CAR 1".into()));
        assert_eq!(profile_name(&m.garage, 0, s), None);
    }

    /// The record constructor against the original's (tools/save_gaps_reference.py `ctor` ->
    /// testdata/save_gaps_ctor_ref.txt: 304 generator states, the memory filled with 0xAA): every byte of the
    /// 8176 and the generator's state after it.
    #[test]
    fn record_constructor_matches_the_original() {
        let mut n = 0;
        for line in include_str!("testdata/save_gaps_ctor_ref.txt").lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let s: Vec<u32> = input.split_whitespace().map(|x| x.parse().unwrap()).collect();
            let mut rng = crate::vehicles::parked::Mwc { lo: s[0], carry: s[1] };
            let b = record_ctor_bytes(&mut rng, 0xAA);
            let w: Vec<&str> = want.split_whitespace().collect();
            let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
            let want_hex: String = w[..w.len() - 2].concat();
            if hex != want_hex {
                let k = (0..8176).find(|&i| hex[2 * i..2 * i + 2] != want_hex[2 * i..2 * i + 2]).unwrap();
                panic!("{input}: byte {k} {} want {}", &hex[2 * k..2 * k + 8], &want_hex[2 * k..2 * k + 8]);
            }
            assert_eq!(format!("{} {}", rng.lo, rng.carry), format!("{} {}", w[w.len() - 2], w[w.len() - 1]), "{input}");
            n += 1;
        }
        assert_eq!(n, 304);
    }

    fn car(name: &str) -> Car {
        Car { TypeName: name.to_string(), vehicle: name.to_string(), ..Default::default() }
    }

    /// One entry as the reference prints it: kind, name, record there, the record's name, id.
    fn show(kind: i32, name: &str, rec: Option<&Car>, uid: i32) -> String {
        let n = if name.is_empty() { "-" } else { name };
        let r = rec.map_or("blank".to_string(), |c| if c.TypeName.is_empty() { "-".into() } else { c.TypeName.clone() });
        format!("{},{},{},{},{}", kind, n, rec.is_some() as i32, r, uid)
    }

    fn state(l: &PinkLocals, lot: &PinkSlips) -> String {
        let mut out = format!("| {}", lot.uid_counter as i32);
        for e in &l.at {
            out += &format!(" {}", show(e.kind, &e.name, e.record.as_ref(), e.uid));
        }
        for s in &lot.slots {
            let rec = if s.empty { None } else { crate::mc4::pink_record(s) };
            out += &format!(" {}", show(s.kind, &s.name, rec.as_ref(), s.uid));
        }
        out
    }

    /// The pink slip profiles by location against the original's eight functions (tools/save_gaps_reference.py
    /// `pink` -> testdata/save_gaps_pink_ref.txt: 400 new careers with random counters, 8,521 random operations at
    /// registered, unregistered and shared locations).
    #[test]
    fn pink_locations_match_the_original() {
        let (mut l, mut lot) = (PinkLocals::default(), PinkSlips::default());
        let (mut n, mut bad) = (0, 0);
        for line in include_str!("testdata/save_gaps_pink_ref.txt").lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let f: Vec<&str> = input.split_whitespace().collect();
            let a = |i: usize| f[i].parse::<i32>().unwrap();
            let ret = match f[0] {
                "RESET" => {
                    l = PinkLocals::default();
                    lot = PinkSlips::default();
                    lot_ensure(&mut lot);
                    lot.uid_counter = a(1) as u32;
                    String::new()
                }
                "A" => format!("{} ", l.add(&mut lot, a(1), &car(&format!("r{}", a(2))), a(3))),
                "L" => {
                    let s = format!("s{}", a(2));
                    format!("{} ", l.load(&mut lot, a(1), &s, &car(&s), a(3)))
                }
                "N" => format!("{} ", l.num(&lot, a(1))),
                "U" => format!("{} ", l.uid(&lot, a(1), a(2))),
                "T" => format!("{} ", l.kind(&lot, a(1), a(2))),
                "G" => format!("{} ", match l.find(&lot, a(1), a(2)) { Some(PinkAt::Local(k)) => k as i32, Some(PinkAt::Lot(k)) => 5 + k as i32, None => -1 }),
                "M" => format!("{} ", l.name(&lot, a(1), a(2)).map_or("(null)".to_string(), |s| if s.is_empty() { "-".into() } else { s.to_string() })),
                "R" => {
                    l.remove(&mut lot, a(1), a(2));
                    "0 ".to_string()
                }
                o => panic!("op {o}"),
            };
            let got = format!("{}{}", ret, state(&l, &lot));
            n += 1;
            if got.trim() != want.trim() {
                bad += 1;
                if bad < 5 {
                    eprintln!("{input}\n  want {want}\n  got  {got}");
                }
            }
        }
        assert!(n > 8000, "{n} lines");
        assert_eq!(bad, 0, "{bad} of {n} operations differ");
    }
}
