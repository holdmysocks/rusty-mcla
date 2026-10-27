//! The evolution blend against the original's own generated code run offline (tools/ptfx_evolve_reference.py, recomp_dispatch; recorded
//! cases in testdata/ptfx_evolve_ref.txt): each case is the original's object tables (a memory image whose pointer words are listed by
//! offset), a query and the value the dispatcher leaves. The port parses the same tables with `EvoGroup::parse` and blends with `evolve`.
//! Evidence: docs/reference/formats/verify_dispatch_evidence.md.
use super::EvoGroup;
use crate::rage::Rd;
use crate::rsc::Resource;

fn bits(s: &str) -> u32 {
    u32::from_str_radix(s, 16).unwrap()
}

fn run(mode_filter: impl Fn(&EvoGroup, u8, u8) -> bool) -> (usize, Vec<String>) {
    let (mut n, mut bad) = (0, Vec::new());
    for line in include_str!("testdata/ptfx_evolve_ref.txt").lines() {
        let t: Vec<&str> = line.split(' ').collect();
        let hex = t[2];
        let mut data: Vec<u8> = (0..hex.len() / 2).map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap()).collect();
        for p in t[3].split(',').filter(|s| !s.is_empty()) {
            let off: usize = p.parse().unwrap();
            let v = u32::from_be_bytes(data[off..off + 4].try_into().unwrap());
            data[off..off + 4].copy_from_slice(&(0x5000_0000 + (v - 0x1000)).to_be_bytes());
        }
        let (g, list, prop): (usize, u8, u8) = (t[4].parse().unwrap(), t[5].parse().unwrap(), t[6].parse().unwrap());
        let time = f32::from_bits(bits(t[7]));
        let w: Vec<f32> = t[8..16].iter().map(|s| f32::from_bits(bits(s))).collect();
        let base: [f32; 4] = std::array::from_fn(|i| f32::from_bits(bits(t[16 + i])));
        let want: Vec<u32> = t[21..25].iter().map(|s| bits(s)).collect();
        let res = Resource { rtype: 0, flags: 0, virtual_size: data.len(), physical_size: 0, data };
        let rd = Rd::new(&res);
        let grp = EvoGroup::parse(&rd, g);
        if !mode_filter(&grp, list, prop) {
            continue;
        }
        n += 1;
        let got = grp.evolve(list, prop, base, time, &w);
        let got: Vec<u32> = got.iter().map(|x| x.to_bits()).collect();
        if got != want {
            bad.push(format!("case {n}: list {list} prop {prop} mode {:?}: port {got:08x?} original {want:08x?}", grp.props.iter().find(|p| p.list == list && p.prop == prop).map(|p| p.mode)));
        }
    }
    (n, bad)
}

fn mode_of(g: &EvoGroup, list: u8, prop: u8) -> Option<u8> {
    g.props.iter().find(|p| p.list == list && p.prop == prop).map(|p| p.mode)
}

#[test]
fn evolution_modes_match_the_original() {
    let mut failed = Vec::new();
    for want in [None, Some(0), Some(1), Some(2), Some(3), Some(4)] {
        let (n, bad) = run(|g, l, p| mode_of(g, l, p) == want);
        eprintln!("mode {want:?}: {n} cases, {} differ", bad.len());
        for b in bad.iter().take(3) {
            eprintln!("  {b}");
        }
        assert!(n > 0, "mode {want:?} has no case");
        if !bad.is_empty() {
            failed.push(want);
        }
    }
    assert!(failed.is_empty(), "modes that differ: {failed:?}");
}
