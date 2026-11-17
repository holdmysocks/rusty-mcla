//! Baseline JPEG encoder and decoder that reproduce IJG libjpeg 6b bit for bit.
//!
//! The original game stores photo-album pictures as JPEGs made and read by a
//! statically linked libjpeg 6b. This module has no dependencies and follows
//! that library's integer arithmetic exactly:
//!
//! * encoder: jccolor.c (RGB to YCbCr tables), jcsample.c (h2v2 downsampling
//!   with the alternating 1,2 bias), jcprepct.c (edge expansion), jfdctint.c
//!   (slow-but-accurate integer FDCT), jcdctmgr.c (quantisation), jcparam.c
//!   (quality scaling, standard tables), jchuff.c (Huffman coding) and
//!   jcmarker.c (marker order). Settings: JFIF 1.01, density unit 0, 1x1,
//!   4:2:0, standard Huffman tables, no restart markers, not optimised.
//! * decoder: jdhuff.c, jidctint.c (slow integer IDCT with the range-limit
//!   table), jdsample.c (fancy h2v1 / h2v2 upsampling, plain and integral
//!   replication) and jdcolor.c (YCbCr to RGB tables). Baseline sequential
//!   Huffman files only.
//!
//! Verified against libjpeg-turbo's accurate integer path (documented to be
//! bitwise identical to libjpeg 6b), see the tests and `testdata/jpeg6b`.
#![allow(dead_code)]

const STD_LUM_Q: [u8; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56,
    14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104,
    113, 92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];
const STD_CHR_Q: [u8; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99,
    99, 47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

/// jpeg_natural_order: natural position of zigzag index i (16 extra entries
/// of 63 guard against corrupt run lengths, as in libjpeg).
const NATURAL_ORDER: [usize; 80] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63, 63,
    63, 63, 63, 63, 63, 63,
];

// Standard Huffman tables (jcparam.c, ITU T.81 annex K).
const DC_LUM_BITS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const DC_LUM_VALS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const AC_LUM_BITS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 125];
const AC_LUM_VALS: [u8; 162] = [1, 2, 3, 0, 4, 17, 5, 18, 33, 49, 65, 6, 19, 81, 97, 7, 34, 113, 20, 50, 129, 145, 161, 8, 35, 66, 177, 193, 21, 82, 209, 240, 36, 51, 98, 114, 130, 9, 10, 22, 23, 24, 25, 26, 37, 38, 39, 40, 41, 42, 52, 53, 54, 55, 56, 57, 58, 67, 68, 69, 70, 71, 72, 73, 74, 83, 84, 85, 86, 87, 88, 89, 90, 99, 100, 101, 102, 103, 104, 105, 106, 115, 116, 117, 118, 119, 120, 121, 122, 131, 132, 133, 134, 135, 136, 137, 138, 146, 147, 148, 149, 150, 151, 152, 153, 154, 162, 163, 164, 165, 166, 167, 168, 169, 170, 178, 179, 180, 181, 182, 183, 184, 185, 186, 194, 195, 196, 197, 198, 199, 200, 201, 202, 210, 211, 212, 213, 214, 215, 216, 217, 218, 225, 226, 227, 228, 229, 230, 231, 232, 233, 234, 241, 242, 243, 244, 245, 246, 247, 248, 249, 250];
const DC_CHR_BITS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const DC_CHR_VALS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const AC_CHR_BITS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 119];
const AC_CHR_VALS: [u8; 162] = [0, 1, 2, 3, 17, 4, 5, 33, 49, 6, 18, 65, 81, 7, 97, 113, 19, 34, 50, 129, 8, 20, 66, 145, 161, 177, 193, 9, 35, 51, 82, 240, 21, 98, 114, 209, 10, 22, 36, 52, 225, 37, 241, 23, 24, 25, 26, 38, 39, 40, 41, 42, 53, 54, 55, 56, 57, 58, 67, 68, 69, 70, 71, 72, 73, 74, 83, 84, 85, 86, 87, 88, 89, 90, 99, 100, 101, 102, 103, 104, 105, 106, 115, 116, 117, 118, 119, 120, 121, 122, 130, 131, 132, 133, 134, 135, 136, 137, 138, 146, 147, 148, 149, 150, 151, 152, 153, 154, 162, 163, 164, 165, 166, 167, 168, 169, 170, 178, 179, 180, 181, 182, 183, 184, 185, 186, 194, 195, 196, 197, 198, 199, 200, 201, 202, 210, 211, 212, 213, 214, 215, 216, 217, 218, 226, 227, 228, 229, 230, 231, 232, 233, 234, 242, 243, 244, 245, 246, 247, 248, 249, 250];

const CONST_BITS: i32 = 13;
const PASS1_BITS: i32 = 2;
const FIX_0_298631336: i64 = 2446;
const FIX_0_390180644: i64 = 3196;
const FIX_0_541196100: i64 = 4433;
const FIX_0_765366865: i64 = 6270;
const FIX_0_899976223: i64 = 7373;
const FIX_1_175875602: i64 = 9633;
const FIX_1_501321110: i64 = 12299;
const FIX_1_847759065: i64 = 15137;
const FIX_1_961570560: i64 = 16069;
const FIX_2_053119869: i64 = 16819;
const FIX_2_562915447: i64 = 20995;
const FIX_3_072711026: i64 = 25172;

#[inline]
fn descale(x: i64, n: i32) -> i64 {
    (x + (1i64 << (n - 1))) >> n
}

// ---------------------------------------------------------------- encoder

/// jcparam.c jpeg_set_quality(quality, force_baseline = TRUE) for one table.
fn scaled_quant(basic: &[u8; 64], quality: i32) -> [u16; 64] {
    let q = quality.clamp(1, 100);
    let scale: i64 = if q < 50 { 5000 / q as i64 } else { 200 - q as i64 * 2 };
    let mut out = [0u16; 64];
    for i in 0..64 {
        let mut t = (basic[i] as i64 * scale + 50) / 100;
        if t <= 0 {
            t = 1;
        }
        if t > 32767 {
            t = 32767;
        }
        if t > 255 {
            t = 255;
        }
        out[i] = t as u16;
    }
    out
}

