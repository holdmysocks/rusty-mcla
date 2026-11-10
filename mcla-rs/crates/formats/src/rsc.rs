//! RSC (RAGE resource) container, version 5 as shipped in MC:LA.
//!
//! ```text
//! u32le 0x52534305 ("\x05CSR" == "RSC\x05" little-endian)
//! u32be type
//! u32be flags      virtual size = (f & 0x7FF) << (((f >> 11) & 0xF) + 8); rest = physical
//! XMemCompress stream (see xmem.rs)
//! ```
//! Decompressed data is the virtual (CPU) block followed by the physical (GPU) block.
//! Pointers inside resources are 0x5XXXXXXX (virtual) / 0x6XXXXXXX (physical).

use crate::bin::{u32be, u32le};
use crate::{bail, Result};

pub const RSC_MAGIC: u32 = 0x5253_4305;

#[derive(Debug, Clone)]
pub struct Resource {
    pub rtype: u32,
    pub flags: u32,
    pub virtual_size: usize,
    pub physical_size: usize,
    /// virtual block followed by physical block
    pub data: Vec<u8>,
}

pub fn virtual_size_from_flags(f: u32) -> usize {
    ((f & 0x7FF) as usize) << (((f >> 11) & 0xF) + 8)
}

/// The original's resource-info check (`F-46450`) on the first bytes of a file: it reads 12 bytes and wants all 12, the
/// magic in the first word; then the type is the second word and the flags the third (both big-endian).
pub fn header(head: &[u8]) -> Option<(u32, u32)> {
    if head.len() < 12 || u32le(head, 0) != RSC_MAGIC {
        return None;
    }
    Some((u32be(head, 4), u32be(head, 8)))
}

impl Resource {
    pub fn parse(blob: &[u8]) -> Result<Resource> {
        // (the stream after the header must also have bytes: 20 in all)
        let Some((rtype, flags)) = header(blob).filter(|_| blob.len() >= 20) else { bail!("not an RSC5 resource") };
        let data = crate::xmem::decompress(&blob[12..])?;
        let vs = virtual_size_from_flags(flags).min(data.len());
        Ok(Resource { rtype, flags, virtual_size: vs, physical_size: data.len() - vs, data })
    }

    #[inline]
    pub fn virt(&self) -> &[u8] {
        &self.data[..self.virtual_size]
    }
    #[inline]
    pub fn phys(&self) -> &[u8] {
        &self.data[self.virtual_size..]
    }

    /// Translate a tagged pointer to an offset into `data`. Returns None for null
    /// or out-of-range pointers.
    #[inline]
    pub fn ptr(&self, p: u32) -> Option<usize> {
        if p == 0 {
            return None;
        }
        let off = (p & 0x0FFF_FFFF) as usize;
        match p >> 28 {
            5 => (off < self.virtual_size).then_some(off),
            6 => {
                let o = self.virtual_size + off;
                (o < self.data.len()).then_some(o)
            }
            _ => None,
        }
    }
}
