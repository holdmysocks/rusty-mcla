//! (tests only) What the offline comparisons of the thrown rider's articulated body share: the case files that
//! `tools/artic_reference.py` writes from the original's routines run unchanged, and the flat guest memory a case is
//! laid out in. The tests themselves are next to the routines they check (`artic.rs`, `artic_island.rs`,
//! `artic_build.rs`, `artic_run.rs`, each a `mod offline`); the evidence is in
//! docs/reference/vehicle/offline_leaf_checks.md.
//!
//! A case file line: `kind | registers | memory in | memory out | r3`; registers `code:words` (3 .. 10 the general registers,
//! 11 .. 13 the vector registers v1 .. v3 as lanes x y z w, 14 f1 as a float's bits), memory `addr:words`, all hex. The
//! guest memory is flat from 0x1000: the collider, the body, 4 links and 3 joints at the addresses below. Addresses at
//! 0x82000000 and up are image globals ([`Mem::word`] only).

use super::artic::{Articulated, V4};
use std::collections::HashMap;

pub const COLL: u32 = 0x1000;
pub const BODY: u32 = 0x1300;
pub const LINK0: u32 = 0x1510;
pub const JOINT0: u32 = 0x2500;
pub const LSZ: u32 = 1008;
pub const JSZ: u32 = 1408;
const BASE: u32 = 0x1000;
const SIZE: usize = 0x5000;

pub struct Mem {
    lo: Vec<u8>,
    hi: HashMap<u32, u32>,
}

impl Mem {
    pub fn new() -> Mem {
        Mem { lo: vec![0; SIZE], hi: HashMap::new() }
    }
    pub fn set(&mut self, a: u32, w: u32) {
        if a >= BASE && ((a - BASE) as usize) + 4 <= SIZE {
            let o = (a - BASE) as usize;
            self.lo[o..o + 4].copy_from_slice(&w.to_be_bytes());
        } else {
            self.hi.insert(a, w);
        }
    }
    pub fn word(&self, a: u32) -> u32 {
        if a >= BASE && ((a - BASE) as usize) + 4 <= SIZE {
            let o = (a - BASE) as usize;
            u32::from_be_bytes(self.lo[o..o + 4].try_into().unwrap())
        } else {
            *self.hi.get(&a).unwrap_or(&0)
        }
    }
    pub fn bytes(&self, a: u32, n: usize) -> &[u8] {
        let o = (a - BASE) as usize;
        &self.lo[o..o + n]
    }
    pub fn vec(&self, a: u32) -> V4 {
        V4::from_bits([0, 1, 2, 3].map(|i| self.word(a + 4 * i)))
    }
    /// The body of the layout above as the port reads it: `n` links, `nj` joints (hinge or swing-twist blocks).
    pub fn articulated(&self, n: usize, nj: usize) -> Articulated {
        let links: Vec<(u32, &[u8])> = (0..n as u32).map(|i| (LINK0 + LSZ * i, self.bytes(LINK0 + LSZ * i, LSZ as usize))).collect();
        let joints: Vec<&[u8]> = (0..nj as u32).map(|j| self.bytes(JOINT0 + JSZ * j, JSZ as usize)).collect();
        Articulated::from_guest(self.bytes(COLL, 752), self.bytes(BODY, 528), &links, &joints)
    }
}

pub struct Rec {
    pub kind: String,
    pub regs: Vec<(u32, Vec<u32>)>,
    pub mem: Mem,
    pub outs: Vec<(u32, Vec<u32>)>,
    pub ret: u32,
}

impl Rec {
    pub fn reg(&self, code: u32) -> &[u32] {
        &self.regs.iter().find(|r| r.0 == code).unwrap_or_else(|| panic!("no register {code}")).1
    }
    pub fn r(&self, code: u32) -> u32 {
        self.reg(code)[0]
    }
    pub fn v(&self, code: u32) -> V4 {
        V4::from_bits(self.reg(code).try_into().unwrap())
    }
    /// The original's words at `addr` (inside one of the output runs).
    pub fn want(&self, addr: u32, n: usize) -> Vec<u32> {
        for (a, ws) in &self.outs {
            if addr >= *a && ((addr - a) / 4) as usize + n <= ws.len() {
                let i = ((addr - a) / 4) as usize;
                return ws[i..i + n].to_vec();
            }
        }
        panic!("no output at {addr:x}");
    }
}

fn words(s: &str) -> Vec<u32> {
    if s.is_empty() {
        return Vec::new();
    }
    // (a word that would read as an address of the original is written with an h in front, tools/cite_safe.py)
    s.split(',').map(|w| u32::from_str_radix(w.trim_start_matches('h'), 16).unwrap()).collect()
}

fn runs(s: &str) -> Vec<(u32, Vec<u32>)> {
    s.split(';').filter(|x| !x.is_empty()).map(|x| {
        let (a, w) = x.split_once(':').unwrap();
        (u32::from_str_radix(a.trim_start_matches('h'), 16).unwrap(), words(w))
    }).collect()
}

/// The cases of `kind` in a case file.
pub fn records(text: &'static str, kind: &'static str) -> impl Iterator<Item = Rec> {
    text.lines().filter(move |l| l.split('|').next() == Some(kind)).map(|l| {
        let f: Vec<&str> = l.split('|').collect();
        let regs = f[1].split(';').filter(|x| !x.is_empty()).map(|x| {
            let (c, w) = x.split_once(':').unwrap();
            (c.parse().unwrap(), words(w))
        }).collect();
        let mut mem = Mem::new();
        for (a, ws) in runs(f[2]) {
            for (i, w) in ws.iter().enumerate() {
                mem.set(a + 4 * i as u32, *w);
            }
        }
        Rec { kind: kind.to_string(), regs, mem, outs: runs(f[3]), ret: u32::from_str_radix(f[4].trim().trim_start_matches('h'), 16).unwrap() }
    })
}

/// Words that differ, with where: a NaN against a NaN is the same (the harness's x86 default NaN is not the console's), a
/// flag or index word (`float` false) must be equal.
pub fn diff(name: &str, got: &[u32], want: &[u32], float: bool, bad: &mut Vec<String>) {
    assert_eq!(got.len(), want.len(), "{name}: length");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        let nan = |x: u32| float && (x & 0x7F80_0000) == 0x7F80_0000 && (x & 0x007F_FFFF) != 0;
        if g != w && !(nan(*g) && nan(*w)) {
            bad.push(format!("{name}[{i}]: port {g:08x} original {w:08x}"));
        }
    }
}

pub fn words_of(b: &[u8]) -> Vec<u32> {
    b.chunks(4).map(|c| u32::from_be_bytes(c.try_into().unwrap())).collect()
}

pub fn report(what: &str, n: usize, bad: &[String]) {
    assert!(bad.is_empty(), "{what}: {} of {n} cases differ, first: {:?}", bad.len(), &bad[..bad.len().min(8)]);
    println!("{what}: {n} cases, all equal");
}
