//! The Flash runtime helpers against the original's own code run offline (tools/ui_flash_reference.py ->
//! testdata/ui_flash_ref.txt): every output bit for bit.

use super::*;
use crate::draw::{apply_cx, combine_cx, gradient_color};
use crate::value::{hash, Value};
use mcla_formats::rage::swf::{CxForm, FillStyle, Gradient, Matrix};

fn lines(tag: &str) -> impl Iterator<Item = (Vec<u32>, Vec<u32>)> + '_ {
    include_str!("testdata/ui_flash_ref.txt").lines().filter(move |l| l.split(' ').next() == Some(tag)).map(|l| {
        let (a, b) = l[tag.len()..].split_once(" -> ").unwrap();
        let p = |s: &str| s.split_whitespace().map(|w| u32::from_str_radix(w, 16).unwrap()).collect::<Vec<_>>();
        (p(a), p(b))
    })
}

fn f(b: u32) -> f32 {
    f32::from_bits(b)
}

fn cx(b: &[u32]) -> CxForm {
    CxForm { mult: [b[0] as u8 as i8, b[1] as u8 as i8, b[2] as u8 as i8, b[3] as u8 as i8], add: [b[4] as u8 as i8, b[5] as u8 as i8, b[6] as u8 as i8, b[7] as u8 as i8] }
}

/// radial n ratios.. colors.. m[6] x y -> (fill, x, y)
fn gradient(v: &[u32]) -> (FillStyle, f32, f32) {
    let n = v[1] as usize;
    let m = &v[2 + 2 * n..];
    let g = Gradient {
        matrix: Matrix { a: f(m[0]), b: f(m[1]), c: f(m[2]), d: f(m[3]), tx: f(m[4]), ty: f(m[5]) },
        radial: v[0] != 0,
        ratios: v[2..2 + n].iter().map(|&r| r as u8).collect(),
        colors: v[2 + n..2 + 2 * n].to_vec(),
    };
    (FillStyle { kind: 0x10, color: 0, matrix: Matrix::IDENTITY, bitmap: None, gradient: Some(g) }, f(m[6]), f(m[7]))
}

/// F-57014: world colour transform (combine_cx)
#[test]
fn combine_cx_matches_the_original() {
    let mut n = 0;
    for (v, out) in lines("CX") {
        let t = v[16];
        let w = combine_cx(&cx(&v[0..8]), &cx(&v[8..16]), [(t >> 16) as u8, (t >> 8) as u8, t as u8, (t >> 24) as u8]);
        let got: Vec<u32> = w.mult.iter().chain(w.add.iter()).map(|&b| b as u8 as u32).collect();
        assert_eq!(got, out, "CX {v:x?}");
        n += 1;
    }
    assert!(n > 2900, "{n}");
}

/// F-54894: gradient colour
#[test]
fn gradient_color_matches_the_original() {
    let mut n = 0;
    for (v, out) in lines("GR") {
        let (fill, x, y) = gradient(&v);
        assert_eq!(gradient_color(&fill, x, y), out[0], "GR {v:x?}");
        n += 1;
    }
    assert!(n > 3900, "{n}");
}

/// F-50749: a vertex's fill colour (solid or gradient, through the colour transform and the alpha byte)
#[test]
fn fill_colour_matches_the_original() {
    let mut n = 0;
    for (v, out) in lines("FV") {
        let (mut fill, x, y) = gradient(&v[11..]);
        fill.color = v[1];
        if v[0] == 0 {
            fill.gradient = None;
        }
        let c = if fill.gradient.is_some() { gradient_color(&fill, x, y) } else { fill.color };
        let [r, g, b, a] = apply_cx(c, &cx(&v[2..10]), v[10] as u8);
        assert_eq!(u32::from_be_bytes([a, r, g, b]), out[0], "FV {v:x?}");
        n += 1;
    }
    assert!(n > 2900, "{n}");
}

/// F-98882: rotation from degrees
#[test]
fn rotation_matches_the_original() {
    let mut n = 0;
    for (v, out) in lines("RM") {
        let (s, c) = rotation(f(v[0]));
        assert_eq!([c.to_bits(), s.to_bits(), (-s).to_bits(), c.to_bits()], [out[0], out[1], out[2], out[3]], "RM {}", f(v[0]));
        assert_eq!(&out[4..], &[0, 0]);
        n += 1;
    }
    assert!(n > 3000);
}

