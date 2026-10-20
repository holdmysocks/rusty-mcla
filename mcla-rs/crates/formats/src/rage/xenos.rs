//! Xbox 360 (Xenos) texture formats, tiling and endian handling.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TexFormat {
    Rgba8,      // k_8_8_8_8 (stored as ARGB words, swizzled)
    R8,         // k_8
    Rg8,        // k_8_8
    Rgb565,     // k_5_6_5
    Rgba4,      // k_4_4_4_4
    Rgb5a1,     // k_1_5_5_5
    Dxt1,       // BC1
    Dxt3,       // BC2
    Dxt5,       // BC3
    Dxn,        // BC5
    Dxt5a,      // BC4 (alpha block only)
    Ctx1,       // 2-channel normal compression, no PC equivalent
    Unknown(u8),
}

impl TexFormat {
    pub fn from_xenos(v: u8) -> TexFormat {
        match v {
            2 => TexFormat::R8,
            3 => TexFormat::Rgb5a1,
            4 => TexFormat::Rgb565,
            6 => TexFormat::Rgba8,
            10 => TexFormat::Rg8,
            15 => TexFormat::Rgba4,
            18 | 51 => TexFormat::Dxt1,
            19 | 52 => TexFormat::Dxt3,
            20 | 53 => TexFormat::Dxt5,
            49 => TexFormat::Dxn,
            59 => TexFormat::Dxt5a,
            60 => TexFormat::Ctx1,
            o => TexFormat::Unknown(o),
        }
    }
    /// (block width in texels, bytes per block)
    pub fn block(&self) -> (usize, usize) {
        match self {
            TexFormat::Rgba8 => (1, 4),
            TexFormat::R8 => (1, 1),
            TexFormat::Rg8 | TexFormat::Rgb565 | TexFormat::Rgba4 | TexFormat::Rgb5a1 => (1, 2),
            TexFormat::Dxt1 | TexFormat::Dxt5a | TexFormat::Ctx1 => (4, 8),
            TexFormat::Dxt3 | TexFormat::Dxt5 | TexFormat::Dxn => (4, 16),
            TexFormat::Unknown(_) => (1, 4),
        }
    }
    pub fn is_compressed(&self) -> bool {
        self.block().0 == 4
    }
}

/// Classic `XGAddress2DTiledOffset`. `x`,`y` in blocks, `width` in blocks (pitch),
/// `bpb` bytes per block. Returns the block index in the tiled image.
#[inline]
pub fn tiled_offset_2d(x: usize, y: usize, width: usize, bpb: usize) -> usize {
    let aligned_width = (width + 31) & !31;
    let log_bpp = (bpb >> 2) + ((bpb >> 1) >> (bpb >> 2));
    let macro_ = ((x >> 5) + (y >> 5) * (aligned_width >> 5)) << (log_bpp + 7);
    let micro = ((x & 7) + ((y & 6) << 2)) << log_bpp;
    let offset = macro_ + ((micro & !15) << 1) + (micro & 15) + ((y & 8) << (3 + log_bpp)) + ((y & 1) << 4);
    (((offset & !511) << 3)
        + ((offset & 448) << 2)
        + (offset & 63)
        + ((y & 16) << 7)
        + (((((y & 8) >> 2) + (x >> 3)) & 3) << 6))
        >> log_bpp
}

/// Untile a 2D level. `width_blocks`/`height_blocks` are the level's size in blocks,
/// `pitch_blocks` the row pitch used when tiling. Output is tightly packed rows.
pub fn untile(src: &[u8], width_blocks: usize, height_blocks: usize, pitch_blocks: usize, bpb: usize) -> Vec<u8> {
    let mut out = vec![0u8; width_blocks * height_blocks * bpb];
    for y in 0..height_blocks {
        for x in 0..width_blocks {
            let s = tiled_offset_2d(x, y, pitch_blocks, bpb) * bpb;
            let d = (y * width_blocks + x) * bpb;
            if s + bpb <= src.len() {
                out[d..d + bpb].copy_from_slice(&src[s..s + bpb]);
            }
        }
    }
    out
}

