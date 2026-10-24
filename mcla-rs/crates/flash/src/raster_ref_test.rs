//! The sped-up rasteriser against the reference (`raster_ref`, the code before 2026-10-23): byte for byte.

use crate::draw::{DrawList, Item, TexRef, Vertex};
use std::sync::Arc;

struct Rng(u64);
impl Rng {
    fn f(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / (1u64 << 24) as f32
    }
    fn u8(&mut self) -> u8 { (self.f() * 256.0) as u8 }
}

fn list(r: &mut Rng) -> DrawList {
    let mut items = Vec::new();
    let mut depth = 0;
    for _ in 0..(5 + (r.f() * 20.0) as usize) {
        let k = r.f();
        if k < 0.08 { items.push(Item::MaskBegin); }
        else if k < 0.16 { items.push(Item::MaskEnd); depth += 1; }
        else if k < 0.2 && depth > 0 { items.push(Item::MaskPop); depth -= 1; }
        else {
            let tex = if r.f() < 0.5 { TexRef::None } else { TexRef::Bitmap(0) };
            let kind = [0x40u8, 0x41, 0x42, 0x43][(r.f() * 4.0) as usize];
            let mut verts = Vec::new();
            for _ in 0..3 * (1 + (r.f() * 4.0) as usize) {
                verts.push(Vertex { pos: [r.f() * 160.0 - 30.0, r.f() * 160.0 - 30.0], uv: [r.f() * 3.0 - 1.0, r.f() * 3.0 - 1.0], color: [r.u8(), r.u8(), r.u8(), r.u8()] });
            }
            items.push(Item::Triangles { tex, kind, verts });
        }
    }
    DrawList { items }
}

#[test]
fn new_raster_matches_old() {
    let mut r = Rng(12345);
    let tex = Arc::new(crate::raster::TexImage { w: 7, h: 5, rgba: (0..140).map(|i| (i * 37 % 256) as u8).collect() });
    let tex_old = Arc::new(crate::raster_ref::TexImage { w: 7, h: 5, rgba: tex.rgba.clone() });
    let mut big = 0;
    for case in 0..400 {
        let l = list(&mut r);
        let samples: &[(f32, f32)] = if case % 2 == 0 { &crate::raster::SAMPLES_4X } else { &[(0.5, 0.5)] };
        let (scale, off) = (0.5 + r.f() * 1.5, [r.f() * 10.0 - 5.0, r.f() * 10.0 - 5.0]);
        let mut a = crate::raster::Canvas::with_samples(100, 90, [10, 20, 30, 40], samples);
        a.draw(&l, scale, off, &|t| (t != TexRef::None).then(|| tex.clone()));
        let mut b = crate::raster_ref::Canvas::with_samples(100, 90, [10, 20, 30, 40], samples);
        b.draw(&l, scale, off, &|t| (t != TexRef::None).then(|| tex_old.clone()));
        assert_eq!(a.rgba, b.rgba, "case {case}");
        if a.rgba.iter().any(|&x| x != 10 && x != 20 && x != 30 && x != 40) { big += 1; }
    }
    assert!(big > 300, "{big}");
}

#[test]
fn round_pos_is_round() {
    // every f32 from 0 to 300 (and NaN) rounds as f32::round
    let mut x = 0f32;
    while x <= 300.0 {
        assert_eq!(super::raster::round_pos_for_test(x).to_bits(), x.round().to_bits(), "{x}");
        x = f32::from_bits(x.to_bits() + 1);
    }
    assert_eq!(super::raster::round_pos_for_test(f32::NAN) as u8, f32::NAN.round() as u8);
}
