//! Small helpers shared by the decoders: big-endian reads that never panic, Python-compatible rounding, half floats, the game's string hash.

use serde_json::{Number, Value};

/// Big-endian reads at absolute offsets. Out-of-range reads give zeros: the decoders check the size of every entry first, so this only
/// keeps a damaged file from panicking (the launcher is built with `panic = "abort"`).
#[derive(Clone, Copy)]
pub struct B<'a>(pub &'a [u8]);

impl<'a> B<'a> {
    fn arr<const N: usize>(&self, o: usize) -> [u8; N] {
        let mut a = [0u8; N];
        if let Some(s) = o.checked_add(N).and_then(|e| self.0.get(o..e)) {
            a.copy_from_slice(s);
        }
        a
    }
    pub fn u8(&self, o: usize) -> u8 {
        self.0.get(o).copied().unwrap_or(0)
    }
    pub fn i8(&self, o: usize) -> i8 {
        self.u8(o) as i8
    }
    pub fn u16(&self, o: usize) -> u16 {
        u16::from_be_bytes(self.arr(o))
    }
    pub fn u32(&self, o: usize) -> u32 {
        u32::from_be_bytes(self.arr(o))
    }
    pub fn i32(&self, o: usize) -> i32 {
        i32::from_be_bytes(self.arr(o))
    }
    /// A 32-bit float as a double (Python's `struct.unpack('>f')`).
    pub fn f32(&self, o: usize) -> f64 {
        f32::from_be_bytes(self.arr(o)) as f64
    }
    pub fn half(&self, o: usize) -> f64 {
        half(self.u16(o))
    }
    pub fn slice(&self, o: usize, n: usize) -> &'a [u8] {
        o.checked_add(n).and_then(|e| self.0.get(o..e)).unwrap_or(&[])
    }
}

/// IEEE 754 binary16 to double (Python's `struct.unpack('>e')`).
pub fn half(bits: u16) -> f64 {
    let sign = if bits >> 15 == 1 { -1.0 } else { 1.0 };
    let e = ((bits >> 10) & 0x1f) as i32;
    let m = (bits & 0x3ff) as f64;
    if e == 0 {
        sign * m * 2f64.powi(-24)
    } else if e == 31 {
        if m == 0.0 {
            sign * f64::INFINITY
        } else {
            f64::NAN
        }
    } else {
        sign * (1.0 + m / 1024.0) * 2f64.powi(e - 15)
    }
}

/// Python's `round(x, n)`: the double nearest to the exactly rounded decimal, ties to even (Rust's `{:.n}` formatting rounds the same way).
pub fn round_to(x: f64, n: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    format!("{:.*}", n, x).parse().unwrap_or(x)
}

/// Python's `round(x)` for a float: ties to even.
pub fn round_int(x: f64) -> i64 {
    x.round_ties_even() as i64
}

/// A JSON number from a double; a NaN or infinity (which JSON cannot hold) becomes null.
pub fn num(x: f64) -> Value {
    Number::from_f64(x).map(Value::Number).unwrap_or(Value::Null)
}

/// An f32 (given widened) as the shortest decimal that reads back as the same f32 (Rust's `Display` of f32), as a double.
pub fn f32_shortest(x: f64) -> f64 {
    format!("{}", x as f32).parse().unwrap_or(x)
}

pub fn nums(xs: &[f64]) -> Value {
    Value::Array(xs.iter().map(|x| num(*x)).collect())
}

pub fn hex(b: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        let _ = write!(s, "{x:02x}");
    }
    s
}

pub fn hex32(v: u32) -> String {
    format!("0x{v:08x}")
}

/// Python's `bytes.split(b'\0')[0].decode('latin1')`.
pub fn cstr(b: &[u8]) -> String {
    b.iter().take_while(|c| **c != 0).map(|c| *c as char).collect()
}

/// Python's `bytes.decode('latin1')`.
pub fn latin1(b: &[u8]) -> String {
    b.iter().map(|c| *c as char).collect()
}

/// Python's `names[v] if 0 <= v < len(names) else v`.
pub fn enum_name(names: &[&str], v: i64) -> Value {
    if v >= 0 && (v as usize) < names.len() {
        Value::from(names[v as usize])
    } else {
        Value::from(v)
    }
}

/// The game's string hash (sub_821C9790) as the vehicle and options decoders had it: lower-cased bytes, one-at-a-time.
pub fn joaat_lower(s: &str) -> u32 {
    let mut h: u32 = 0;
    for c in s.to_lowercase().chars() {
        h = h.wrapping_add(c as u32 & 0xff);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h.wrapping_add(h << 15)
}

/// The same hash as `career.py` had it: A-Z lower-cased, a backslash read as a slash, nothing else changed.
pub fn joaat_career(s: &str) -> u32 {
    let mut h: u32 = 0;
    for c in s.chars() {
        let mut o = c as u32;
        if (65..=90).contains(&o) {
            o += 32;
        } else if o == 92 {
            o = 47;
        }
        h = h.wrapping_add(o);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h.wrapping_add(h << 15)
}

/// Python's `colorsys.rgb_to_hsv`.
pub fn rgb_to_hsv(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let maxc = r.max(g).max(b);
    let minc = r.min(g).min(b);
    let rangec = maxc - minc;
    let v = maxc;
    if minc == maxc {
        return (0.0, 0.0, v);
    }
    let s = rangec / maxc;
    let rc = (maxc - r) / rangec;
    let gc = (maxc - g) / rangec;
    let bc = (maxc - b) / rangec;
    let h = if r == maxc {
        bc - gc
    } else if g == maxc {
        2.0 + rc - bc
    } else {
        4.0 + gc - rc
    };
    (py_mod(h / 6.0, 1.0), s, v)
}

/// Python's float `%` (the result has the sign of the divisor).
fn py_mod(a: f64, b: f64) -> f64 {
    let m = a % b;
    if m != 0.0 && (m < 0.0) != (b < 0.0) {
        m + b
    } else {
        m
    }
}

/// Drops the keys of `drop` at every depth (the importer's `strip`).
pub fn strip(v: &Value, drop: &[&str]) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.iter().filter(|(k, _)| !drop.contains(&k.as_str())).map(|(k, v)| (k.clone(), strip(v, drop))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(|x| strip(x, drop)).collect()),
        other => other.clone(),
    }
}

/// Python's truthiness of a JSON value as far as the decoders need it.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}
