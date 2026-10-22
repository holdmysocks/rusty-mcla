//! Parsers for RAGE resource objects as laid out in MC:LA (Xbox 360).
//!
//! Everything inside a resource is big-endian. Pointers are tagged
//! (`0x5XXXXXXX` virtual, `0x6XXXXXXX` physical) and resolved through
//! [`crate::rsc::Resource::ptr`]. The first dword of every object is a
//! little-endian PC vtable address that we use as a class identifier.

pub mod geometry;
pub mod navsys;
pub mod navmesh;
pub mod scan;
pub mod sector;
pub mod skeleton;
pub mod texture;
pub mod xenos;
pub mod drawable;
pub mod vehtune;
pub mod simtune;
pub mod aitune;
pub mod ui;
pub mod swf;
pub mod props;
pub mod fragphys;
pub mod lights;
pub mod parking;
pub mod bound;
pub mod character;
pub mod blendshape;
pub mod anim;
pub mod ptfx;
pub mod aomap;

use crate::rsc::Resource;
use crate::{bail, Result};

/// Thin typed reader over a resource.
#[derive(Clone, Copy)]
pub struct Rd<'a> {
    pub res: &'a Resource,
}

impl<'a> Rd<'a> {
    pub fn new(res: &'a Resource) -> Self {
        Rd { res }
    }
    #[inline]
    pub fn data(&self) -> &'a [u8] {
        &self.res.data
    }
    #[inline]
    pub fn has(&self, off: usize, len: usize) -> bool {
        off.checked_add(len).map_or(false, |e| e <= self.res.data.len())
    }
    #[inline]
    pub fn u8(&self, off: usize) -> u8 {
        self.res.data.get(off).copied().unwrap_or(0)
    }
    #[inline]
    pub fn u16(&self, off: usize) -> u16 {
        if self.has(off, 2) {
            crate::bin::u16be(&self.res.data, off)
        } else {
            0
        }
    }
    #[inline]
    pub fn u32(&self, off: usize) -> u32 {
        if self.has(off, 4) {
            crate::bin::u32be(&self.res.data, off)
        } else {
            0
        }
    }
    #[inline]
    pub fn u32le(&self, off: usize) -> u32 {
        if self.has(off, 4) {
            crate::bin::u32le(&self.res.data, off)
        } else {
            0
        }
    }
    #[inline]
    pub fn f32(&self, off: usize) -> f32 {
        f32::from_bits(self.u32(off))
    }
    /// Read a tagged pointer at `off` and resolve it to an offset.
    #[inline]
    pub fn ptr(&self, off: usize) -> Option<usize> {
        self.res.ptr(self.u32(off))
    }
    /// Class id (LE vtable) of the object at `off`.
    #[inline]
    pub fn vtable(&self, off: usize) -> u32 {
        self.u32le(off)
    }
    pub fn cstr(&self, off: usize) -> String {
        let d = &self.res.data;
        if off >= d.len() {
            return String::new();
        }
        let end = d[off..].iter().position(|&b| b == 0).map_or(d.len(), |p| off + p);
        String::from_utf8_lossy(&d[off..end]).into_owned()
    }
    pub fn cstr_ptr(&self, off: usize) -> Option<String> {
        self.ptr(off).map(|o| self.cstr(o))
    }
    pub fn slice(&self, off: usize, len: usize) -> Result<&'a [u8]> {
        if !self.has(off, len) {
            bail!("slice {:#x}+{:#x} outside resource ({:#x})", off, len, self.res.data.len());
        }
        Ok(&self.res.data[off..off + len])
    }
}

/// `rage::pgDictionary<T>`: sorted hash keys + parallel value pointers.
#[derive(Debug, Clone, Default)]
pub struct Dictionary {
    pub hashes: Vec<u32>,
    /// offsets of the value objects (None for null pointers)
    pub values: Vec<Option<usize>>,
}

impl Dictionary {
    pub fn parse(rd: &Rd, off: usize) -> Result<Dictionary> {
        let hp = rd.ptr(off + 0x10);
        let count = rd.u16(off + 0x14) as usize;
        let vp = rd.ptr(off + 0x18);
        let vcount = rd.u16(off + 0x1c) as usize;
        let n = count.min(vcount);
        let (Some(hp), Some(vp)) = (hp, vp) else {
            return Ok(Dictionary::default());
        };
        if !rd.has(hp, n * 4) || !rd.has(vp, n * 4) {
            bail!("dictionary arrays out of range");
        }
        let hashes = (0..n).map(|i| rd.u32(hp + i * 4)).collect();
        let values = (0..n).map(|i| rd.ptr(vp + i * 4)).collect();
        Ok(Dictionary { hashes, values })
    }

    pub fn get(&self, hash: u32) -> Option<usize> {
        match self.hashes.binary_search(&hash) {
            Ok(i) => self.values[i],
            Err(_) => self.hashes.iter().position(|&h| h == hash).and_then(|i| self.values[i]),
        }
    }

    pub fn len(&self) -> usize {
        self.hashes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }
}

/// Known class ids (little-endian vtable addresses in the PC build that produced the data).
pub mod vt {
    pub const TEXTURE_DICT: u32 = 0x005950ec;
    pub const CITY_SECTOR: u32 = 0x005cb710;
    pub const SHADER: u32 = 0x005a52fc;
    pub const GEOMETRY: u32 = 0x005a716c;
    pub const TEXTURE_REF: u32 = 0x005ad314;
    pub const VERTEX_BUFFER: u32 = 0x005b45d8;
    pub const INDEX_BUFFER: u32 = 0x005b41e0;
    pub const CITY_TEXTURE: u32 = 0x005a3ccc;
}
