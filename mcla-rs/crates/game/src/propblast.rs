//! The prop blast of the original, small routines as pure functions (docs/reference/ai/prop_blast.md).
//!
//! * the explosion behaviour's `IsFinished` (F-35914), `Start` (F-67359) and update slot (F-61628),
//! * the list of the props a blast may pick (F-76379): breakable oil pumps and gas-station balls,
//! * the random pick from that list (F-41412) with the career's generator,
//! * the obstacle extents a prop type takes from its bound (F-34384).
//!
//! Each one was run offline from the original's own code (work/racerai/propblast) on random cases; the reference
//! lines are `testdata/propblast_ref.txt` and the tests below replay them.

/// The two file names (without directory) the blast list takes.
pub const BLAST_NAMES: [&str; 2] = ["la_prop_oil_pump_01x", "la_prop_gasstation_ball_01x"];

/// The part of a file name after its last '/' (the whole name without one).
pub fn base_name(name: &str) -> &str {
    match name.rfind('/') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

/// Case-insensitive (ASCII) equality with one of the two blast names, on the name after the last '/'.
pub fn is_blast_name(name: &str) -> bool {
    let b = base_name(name);
    BLAST_NAMES.iter().any(|n| b.eq_ignore_ascii_case(n))
}

/// A prop of the placed-prop list as the collection looks at it.
pub struct BlastCandidate<'a> {
    /// The instance's virtual slot 232 answered non-null (inferred: "is a real, loaded instance").
    pub alive: bool,
    /// The prop type's word at +408 is non-zero (a breakable type).
    pub breakable: bool,
    /// The type's file name (word +176).
    pub name: &'a str,
}

/// F-76379: the indices (into the list) of every prop that is present, alive, breakable and named
/// like a blast prop; in list order, without a limit (the original's array holds 40 and does not check).
pub fn collect_blast_props(list: &[Option<BlastCandidate>]) -> Vec<usize> {
    list.iter()
        .enumerate()
        .filter_map(|(i, p)| match p {
            Some(p) if p.alive && p.breakable && is_blast_name(p.name) => Some(i),
            _ => None,
        })
        .collect()
}

/// F-41412: one step of the career's multiply-with-carry generator, state `(lo, carry)` = the two words at +0 / +4
/// of the random block: `x = lo * 0x5CDCFAA7 + carry`, new lo = low 32 bits, new carry = high 32 bits; the
/// draw is `lo' & 0x7FFFFFFF`. The index is the draw modulo `count` (a u16; zero traps in the original, `None`).
/// Returns the new state and the index.
pub fn blast_pick(state: (u32, u32), count: u16) -> ((u32, u32), Option<usize>) {
    // the trap on an empty list is the first instruction after the count load: the state does not move
    if count == 0 {
        return (state, None);
    }
    let x = state.0 as u64 * 0x5CDC_FAA7 + state.1 as u64;
    let new = (x as u32, (x >> 32) as u32);
    (new, Some(((new.0 & 0x7FFF_FFFF) % count as u32) as usize))
}

/// The explosion behaviour's tune (the object at +16): `duration` +4, `start` +8, `radius` +12, `rate` +16.
/// (The field names are inferred from their use; the tune's parser was not found.)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExplosionTune {
    pub duration: f32,
    pub start: f32,
    pub radius: f32,
    pub rate: f32,
}

/// F-35914: no tune gives false, else `timer > duration` (a NaN gives false).
pub fn explosion_is_finished(tune: Option<&ExplosionTune>, timer: f32) -> bool {
    match tune {
        None => false,
        Some(t) => timer > t.duration,
    }
}

/// F-67359: `(radius, timer)` the behaviour starts with, `(tune.radius, tune.start)`; the owner's shape then gets
/// SetRadius(tune.radius).
pub fn explosion_start(t: &ExplosionTune) -> (f32, f32) {
    (t.radius, t.start)
}

/// What the update slot does in one tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExplosionStep {
    pub timer: f32,
    pub radius: f32,
    /// The radius handed to SetRadius, when the tick set one.
    pub set_radius: Option<f32>,
    /// When the tick set one: the radius before the tick (the argument of the physics-level call that follows).
    pub old_radius: Option<f32>,
}

/// The update slot (vtable slot 5): `timer += (start - duration) * rate * dt` (the product rounded to f32, then one
/// fused multiply-add); then, if the slot-2 test (`timer > duration`) holds and the owner has an instance
/// (`owner_index != 0xFFFF`): `radius = timer * dt + radius` (fused) and SetRadius(radius).
pub fn explosion_update(t: &ExplosionTune, timer: f32, radius: f32, dt: f32, owner_index: u16) -> ExplosionStep {
    let step = (t.start - t.duration) * t.rate;
    let timer = (step as f64).mul_add(dt as f64, timer as f64) as f32;
    if explosion_is_finished(Some(t), timer) && owner_index != 0xFFFF {
        let r = (timer as f64).mul_add(dt as f64, radius as f64) as f32;
        ExplosionStep { timer, radius: r, set_radius: Some(r), old_radius: Some(radius) }
    } else {
        ExplosionStep { timer, radius, set_radius: None, old_radius: None }
    }
}