/// jfdctint.c jpeg_fdct_islow on level-shifted samples (output scaled by 8).
fn fdct_islow(data: &mut [i64; 64]) {
    for pass in 0..2 {
        for i in 0..8 {
            let (base, step) = if pass == 0 { (i * 8, 1) } else { (i, 8) };
            let d: [i64; 8] = std::array::from_fn(|k| data[base + k * step]);
            let t0 = d[0] + d[7];
            let mut t7 = d[0] - d[7];
            let t1 = d[1] + d[6];
            let mut t6 = d[1] - d[6];
            let t2 = d[2] + d[5];
            let mut t5 = d[2] - d[5];
            let t3 = d[3] + d[4];
            let mut t4 = d[3] - d[4];
            let t10 = t0 + t3;
            let t13 = t0 - t3;
            let t11 = t1 + t2;
            let t12 = t1 - t2;
            let (o0, o4, sh);
            if pass == 0 {
                o0 = (t10 + t11) << PASS1_BITS;
                o4 = (t10 - t11) << PASS1_BITS;
                sh = CONST_BITS - PASS1_BITS;
            } else {
                o0 = descale(t10 + t11, PASS1_BITS);
                o4 = descale(t10 - t11, PASS1_BITS);
                sh = CONST_BITS + PASS1_BITS;
            }
            let z1 = (t12 + t13) * FIX_0_541196100;
            let o2 = descale(z1 + t13 * FIX_0_765366865, sh);
            let o6 = descale(z1 + t12 * (-FIX_1_847759065), sh);
            let mut z1 = t4 + t7;
            let mut z2 = t5 + t6;
            let mut z3 = t4 + t6;
            let mut z4 = t5 + t7;
            let z5 = (z3 + z4) * FIX_1_175875602;
            t4 *= FIX_0_298631336;
            t5 *= FIX_2_053119869;
            t6 *= FIX_3_072711026;
            t7 *= FIX_1_501321110;
            z1 *= -FIX_0_899976223;
            z2 *= -FIX_2_562915447;
            z3 *= -FIX_1_961570560;
            z4 *= -FIX_0_390180644;
            z3 += z5;
            z4 += z5;
            let o7 = descale(t4 + z1 + z3, sh);
            let o5 = descale(t5 + z2 + z4, sh);
            let o3 = descale(t6 + z2 + z3, sh);
            let o1 = descale(t7 + z1 + z4, sh);
            let o = [o0, o1, o2, o3, o4, o5, o6, o7];
            for k in 0..8 {
                data[base + k * step] = o[k];
            }
        }
    }
}

/// jcdctmgr.c forward_DCT for one block: samples (0..255) -> quantised
/// coefficients in natural order.
fn fdct_quant(samples: &[u8; 64], q: &[u16; 64]) -> [i16; 64] {
    let mut ws = [0i64; 64];
    for i in 0..64 {
        ws[i] = samples[i] as i64 - 128;
    }
    fdct_islow(&mut ws);
    let mut out = [0i16; 64];
    for i in 0..64 {
        let qval = (q[i] as i64) << 3;
        let mut t = ws[i];
        if t < 0 {
            t = -t;
            t += qval >> 1;
            t /= qval;
            t = -t;
        } else {
            t += qval >> 1;
            t /= qval;
        }
        out[i] = t as i16;
    }
    out
}

struct HuffEnc {
    code: [u32; 256],
    size: [u8; 256],
}

/// jchuff.c jpeg_make_c_derived_tbl.
fn make_enc(bits: &[u8; 16], vals: &[u8]) -> HuffEnc {
    let mut huffsize = Vec::new();
    for l in 1..=16usize {
        for _ in 0..bits[l - 1] {
            huffsize.push(l as u8);
        }
    }
    let mut huffcode = vec![0u32; huffsize.len()];
    let mut code = 0u32;
    let mut p = 0;
    if !huffsize.is_empty() {
        let mut si = huffsize[0];
        while p < huffsize.len() {
            while p < huffsize.len() && huffsize[p] == si {
                huffcode[p] = code;
                code += 1;
                p += 1;
            }
            code <<= 1;
            si += 1;
        }
    }
    let mut t = HuffEnc { code: [0; 256], size: [0; 256] };
    for p in 0..huffsize.len() {
        let v = vals[p] as usize;
        t.code[v] = huffcode[p];
        t.size[v] = huffsize[p];
    }
    t
}

struct BitWriter {
    out: Vec<u8>,
    buf: u64,
    cnt: u32,
}

impl BitWriter {
    fn emit(&mut self, code: u32, size: u32) {
        if size == 0 {
            return;
        }
        self.buf = (self.buf << size) | (code as u64 & ((1u64 << size) - 1));
        self.cnt += size;
        while self.cnt >= 8 {
            let b = ((self.buf >> (self.cnt - 8)) & 0xFF) as u8;
            self.out.push(b);
            if b == 0xFF {
                self.out.push(0);
            }
            self.cnt -= 8;
        }
        self.buf &= (1u64 << self.cnt) - 1;
    }
    /// jchuff.c flush_bits: pad with 1 bits to a byte boundary.
    fn flush(&mut self) {
        self.emit(0x7F, 7);
        self.buf = 0;
        self.cnt = 0;
    }
}

fn nbits_of(mut v: i32) -> u32 {
    let mut n = 0;
    while v != 0 {
        n += 1;
        v >>= 1;
    }
    n
}

/// jchuff.c encode_one_block.
fn encode_block(w: &mut BitWriter, blk: &[i16; 64], last_dc: &mut i32, dc: &HuffEnc, ac: &HuffEnc) {
    let mut temp = blk[0] as i32 - *last_dc;
    *last_dc = blk[0] as i32;
    let mut temp2 = temp;
    if temp < 0 {
        temp = -temp;
        temp2 -= 1;
    }
    let nb = nbits_of(temp);
    w.emit(dc.code[nb as usize], dc.size[nb as usize] as u32);
    if nb != 0 {
        w.emit(temp2 as u32, nb);
    }
    let mut r = 0;
    for k in 1..64 {
        let mut temp = blk[NATURAL_ORDER[k]] as i32;
        if temp == 0 {
            r += 1;
            continue;
        }
        while r > 15 {
            w.emit(ac.code[0xF0], ac.size[0xF0] as u32);
            r -= 16;
        }
        let mut temp2 = temp;
        if temp < 0 {
            temp = -temp;
            temp2 -= 1;
        }
        let nb = nbits_of(temp);
        let i = (r << 4) + nb as usize;
        w.emit(ac.code[i], ac.size[i] as u32);
        w.emit(temp2 as u32, nb);
        r = 0;
    }
    if r > 0 {
        w.emit(ac.code[0], ac.size[0] as u32);
    }
}

