//! (tests only) F-42446's effect rule against the original's code run offline (tools/leaves3_reference.py): the wheel, its
//! ground material, the effect table and the globals laid out as the original reads them; F-92300 (the held compression)
//! answers from the case, F-63975 (start or go on with an effect) and F-33510 (stop it) write down what they were given.

use super::*;
use crate::physics::artic_offline::{diff, records, report, Rec};

const DATA: &str = include_str!("testdata/leaves3_ref.txt");
const S: u32 = 0x1000;
const W: u32 = 0x1100;
const MAT: u32 = 0x1400;
const TUNE: u32 = 0x1440;
const DRV: u32 = 0x14C0;
const TBL: u32 = 0x1700;
const G_WET: u32 = 0x8000_0000 + 0x0287_E130;
const G_DT: u32 = 0x8000_0000 + 0x027D_7508;

fn fl(r: &Rec, a: u32) -> f32 {
    f32::from_bits(r.mem.word(a))
}

/// 900 cases: burnouts under and over 8 m/s, wet and dry roads (the wetness at, a hair under and over 0.9), thresholds 0 and
/// above, material indices in and out of the table (negative ones too), an effect playing or not, the linger count from
/// -3 to 13, radii at the ramp's ends, velocities in all three ranges. What the original hands to the effect (or the stop) is
/// what the port keeps in the state: the effect, skidAmount, rainy, donut, tireSize, speed, fwd and the linger count.
#[test]
fn the_effect_rule_matches_the_original() {
    let (mut n, mut bad) = (0, Vec::new());
    for r in records(DATA, "tf") {
        let rows = r.mem.word(TBL + 4);
        let table: Vec<[Option<u32>; 4]> = (0..rows)
            .map(|k| {
                let col = |c: u32| Some(r.mem.word(r.mem.word(TBL + 8 + 8 * c) + 4 * k)).filter(|w| *w != 0);
                [col(0), col(1), col(2), col(3)]
            })
            .collect();
        let fx = TyreFx { table, mats: HashMap::new(), cars: HashMap::new(), wet: fl(&r, G_WET) };
        let ixw = r.mem.word(MAT + 44);
        let wheel = WheelIn {
            ix: [(ixw >> 16) as u16 as i16, ixw as u16 as i16],
            thr: [fl(&r, MAT + 48), fl(&r, MAT + 52)],
            cc: fl(&r, 0x3A10),
            limit: fl(&r, TUNE + 84),
            slide: fl(&r, W + 520),
            on: (r.mem.word(W + 528) >> 16) & 0xFF != 0,
            vlong: fl(&r, W + 296),
            vlat: fl(&r, W + 292),
            radius: fl(&r, W + 420),
        };
        let active = r.mem.word(S) != 0;
        let mut s = State {
            rule: if active { r.mem.word(S + 52) } else { 0 },
            skid: fl(&r, S + 32),
            rainy: fl(&r, S + 36),
            donut: fl(&r, S + 40),
            linger: r.mem.word(S + 48) as i32,
            ..Default::default()
        };
        let vel = Vec3::new(fl(&r, 0x3800), fl(&r, 0x3804), fl(&r, 0x3808));
        fx.decide_in(&mut s, &wheel, r.r(7) != 0, r.mem.word(DRV + 372) as u8, vel, fl(&r, G_DT));
        let name = format!("case {n}");
        let stored = r.want(S + 32, 5);
        diff(&format!("{name} skid rainy donut"), &[s.skid.to_bits(), s.rainy.to_bits(), s.donut.to_bits()], &stored[..3], true, &mut bad);
        diff(&format!("{name} linger"), &[s.linger as u32], &stored[4..], false, &mut bad);
        let cap = r.want(0x3A20, 7);
        let (stops, calls) = (r.want(0x3A50, 2)[0], r.want(0x3A50, 2)[1]);
        let rule = if calls == 1 { cap[0] } else { 0 };
        diff(&format!("{name} effect (stops {stops}, starts {calls})"), &[s.rule], &[rule], false, &mut bad);
        if calls == 1 {
            let mine = [s.fwd as u32, s.skid.to_bits(), s.rainy.to_bits(), s.donut.to_bits(), s.tire_size.to_bits(), s.speed.to_bits()];
            diff(&format!("{name} handed over"), &mine, &cap[1..], true, &mut bad);
        }
        n += 1;
    }
    report("F-42446 effect rule", n, &bad);
}
