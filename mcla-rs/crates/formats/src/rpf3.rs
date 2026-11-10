//! RAGE Package Format v3 as used by Midnight Club: Los Angeles (Xbox 360).
//!
//! ```text
//! header (0x800 bytes, little-endian):
//!   u32 "RPF3"  u32 toc_size  u32 entry_count  u32 ?  u32 encrypted(0xFFFFFFFF)
//! toc @0x800 (AES-256-ECB x16 when encrypted), entry_count * 16 bytes:
//!   directory:  u32 name_hash   u32 0          u32 0x80000000|first_child   u32 child_count
//!   file:       u32 name_hash   u32 size       u32 offset                   u32 flags
//!       flags bit30 set  => raw-deflate compressed, (flags & 0x3FFFFFFF) = compressed size
//!       flags bit31 set  => RSC resource: data at (offset & ~0xFFF), (offset & 0xFFF) = type,
//!                           flags = resource memory flags
//! names block follows the entries (only "/" for the root; everything else is hashed).
//! ```

use crate::bin::u32le;
use crate::crypto::decrypt_toc;
use crate::{bail, Context, ReadAt, Result};
use std::io::Read;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dir { first: u32, count: u32 },
    File { size: u32, offset: u32, compressed: Option<u32> },
    Resource { size: u32, offset: u32, rtype: u32, flags: u32 },
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub hash: u32,
    pub kind: Kind,
    pub parent: u32,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        matches!(self.kind, Kind::Dir { .. })
    }
}

/// What an entry's words 1..3 (`size`, `c`, `d`) make it. A resource is any entry whose `d` has bit 31; the original's
/// resource-info routine (`F-30036`) then hands out `d` as the flags and the low byte of `c` as the type (`Kind::Resource`
/// keeps 12 bits of type: they differ only if bits 8..11 of `c` are set, which no entry of the disc's archives has).
pub fn classify(b: u32, c: u32, d: u32, count: usize) -> Kind {
    if c & 0x8000_0000 != 0 && d & 0xC000_0000 == 0 && ((c & 0x7FFF_FFFF) as usize) <= count {
        Kind::Dir { first: c & 0x7FFF_FFFF, count: d }
    } else if d & 0x8000_0000 != 0 {
        Kind::Resource { size: b, offset: c & !0xFFF, rtype: c & 0xFFF, flags: d }
    } else if d & 0x4000_0000 != 0 {
        Kind::File { size: b, offset: c, compressed: Some(d & 0x3FFF_FFFF) }
    } else {
        Kind::File { size: b, offset: c, compressed: None }
    }
}

pub struct Rpf3<R: ReadAt> {
    pub src: Arc<R>,
    pub entries: Vec<Entry>,
    /// The key the table was opened with (for script payloads, see [`crate::sco::to_plain`]).
    pub key: [u8; 32],
}