/// The type's bound as F-34384 reads it.
#[derive(Clone, Copy, Debug)]
pub struct BoundIn {
    /// Byte +4: 1 = sphere-like (radius at +128), 12 = composite.
    pub kind: u8,
    /// Float +128 when `kind == 1`.
    pub radius: f32,
    /// Floats +16, +24, +32, +40: the box's x / z extents (`min.x`, `min.z`, `max.x`, `max.z`).
    pub min_x: f32,
    pub min_z: f32,
    pub max_x: f32,
    pub max_z: f32,
    /// Composite only: how many of its parts are present.
    pub parts_present: usize,
    /// Composite only: part 1 is present and is a capsule bound, its radius (float +128).
    pub part1_capsule_radius: Option<f32>,
}

/// F-34384: the prop type's obstacle extents `[+336, +340, +344]` (x size, z size, half-diagonal).
/// `fixed` is the caller's flag (no breakable group and a negative minMoveForce).
pub fn prop_extents(fixed: bool, b: &BoundIn) -> [f32; 3] {
    let mut r = None;
    if fixed && b.kind == 12 && b.parts_present > 1 {
        r = b.part1_capsule_radius;
    } else if b.kind == 1 {
        r = Some(b.radius);
    }
    if let Some(r) = r {
        return [r * 2.0, r * 2.0, r];
    }
    // fsel(a - b >= 0, a, b): a NaN difference picks b
    let pick = |a: f32, b: f32| if a - b >= 0.0 { a } else { b };
    let ez = pick(b.max_z.abs(), b.min_z.abs());
    let ex = pick(b.max_x.abs(), b.min_x.abs());
    // fmuls ez ez, fmadds ex ex (+) : sqrt of the fused sum, rounded to f32 first
    let sq = (ex as f64).mul_add(ex as f64, (ez * ez) as f64) as f32;
    let diag = (sq as f64).sqrt() as f32;
    [ex * 2.0, ez * 2.0, diag]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(w: &str) -> f32 {
        f32::from_bits(u32::from_str_radix(w.trim_start_matches('h'), 16).unwrap())
    }
    fn word(w: &str) -> u32 {
        u32::from_str_radix(w.trim_start_matches('h'), 16).unwrap()
    }
    const REF: &str = include_str!("testdata/propblast_ref.txt");

    fn same(a: f32, b: f32) -> bool {
        a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
    }

    #[test]
    fn is_finished_matches_the_original() {
        let (mut n, mut ok) = (0, 0);
        for l in REF.lines().filter(|l| l.starts_with("A ")) {
            let w: Vec<&str> = l.split(' ').collect();
            let tune = (w[1] == "1").then(|| ExplosionTune { duration: bits(w[3]), start: 0.0, radius: 0.0, rate: 0.0 });
            let got = explosion_is_finished(tune.as_ref(), bits(w[2]));
            n += 1;
            ok += (got == (w[4] == "1")) as usize;
        }
        assert!(n >= 300 && ok == n, "{ok}/{n}");
    }

    #[test]
    fn start_matches_the_original() {
        let mut n = 0;
        for l in REF.lines().filter(|l| l.starts_with("B ")) {
            let w: Vec<&str> = l.split(' ').collect();
            let t = ExplosionTune { duration: 0.0, start: bits(w[1]), radius: bits(w[2]), rate: 0.0 };
            let (radius, timer) = explosion_start(&t);
            assert!(same(radius, bits(w[4])) && same(timer, bits(w[5])), "{l}");
            assert!(same(radius, bits(w[6])), "{l}");
            n += 1;
        }
        assert!(n >= 300);
    }

    #[test]
    fn update_matches_the_original() {
        let mut n = 0;
        for l in REF.lines().filter(|l| l.starts_with("C ")) {
            let w: Vec<&str> = l.split(' ').collect();
            let t = ExplosionTune { duration: bits(w[4]), start: bits(w[5]), radius: 0.0, rate: bits(w[6]) };
            let s = explosion_update(&t, bits(w[2]), bits(w[3]), bits(w[1]), w[7].parse().unwrap());
            // w[8] = "->", timer', radius', calls, setradius arg, calls, calls, old radius
            assert!(same(s.timer, bits(w[9])) && same(s.radius, bits(w[10])), "{l}");
            assert_eq!(s.set_radius.is_some(), w[11] == "1", "{l}");
            if let Some(r) = s.set_radius {
                assert!(same(r, bits(w[12])) && same(s.old_radius.unwrap(), bits(w[15])), "{l}");
            }
            n += 1;
        }
        assert!(n >= 400);
    }

    #[test]
    fn collect_matches_the_original() {
        let mut n = 0;
        for l in REF.lines().filter(|l| l.starts_with("D ")) {
            let (head, tail) = l.split_once(" -> ").unwrap();
            let items: Vec<&str> = head.split(' ').skip(2).collect();
            let list: Vec<Option<BlastCandidate>> = items
                .iter()
                .map(|s| {
                    if *s == "-" {
                        None
                    } else {
                        let (f, name) = s.split_once(':').unwrap();
                        Some(BlastCandidate { alive: f.starts_with('1'), breakable: f.ends_with('1'), name })
                    }
                })
                .collect();
            let want: Vec<usize> = tail.split(' ').skip(1).map(|x| x.parse().unwrap()).collect();
            assert_eq!(collect_blast_props(&list), want, "{l}");
            n += 1;
        }
        assert!(n >= 300);
    }

    #[test]
    fn pick_matches_the_original() {
        let (mut n, mut hit) = (0, 0);
        for l in REF.lines().filter(|l| l.starts_with("E ")) {
            let w: Vec<&str> = l.split(' ').collect();
            let (st, idx) = blast_pick((word(w[2]), word(w[3])), w[1].parse().unwrap());
            assert_eq!(st, (word(w[5]), word(w[6])), "{l}");
            if w[7] != "-" {
                assert_eq!(0xAB00_0000 | idx.unwrap() as u32, word(w[7]), "{l}");
                hit += 1;
            }
            n += 1;
        }
        assert!(n >= 400 && hit > 350);
    }

    #[test]
    fn extents_match_the_original() {
        let mut n = 0;
        for l in REF.lines().filter(|l| l.starts_with("F ")) {
            let w: Vec<&str> = l.split(' ').collect();
            let kind: u8 = w[1].parse().unwrap();
            let capok = w[4] == "1" && w[5] == "1";
            let b = BoundIn {
                kind,
                radius: bits(w[6]),
                min_x: bits(w[8]),
                min_z: bits(w[9]),
                max_x: bits(w[10]),
                max_z: bits(w[11]),
                parts_present: w[3].parse().unwrap(),
                part1_capsule_radius: capok.then(|| bits(w[7])),
            };
            let got = prop_extents(w[2] == "1", &b);
            for k in 0..3 {
                assert!(same(got[k], bits(w[13 + k])), "{l} lane {k}: {:08x}", got[k].to_bits());
            }
            n += 1;
        }
        assert!(n >= 600);
    }
}

