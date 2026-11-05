//! The hydraulics against the original's code run offline (tools/hydraulics_reference.py: the wheel button handler
//! F-39230, the switch-on F-71695, the switch-off F-28175, the update F-93185 with the order step F-21741 and the ram
//! mover F-73777): 100 sequences of 300 frames of random tunes, offsets, steps, switch flags and wheel presses and
//! releases, every field after every frame.
//!
//! `HYD_REF_STATS=1` makes the test count instead of stopping at the first difference: the free-running count (the
//! first frame of each sequence that differs, by field set) and a per-frame count where the port's state is set to the
//! original's before every frame, so one difference does not hide the next ones.

use super::car::{HYD_FITTED, HYD_ON};
use super::vehsim::Hydraulics;
use mcla_formats::rage::simtune::HydraulicsTune;
use std::collections::BTreeMap;

fn h(s: &str) -> u32 {
    u32::from_str_radix(s, 16).unwrap()
}

/// One frame of the reference: what the original's object held after the frame
#[derive(Clone, Copy, Default)]
struct Ref {
    on: bool,
    asked: [u8; 4],
    queued: [u8; 4],
    order: [i8; 4],
    moving: [bool; 4],
    latched: [bool; 4],
    press: [u32; 4],
    settle: [u32; 4],
    height: [u32; 4],
}

fn digits(s: &str) -> [u8; 4] {
    let b = s.as_bytes();
    [b[0] - b'0', b[1] - b'0', b[2] - b'0', b[3] - b'0']
}

fn hex4(s: &str) -> [u32; 4] {
    let v: Vec<u32> = s.split(',').map(h).collect();
    [v[0], v[1], v[2], v[3]]
}

fn parse_ref(w: &[&str]) -> Ref {
    // w: on asked queued order moving latched press settle height
    let order = w[3].bytes().map(|c| match c { b'u' => 1, b'd' => -1, _ => 0 }).collect::<Vec<i8>>();
    Ref {
        on: w[0] == "1",
        asked: digits(w[1]),
        queued: digits(w[2]),
        order: [order[0], order[1], order[2], order[3]],
        moving: digits(w[4]).map(|x| x != 0),
        latched: digits(w[5]).map(|x| x != 0),
        press: hex4(w[6]),
        settle: hex4(w[7]),
        height: hex4(w[8]),
    }
}

fn snap(p: &Hydraulics) -> Ref {
    Ref {
        on: p.on,
        asked: p.asked,
        queued: p.queued,
        order: p.order,
        moving: p.moving,
        latched: p.latched,
        press: p.press_time.map(f32::to_bits),
        settle: p.settle.map(f32::to_bits),
        height: p.height.map(f32::to_bits),
    }
}

/// The names of the fields that differ (with the wheel)
fn diff(a: &Ref, b: &Ref) -> Vec<String> {
    let mut d = Vec::new();
    if a.on != b.on {
        d.push("on".to_string());
    }
    for i in 0..4 {
        if a.asked[i] != b.asked[i] {
            d.push(format!("asked{i}"));
        }
        if a.queued[i] != b.queued[i] {
            d.push(format!("queued{i}"));
        }
        if a.order[i] != b.order[i] {
            d.push(format!("order{i}"));
        }
        if a.moving[i] != b.moving[i] {
            d.push(format!("moving{i}"));
        }
        if a.latched[i] != b.latched[i] {
            d.push(format!("latched{i}"));
        }
        if a.press[i] != b.press[i] {
            d.push(format!("press{i}"));
        }
        if a.settle[i] != b.settle[i] {
            d.push(format!("settle{i}"));
        }
        if a.height[i] != b.height[i] {
            d.push(format!("height{i}"));
        }
    }
    d
}

fn show(r: &Ref) -> String {
    format!(
        "on {} asked {:?} queued {:?} order {:?} moving {:?} latched {:?} press {:?} settle {:?} height {:?}",
        r.on,
        r.asked,
        r.queued,
        r.order,
        r.moving,
        r.latched,
        r.press.map(f32::from_bits),
        r.settle.map(f32::from_bits),
        r.height.map(f32::from_bits)
    )
}

/// the field name without its wheel digit
fn kind(f: &str) -> String {
    f.trim_end_matches(|c: char| c.is_ascii_digit()).to_string()
}