/// Undo the GPU endian mode: 1 = swap 16-bit, 2 = swap 32-bit, 3 = swap 16-in-32.
pub fn unswap(buf: &mut [u8], endian: u8) {
    match endian {
        1 => {
            for c in buf.chunks_exact_mut(2) {
                c.swap(0, 1);
            }
        }
        2 => {
            for c in buf.chunks_exact_mut(4) {
                c.swap(0, 3);
                c.swap(1, 2);
            }
        }
        3 => {
            for c in buf.chunks_exact_mut(4) {
                c.swap(0, 2);
                c.swap(1, 3);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Software decoders (debug export + formats the GPU cannot take directly)
// ---------------------------------------------------------------------------

fn rgb565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 31) as u32;
    let g = ((c >> 5) & 63) as u32;
    let b = (c & 31) as u32;
    [((r * 255 + 15) / 31) as u8, ((g * 255 + 31) / 63) as u8, ((b * 255 + 15) / 31) as u8]
}

fn decode_bc1_block(b: &[u8], out: &mut [[u8; 4]; 16], alpha: bool) {
    let c0 = u16::from_le_bytes([b[0], b[1]]);
    let c1 = u16::from_le_bytes([b[2], b[3]]);
    let p0 = rgb565(c0);
    let p1 = rgb565(c1);
    let mut pal = [[0u8; 4]; 4];
    pal[0] = [p0[0], p0[1], p0[2], 255];
    pal[1] = [p1[0], p1[1], p1[2], 255];
    if c0 > c1 || !alpha {
        for i in 0..3 {
            pal[2][i] = ((2 * p0[i] as u32 + p1[i] as u32) / 3) as u8;
            pal[3][i] = ((p0[i] as u32 + 2 * p1[i] as u32) / 3) as u8;
        }
        pal[2][3] = 255;
        pal[3][3] = 255;
    } else {
        for i in 0..3 {
            pal[2][i] = ((p0[i] as u32 + p1[i] as u32) / 2) as u8;
        }
        pal[2][3] = 255;
        pal[3] = [0, 0, 0, 0];
    }
    let bits = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    for i in 0..16 {
        out[i] = pal[((bits >> (2 * i)) & 3) as usize];
    }
}

fn decode_bc4_block(b: &[u8], out: &mut [u8; 16]) {
    let a0 = b[0] as u32;
    let a1 = b[1] as u32;
    let mut pal = [0u8; 8];
    pal[0] = a0 as u8;
    pal[1] = a1 as u8;
    if a0 > a1 {
        for i in 1..7 {
            pal[i + 1] = (((7 - i as u32) * a0 + i as u32 * a1) / 7) as u8;
        }
    } else {
        for i in 1..5 {
            pal[i + 1] = (((5 - i as u32) * a0 + i as u32 * a1) / 5) as u8;
        }
        pal[6] = 0;
        pal[7] = 255;
    }
    let mut bits: u64 = 0;
    for i in 0..6 {
        bits |= (b[2 + i] as u64) << (8 * i);
    }
    for i in 0..16 {
        out[i] = pal[((bits >> (3 * i)) & 7) as usize];
    }
}

/// The Xenos' piecewise-linear gamma to linear (a gamma texture's fetch): slopes 1/4, 1/2, 1, 2 from 0, 64, 96, 192
/// of 255, with a truncated correction, in 1023ths (as Xenia's `PWLGammaToLinear`; V against the original's damage
/// map, `Texture::to_rgba8_gpu`).
pub fn pwl_gamma_to_linear(g: f32) -> f32 {
    let g = g.clamp(0.0, 1.0);
    let (scale, offset) = if g >= 96.0 / 255.0 {
        if g >= 192.0 / 255.0 { (8.0 / 1024.0, -1024.0) } else { (4.0 / 1024.0, -256.0) }
    } else if g >= 64.0 / 255.0 {
        (2.0 / 1024.0, -64.0)
    } else {
        (1.0 / 1024.0, 0.0)
    };
    let mut linear = g * ((255.0 * 1024.0) * scale) + offset;
    linear += (linear * scale).trunc();
    linear * (1.0 / 1023.0)
}

/// A DXT5 level decoded as the GPU's texture unit does (an ATI / AMD one, the Xenos' kind): end points widened by
/// repeating their top bits, the in-between steps with weights in 64ths, (A e0 + B e1 + 32) >> 6. V: every texel of
/// the R8's `_maxdamage` layout as the original's damage map holds it after a reset on an AMD GPU (work/dmgmap,
/// dxtbrute.py); `decode_to_rgba8` rounds its steps differently (up to 2 off).
pub fn decode_dxt5_gpu(data: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mix = |e0: u32, e1: u32, a: u32, b: u32| ((a * e0 + b * e1 + 32) >> 6) as u8;
    let mut out = vec![0u8; w * h * 4];
    let (bw, bh) = ((w + 3) / 4, (h + 3) / 4);
    for by in 0..bh {
        for bx in 0..bw {
            let o = (by * bw + bx) * 16;
            let Some(b) = data.get(o..o + 16) else { return out };
            let (a0, a1) = (b[0] as u32, b[1] as u32);
            let mut pa = [a0 as u8, a1 as u8, 0, 0, 0, 0, 0, 255];
            if a0 > a1 {
                for (k, (wa, wb)) in [(55, 9), (46, 18), (37, 27), (27, 37), (18, 46), (9, 55)].into_iter().enumerate() {
                    pa[k + 2] = mix(a0, a1, wa, wb);
                }
            } else {
                for (k, (wa, wb)) in [(51, 13), (38, 26), (26, 38), (13, 51)].into_iter().enumerate() {
                    pa[k + 2] = mix(a0, a1, wa, wb);
                }
            }
            let c0 = u16::from_le_bytes([b[8], b[9]]) as u32;
            let c1 = u16::from_le_bytes([b[10], b[11]]) as u32;
            let wide = |c: u32| {
                let (r, g, bl) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
                [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (bl << 3) | (bl >> 2)]
            };
            let (e0, e1) = (wide(c0), wide(c1));
            let mut pc = [[0u8; 3]; 4];
            for i in 0..3 {
                pc[0][i] = e0[i] as u8;
                pc[1][i] = e1[i] as u8;
                // (c0 <= c1: the half step and black, as BC1 - only flat blocks of that kind in the layouts checked: I)
                if c0 > c1 {
                    pc[2][i] = mix(e0[i], e1[i], 43, 21);
                    pc[3][i] = mix(e0[i], e1[i], 21, 43);
                } else {
                    pc[2][i] = mix(e0[i], e1[i], 32, 32);
                }
            }
            let abits = u64::from_le_bytes([b[2], b[3], b[4], b[5], b[6], b[7], 0, 0]);
            let cbits = u32::from_le_bytes([b[12], b[13], b[14], b[15]]);
            for k in 0..16 {
                let (x, y) = (bx * 4 + k % 4, by * 4 + k / 4);
                if x < w && y < h {
                    let c = pc[((cbits >> (2 * k)) & 3) as usize];
                    let p = (y * w + x) * 4;
                    out[p..p + 4].copy_from_slice(&[c[0], c[1], c[2], pa[((abits >> (3 * k)) & 7) as usize]]);
                }
            }
        }
    }
    out
}

/// Decode a tightly packed (untiled, byte-order fixed) level to RGBA8.
pub fn decode_to_rgba8(fmt: TexFormat, data: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    let put = |out: &mut [u8], x: usize, y: usize, px: [u8; 4]| {
        if x < w && y < h {
            let o = (y * w + x) * 4;
            out[o..o + 4].copy_from_slice(&px);
        }
    };
    match fmt {
        TexFormat::Rgba8 => {
            // Stored as ARGB (after 32-bit swap) → RGBA
            for i in 0..w * h {
                if i * 4 + 4 > data.len() {
                    break;
                }
                let a = data[i * 4];
                let r = data[i * 4 + 1];
                let g = data[i * 4 + 2];
                let b = data[i * 4 + 3];
                out[i * 4..i * 4 + 4].copy_from_slice(&[r, g, b, a]);
            }
        }
        TexFormat::R8 => {
            for i in 0..w * h {
                let v = data.get(i).copied().unwrap_or(0);
                out[i * 4..i * 4 + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        TexFormat::Rg8 => {
            for i in 0..w * h {
                let g = data.get(i * 2).copied().unwrap_or(0);
                let r = data.get(i * 2 + 1).copied().unwrap_or(0);
                out[i * 4..i * 4 + 4].copy_from_slice(&[r, g, 0, 255]);
            }
        }
        TexFormat::Rgb565 => {
            for i in 0..w * h {
                if i * 2 + 2 > data.len() {
                    break;
                }
                let c = u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
                let p = rgb565(c);
                out[i * 4..i * 4 + 4].copy_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
        TexFormat::Rgba4 => {
            for i in 0..w * h {
                if i * 2 + 2 > data.len() {
                    break;
                }
                let c = u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
                let a = ((c >> 12) & 15) as u8 * 17;
                let r = ((c >> 8) & 15) as u8 * 17;
                let g = ((c >> 4) & 15) as u8 * 17;
                let b = (c & 15) as u8 * 17;
                out[i * 4..i * 4 + 4].copy_from_slice(&[r, g, b, a]);
            }
        }
        TexFormat::Rgb5a1 => {
            for i in 0..w * h {
                if i * 2 + 2 > data.len() {
                    break;
                }
                let c = u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
                let a = if c & 0x8000 != 0 { 255 } else { 0 };
                let r = (((c >> 10) & 31) as u32 * 255 / 31) as u8;
                let g = (((c >> 5) & 31) as u32 * 255 / 31) as u8;
                let b = ((c & 31) as u32 * 255 / 31) as u8;
                out[i * 4..i * 4 + 4].copy_from_slice(&[r, g, b, a]);
            }
        }
        TexFormat::Dxt1 | TexFormat::Dxt3 | TexFormat::Dxt5 | TexFormat::Dxn | TexFormat::Dxt5a | TexFormat::Ctx1 => {
            let (_, bpb) = fmt.block();
            let bw = (w + 3) / 4;
            let bh = (h + 3) / 4;
            let mut px = [[0u8; 4]; 16];
            let mut a = [0u8; 16];
            let mut g = [0u8; 16];
            for by in 0..bh {
                for bx in 0..bw {
                    let o = (by * bw + bx) * bpb;
                    if o + bpb > data.len() {
                        continue;
                    }
                    let b = &data[o..o + bpb];
                    match fmt {
                        TexFormat::Dxt1 => decode_bc1_block(b, &mut px, true),
                        TexFormat::Dxt3 => {
                            decode_bc1_block(&b[8..], &mut px, false);
                            for i in 0..16 {
                                let v = (b[i / 2] >> ((i & 1) * 4)) & 15;
                                px[i][3] = v * 17;
                            }
                        }
                        TexFormat::Dxt5 => {
                            decode_bc1_block(&b[8..], &mut px, false);
                            decode_bc4_block(b, &mut a);
                            for i in 0..16 {
                                px[i][3] = a[i];
                            }
                        }
                        TexFormat::Dxn => {
                            decode_bc4_block(b, &mut a);
                            decode_bc4_block(&b[8..], &mut g);
                            for i in 0..16 {
                                // reconstruct z for a tangent-space normal
                                let x = a[i] as f32 / 127.5 - 1.0;
                                let y = g[i] as f32 / 127.5 - 1.0;
                                let z = (1.0 - x * x - y * y).max(0.0).sqrt();
                                px[i] = [a[i], g[i], ((z * 0.5 + 0.5) * 255.0) as u8, 255];
                            }
                        }
                        TexFormat::Dxt5a => {
                            decode_bc4_block(b, &mut a);
                            for i in 0..16 {
                                px[i] = [a[i], a[i], a[i], 255];
                            }
                        }
                        TexFormat::Ctx1 => {
                            // CTX1: two 8:8 endpoints (x,y) + 2-bit indices, like BC1 colours.
                            let x0 = b[0];
                            let y0 = b[1];
                            let x1 = b[2];
                            let y1 = b[3];
                            let pal = [
                                [x0, y0],
                                [x1, y1],
                                [((2 * x0 as u32 + x1 as u32) / 3) as u8, ((2 * y0 as u32 + y1 as u32) / 3) as u8],
                                [((x0 as u32 + 2 * x1 as u32) / 3) as u8, ((y0 as u32 + 2 * y1 as u32) / 3) as u8],
                            ];
                            let bits = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
                            for i in 0..16 {
                                let p = pal[((bits >> (2 * i)) & 3) as usize];
                                let x = p[0] as f32 / 127.5 - 1.0;
                                let y = p[1] as f32 / 127.5 - 1.0;
                                let z = (1.0 - x * x - y * y).max(0.0).sqrt();
                                px[i] = [p[0], p[1], ((z * 0.5 + 0.5) * 255.0) as u8, 255];
                            }
                        }
                        _ => {}
                    }
                    for i in 0..16 {
                        put(&mut out, bx * 4 + (i & 3), by * 4 + (i >> 2), px[i]);
                    }
                }
            }
        }
        TexFormat::Unknown(_) => {}
    }
    out
}
