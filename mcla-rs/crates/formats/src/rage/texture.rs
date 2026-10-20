//! `grcTexture` (Xbox 360 flavour) → untiled, byte-order-fixed texel data.

use super::xenos::{self, TexFormat};
use super::Rd;
use crate::{bail, Result};

#[derive(Debug, Clone)]
pub struct Texture {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub format: TexFormat,
    /// Mip levels, tightly packed, little-endian block/texel order (PC conventions).
    pub levels: Vec<Vec<u8>>,
    pub fetch: [u32; 6],
}

impl Texture {
    /// Parse the texture object at `off`.
    pub fn parse(rd: &Rd, off: usize) -> Result<Texture> {
        // Two layouts exist: {+14 name, +18 d3d, +1c w/h} and {+18 name, +1c d3d, +20 w/h}.
        // Detect the D3D header by its BaseFlush/MipFlush words (0xFFFF0000).
        let looks_d3d = |o: usize| rd.u32(o + 0x14) == 0xFFFF_0000 || rd.u32(o + 0x18) == 0xFFFF_0000;
        let mut d3d = None;
        let mut name = String::new();
        for f in [0x14usize, 0x18, 0x1c, 0x20] {
            if let Some(p) = rd.ptr(off + f) {
                if d3d.is_none() && looks_d3d(p) {
                    d3d = Some(p);
                } else if name.is_empty() {
                    let s = rd.cstr(p);
                    if !s.is_empty() && s.len() < 128 && s.bytes().all(|b| (0x20..0x7f).contains(&b)) {
                        name = s;
                    }
                }
            }
        }
        let d3d = d3d.ok_or_else(|| crate::anyhow!("texture without D3D header"))?;
        let mut fetch = [0u32; 6];
        for (i, f) in fetch.iter_mut().enumerate() {
            *f = rd.u32(d3d + 0x1c + i * 4);
        }
        Self::from_fetch(rd, name, fetch)
    }

    /// The textures of the resource whose names end in `suffix`, found by name alone (ones no dictionary lists, e.g.
    /// a car's `<car>_maxdamage` in its .r83): the texture objects whose name pointer points at such a string.
    pub fn find_by_suffix(rd: &Rd, suffix: &str) -> Vec<Texture> {
        let d = rd.data();
        let mut out = Vec::new();
        let mut want = suffix.as_bytes().to_vec();
        want.push(0);
        let mut at = 0;
        while let Some(k) = d[at..].windows(want.len()).position(|w| w == want.as_slice()) {
            at = at + k + 1;
            // (the name's first byte: back to the byte after a 0 / 0xCD filler)
            let mut s = at - 1;
            while s > 0 && d[s - 1] != 0 && d[s - 1] != 0xCD && at - 1 - s < 128 {
                s -= 1;
            }
            for h in (0..d.len().saturating_sub(4)).step_by(4) {
                if rd.ptr(h) != Some(s) {
                    continue;
                }
                for back in [0x14usize, 0x18] {
                    if let Some(o) = h.checked_sub(back) {
                        if let Ok(t) = Texture::parse(rd, o) {
                            if t.name.ends_with(suffix) {
                                out.push(t);
                                break;
                            }
                        }
                    }
                }
            }
        }
        out
    }