#[test]
fn hydraulics_match_the_original() {
    let text = include_str!("testdata/hydraulics_ref.txt");
    let stats = std::env::var("HYD_REF_STATS").is_ok();
    let mut tune = HydraulicsTune::default();
    let (mut port, mut held, mut prev, mut seq, mut skip, mut frame) = (Hydraulics::default(), 0u8, Ref::default(), 0, false, 0);
    // (the wheels' +432 as the reference writes them: the heights after the frame's switch, before its update; and the
    // two wheel tunes' SuspensionOffset)
    let (mut wheel, mut offset) = ([0.0f32; 4], [0.0f32; 2]);
    let mut prev_wheel = [0.0f32; 4];
    let (mut frames, mut skipped_seqs, mut skipped_frames) = (0, 0, 0);
    // free running: the first differing frame of each sequence, by field kinds
    let mut first: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut free_bad = 0;
    let mut free_done = false;
    // per frame (port set to the original's before the frame): by field kind
    let mut per: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for line in text.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        if w[0] == "q" {
            let v: Vec<f32> = w[1..8].iter().map(|s| f32::from_bits(h(s))).collect();
            tune = HydraulicsTune { latch_time: v[0], front_rise: v[1], front_drop: v[2], back_rise: v[3], back_drop: v[4], rate_rise: v[5], rate_drop: v[6], ..Default::default() };
            // the dance patterns (update +20) are not in the port
            skip = w[10] != "0";
            offset = [f32::from_bits(h(w[8])), f32::from_bits(h(w[9]))];
            wheel = [0.0; 4];
            prev_wheel = [0.0; 4];
            if skip {
                skipped_seqs += 1;
            }
            port = Hydraulics::default();
            held = 0;
            prev = Ref::default();
            seq += 1;
            frame = 0;
            free_done = false;
            continue;
        }
        if skip {
            skipped_frames += 1;
            continue;
        }
        // f <flag><e0..e3> <dt> -> on asked queued order moving latched press settle height
        let ev = w[1].as_bytes();
        let dt = f32::from_bits(h(w[2]));
        for i in 0..4 {
            match ev[1 + i] {
                b'1' => held |= 1 << i,
                b'2' => held &= !(1 << i),
                _ => {}
            }
        }
        let input = held | HYD_FITTED | if ev[0] == b'1' { HYD_ON } else { 0 };
        let want = parse_ref(&w[4..]);
        let (before, was_on) = (port.height, port.on);
        port.step(&tune, input, wheel, offset, dt);
        if !(port.on && !was_on) {
            wheel = before;
        }
        let got = snap(&port);
        let d = diff(&want, &got);
        if !d.is_empty() {
            if !stats {
                panic!(
                    "sequence {seq} frame {frame}: {} differ\n  input: flag {} events {} dt {dt} (latch {} rise {}/{} drop {}/{} rate {}/{})\n  original: {}\n  port:     {}\n  line: {line}",
                    d.join(", "),
                    ev[0] as char,
                    std::str::from_utf8(&ev[1..]).unwrap(),
                    tune.latch_time,
                    tune.front_rise,
                    tune.back_rise,
                    tune.front_drop,
                    tune.back_drop,
                    tune.rate_rise,
                    tune.rate_drop,
                    show(&want),
                    show(&got)
                );
            }
            if !free_done {
                free_done = true;
                free_bad += 1;
                let mut ks: Vec<String> = d.iter().map(|f| kind(f)).collect();
                ks.dedup();
                first.entry(ks.join("+")).or_insert((0, format!("sequence {seq} frame {frame}: {line}\n    original: {}\n    port:     {}", show(&want), show(&got)))).0 += 1;
            }
        }
        if stats {
            // the same frame from the original's previous state
            let mut p = Hydraulics {
                on: prev.on,
                asked: prev.asked,
                queued: prev.queued,
                order: prev.order,
                height: prev.height.map(f32::from_bits),
                press_time: prev.press.map(f32::from_bits),
                settle: prev.settle.map(f32::from_bits),
                moving: prev.moving,
                latched: prev.latched,
                buttons: held_before(held, ev),
                ..Default::default()
            };
            // (the wheels' +432 the reference wrote last frame: the original's heights before that frame's update, or the
            // previous frame's when it switched on)
            p.step(&tune, input, prev_wheel, offset, dt);
            let d = diff(&want, &snap(&p));
            let mut ks: Vec<String> = d.iter().map(|f| kind(f)).collect();
            ks.dedup();
            if !ks.is_empty() {
                let e = per.entry(ks.join("+")).or_insert((0, format!("sequence {seq} frame {frame}: {line}\n    before:   {}\n    original: {}\n    port:     {}", show(&prev), show(&want), show(&snap(&p)))));
                e.0 += 1;
            }
        }
        if !(want.on && !prev.on) {
            prev_wheel = prev.height.map(f32::from_bits);
        }
        prev = want;
        frames += 1;
        frame += 1;
    }
    if stats {
        println!("{frames} frames compared, {skipped_seqs} sequences ({skipped_frames} frames) with a dance pattern skipped");
        println!("\nFREE RUNNING: {free_bad} of {} sequences differ; first differing frame by field set:", seq - skipped_seqs);
        for (k, (n, ex)) in &first {
            println!("  [{n}] {k}\n    e.g. {ex}");
        }
        println!("\nPER FRAME (port state set to the original's before each frame): differences by field set:");
        for (k, (n, ex)) in &per {
            println!("  [{n}] {k}\n    e.g. {ex}");
        }
    } else {
        assert_eq!(frames, 27_000);
    }
}

/// the pressed-buttons mask the port held before this frame's events
fn held_before(held_now: u8, ev: &[u8]) -> u8 {
    let mut m = held_now;
    for i in 0..4 {
        match ev[1 + i] {
            b'1' => m &= !(1 << i),
            b'2' => m |= 1 << i,
            _ => {}
        }
    }
    m
}
