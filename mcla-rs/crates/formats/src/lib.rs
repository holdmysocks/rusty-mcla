//! File-format support for Midnight Club: Los Angeles (Xbox 360, RAGE engine).
//!
//! * [`iso`]  – XGD2 disc images (ISO9660 decoy + XDVDFS game partition)
//! * [`rpf3`] – RAGE Package Format v3 archives (AES-encrypted TOC, hashed names)
//! * [`xmem`] – Xbox 360 XMemCompress (LZX) streams used by RSC resources
//! * [`hash`] – Jenkins one-at-a-time string hashing used for file names
//! * [`names`] – hash → name dictionary

pub mod audcfg;
pub mod avm1;
pub mod crypto;
pub mod dds;
pub mod guest_math;
pub mod hash;
#[cfg(test)]
mod lookup_tests;
pub mod iso;
pub mod jpeg;
pub mod keys;
pub mod names;
pub mod rage;
pub mod rpf3;
pub mod rsc;
pub mod sco;
pub mod strtbl;
pub mod xex;
pub mod xmem;

pub use anyhow::{anyhow, bail, Context, Result};

/// A read-only byte source that can be sliced at arbitrary offsets.
pub trait ReadAt: Send + Sync {
    fn len(&self) -> u64;
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<()>;
    fn read_vec(&self, off: u64, len: usize) -> Result<Vec<u8>> {
        let mut v = vec![0u8; len];
        self.read_at(off, &mut v)?;
        Ok(v)
    }
}

impl ReadAt for memmap2::Mmap {
    fn len(&self) -> u64 {
        (**self).len() as u64
    }
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<()> {
        let s = &self[..];
        let start = off as usize;
        let end = start
            .checked_add(buf.len())
            .ok_or_else(|| anyhow!("overflow"))?;
        if end > s.len() {
            bail!(
                "read past end of mapping: {:#x}+{:#x} > {:#x}",
                off,
                buf.len(),
                s.len()
            );
        }
        buf.copy_from_slice(&s[start..end]);
        Ok(())
    }
}

/// A window into another [`ReadAt`].
pub struct SubRange<R: ReadAt> {
    pub inner: std::sync::Arc<R>,
    pub base: u64,
    pub len: u64,
}

impl<R: ReadAt> ReadAt for SubRange<R> {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<()> {
        if off + buf.len() as u64 > self.len {
            bail!("read past end of sub-range");
        }
        self.inner.read_at(self.base + off, buf)
    }
}

/// Little helpers for binary parsing.
pub mod bin {
    #[inline]
    pub fn u16le(b: &[u8], o: usize) -> u16 {
        u16::from_le_bytes([b[o], b[o + 1]])
    }
    #[inline]
    pub fn u32le(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
    }
    #[inline]
    pub fn u16be(b: &[u8], o: usize) -> u16 {
        u16::from_be_bytes([b[o], b[o + 1]])
    }
    #[inline]
    pub fn u32be(b: &[u8], o: usize) -> u32 {
        u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
    }
    #[inline]
    pub fn f32be(b: &[u8], o: usize) -> f32 {
        f32::from_bits(u32be(b, o))
    }
    #[inline]
    pub fn u64be(b: &[u8], o: usize) -> u64 {
        u64::from_be_bytes(b[o..o + 8].try_into().unwrap())
    }
    #[inline]
    pub fn i16be(b: &[u8], o: usize) -> i16 {
        u16be(b, o) as i16
    }
}
