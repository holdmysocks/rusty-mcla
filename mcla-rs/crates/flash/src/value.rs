//! Script values of the original runtime (`swfSCRIPTOBJECT` value records:
//! payload + type, 0/1 string, 2 bool, 3 int, 4 float, 5 object, 6 native,
//! 7 undefined) and its conversions (`0x825EF780` bool, `0x825EF808` int,
//! `0x825EF8C8` float, `0x825EFB40` string).
//!
//! Numbers are single precision; `null`/`undefined` pushed by script are the
//! integer 0; strings are false.

use std::rc::Rc;

pub type InstId = u32;
pub type ObjId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ref {
    Inst(InstId),
    Obj(ObjId),
}

pub type NativeFn = fn(&mut crate::player::Player, Ref, &[Value]) -> Value;

#[derive(Clone)]
pub enum Value {
    Undef,
    Bool(bool),
    Int(i32),
    Float(f32),
    Str(Rc<str>),
    Obj(Ref),
    Native(NativeFn),
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Undef => write!(f, "undefined"),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Int(i) => write!(f, "{}i", i),
            Value::Float(x) => write!(f, "{}f", x),
            Value::Str(s) => write!(f, "{:?}", s),
            Value::Obj(r) => write!(f, "{:?}", r),
            Value::Native(_) => write!(f, "<native>"),
        }
    }
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(Rc::from(s))
    }

    pub fn is_str(&self) -> bool {
        matches!(self, Value::Str(_))
    }

    pub fn obj(&self) -> Option<Ref> {
        match self {
            Value::Obj(r) => Some(*r),
            _ => None,
        }
    }

    pub fn to_bool(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(x) => *x != 0.0,
            Value::Obj(_) | Value::Native(_) => true,
            Value::Str(_) | Value::Undef => false,
        }
    }

    pub fn to_f32(&self) -> f32 {
        match self {
            Value::Str(s) => atof(s),
            Value::Bool(b) => *b as i32 as f32,
            Value::Int(i) => *i as f32,
            Value::Float(x) => *x,
            _ => 0.0,
        }
    }

    pub fn to_i32(&self) -> i32 {
        match self {
            Value::Str(s) => atoi(s),
            Value::Bool(b) => *b as i32,
            Value::Int(i) => *i,
            // fctiwz: truncate towards zero, saturating
            Value::Float(x) => {
                if x.is_nan() {
                    i32::MIN
                } else {
                    x.trunc().clamp(i32::MIN as f32, i32::MAX as f32) as i32
                }
            }
            _ => 0,
        }
    }

    /// String form: ints "%d", floats "%0.f" when integral else "%f",
    /// booleans "true"/"false", objects and undefined "".
    pub fn to_string(&self) -> Rc<str> {
        match self {
            Value::Str(s) => s.clone(),
            Value::Bool(b) => Rc::from(if *b { "true" } else { "false" }),
            Value::Int(i) => Rc::from(i.to_string()),
            Value::Float(x) => Rc::from(format_float(*x)),
            _ => Rc::from(""),
        }
    }
}

pub fn format_float(x: f32) -> String {
    let t = if x.is_nan() { i32::MIN } else { x.trunc().clamp(i32::MIN as f32, i32::MAX as f32) as i32 };
    let xd = x as f64;
    if t as f32 == x {
        format!("{:.0}", xd)
    } else if x.is_infinite() {
        // the original's C library: "1.#INF00"
        if x > 0.0 { "1.#INF00".into() } else { "-1.#INF00".into() }
    } else if x.is_nan() {
        // ... and "1.#QNAN0" for every NaN but the default negative one, "-1.#IND00" (F-24664 run offline, all payloads tried)
        let bits = x.to_bits();
        match (bits >> 31 != 0, bits & 0x7f_ffff) {
            (true, 0x40_0000) => "-1.#IND00".into(),
            (true, _) => "-1.#QNAN0".into(),
            (false, _) => "1.#QNAN0".into(),
        }
    } else if xd.abs() >= 1e17 {
        big_fixed(xd)
    } else {
        format!("{:.6}", xd)
    }
}

/// `%f` of a double of 1e17 or more as the original's C library prints it (F-24664 run offline): 17 significant digits, correctly
/// rounded, the rest zeros ("300000000549775580000000000000000000000.000000", not the exact expansion).
fn big_fixed(x: f64) -> String {
    let s = format!("{:.16e}", x.abs());
    let (m, e) = s.split_once('e').unwrap();
    let digits: String = m.chars().filter(|c| c.is_ascii_digit()).collect();
    let exp: usize = e.parse().unwrap();
    let mut out = String::new();
    if x < 0.0 {
        out.push('-');
    }
    out += &digits;
    out.extend(std::iter::repeat('0').take(exp + 1 - digits.len()));
    out += ".000000";
    out
}

/// C `atof`: leading whitespace, optional sign, digits, fraction, exponent.
pub fn atof(s: &str) -> f32 {
    let t = s.trim_start();
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    t[..i].parse::<f64>().map(|v| v as f32).unwrap_or(0.0)
}

/// C `atoi`.
pub fn atoi(s: &str) -> i32 {
    let t = s.trim_start();
    let b = t.as_bytes();
    let mut i = 0;
    let neg = if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
        b[0] == b'-'
    } else {
        false
    };
    let mut v: i64 = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        v = (v * 10 + (b[i] - b'0') as i64).min(i64::from(i32::MAX) + 1);
        i += 1;
    }
    let v = if neg { -v } else { v };
    v.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// rage's string hash (Jenkins one-at-a-time over lower-case ASCII).
pub fn hash(s: &str) -> u32 {
    let mut h: u32 = 0;
    for mut c in s.bytes() {
        if c.is_ascii_uppercase() {
            c += 32;
        }
        h = h.wrapping_add(c as u32);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h.wrapping_add(h << 15)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert_eq!(&*Value::Float(5.0).to_string(), "5");
        assert_eq!(&*Value::Float(0.5).to_string(), "0.500000");
        assert_eq!(&*Value::Int(-3).to_string(), "-3");
        assert!(!Value::str("1").to_bool());
        assert_eq!(Value::str(" 12abc").to_i32(), 12);
        assert_eq!(Value::str("3.5e1x").to_f32(), 35.0);
        assert_eq!(Value::Float(-2.7).to_i32(), -2);
    }

    #[test]
    fn property_hashes_match_the_executable() {
        // constants compared in the instance getMember (0x825FC7B0)
        assert_eq!(hash("_x"), 0xce44b0af);
        assert_eq!(hash("_alpha"), 0xb18445fe);
        assert_eq!(hash("onEnterFrame"), 0x554c580b);
        assert_eq!(hash("textColor"), 0x7605bece);
    }
}