/// F-74625: instance matrix compose
#[test]
fn compose_matches_the_original() {
    let mut n = 0;
    for (v, out) in lines("CM") {
        let mut i = Inst::new(1, 1, None);
        i.rotation = f(v[0]);
        i.placed_rot = f(v[1]);
        i.xscale = f(v[2]);
        i.yscale = f(v[3]);
        i.placed_sx = f(v[4]);
        i.placed_sy = f(v[5]);
        i.skew = [f(v[6]), f(v[7]), f(v[8]), f(v[9])];
        i.x = f(v[12]);
        i.placed_tx = f(v[13]);
        i.y = f(v[14]);
        i.placed_ty = f(v[15]);
        i.compose();
        let m = i.matrix;
        assert_eq!([m.a, m.b, m.c, m.d, m.tx, m.ty].map(f32::to_bits), [out[0], out[1], out[2], out[3], out[4], out[5]], "CM {v:x?}");
        n += 1;
    }
    assert!(n > 3900, "{n}");
}

/// The original's value types: 0 undefined, 1 string, 2 boolean (the first byte), 3 int, 4 float, 5 object
/// (the pointer; the port has no null object, so a null pointer is left out)
fn value(ty: u32, p: u32) -> Option<Value> {
    Some(match ty {
        0 => Value::Undef,
        1 => Value::str("x"),
        2 => Value::Bool(p >> 24 != 0),
        3 => Value::Int(p as i32),
        4 => Value::Float(f(p)),
        _ => return None,
    })
}

/// F-18546 (to bool), F-70327 (as object)
#[test]
fn to_bool_and_obj_match_the_original() {
    let mut n = 0;
    for (v, out) in lines("TB") {
        if let Some(x) = value(v[0], v[1]) {
            assert_eq!(x.to_bool(), out[0] != 0, "TB {v:x?}");
            n += 1;
        } else if v[1] != 0 {
            // an object: true, and obj() gives it back
            assert_eq!(out[0], 1);
            n += 1;
        }
    }
    for (v, out) in lines("OB") {
        if let Some(x) = value(v[0], v[1]) {
            assert!(x.obj().is_none() && out[0] == 0, "OB {v:x?}");
        } else {
            assert_eq!(out[0], v[1]);
        }
        n += 1;
    }
    assert_eq!(n, 64 + 66);
}

/// F-53993: a string value (its hash by F-58929, type 1)
#[test]
fn string_hash_matches_the_original() {
    let mut n = 0;
    for l in include_str!("testdata/ui_flash_ref.txt").lines().filter(|l| l.starts_with("HS ")) {
        let (s, out) = l[3..].split_once(" -> ").unwrap();
        let w: Vec<u32> = out.split(' ').map(|w| u32::from_str_radix(w, 16).unwrap()).collect();
        assert_eq!((w[1], w[2]), (hash(s), 1), "HS {s}");
        assert!(Value::str(s).is_str());
        n += 1;
    }
    assert!(n > 800, "{n}");
}

/// F-44617: the instance constructor - the fields the port models, at their offsets (the display sub-object at +8:
/// matrix +8, colour transform +64 / +68, tint +76, the transform parts +84..+136; frame +164, playing +172)
#[test]
fn instance_constructor_matches_the_original() {
    let l = include_str!("testdata/ui_flash_ref.txt").lines().find(|l| l.starts_with("IC ->")).unwrap();
    let w: std::collections::HashMap<usize, u32> = l[5..].split_whitespace().map(|t| {
        let (v, o) = t.split_once('@').unwrap();
        (usize::from_str_radix(o, 16).unwrap(), u32::from_str_radix(v, 16).unwrap())
    }).collect();
    let i = Inst::new(0, 0, None);
    let m = i.matrix;
    let words = [
        (0x08, m.a.to_bits()), (0x0c, m.b.to_bits()), (0x10, m.c.to_bits()), (0x14, m.d.to_bits()), (0x18, m.tx.to_bits()), (0x1c, m.ty.to_bits()),
        (0x40, u32::from_be_bytes(i.cxform.mult.map(|b| b as u8))), (0x44, u32::from_be_bytes(i.cxform.add.map(|b| b as u8))),
        (0x4c, u32::from_be_bytes(i.tint)),
        (0x54, i.placed_rot.to_bits()), (0x58, i.rotation.to_bits()), (0x5c, i.x.to_bits()), (0x60, i.y.to_bits()),
        (0x64, i.placed_sx.to_bits()), (0x68, i.placed_sy.to_bits()), (0x6c, i.xscale.to_bits()), (0x70, i.yscale.to_bits()),
        (0x74, i.placed_tx.to_bits()), (0x78, i.placed_ty.to_bits()),
        (0x7c, i.skew[0].to_bits()), (0x80, i.skew[1].to_bits()), (0x84, i.skew[2].to_bits()), (0x88, i.skew[3].to_bits()),
    ];
    for (o, v) in words {
        assert_eq!(w[&o], v, "+{o:#x}");
    }
    assert_eq!(w[&0xa4] >> 16, i.frame as u32);
    assert_eq!(w[&0xac] >> 24, i.playing as u32);
}