#[cfg(test)]
mod stored_tests {
    /// The city's prop types (`resources/prop/0x0e66973c.r131`): the obstacle extents the resource stores at
    /// +336 / +340 / +344 (what the port reads, `Prop::obstacle`) against F-34384 run on each type's own bound
    /// (`*(*(type + 228) + 12)`), with the fixed flag either way: 225 of 230 equal bit for bit, the other 5 (the
    /// newspaper vending boxes) differ by one bit in the half diagonal only. Skipped without the data.
    #[test]
    fn stored_prop_extents_are_f34384s() {
        let Ok(root) = std::env::var("MCLA_DATA_DIR") else { return };
        let Ok(res) = crate::assets::load_resource_file(&std::path::Path::new(&root).join("resources/prop/0x0e66973c.r131")) else { return };
        let props = mcla_formats::rage::props::parse_props(&res);
        let rd = mcla_formats::rage::Rd::new(&res);
        let (mut n, mut exact, mut near) = (0, 0, 0);
        for p in &props {
            let Some(b) = rd.ptr(p.off + 228).and_then(|a| rd.ptr(a + 12)) else { continue };
            let kind = rd.u8(b + 4);
            let (mut present, mut cap) = (0, None);
            if kind == 12 {
                if let Some(arr) = rd.ptr(b + 128) {
                    present = (0..rd.u16(b + 146) as usize).filter(|i| rd.ptr(arr + 4 * i).is_some()).count();
                    cap = rd.ptr(arr + 4).filter(|p1| rd.u8(p1 + 4) == 1).map(|p1| rd.f32(p1 + 128));
                }
            }
            let bi = super::BoundIn { kind, radius: rd.f32(b + 128), min_x: rd.f32(b + 16), min_z: rd.f32(b + 24), max_x: rd.f32(b + 32), max_z: rd.f32(b + 40), parts_present: present, part1_capsule_radius: cap };
            let st = p.obstacle;
            let ways = [super::prop_extents(true, &bi), super::prop_extents(false, &bi)];
            n += 1;
            if ways.iter().any(|w| (0..3).all(|k| w[k].to_bits() == st[k].to_bits())) {
                exact += 1;
            } else if ways.iter().any(|w| w[0].to_bits() == st[0].to_bits() && w[1].to_bits() == st[1].to_bits() && w[2].to_bits().abs_diff(st[2].to_bits()) <= 1) {
                near += 1;
            } else {
                panic!("{}: stored {:?}, F-34384 {:?}", p.name, st, ways);
            }
        }
        assert_eq!((n, exact, near), (230, 225, 5));
    }
}