impl<R: ReadAt> Rpf3<R> {
    /// `key` comes from [`crate::keys::derive`].
    pub fn open(src: Arc<R>, key: &[u8; 32]) -> Result<Self> {
        let hdr = src.read_vec(0, 0x20).context("rpf header")?;
        if &hdr[0..4] != b"RPF3" {
            bail!("not an RPF3 archive");
        }
        let toc_size = u32le(&hdr, 4) as usize;
        let count = u32le(&hdr, 8) as usize;
        let encrypted = u32le(&hdr, 0x10) == 0xFFFF_FFFF;
        let mut toc = src.read_vec(0x800, toc_size).context("rpf toc")?;
        if encrypted {
            decrypt_toc(&mut toc, key);
        }
        if count * 16 > toc.len() {
            bail!("toc too small for {} entries", count);
        }
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let e = &toc[i * 16..i * 16 + 16];
            let a = u32le(e, 0);
            let b = u32le(e, 4);
            let c = u32le(e, 8);
            let d = u32le(e, 12);
            // Directories: bit31 of the third word set, fourth word is a small child count.
            // (The root entry carries 1 in its second word, so do not require b == 0.)
            entries.push(Entry { hash: a, kind: classify(b, c, d, count), parent: u32::MAX });
        }
        // parent links
        for i in 0..entries.len() {
            if let Kind::Dir { first, count } = entries[i].kind {
                for j in first..first + count {
                    if let Some(ch) = entries.get_mut(j as usize) {
                        ch.parent = i as u32;
                    }
                }
            }
        }
        Ok(Rpf3 { src, entries, key: *key })
    }

    /// Raw bytes of a file entry (decompressed if needed). Resources are returned
    /// as the complete on-disk blob (RSC header + XMem stream); see [`crate::rsc`].
    pub fn read(&self, idx: usize) -> Result<Vec<u8>> {
        let e = &self.entries[idx];
        match e.kind {
            Kind::Dir { .. } => bail!("entry {} is a directory", idx),
            Kind::File { size, offset, compressed: None } => self.src.read_vec(offset as u64, size as usize),
            Kind::File { size, offset, compressed: Some(csize) } => {
                let raw = self.src.read_vec(offset as u64, csize as usize)?;
                let mut out = Vec::with_capacity(size as usize);
                flate2::read::DeflateDecoder::new(&raw[..]).read_to_end(&mut out)?;
                if out.len() != size as usize {
                    log::warn!("entry {:08x}: inflated {} != {}", e.hash, out.len(), size);
                }
                Ok(out)
            }
            Kind::Resource { size, offset, .. } => self.src.read_vec(offset as u64, size as usize),
        }
    }

    /// Full path of an entry, using `names` for known hashes and `0x%08x` otherwise.
    pub fn path_of(&self, idx: usize, names: &crate::names::Names) -> String {
        let mut parts = Vec::new();
        let mut i = idx;
        while i != 0 {
            let e = &self.entries[i];
            parts.push(names.get(e.hash));
            if e.parent == u32::MAX || e.parent as usize >= self.entries.len() {
                break;
            }
            i = e.parent as usize;
        }
        parts.reverse();
        parts.join("/")
    }

    pub fn children(&self, dir: usize) -> std::ops::Range<usize> {
        match self.entries[dir].kind {
            Kind::Dir { first, count } => first as usize..(first + count) as usize,
            _ => 0..0,
        }
    }

    /// Look up a child of `dir` by name hash.
    pub fn child(&self, dir: usize, hash: u32) -> Option<usize> {
        self.children(dir).find(|&i| self.entries[i].hash == hash)
    }

    /// Resolve a slash-separated path (case-insensitive) to an entry index, as the original's find (`F-82183`) does: one
    /// leading separator is skipped; each component (up to `/`, `\` or the end) is hashed (`F-58929`, seed 0) and binary
    /// searched among the current directory's children by hash (so an empty component, as in `a//b`, is a name of
    /// hash 0); after a match all following separators are skipped; an entry that is not a directory ends the walk (it is
    /// the answer even if components remain); a path that ends at a directory answers that directory.
    pub fn lookup(&self, path: &str) -> Option<usize> {
        let b = path.as_bytes();
        let b = &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())];
        let sep = |c: u8| c == b'/' || c == b'\\';
        let mut p = b.first().map_or(0, |&c| sep(c) as usize);
        let mut cur = 0usize;
        if p == b.len() {
            return Some(0);
        }
        loop {
            let end = b[p..].iter().position(|&c| sep(c)).map_or(b.len(), |i| p + i);
            let hash = crate::hash::at_string_hash(&b[p..end], 0);
            p = end;
            let Some(Entry { kind: Kind::Dir { first, count }, .. }) = self.entries.get(cur) else { return None };
            let (mut lo, mut hi) = (*first, first.wrapping_add(*count).wrapping_sub(1));
            let found = loop {
                if lo > hi {
                    return None;
                }
                let mid = lo.wrapping_add(hi) >> 1;
                let h = self.entries.get(mid as usize)?.hash;
                if hash < h {
                    hi = mid.wrapping_sub(1);
                } else if hash > h {
                    lo = mid.wrapping_add(1);
                } else {
                    break mid as usize;
                }
            };
            while p < b.len() && sep(b[p]) {
                p += 1;
            }
            cur = found;
            if !self.entries[found].is_dir() || p == b.len() {
                return Some(found);
            }
        }
    }
}