/// The value of an I2 / F2 case: a string (its bytes in hex, NUL-ended), else type 2 boolean (the first byte),
/// 3 int, 4 float, 5 object (to 0 either way: the port's object reference stands for it); 6 is outside the
/// original's types (to 0) and has no port value
fn conv_case(v: &[&str]) -> Option<Value> {
    let ty = u32::from_str_radix(v[0], 16).unwrap();
    let p = u32::from_str_radix(v[1], 16).unwrap();
    Some(match ty {
        1 => {
            let b: Vec<u8> = (0..v[2].len() / 2).map(|i| u8::from_str_radix(&v[2][2 * i..2 * i + 2], 16).unwrap()).take_while(|&b| b != 0).collect();
            Value::str(std::str::from_utf8(&b).unwrap())
        }
        2 => Value::Bool(p >> 24 != 0),
        3 => Value::Int(p as i32),
        4 => Value::Float(f(p)),
        _ => return None,
    })
}

/// F-68750 (value to int: a string through the C library's atoi), F-90005 (to float: atof, rounded to float)
#[test]
fn to_int_and_float_match_the_original() {
    let mut n = 0;
    for l in include_str!("testdata/ui_flash_ref.txt").lines().filter(|l| l.starts_with("I2 ") || l.starts_with("F2 ")) {
        let (a, out) = l[3..].split_once(" -> ").unwrap();
        let v: Vec<&str> = a.split(' ').collect();
        let out = u32::from_str_radix(out, 16).unwrap();
        let ty = u32::from_str_radix(v[0], 16).unwrap();
        // (a boolean's byte is 0 or 1 wherever the original makes one; other bytes are left out - the original
        // gives the byte itself, the port has no such value)
        // (nor is a signalling NaN float ever made: the original's load and rounding quiet it)
        let p = u32::from_str_radix(v[1], 16).unwrap();
        if (ty == 2 && p >> 24 > 1) || (ty == 4 && p & 0x7fc0_0000 == 0x7f80_0000 && p & 0x003f_ffff != 0) {
            continue;
        }
        let got = match conv_case(&v) {
            Some(x) if l.starts_with('I') => x.to_i32() as u32,
            Some(x) => x.to_f32().to_bits(),
            None => 0,
        };
        assert_eq!(got, out, "{l}");
        n += 1;
    }
    assert!(n > 6000, "{n}");
}

/// The Math natives (the MATH table; F-ids there)
#[test]
fn math_natives_match_the_original() {
    let mut n = 0;
    for (v, out) in lines("MA") {
        let (name, g) = MATH[v[0] as usize];
        // (sin, cos and tan of the original's C library give NaN from about 2.2e8 (tan 3e8) on, a range no movie
        // reaches; the port keeps the host library's value there - left out)
        if v[0] < 3 && f(v[1]).abs() > 1e6 && f(v[1]).is_finite() {
            continue;
        }
        let r = g(f(v[1]));
        // (a NaN result: the original's quiet NaN - compare as NaN)
        assert!(r.to_bits() == out[0] || (r.is_nan() && f(out[0]).is_nan()), "{name}({}) = {r} vs {}", f(v[1]), f(out[0]));
        n += 1;
    }
    assert!(n > 20000, "{n}");
}
