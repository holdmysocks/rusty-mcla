//! (tests only) F-10653's hit queue against the original's code run offline (tools/leaves3_reference.py).

use super::*;
use crate::physics::artic_offline::{records, report};

const DATA: &str = include_str!("testdata/leaves3_ref.txt");
const OBJ: u32 = 0x1000;

/// 150 cases: a count of 0 .. 9, 20 and 2^31 - 1 (the port's queue never holds more than 8: it is capped there), the new
/// hit's point (a 16-byte read), amount and kind (the register's low byte). The slot the hit lands in holds the point,
/// the amount and the kind byte; a full queue (8 or more) is left as it was. The slot's fourth point word is not kept.
#[test]
fn the_queue_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(DATA, "qu") {
        let count = r.mem.word(OBJ + 2144);
        let mut p = DamageParts::default();
        for _ in 0..count.min(8) {
            p.hits.push(PartHit { point: Vec3::splat(9.0), amount: 9.0, kind: 9 });
        }
        let pt = r.r(4);
        let hit = PartHit {
            point: Vec3::new(f32::from_bits(r.mem.word(pt)), f32::from_bits(r.mem.word(pt + 4)), f32::from_bits(r.mem.word(pt + 8))),
            amount: f32::from_bits(r.r(14)),
            kind: r.r(5) as u8,
        };
        p.queue(hit);
        let after = r.want(OBJ + 2144, 1)[0];
        if p.hits.len() as u32 != after.min(8) {
            bad.push(format!("case {n}: count port {} original {after}", p.hits.len()));
        }
        if count < 8 {
            let slot = r.want(OBJ + 1888 + 32 * count, 8);
            let h = &p.hits[count as usize];
            let got = [h.point.x.to_bits(), h.point.y.to_bits(), h.point.z.to_bits(), h.amount.to_bits(), (h.kind as u32) << 24];
            let want = [slot[0], slot[1], slot[2], slot[4], slot[5] & 0xFF00_0000];
            if got != want {
                bad.push(format!("case {n}: slot {count} port {got:08x?} original {want:08x?}"));
            }
        }
        n += 1;
    }
    report("F-10653 queue", n, &bad);
}