fn put16(o: &mut Vec<u8>, v: usize) {
    o.push((v >> 8) as u8);
    o.push(v as u8);
}

fn emit_dqt(o: &mut Vec<u8>, index: u8, q: &[u16; 64]) {
    o.extend_from_slice(&[0xFF, 0xDB]);
    put16(o, 67);
    o.push(index);
    for i in 0..64 {
        o.push(q[NATURAL_ORDER[i]] as u8);
    }
}

fn emit_dht(o: &mut Vec<u8>, index: u8, bits: &[u8; 16], vals: &[u8]) {
    o.extend_from_slice(&[0xFF, 0xC4]);
    put16(o, 2 + 1 + 16 + vals.len());
    o.push(index);
    o.extend_from_slice(bits);
    o.extend_from_slice(vals);
}

/// RGB (3 bytes per pixel, top row first) to a baseline JPEG exactly as
/// libjpeg 6b writes it with jpeg_set_defaults + jpeg_set_quality(q, TRUE)
/// on a JCS_RGB input (YCbCr, 4:2:0).
pub fn encode(rgb: &[u8], w: usize, h: usize, quality: i32) -> Vec<u8> {
    assert!(w > 0 && h > 0 && w < 65536 && h < 65536 && rgb.len() >= w * h * 3);
    let qlum = scaled_quant(&STD_LUM_Q, quality);
    let qchr = scaled_quant(&STD_CHR_Q, quality);

    // jccolor.c rgb_ycc_start tables (SCALEBITS 16).
    const ONE_HALF: i32 = 1 << 15;
    const CBCR_OFFSET: i32 = 128 << 16;
    let fix = |x: f64| (x * 65536.0 + 0.5) as i32;
    let mut tab = vec![0i32; 8 * 256];
    for i in 0..256usize {
        let iv = i as i32;
        tab[i] = fix(0.29900) * iv;
        tab[i + 256] = fix(0.58700) * iv;
        tab[i + 512] = fix(0.11400) * iv + ONE_HALF;
        tab[i + 768] = -fix(0.16874) * iv;
        tab[i + 1024] = -fix(0.33126) * iv;
        tab[i + 1280] = fix(0.50000) * iv + CBCR_OFFSET + ONE_HALF - 1;
        tab[i + 1536] = -fix(0.41869) * iv;
        tab[i + 1792] = -fix(0.08131) * iv;
    }
    let mut yp = vec![0u8; w * h];
    let mut cbp = vec![0u8; w * h];
    let mut crp = vec![0u8; w * h];
    for i in 0..w * h {
        let (r, g, b) = (rgb[i * 3] as usize, rgb[i * 3 + 1] as usize, rgb[i * 3 + 2] as usize);
        yp[i] = ((tab[r] + tab[g + 256] + tab[b + 512]) >> 16) as u8;
        cbp[i] = ((tab[r + 768] + tab[g + 1024] + tab[b + 1280]) >> 16) as u8;
        crp[i] = ((tab[r + 1280] + tab[g + 1536] + tab[b + 1792]) >> 16) as u8;
    }

    // Padded planes (jcprepct.c / jcsample.c edge expansion).
    let mcu_cols = (w + 15) / 16;
    let mcu_rows = (h + 15) / 16;
    let yw = mcu_cols * 16;
    let yh = mcu_rows * 16;
    let mut ypad = vec![0u8; yw * yh];
    for y in 0..yh {
        for x in 0..yw {
            ypad[y * yw + x] = yp[y.min(h - 1) * w + x.min(w - 1)];
        }
    }
    let cw = mcu_cols * 8;
    let ch = mcu_rows * 8;
    let real_rows = (h + 1) / 2;
    let down = |p: &[u8]| -> Vec<u8> {
        let mut o = vec![0u8; cw * ch];
        for oy in 0..ch {
            let sy = oy.min(real_rows - 1);
            let mut bias = 1u32;
            for ox in 0..cw {
                let x0 = (2 * ox).min(w - 1);
                let x1 = (2 * ox + 1).min(w - 1);
                let y0 = (2 * sy).min(h - 1);
                let y1 = (2 * sy + 1).min(h - 1);
                let s = p[y0 * w + x0] as u32 + p[y0 * w + x1] as u32 + p[y1 * w + x0] as u32
                    + p[y1 * w + x1] as u32
                    + bias;
                o[oy * cw + ox] = (s >> 2) as u8;
                bias ^= 3;
            }
        }
        o
    };
    let cbd = down(&cbp);
    let crd = down(&crp);

    let wib = (w + 7) / 8; // luma width in blocks
    let hib = (h + 7) / 8;
    let last_col_width = if wib % 2 == 0 { 2 } else { wib % 2 };
    let last_row_height = if hib % 2 == 0 { 2 } else { hib % 2 };

    let dc_l = make_enc(&DC_LUM_BITS, &DC_LUM_VALS);
    let ac_l = make_enc(&AC_LUM_BITS, &AC_LUM_VALS);
    let dc_c = make_enc(&DC_CHR_BITS, &DC_CHR_VALS);
    let ac_c = make_enc(&AC_CHR_BITS, &AC_CHR_VALS);

    let mut o: Vec<u8> = Vec::new();
    o.extend_from_slice(&[0xFF, 0xD8]);
    // JFIF APP0, version 1.01, unit 0, density 1x1, no thumbnail.
    o.extend_from_slice(&[0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0]);
    emit_dqt(&mut o, 0, &qlum);
    emit_dqt(&mut o, 1, &qchr);
    o.extend_from_slice(&[0xFF, 0xC0]);
    put16(&mut o, 17);
    o.push(8);
    put16(&mut o, h);
    put16(&mut o, w);
    o.push(3);
    o.extend_from_slice(&[1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    emit_dht(&mut o, 0x00, &DC_LUM_BITS, &DC_LUM_VALS);
    emit_dht(&mut o, 0x10, &AC_LUM_BITS, &AC_LUM_VALS);
    emit_dht(&mut o, 0x01, &DC_CHR_BITS, &DC_CHR_VALS);
    emit_dht(&mut o, 0x11, &AC_CHR_BITS, &AC_CHR_VALS);
    o.extend_from_slice(&[0xFF, 0xDA]);
    put16(&mut o, 12);
    o.extend_from_slice(&[3, 1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0]);

    let mut bw = BitWriter { out: o, buf: 0, cnt: 0 };
    let mut last = [0i32; 3];
    let get = |p: &[u8], stride: usize, bx: usize, by: usize| -> [u8; 64] {
        let mut s = [0u8; 64];
        for r in 0..8 {
            for c in 0..8 {
                s[r * 8 + c] = p[(by * 8 + r) * stride + bx * 8 + c];
            }
        }
        s
    };
    for mr in 0..mcu_rows {
        for mc in 0..mcu_cols {
            // jccoefct.c compress_data: luma blocks incl. dummy blocks.
            let blockcnt = if mc < mcu_cols - 1 { 2 } else { last_col_width };
            let mut yb = [[0i16; 64]; 4];
            for yi in 0..2 {
                if mr < mcu_rows - 1 || yi < last_row_height {
                    for bi in 0..blockcnt {
                        yb[yi * 2 + bi] = fdct_quant(&get(&ypad, yw, mc * 2 + bi, mr * 2 + yi), &qlum);
                    }
                    if blockcnt < 2 {
                        yb[yi * 2 + 1] = [0; 64];
                        yb[yi * 2 + 1][0] = yb[yi * 2][0];
                    }
                } else {
                    let dc = yb[yi * 2 - 1][0];
                    yb[yi * 2] = [0; 64];
                    yb[yi * 2][0] = dc;
                    yb[yi * 2 + 1] = [0; 64];
                    yb[yi * 2 + 1][0] = dc;
                }
            }
            for b in &yb {
                encode_block(&mut bw, b, &mut last[0], &dc_l, &ac_l);
            }
            let cb = fdct_quant(&get(&cbd, cw, mc, mr), &qchr);
            encode_block(&mut bw, &cb, &mut last[1], &dc_c, &ac_c);
            let cr = fdct_quant(&get(&crd, cw, mc, mr), &qchr);
            encode_block(&mut bw, &cr, &mut last[2], &dc_c, &ac_c);
        }
    }
    bw.flush();
    let mut out = bw.out;
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

// ---------------------------------------------------------------- decoder

struct HuffDec {
    mincode: [i32; 18],
    maxcode: [i32; 18],
    valptr: [usize; 17],
    vals: Vec<u8>,
}

/// jdhuff.c jpeg_make_d_derived_tbl (bit-serial decoding form).
fn make_dec(bits: &[u8; 16], vals: &[u8]) -> Option<HuffDec> {
    let total: usize = bits.iter().map(|&b| b as usize).sum();
    if total > 256 || total > vals.len() {
        return None;
    }
    let mut t = HuffDec { mincode: [0; 18], maxcode: [-1; 18], valptr: [0; 17], vals: vals.to_vec() };
    let mut code = 0i32;
    let mut p = 0usize;
    for l in 1..=16usize {
        t.valptr[l] = p;
        t.mincode[l] = code;
        code += bits[l - 1] as i32;
        p += bits[l - 1] as usize;
        t.maxcode[l] = if bits[l - 1] > 0 { code - 1 } else { -1 };
        if code > (1 << l) {
            return None;
        }
        code <<= 1;
    }
    Some(t)
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u32,
    cnt: u32,
    starved: bool,
}

impl<'a> BitReader<'a> {
    fn bit(&mut self) -> u32 {
        if self.cnt == 0 {
            let mut b = 0u8;
            if self.pos < self.data.len() {
                let c = self.data[self.pos];
                if c == 0xFF {
                    if self.pos + 1 < self.data.len() && self.data[self.pos + 1] == 0 {
                        b = 0xFF;
                        self.pos += 2;
                    } else {
                        self.starved = true; // marker or end of data
                    }
                } else {
                    b = c;
                    self.pos += 1;
                }
            } else {
                self.starved = true;
            }
            self.buf = b as u32;
            self.cnt = 8;
        }
        self.cnt -= 1;
        (self.buf >> self.cnt) & 1
    }
    fn bits(&mut self, n: u32) -> i32 {
        let mut v = 0i32;
        for _ in 0..n {
            v = (v << 1) | self.bit() as i32;
        }
        v
    }
    fn huff(&mut self, t: &HuffDec) -> Option<u8> {
        let mut code = 0i32;
        for l in 1..=16usize {
            code = (code << 1) | self.bit() as i32;
            if t.maxcode[l] >= 0 && code <= t.maxcode[l] && code >= t.mincode[l] {
                return t.vals.get(t.valptr[l] + (code - t.mincode[l]) as usize).copied();
            }
        }
        None
    }
}

fn extend(r: i32, s: u32) -> i32 {
    if s == 0 {
        0
    } else if r < (1 << (s - 1)) {
        r + (-1 << s) + 1
    } else {
        r
    }
}

struct Comp {
    id: u8,
    h: usize,
    v: usize,
    tq: usize,
    q: Option<[u16; 64]>,
    wib: usize,
    hib: usize,
    dw: usize,
    dh: usize,
    bw: usize, // padded blocks per row
    bh: usize,
    coef: Vec<i16>,
}

fn be16(d: &[u8], p: usize) -> Option<usize> {
    Some(((*d.get(p)? as usize) << 8) | *d.get(p + 1)? as usize)
}

/// jidctint.c jpeg_idct_islow with the post-IDCT range-limit table.
fn idct_islow(coef: &[i16], q: &[u16; 64], out: &mut [u8; 64]) {
    let mut ws = [0i64; 64];
    for c in 0..8 {
        let dq = |r: usize| coef[r * 8 + c] as i64 * q[r * 8 + c] as i64;
        let z2 = dq(2);
        let z3 = dq(6);
        let z1 = (z2 + z3) * FIX_0_541196100;
        let tmp2 = z1 + z3 * (-FIX_1_847759065);
        let tmp3 = z1 + z2 * FIX_0_765366865;
        let z2 = dq(0);
        let z3 = dq(4);
        let tmp0 = (z2 + z3) << CONST_BITS;
        let tmp1 = (z2 - z3) << CONST_BITS;
        let tmp10 = tmp0 + tmp3;
        let tmp13 = tmp0 - tmp3;
        let tmp11 = tmp1 + tmp2;
        let tmp12 = tmp1 - tmp2;
        let mut tmp0 = dq(7);
        let mut tmp1 = dq(5);
        let mut tmp2 = dq(3);
        let mut tmp3 = dq(1);
        let mut z1 = tmp0 + tmp3;
        let mut z2 = tmp1 + tmp2;
        let mut z3 = tmp0 + tmp2;
        let mut z4 = tmp1 + tmp3;
        let z5 = (z3 + z4) * FIX_1_175875602;
        tmp0 *= FIX_0_298631336;
        tmp1 *= FIX_2_053119869;
        tmp2 *= FIX_3_072711026;
        tmp3 *= FIX_1_501321110;
        z1 *= -FIX_0_899976223;
        z2 *= -FIX_2_562915447;
        z3 *= -FIX_1_961570560;
        z4 *= -FIX_0_390180644;
        z3 += z5;
        z4 += z5;
        tmp0 += z1 + z3;
        tmp1 += z2 + z4;
        tmp2 += z2 + z3;
        tmp3 += z1 + z4;
        let sh = CONST_BITS - PASS1_BITS;
        ws[c] = descale(tmp10 + tmp3, sh);
        ws[56 + c] = descale(tmp10 - tmp3, sh);
        ws[8 + c] = descale(tmp11 + tmp2, sh);
        ws[48 + c] = descale(tmp11 - tmp2, sh);
        ws[16 + c] = descale(tmp12 + tmp1, sh);
        ws[40 + c] = descale(tmp12 - tmp1, sh);
        ws[24 + c] = descale(tmp13 + tmp0, sh);
        ws[32 + c] = descale(tmp13 - tmp0, sh);
    }
    // jdct.h / jdmaster.c prepare_range_limit_table, post-IDCT half.
    let limit = |x: i64| -> u8 {
        let i = (x & 1023) as usize;
        if i < 128 {
            (128 + i) as u8
        } else if i < 512 {
            255
        } else if i < 896 {
            0
        } else {
            (i - 896) as u8
        }
    };
    for r in 0..8 {
        let w = |k: usize| ws[r * 8 + k];
        let z2 = w(2);
        let z3 = w(6);
        let z1 = (z2 + z3) * FIX_0_541196100;
        let tmp2 = z1 + z3 * (-FIX_1_847759065);
        let tmp3 = z1 + z2 * FIX_0_765366865;
        let tmp0 = (w(0) + w(4)) << CONST_BITS;
        let tmp1 = (w(0) - w(4)) << CONST_BITS;
        let tmp10 = tmp0 + tmp3;
        let tmp13 = tmp0 - tmp3;
        let tmp11 = tmp1 + tmp2;
        let tmp12 = tmp1 - tmp2;
        let mut tmp0 = w(7);
        let mut tmp1 = w(5);
        let mut tmp2 = w(3);
        let mut tmp3 = w(1);
        let mut z1 = tmp0 + tmp3;
        let mut z2 = tmp1 + tmp2;
        let mut z3 = tmp0 + tmp2;
        let mut z4 = tmp1 + tmp3;
        let z5 = (z3 + z4) * FIX_1_175875602;
        tmp0 *= FIX_0_298631336;
        tmp1 *= FIX_2_053119869;
        tmp2 *= FIX_3_072711026;
        tmp3 *= FIX_1_501321110;
        z1 *= -FIX_0_899976223;
        z2 *= -FIX_2_562915447;
        z3 *= -FIX_1_961570560;
        z4 *= -FIX_0_390180644;
        z3 += z5;
        z4 += z5;
        tmp0 += z1 + z3;
        tmp1 += z2 + z4;
        tmp2 += z2 + z3;
        tmp3 += z1 + z4;
        let sh = CONST_BITS + PASS1_BITS + 3;
        out[r * 8] = limit(descale(tmp10 + tmp3, sh));
        out[r * 8 + 7] = limit(descale(tmp10 - tmp3, sh));
        out[r * 8 + 1] = limit(descale(tmp11 + tmp2, sh));
        out[r * 8 + 6] = limit(descale(tmp11 - tmp2, sh));
        out[r * 8 + 2] = limit(descale(tmp12 + tmp1, sh));
        out[r * 8 + 5] = limit(descale(tmp12 - tmp1, sh));
        out[r * 8 + 3] = limit(descale(tmp13 + tmp0, sh));
        out[r * 8 + 4] = limit(descale(tmp13 - tmp0, sh));
    }
}

/// jdsample.c: bring one decoded component plane to full resolution
/// (width w, height h). Returns None for sampling ratios libjpeg rejects.
fn upsample(c: &Comp, plane: &[u8], stride: usize, max_h: usize, max_v: usize, w: usize, h: usize) -> Option<Vec<u8>> {
    let mut out = vec![0u8; w * h];
    let row = |r: usize| &plane[r * stride..r * stride + c.dw];
    let fancy = c.dw > 2;
    if c.h == max_h && c.v == max_v {
        for y in 0..h {
            out[y * w..y * w + w].copy_from_slice(&plane[y * stride..y * stride + w]);
        }
    } else if c.h * 2 == max_h && c.v == max_v {
        let mut line = vec![0u8; c.dw * 2];
        for y in 0..h {
            let inp = row(y);
            if fancy {
                // h2v1_fancy_upsample
                let n = c.dw;
                line[0] = inp[0];
                line[1] = ((inp[0] as u32 * 3 + inp[1] as u32 + 2) >> 2) as u8;
                for i in 1..n - 1 {
                    let iv = inp[i] as u32 * 3;
                    line[2 * i] = ((iv + inp[i - 1] as u32 + 1) >> 2) as u8;
                    line[2 * i + 1] = ((iv + inp[i + 1] as u32 + 2) >> 2) as u8;
                }
                line[2 * n - 2] = ((inp[n - 1] as u32 * 3 + inp[n - 2] as u32 + 1) >> 2) as u8;
                line[2 * n - 1] = inp[n - 1];
            } else {
                for i in 0..c.dw {
                    line[2 * i] = inp[i];
                    line[2 * i + 1] = inp[i];
                }
            }
            out[y * w..y * w + w].copy_from_slice(&line[..w]);
        }
    } else if c.h * 2 == max_h && c.v * 2 == max_v {
        let mut line = vec![0u8; c.dw * 2];
        for y in 0..h {
            let r = y / 2;
            if fancy {
                // h2v2_fancy_upsample; rows outside the image repeat the edge row.
                let r1 = if y & 1 == 0 { r.saturating_sub(1) } else { (r + 1).min(c.dh - 1) };
                let (i0, i1) = (row(r), row(r1));
                let n = c.dw;
                let col = |i: usize| i0[i] as u32 * 3 + i1[i] as u32;
                let mut this = col(0);
                let mut next = col(1);
                line[0] = ((this * 4 + 8) >> 4) as u8;
                line[1] = ((this * 3 + next + 7) >> 4) as u8;
                let mut lastc = this;
                this = next;
                for i in 1..n - 1 {
                    next = col(i + 1);
                    line[2 * i] = ((this * 3 + lastc + 8) >> 4) as u8;
                    line[2 * i + 1] = ((this * 3 + next + 7) >> 4) as u8;
                    lastc = this;
                    this = next;
                }
                line[2 * n - 2] = ((this * 3 + lastc + 8) >> 4) as u8;
                line[2 * n - 1] = ((this * 4 + 7) >> 4) as u8;
            } else {
                let inp = row(r);
                for i in 0..c.dw {
                    line[2 * i] = inp[i];
                    line[2 * i + 1] = inp[i];
                }
            }
            out[y * w..y * w + w].copy_from_slice(&line[..w]);
        }
    } else if max_h % c.h == 0 && max_v % c.v == 0 {
        // int_upsample: pixel replication
        let (hx, vx) = (max_h / c.h, max_v / c.v);
        for y in 0..h {
            let inp = row(y / vx);
            for x in 0..w {
                out[y * w + x] = inp[x / hx];
            }
        }
    } else {
        return None;
    }
    Some(out)
}

fn sof_ok(m: u8) -> bool {
    m == 0xC0 || m == 0xC1
}

/// JPEG to RGB (3 bytes per pixel) as libjpeg 6b's default decoder gives for
/// baseline Huffman files (islow IDCT, fancy upsampling, RGB output).
/// Returns (pixels, width, height); None for unsupported or corrupt data.
pub fn decode(data: &[u8]) -> Option<(Vec<u8>, usize, usize)> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut qt: [Option<[u16; 64]>; 4] = [None; 4];
    let mut dct: [Option<HuffDec>; 4] = [None, None, None, None];
    let mut act: [Option<HuffDec>; 4] = [None, None, None, None];
    let mut comps: Vec<Comp> = Vec::new();
    let (mut width, mut height) = (0usize, 0usize);
    let (mut max_h, mut max_v) = (1usize, 1usize);
    let mut restart = 0usize;
    let mut jfif = false;
    let mut pos = 2usize;
    let mut scans = 0;
    loop {
        // find next marker
        while *data.get(pos)? != 0xFF {
            pos += 1;
        }
        while *data.get(pos)? == 0xFF {
            pos += 1;
        }
        let m = *data.get(pos)?;
        pos += 1;
        if m == 0 || m == 0x01 || (0xD0..=0xD7).contains(&m) {
            continue;
        }
        if m == 0xD8 {
            return None;
        }
        if m == 0xD9 {
            break;
        }
        let len = be16(data, pos)?;
        if len < 2 || pos + len > data.len() {
            return None;
        }
        let seg = &data[pos + 2..pos + len];
        match m {
            0xC0 | 0xC1 => {
                if !comps.is_empty() || seg.len() < 6 || seg[0] != 8 {
                    return None;
                }
                height = be16(seg, 1)?;
                width = be16(seg, 3)?;
                let n = seg[5] as usize;
                if width == 0 || height == 0 || (n != 1 && n != 3) || seg.len() < 6 + 3 * n {
                    return None;
                }
                for i in 0..n {
                    let (id, hv, tq) = (seg[6 + i * 3], seg[7 + i * 3], seg[8 + i * 3] as usize);
                    let (h, v) = ((hv >> 4) as usize, (hv & 15) as usize);
                    if h == 0 || h > 4 || v == 0 || v > 4 || tq > 3 {
                        return None;
                    }
                    max_h = max_h.max(h);
                    max_v = max_v.max(v);
                    comps.push(Comp { id, h, v, tq, q: None, wib: 0, hib: 0, dw: 0, dh: 0, bw: 0, bh: 0, coef: Vec::new() });
                }
                let mcu_cols = (width + 8 * max_h - 1) / (8 * max_h);
                let mcu_rows = (height + 8 * max_v - 1) / (8 * max_v);
                for c in comps.iter_mut() {
                    c.wib = (width * c.h + max_h * 8 - 1) / (max_h * 8);
                    c.hib = (height * c.v + max_v * 8 - 1) / (max_v * 8);
                    c.dw = (width * c.h + max_h - 1) / max_h;
                    c.dh = (height * c.v + max_v - 1) / max_v;
                    c.bw = mcu_cols * c.h;
                    c.bh = mcu_rows * c.v;
                    c.coef = vec![0i16; c.bw * c.bh * 64];
                }
            }
            0xC2..=0xCF if m != 0xC4 && m != 0xC8 && m != 0xCC => return None,
            0xDB => {
                let mut s = seg;
                while !s.is_empty() {
                    let (pq, tq) = (s[0] >> 4, (s[0] & 15) as usize);
                    if tq > 3 || pq > 1 {
                        return None;
                    }
                    let need = 1 + 64 * (pq as usize + 1);
                    if s.len() < need {
                        return None;
                    }
                    let mut t = [0u16; 64];
                    for i in 0..64 {
                        let v = if pq == 0 { s[1 + i] as u16 } else { ((s[1 + i * 2] as u16) << 8) | s[2 + i * 2] as u16 };
                        t[NATURAL_ORDER[i]] = v;
                    }
                    qt[tq] = Some(t);
                    s = &s[need..];
                }
            }
            0xC4 => {
                let mut s = seg;
                while !s.is_empty() {
                    if s.len() < 17 {
                        return None;
                    }
                    let (tc, th) = (s[0] >> 4, (s[0] & 15) as usize);
                    if tc > 1 || th > 3 {
                        return None;
                    }
                    let mut bits = [0u8; 16];
                    bits.copy_from_slice(&s[1..17]);
                    let n: usize = bits.iter().map(|&b| b as usize).sum();
                    if n > 256 || s.len() < 17 + n {
                        return None;
                    }
                    let t = make_dec(&bits, &s[17..17 + n])?;
                    if tc == 0 {
                        dct[th] = Some(t);
                    } else {
                        act[th] = Some(t);
                    }
                    s = &s[17 + n..];
                }
            }
            0xDD => {
                restart = be16(seg, 0)?;
            }
            0xE0 => {
                if seg.len() >= 5 && &seg[..5] == b"JFIF\0" {
                    jfif = true;
                }
            }
            0xDA => {
                if comps.is_empty() {
                    return None;
                }
                let ns = *seg.first()? as usize;
                if ns == 0 || ns > comps.len() || seg.len() < 1 + 2 * ns + 3 {
                    return None;
                }
                let (ss, se, ahl) = (seg[1 + 2 * ns], seg[2 + 2 * ns], seg[3 + 2 * ns]);
                if ss != 0 || se != 63 || ahl != 0 {
                    return None;
                }
                let mut sc: Vec<(usize, usize, usize)> = Vec::new(); // comp index, dc table, ac table
                for i in 0..ns {
                    let id = seg[1 + 2 * i];
                    let tb = seg[2 + 2 * i];
                    let ci = comps.iter().position(|c| c.id == id)?;
                    if sc.iter().any(|x| x.0 == ci) || (tb >> 4) > 3 || (tb & 15) > 3 {
                        return None;
                    }
                    if comps[ci].q.is_none() {
                        comps[ci].q = Some(qt[comps[ci].tq]?);
                    }
                    sc.push((ci, (tb >> 4) as usize, (tb & 15) as usize));
                }
                for &(_, d, a) in &sc {
                    if dct[d].is_none() || act[a].is_none() {
                        return None;
                    }
                }
                pos += len;
                let mut br = BitReader { data, pos, buf: 0, cnt: 0, starved: false };
                let mut last = [0i32; 4];
                // MCU geometry
                let (mcus_x, mcus_y) = if ns == 1 {
                    (comps[sc[0].0].wib, comps[sc[0].0].hib)
                } else {
                    ((width + 8 * max_h - 1) / (8 * max_h), (height + 8 * max_v - 1) / (8 * max_v))
                };
                let mut rst_n = 0u8;
                let mut count = 0usize;
                for my in 0..mcus_y {
                    for mx in 0..mcus_x {
                        if restart > 0 && count > 0 && count % restart == 0 {
                            // process_restart: expect RSTn at the byte boundary
                            br.cnt = 0;
                            let p = br.pos;
                            if data.get(p) != Some(&0xFF) || data.get(p + 1) != Some(&(0xD0 + rst_n)) {
                                return None;
                            }
                            br.pos = p + 2;
                            br.starved = false;
                            rst_n = (rst_n + 1) & 7;
                            last = [0; 4];
                        }
                        count += 1;
                        for (si, &(ci, d, a)) in sc.iter().enumerate() {
                            let (bh_n, bv_n) = if ns == 1 { (1, 1) } else { (comps[ci].h, comps[ci].v) };
                            for by in 0..bv_n {
                                for bx in 0..bh_n {
                                    let (blx, bly) = if ns == 1 { (mx, my) } else { (mx * comps[ci].h + bx, my * comps[ci].v + by) };
                                    let off = (bly * comps[ci].bw + blx) * 64;
                                    let (dt, at) = (dct[d].as_ref()?, act[a].as_ref()?);
                                    let blk = &mut comps[ci].coef[off..off + 64];
                                    let s = br.huff(dt)? as u32;
                                    if s > 15 {
                                        return None;
                                    }
                                    let r = br.bits(s);
                                    last[si] = last[si].wrapping_add(extend(r, s));
                                    blk[0] = last[si] as i16;
                                    let mut k = 1usize;
                                    while k < 64 {
                                        let rs = br.huff(at)? as u32;
                                        let (r, s) = (rs >> 4, rs & 15);
                                        if s != 0 {
                                            k += r as usize;
                                            let v = extend(br.bits(s), s);
                                            blk[NATURAL_ORDER[k]] = v as i16;
                                        } else {
                                            if r != 15 {
                                                break;
                                            }
                                            k += 15;
                                        }
                                        k += 1;
                                    }
                                    if br.starved {
                                        return None;
                                    }
                                }
                            }
                        }
                    }
                }
                pos = br.pos;
                scans += 1;
                continue;
            }
            _ => {}
        }
        pos += len;
    }
    if scans == 0 || comps.is_empty() {
        return None;
    }
    // IDCT every component into a plane.
    let mut planes: Vec<(Vec<u8>, usize)> = Vec::new();
    for c in &comps {
        let q = c.q.as_ref()?;
        let stride = c.bw * 8;
        let mut plane = vec![0u8; stride * c.bh * 8];
        let mut tmp = [0u8; 64];
        for by in 0..c.bh {
            for bx in 0..c.bw {
                let off = (by * c.bw + bx) * 64;
                idct_islow(&c.coef[off..off + 64], q, &mut tmp);
                for r in 0..8 {
                    let o = (by * 8 + r) * stride + bx * 8;
                    plane[o..o + 8].copy_from_slice(&tmp[r * 8..r * 8 + 8]);
                }
            }
        }
        planes.push((plane, stride));
    }
    let (w, h) = (width, height);
    let mut full: Vec<Vec<u8>> = Vec::new();
    for (c, (p, stride)) in comps.iter().zip(planes.iter()) {
        full.push(upsample(c, p, *stride, max_h, max_v, w, h)?);
    }
    let mut out = vec![0u8; w * h * 3];
    if comps.len() == 1 {
        for i in 0..w * h {
            let v = full[0][i];
            out[i * 3..i * 3 + 3].copy_from_slice(&[v, v, v]);
        }
        return Some((out, w, h));
    }
    let is_rgb = !jfif && comps[0].id == b'R' && comps[1].id == b'G' && comps[2].id == b'B';
    if is_rgb {
        for i in 0..w * h {
            out[i * 3] = full[0][i];
            out[i * 3 + 1] = full[1][i];
            out[i * 3 + 2] = full[2][i];
        }
        return Some((out, w, h));
    }
    // jdcolor.c build_ycc_rgb_table (SCALEBITS 16)
    let fix = |x: f64| (x * 65536.0 + 0.5) as i32;
    let one_half = 1i32 << 15;
    let mut cr_r = [0i32; 256];
    let mut cb_b = [0i32; 256];
    let mut cr_g = [0i32; 256];
    let mut cb_g = [0i32; 256];
    for i in 0..256 {
        let x = i as i32 - 128;
        cr_r[i] = (fix(1.40200) * x + one_half) >> 16;
        cb_b[i] = (fix(1.77200) * x + one_half) >> 16;
        cr_g[i] = -fix(0.71414) * x;
        cb_g[i] = -fix(0.34414) * x + one_half;
    }
    let clamp = |v: i32| v.clamp(0, 255) as u8;
    for i in 0..w * h {
        let (y, cb, cr) = (full[0][i] as i32, full[1][i] as usize, full[2][i] as usize);
        out[i * 3] = clamp(y + cr_r[cr]);
        out[i * 3 + 1] = clamp(y + ((cb_g[cb] + cr_g[cr]) >> 16));
        out[i * 3 + 2] = clamp(y + cb_b[cb]);
    }
    Some((out, w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! fx {
        ($f:expr) => {
            &include_bytes!(concat!("../testdata/jpeg6b/", $f))[..]
        };
    }

    /// Bytes from the first DQT marker to EOI (the part libjpeg-turbo and 6b
    /// share; the APP0 header is compared separately).
    fn from_dqt(d: &[u8]) -> &[u8] {
        let i = d.windows(2).position(|w| w == [0xFF, 0xDB]).unwrap();
        &d[i..]
    }

    fn check_case(name: &str, w: usize, h: usize, q: i32, src: &[u8], jpg: &[u8], dec: &[u8]) {
        let ours = encode(src, w, h, q);
        assert_eq!(&ours[..20], &jpg[..20], "{name} q{q}: SOI + JFIF header");
        assert_eq!(from_dqt(&ours), from_dqt(jpg), "{name} q{q}: encoder bytes DQT..EOI");
        let (px, dw, dh) = decode(jpg).expect("decode");
        assert_eq!((dw, dh), (w, h));
        assert_eq!(px, dec, "{name} q{q}: decoder pixels");
        let (px2, _, _) = decode(&ours).expect("decode own output");
        assert_eq!(px2, dec, "{name} q{q}: decode of own output");
    }

    macro_rules! case {
        ($n:expr, $w:expr, $h:expr, $q:expr) => {
            check_case(
                $n,
                $w,
                $h,
                $q,
                fx!(concat!($n, ".rgb")),
                fx!(concat!($n, "_q", $q, ".jpg")),
                fx!(concat!($n, "_q", $q, ".dec.rgb")),
            )
        };
    }

    #[test]
    fn fixtures_match_libjpeg() {
        case!("photo_100x75", 100, 75, 85);
        case!("photo_100x75", 100, 75, 50);
        case!("edges_37x23", 37, 23, 76);
        case!("edges_37x23", 37, 23, 95);
        case!("noise_64x32", 64, 32, 68);
        case!("noise_64x32", 64, 32, 20);
    }

    #[test]
    fn decode_444_and_422() {
        for (j, d) in [
            (fx!("edges_37x23_q85_444.jpg"), fx!("edges_37x23_q85_444.dec.rgb")),
            (fx!("edges_37x23_q85_422.jpg"), fx!("edges_37x23_q85_422.dec.rgb")),
        ] {
            let (px, w, h) = decode(j).expect("decode");
            assert_eq!((w, h), (37, 23));
            assert_eq!(px, d);
        }
    }

    #[test]
    fn rejects_unsupported_and_corrupt() {
        let jpg = fx!("edges_37x23_q95.jpg");
        let mut prog = jpg.to_vec();
        let i = prog.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
        prog[i + 1] = 0xC2;
        assert!(decode(&prog).is_none());
        assert!(decode(&jpg[..jpg.len() / 2]).is_none());
        assert!(decode(&[1, 2, 3, 4, 5]).is_none());
        assert!(decode(&[]).is_none());
    }

    /// Full corpus made by work/ui2/jpeg/gen.py; set MCLA_JPEG6B_CASES to its
    /// `cases` directory to run it.
    #[test]
    fn corpus_if_present() {
        let Ok(dir) = std::env::var("MCLA_JPEG6B_CASES") else { return };
        let (mut files, mut enc_ok, mut dec_ok, mut pix) = (0, 0, 0, 0usize);
        let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
        names.sort();
        for p in names {
            let n = p.file_name().unwrap().to_str().unwrap().to_string();
            if !n.ends_with(".jpg") {
                continue;
            }
            files += 1;
            let stem = n.trim_end_matches(".jpg");
            let jpg = std::fs::read(&p).unwrap();
            let dec = std::fs::read(p.with_file_name(format!("{stem}.dec.rgb"))).unwrap();
            let (px, w, h) = decode(&jpg).unwrap_or_else(|| panic!("{n}: decode failed"));
            assert!(px == dec, "{n}: decoded pixels differ");
            dec_ok += 1;
            pix += px.len() / 3;
            if stem.ends_with("_444") || stem.ends_with("_422") {
                continue;
            }
            let qpos = stem.rfind("_q").unwrap();
            let q: i32 = stem[qpos + 2..].parse().unwrap();
            let src = std::fs::read(p.with_file_name(format!("{}.rgb", &stem[..qpos]))).unwrap();
            let ours = encode(&src, w, h, q);
            assert_eq!(&ours[..20], &jpg[..20], "{n}: header");
            assert!(from_dqt(&ours) == from_dqt(&jpg), "{n}: encoder bytes differ");
            enc_ok += 1;
        }
        println!("corpus: {files} files, encoder identical {enc_ok}, decoder identical {dec_ok}, {pix} pixels");
    }
}