    pub fn from_fetch(rd: &Rd, name: String, fetch: [u32; 6]) -> Result<Texture> {
        let d0 = fetch[0];
        let d1 = fetch[1];
        let d2 = fetch[2];
        let d4 = fetch[4];
        let d5 = fetch[5];
        let tiled = d0 & 0x8000_0000 != 0;
        let pitch_texels = (((d0 >> 22) & 0x1ff) * 32) as usize;
        let format = TexFormat::from_xenos((d1 & 0x3f) as u8);
        let endian = ((d1 >> 6) & 3) as u8;
        let base = d1 & 0xFFFF_F000;
        let width = (d2 & 0x1fff) + 1;
        let height = ((d2 >> 13) & 0x1fff) + 1;
        let mip_min = ((d4 >> 2) & 0xf) as usize;
        let mip_max = ((d4 >> 6) & 0xf) as usize;
        let packed_mips = (d5 >> 11) & 1 != 0;
        let mip_address = d5 & 0xFFFF_F000;
        let Some(data_off) = rd.res.ptr(base) else {
            bail!("texture {:?}: bad base address {:#x}", name, base);
        };
        let (bw, bpb) = format.block();
        let mut levels = Vec::new();
        let nlevels = if mip_max >= mip_min { mip_max - mip_min + 1 } else { 1 };
        // Level 0 always; further levels only while they are >= 32 texels and
        // stored unpacked (packed mip tails are skipped: not needed for rendering quality).
        let mut level_off = data_off;
        let mut w = width as usize;
        let mut h = height as usize;
        let mut pitch = pitch_texels.max(w);
        // Small textures (a side of 16 texels or less) live inside the packed mip tail:
        // a 32-texel-aligned tiled surface with level 0 at an offset of 16 texels
        // (to the right, or downwards when the texture is wider than tall).
        if packed_mips && tiled && w.min(h) <= 16 {
            let sw = ((w + 31) & !31).max(32);
            let sh = ((h + 31) & !31).max(32);
            let (swb, shb) = (sw / bw, sh / bw);
            let need = swb * shb * bpb;
            if rd.has(data_off, need) {
                // (2026-10-15: the tiled order scatters a level's blocks over the whole 32 x 32 BLOCK tile - a 64 x 16
                // map's level 0, block rows 4 .. 7, lies at bytes 4096 .. 6016 of it, a 16 x 16 one's at 512 .. 1856 -
                // so the bytes handed over are the tile's, not width x height blocks' worth: with the short slice
                // l_sha_win_bs (64 x 16, the shop door frames' height / specular map) read all zero and l_sha_flat_bs
                // (16 x 16) half zero. Checked against the running game's copy. MCLA_TEX_SMALL_OLD=1: as before.)
                let tile = if std::env::var_os("MCLA_TEX_SMALL_OLD").is_some() { need } else { (((swb + 31) & !31) * ((shb + 31) & !31) * bpb).max(need) };
                let tile_end = (data_off + tile).min(rd.data().len());
                let surf = xenos::untile(&rd.data()[data_off..tile_end], swb, shb, swb, bpb);
                let (ox, oy) = if w > h { (0, 16 / bw) } else { (16 / bw, 0) };
                let wb = (w + bw - 1) / bw;
                let hb = (h + bw - 1) / bw;
                if ox + wb <= swb && oy + hb <= shb {
                    let mut lvl = vec![0u8; wb * hb * bpb];
                    for y in 0..hb {
                        let s0 = ((oy + y) * swb + ox) * bpb;
                        lvl[y * wb * bpb..(y + 1) * wb * bpb].copy_from_slice(&surf[s0..s0 + wb * bpb]);
                    }
                    xenos::unswap(&mut lvl, endian);
                    return Ok(Texture { name, width, height, format, levels: vec![lvl], fetch });
                }
            }
        }
        for level in 0..nlevels.max(1) {
            if level > 0 {
                if packed_mips && (w < 32 || h < 32) {
                    // The packed tail (2026-10-13, lighting / p2day): a level with a side of 16 texels or less and every
                    // level after it share ONE tiled surface of 32-texel-aligned size that follows the unpacked levels;
                    // the k-th packed level lies 16 >> k texels to the right in it (downwards when the texture is wider
                    // than tall) - the layout the small textures above have for their level 0. These levels were
                    // dropped ("not needed"): a 64 x 128 window texture (levels 0 .. 2 in the game's fetch constant)
                    // kept two, and a facade 260 m off - 16 texels to the pixel - showed its frame lines as bright
                    // specks where the original shows an even dim line (El Capitan at Selma / Highland,
                    // LIGHTING_WORK.md). Checked on four textures: each tail level's mean colour and opaque share
                    // equal the levels' above (work/lighting/pd_tail.py). MCLA_TEX_TAIL_OLD=1: dropped as before.
                    if tiled && std::env::var_os("MCLA_TEX_TAIL_OLD").is_none() {
                        let mut off = level_off;
                        if level == 1 && mip_address != 0 {
                            match rd.res.ptr(mip_address) {
                                Some(o) => off = o,
                                None => break,
                            }
                        }
                        let swb = ((w + 31) & !31).max(32) / bw;
                        let shb = ((h + 31) & !31).max(32) / bw;
                        let pt = (swb + 31) & !31;
                        let tile = pt * ((shb + 31) & !31) * bpb;
                        if off + tile <= rd.data().len() {
                            let surf = xenos::untile(&rd.data()[off..off + tile], swb, shb, pt, bpb);
                            for k in 0..3usize {
                                if level + k >= nlevels {
                                    break;
                                }
                                let (lw, lh) = ((w >> k).max(1), (h >> k).max(1));
                                let t = (16usize >> k) / bw;
                                let (ox, oy) = if width > height { (0, t) } else { (t, 0) };
                                let (wb, hb) = ((lw + bw - 1) / bw, (lh + bw - 1) / bw);
                                if t == 0 || ox + wb > swb || oy + hb > shb {
                                    break;
                                }
                                let mut lvl = vec![0u8; wb * hb * bpb];
                                for y in 0..hb {
                                    let s0 = ((oy + y) * swb + ox) * bpb;
                                    lvl[y * wb * bpb..(y + 1) * wb * bpb].copy_from_slice(&surf[s0..s0 + wb * bpb]);
                                }
                                xenos::unswap(&mut lvl, endian);
                                levels.push(lvl);
                            }
                        }
                    }
                    break;
                }
                if level == 1 {
                    // mip chain lives at mip_address when given, else follows level 0
                    if mip_address != 0 {
                        match rd.res.ptr(mip_address) {
                            Some(o) => level_off = o,
                            None => break,
                        }
                    }
                }
            }
            let wb = (w + bw - 1) / bw;
            let hb = (h + bw - 1) / bw;
            // Mip levels are stored with pitch and height aligned to 32 blocks and each
            // level padded to a 4 KiB page; level 0 uses the pitch from the fetch constant.
            let pb = if level == 0 || !tiled { ((pitch + bw - 1) / bw).max(wb) } else { (wb + 31) & !31 };
            let alloc_hb = if tiled { (hb + 31) & !31 } else { hb };
            let size = if level == 0 || !tiled { pb * alloc_hb * bpb } else { (pb * alloc_hb * bpb + 4095) & !4095 };
            if !rd.has(level_off, wb * hb * bpb) {
                if level == 0 {
                    bail!("texture {:?}: data out of range", name);
                }
                break;
            }
            let avail = (rd.data().len() - level_off).min(size);
            let src = &rd.data()[level_off..level_off + avail];
            let mut lvl = if tiled {
                xenos::untile(src, wb, hb, pb, bpb)
            } else {
                // linear with pitch
                let mut v = vec![0u8; wb * hb * bpb];
                for y in 0..hb {
                    let s = y * pb * bpb;
                    let e = (s + wb * bpb).min(src.len());
                    if s >= e {
                        break;
                    }
                    v[y * wb * bpb..y * wb * bpb + (e - s)].copy_from_slice(&src[s..e]);
                }
                v
            };
            xenos::unswap(&mut lvl, endian);
            levels.push(lvl);
            level_off += size;
            if w <= 1 && h <= 1 {
                break;
            }
            w = (w / 2).max(1);
            h = (h / 2).max(1);
            pitch = (pitch / 2).max(w);
            if level + 1 >= nlevels.max(1) {
                break;
            }
            if w < 32 && h < 32 && tiled {
                break;
            }
        }
        Ok(Texture { name, width, height, format, levels, fetch })
    }

    /// Mean colour of every stored level (debug aid: all levels should agree).
    pub fn level_means(&self) -> Vec<[u8; 3]> {
        let mut out = Vec::new();
        let (mut w, mut h) = (self.width as usize, self.height as usize);
        for l in &self.levels {
            let px = xenos::decode_to_rgba8(self.format, l, w, h);
            let n = (px.len() / 4).max(1) as u64;
            let mut s = [0u64; 3];
            for c in px.chunks_exact(4) {
                s[0] += c[0] as u64;
                s[1] += c[1] as u64;
                s[2] += c[2] as u64;
            }
            out.push([(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8]);
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
        out
    }

    /// Mean colour of the texture (weighted by alpha, so cut-outs give the colour of what
    /// shows), from a small mip level. Returns (rgb, mean alpha).
    pub fn mean_color(&self) -> ([u8; 3], u8) {
        let (mut w, mut h) = (self.width as usize, self.height as usize);
        let mut pick = 0;
        for k in 0..self.levels.len() {
            pick = k;
            if w.max(h) <= 32 {
                break;
            }
            if k + 1 < self.levels.len() {
                w = (w / 2).max(1);
                h = (h / 2).max(1);
            }
        }
        let px = xenos::decode_to_rgba8(self.format, &self.levels[pick], w, h);
        let (mut s, mut sa, mut n) = ([0u64; 3], 0u64, 0u64);
        for c in px.chunks_exact(4) {
            let a = c[3] as u64;
            s[0] += c[0] as u64 * a;
            s[1] += c[1] as u64 * a;
            s[2] += c[2] as u64 * a;
            sa += a;
            n += 1;
        }
        if sa == 0 {
            return ([128, 128, 128], 0);
        }
        ([(s[0] / sa) as u8, (s[1] / sa) as u8, (s[2] / sa) as u8], (sa / n.max(1)) as u8)
    }

    /// Software-decode level 0 to RGBA8 (debug/export).
    pub fn to_rgba8(&self) -> Vec<u8> {
        xenos::decode_to_rgba8(self.format, &self.levels[0], self.width as usize, self.height as usize)
    }
}
