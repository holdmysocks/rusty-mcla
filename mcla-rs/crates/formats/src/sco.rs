//! Compiled script objects (`script_obj/**`, magic "Scr\x0e"): a 28-byte little-endian
//! header {magic, code size, static count, global count, flags, signature, compressed
//! size} followed by the payload, AES-256-ECB x16 encrypted (same key as the archive
//! table of contents) and then zlib compressed. Decompressed: code, statics, globals.
//!
//! The key is not part of the project, so extraction stores scripts in the **plain form** ([`to_plain`]):
//! the same file with the magic "Scp\x0e" and the payload already decrypted (still zlib compressed).
//! [`decode`] reads the plain form only.
//!
//! 42 small scripts (empty career mission stubs, a few cine and test scripts) use the uncompressed form "scr\x0e":
//! a 24-byte header (no compressed size) followed by code, statics and globals as they are, where only the code is
//! encrypted, in whole 16-byte blocks (a shorter tail stays clear, so stubs of under 16 bytes of code are not
//! encrypted at all). [`to_plain`] turns them into the same "Scp" form.

use crate::crypto::decrypt_toc;
use crate::Result;
use std::io::{Read, Write};

const MAGIC_DISC: &[u8; 3] = b"Scr";
const MAGIC_DISC_RAW: &[u8; 3] = b"scr";
const MAGIC_PLAIN: &[u8; 3] = b"Scp";

/// A script object in one of the disc's forms ("Scr" compressed or "scr" uncompressed), which [`to_plain`] converts.
pub fn is_disc_form(file: &[u8]) -> bool {
    file.starts_with(MAGIC_DISC) || file.starts_with(MAGIC_DISC_RAW)
}

pub struct Sco {
    pub code_size: usize,
    pub statics: usize,
    pub globals: usize,
    pub data: Vec<u8>,
}

/// Decode a script object in the plain form written by extraction.
pub fn decode(file: &[u8]) -> Result<Sco> {
    if file.len() < 28 || (&file[0..3] != MAGIC_PLAIN && !is_disc_form(file)) {
        crate::bail!("not a script object");
    }
    if is_disc_form(file) {
        crate::bail!("script object is in the disc's encrypted form: extract the game data again with this version");
    }
    inflate(file)
}

fn inflate(file: &[u8]) -> Result<Sco> {
    let u = |o: usize| u32::from_le_bytes(file[o..o + 4].try_into().unwrap()) as usize;
    let mut data = Vec::new();
    flate2::read::ZlibDecoder::new(&file[28..]).read_to_end(&mut data).map_err(|e| crate::anyhow!("script inflate: {e}"))?;
    Ok(Sco { code_size: u(4), statics: u(8), globals: u(12), data })
}

/// Convert a script object of the disc ("Scr" or "scr", encrypted) into the plain form ("Scp"). The result is checked by inflating it.
pub fn to_plain(file: &[u8], key: &[u8; 32]) -> Result<Vec<u8>> {
    if file.starts_with(MAGIC_DISC_RAW) {
        return raw_to_plain(file, key);
    }
    if file.len() < 28 || &file[0..3] != MAGIC_DISC {
        crate::bail!("not a script object of the disc");
    }
    let mut out = file.to_vec();
    out[0..3].copy_from_slice(MAGIC_PLAIN);
    decrypt_toc(&mut out[28..], key);
    inflate(&out).map_err(|e| crate::anyhow!("script does not decode with the derived key: {e}"))?;
    Ok(out)
}

/// The uncompressed form: decrypt the code's whole blocks, compress code + statics + globals, and write the 28-byte header.
fn raw_to_plain(file: &[u8], key: &[u8; 32]) -> Result<Vec<u8>> {
    let u = |o: usize| u32::from_le_bytes(file[o..o + 4].try_into().unwrap()) as usize;
    if file.len() < 24 {
        crate::bail!("not a script object of the disc");
    }
    let (code, statics, globals) = (u(4), u(8), u(12));
    let end = 24 + code + 4 * statics + 4 * globals;
    if file.len() != end {
        crate::bail!("uncompressed script object: {} bytes, header says {}", file.len(), end);
    }
    let mut payload = file[24..].to_vec();
    decrypt_toc(&mut payload[..code], key);
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(&payload).map_err(|e| crate::anyhow!("script deflate: {e}"))?;
    let packed = z.finish().map_err(|e| crate::anyhow!("script deflate: {e}"))?;
    let mut out = Vec::with_capacity(28 + packed.len());
    out.extend_from_slice(MAGIC_PLAIN);
    out.push(file[3]);
    out.extend_from_slice(&file[4..24]);
    out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
    out.extend_from_slice(&packed);
    Ok(out)
}

impl Sco {
    /// Printable strings embedded in the code (string push operands).
    pub fn strings(&self, min: usize) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = Vec::new();
        for &b in self.data.iter().chain(std::iter::once(&0u8)) {
            if (0x20..0x7f).contains(&b) {
                cur.push(b);
            } else {
                if cur.len() >= min && b == 0 {
                    out.push(String::from_utf8_lossy(&cur).to_string());
                }
                cur.clear();
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------------
// Byte code. The VM is the stack machine of the engine's other games of that year
// (operands little-endian):
//   1-13 integer add sub mul div mod iszero neg eq ne gt ge lt le, 14-25 the same for
//   floats, 26-30 vector ops, 31-33 and or xor, 34 jump, 35 jump-if-false, 36
//   jump-if-true (u32 target), 37 int->float, 38 float->int, 39 float->vector,
//   40 push i16, 41 push i32, 42 push f32, 43 dup, 44 pop,
//   45 native (u8 args, u8 returns, u32 name hash), 46 call (u32), 47 enter (u8 args,
//   u16 locals), 48 return (u8 args, u8 returns), 49 load, 50 store, 51 store keeping
//   the address, 52 explode, 53 implode (count, address: values <-> stack),
//   54-61 address of local 0..7, 62 local n, 63 static n, 64 global n,
//   65 array element (index, element size, array), 66 switch (u8 n, n x (i32, u32)),
//   67 string (u8 length, bytes), 68 null, 69-72 string copy / int to string / concat
//   (u8 size), 73-79 catch, throw, ..., 80-255 push (opcode - 96).

/// One argument of a native call, as far as a straight run through the code can tell.
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Num(f64),
    Str(String),
    /// address of three numbers (a vector built just before the call)
    Vec3([f32; 3]),
    /// result of the native call at this code offset (object handles)
    Ret(usize),
    Unknown,
}

impl Val {
    pub fn num(&self) -> Option<f64> {
        match self {
            Val::Num(v) => Some(*v),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Val::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }
    pub fn vec3(&self) -> Option<[f32; 3]> {
        match self {
            Val::Vec3(v) => Some(*v),
            _ => None,
        }
    }
    pub fn ret(&self) -> Option<usize> {
        match self {
            Val::Ret(p) => Some(*p),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Call {
    pub pc: usize,
    pub hash: u32,
    pub args: Vec<Val>,
}

#[derive(Clone, Debug)]
enum V {
    Num(f64),
    Str(String),
    Addr(u8, i64),
    Ret(usize),
    Unk,
}

impl Sco {
    pub fn code(&self) -> &[u8] {
        &self.data[..self.code_size.min(self.data.len())]
    }

    /// Run through the code once from top to bottom (no branches taken), following
    /// constants, strings and values stored in locals, and list every native call
    /// with its arguments. The scripts that set a race up are straight-line code, so
    /// this recovers their data.
    pub fn calls(&self) -> Vec<Call> {
        use std::collections::HashMap;
        let code = self.code();
        let n = code.len();
        let u16le = |p: usize| if p + 2 <= n { u16::from_le_bytes([code[p], code[p + 1]]) } else { 0 };
        let u32le = |p: usize| if p + 4 <= n { u32::from_le_bytes([code[p], code[p + 1], code[p + 2], code[p + 3]]) } else { 0 };
        // functions: entry -> (args, returns)
        let mut funcs: HashMap<usize, (usize, usize)> = HashMap::new();
        {
            let mut pc = 0;
            let mut cur: Option<usize> = None;
            while pc < n {
                let op = code[pc];
                let len = op_len(code, pc);
                if op == 47 {
                    cur = Some(pc);
                    funcs.insert(pc, (code.get(pc + 1).copied().unwrap_or(0) as usize, 0));
                } else if op == 48 {
                    if let Some(c) = cur {
                        if let Some(f) = funcs.get_mut(&c) {
                            f.1 = code.get(pc + 2).copied().unwrap_or(0) as usize;
                        }
                    }
                }
                pc += len;
            }
        }
        let mut out = Vec::new();
        let mut stack: Vec<V> = Vec::new();
        let mut mem: HashMap<(u8, i64), V> = HashMap::new();
        let mut pc = 0;
        macro_rules! pop {
            () => {
                stack.pop().unwrap_or(V::Unk)
            };
        }
        while pc < n {
            let op = code[pc];
            let len = op_len(code, pc);
            let a = pc + 1;
            match op {
                80..=255 => stack.push(V::Num(op as f64 - 96.0)),
                54..=61 => stack.push(V::Addr(b'L', op as i64 - 54)),
                40 => stack.push(V::Num(u16le(a) as i16 as f64)),
                41 => stack.push(V::Num(u32le(a) as i32 as f64)),
                42 => stack.push(V::Num(f32::from_bits(u32le(a)) as f64)),
                67 => {
                    let l = code.get(a).copied().unwrap_or(0) as usize;
                    let s = code.get(a + 1..(a + 1 + l).min(n)).unwrap_or(&[]);
                    let s = s.split(|b| *b == 0).next().unwrap_or(&[]);
                    stack.push(V::Str(s.iter().map(|&b| b as char).collect()));
                }
                47 => stack.clear(),
                62 | 63 | 64 => {
                    let kind = [b'L', b'S', b'G'][op as usize - 62];
                    let v = match pop!() {
                        V::Num(v) => v as i64,
                        _ => 0,
                    };
                    stack.push(V::Addr(kind, v));
                }
                49 => {
                    let v = match pop!() {
                        V::Addr(k, i) => mem.get(&(k, i)).cloned().unwrap_or(V::Unk),
                        _ => V::Unk,
                    };
                    stack.push(v);
                }
                50 => {
                    let addr = pop!();
                    let v = pop!();
                    if let V::Addr(k, i) = addr {
                        mem.insert((k, i), v);
                    }
                }
                51 => {
                    let v = pop!();
                    if let Some(V::Addr(k, i)) = stack.last() {
                        mem.insert((*k, *i), v);
                    }
                }
                53 => {
                    let addr = pop!();
                    let cnt = match pop!() {
                        V::Num(c) if (0.0..64.0).contains(&c) => c as usize,
                        _ => 0,
                    };
                    let mut vals: Vec<V> = (0..cnt).map(|_| pop!()).collect();
                    vals.reverse();
                    if let V::Addr(k, i) = addr {
                        for (j, v) in vals.into_iter().enumerate() {
                            mem.insert((k, i + j as i64), v);
                        }
                    }
                }
                52 => {
                    let addr = pop!();
                    let cnt = match pop!() {
                        V::Num(c) if (0.0..64.0).contains(&c) => c as usize,
                        _ => 0,
                    };
                    for j in 0..cnt {
                        stack.push(match addr {
                            V::Addr(k, i) => mem.get(&(k, i + j as i64)).cloned().unwrap_or(V::Unk),
                            _ => V::Unk,
                        });
                    }
                }
                65 => {
                    let arr = pop!();
                    let size = pop!();
                    let idx = pop!();
                    stack.push(match (arr, size, idx) {
                        (V::Addr(k, i), V::Num(s), V::Num(x)) => V::Addr(k, i + 1 + x as i64 * s as i64),
                        _ => V::Unk,
                    });
                }
                45 => {
                    let argc = code.get(a).copied().unwrap_or(0) as usize;
                    let retc = code.get(a + 1).copied().unwrap_or(0) as usize;
                    let hash = u32le(a + 2);
                    let mut args: Vec<V> = (0..argc).map(|_| pop!()).collect();
                    args.reverse();
                    let args = args
                        .into_iter()
                        .map(|v| match v {
                            V::Num(x) => Val::Num(x),
                            V::Str(s) => Val::Str(s),
                            V::Ret(p) => Val::Ret(p),
                            V::Addr(k, i) => {
                                let g = |j: i64| match mem.get(&(k, i + j)) {
                                    Some(V::Num(x)) => Some(*x as f32),
                                    _ => None,
                                };
                                match (g(0), g(1), g(2)) {
                                    (Some(x), Some(y), Some(z)) => Val::Vec3([x, y, z]),
                                    _ => match mem.get(&(k, i)) {
                                        Some(V::Num(x)) => Val::Num(*x),
                                        Some(V::Ret(p)) => Val::Ret(*p),
                                        Some(V::Str(s)) => Val::Str(s.clone()),
                                        _ => Val::Unknown,
                                    },
                                }
                            }
                            V::Unk => Val::Unknown,
                        })
                        .collect();
                    out.push(Call { pc, hash, args });
                    for _ in 0..retc {
                        stack.push(V::Ret(pc));
                    }
                }
                46 => {
                    let (argc, retc) = funcs.get(&(u32le(a) as usize)).copied().unwrap_or((0, 0));
                    for _ in 0..argc {
                        pop!();
                    }
                    for _ in 0..retc {
                        stack.push(V::Unk);
                    }
                }
                43 => {
                    let v = stack.last().cloned().unwrap_or(V::Unk);
                    stack.push(v);
                }
                44 | 35 | 36 | 66 => {
                    pop!();
                }
                1..=5 | 14..=18 => {
                    let b = pop!();
                    let x = pop!();
                    stack.push(match (x, b) {
                        (V::Num(x), V::Num(b)) => V::Num(match op {
                            1 | 14 => x + b,
                            2 | 15 => x - b,
                            3 | 16 => x * b,
                            4 if b != 0.0 => (x / b).trunc(),
                            17 if b != 0.0 => x / b,
                            _ => 0.0,
                        }),
                        // address arithmetic: structure fields
                        (V::Addr(k, i), V::Num(b)) if op == 1 => V::Addr(k, i + b as i64),
                        _ => V::Unk,
                    });
                }
                7 | 19 => {
                    let v = pop!();
                    stack.push(match v {
                        V::Num(x) => V::Num(-x),
                        _ => V::Unk,
                    });
                }
                37 | 38 => {
                    let v = pop!();
                    stack.push(match v {
                        V::Num(x) => V::Num(if op == 38 { x.trunc() } else { x }),
                        _ => V::Unk,
                    });
                }
                39 => {
                    let v = pop!();
                    stack.push(v.clone());
                    stack.push(v.clone());
                    stack.push(v);
                }
                6 => {
                    let v = pop!();
                    stack.push(match v {
                        V::Num(x) => V::Num(if x == 0.0 { 1.0 } else { 0.0 }),
                        _ => V::Unk,
                    });
                }
                8..=13 | 20..=25 | 31..=33 => {
                    pop!();
                    pop!();
                    stack.push(V::Unk);
                }
                26..=29 => {
                    for _ in 0..3 {
                        pop!();
                    }
                }
                68 => stack.push(V::Num(0.0)),
                69..=72 => {
                    pop!();
                    pop!();
                }
                _ => {}
            }
            pc += len;
        }
        out
    }
}

/// Length of the instruction at `pc` (opcode + operands).
fn op_len(code: &[u8], pc: usize) -> usize {
    let op = code[pc];
    let at = |i: usize| code.get(pc + i).copied().unwrap_or(0) as usize;
    match op {
        40 => 3,
        41 | 42 | 34 | 35 | 36 | 46 => 5,
        45 => 7,
        47 => 4,
        48 => 3,
        66 => 2 + at(1) * 8,
        67 => 2 + at(1),
        69..=72 => 2,
        _ => 1,
    }
}

/// A constant in the code, in the order it is pushed.
#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Num(f64),
    Str(String),
}

impl Sco {
    /// Every pushed constant in code order (negations and int-to-float applied).
    /// Tables that scripts fill field by field can be read off this stream.
    pub fn consts(&self) -> Vec<Tok> {
        let code = self.code();
        let n = code.len();
        let mut out: Vec<Tok> = Vec::new();
        let mut pc = 0;
        while pc < n {
            let op = code[pc];
            let a = pc + 1;
            let rd = |k: usize| -> [u8; 4] {
                let mut b = [0u8; 4];
                for (i, x) in b.iter_mut().enumerate().take(k) {
                    *x = code.get(a + i).copied().unwrap_or(0);
                }
                b
            };
            match op {
                80..=255 => out.push(Tok::Num(op as f64 - 96.0)),
                40 => out.push(Tok::Num(i16::from_le_bytes([rd(2)[0], rd(2)[1]]) as f64)),
                41 => out.push(Tok::Num(i32::from_le_bytes(rd(4)) as f64)),
                42 => out.push(Tok::Num(f32::from_le_bytes(rd(4)) as f64)),
                7 | 19 => {
                    if let Some(Tok::Num(v)) = out.last_mut() {
                        *v = -*v;
                    }
                }
                67 => {
                    let l = code.get(a).copied().unwrap_or(0) as usize;
                    let s = code.get(a + 1..(a + 1 + l).min(n)).unwrap_or(&[]);
                    let s = s.split(|b| *b == 0).next().unwrap_or(&[]);
                    out.push(Tok::Str(s.iter().map(|&b| b as char).collect()));
                }
                _ => {}
            }
            pc += op_len(code, pc);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::{BlockEncrypt, KeyInit};
    use std::io::Write;

    #[test]
    fn disc_form_converts_to_plain_and_decodes() {
        let key: [u8; 32] = std::array::from_fn(|i| ((i * 29 + 5) % 251) as u8);
        let code = b"script body \x01\x02\x03".to_vec();
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&code).unwrap();
        let mut payload = z.finish().unwrap();
        let c = aes::Aes256::new((&key).into());
        let whole = payload.len() / 16 * 16;
        for _ in 0..16 {
            for chunk in payload[..whole].chunks_exact_mut(16) {
                c.encrypt_block(chunk.into());
            }
        }
        let mut file = Vec::new();
        file.extend_from_slice(b"Scr\x0e");
        for v in [7u32, 2, 3, 0, 0, payload.len() as u32] {
            file.extend_from_slice(&v.to_le_bytes());
        }
        file.extend_from_slice(&payload);
        // the disc form is refused, the plain form decodes to the original data
        assert!(decode(&file).is_err());
        let plain = to_plain(&file, &key).unwrap();
        assert_eq!(&plain[0..3], b"Scp");
        assert_eq!(plain.len(), file.len());
        let sco = decode(&plain).unwrap();
        assert_eq!((sco.code_size, sco.statics, sco.globals), (7, 2, 3));
        assert_eq!(sco.data, code);
        // a wrong key is caught when converting
        let mut wrong = key;
        wrong[0] ^= 1;
        assert!(to_plain(&file, &wrong).is_err());
    }

    #[test]
    fn uncompressed_form_decrypts_only_whole_code_blocks() {
        let key: [u8; 32] = std::array::from_fn(|i| ((i * 31 + 7) % 251) as u8);
        // 20 bytes of code (one whole block + a clear tail of 4), one static, no globals
        let code: Vec<u8> = (0..20u8).collect();
        let mut enc = code.clone();
        let c = aes::Aes256::new((&key).into());
        for _ in 0..16 {
            c.encrypt_block((&mut enc[..16]).into());
        }
        let mut file = Vec::new();
        file.extend_from_slice(b"scr\x0e");
        for v in [20u32, 1, 0, 1, 0] {
            file.extend_from_slice(&v.to_le_bytes());
        }
        file.extend_from_slice(&enc);
        file.extend_from_slice(&[0xaa, 0xbb, 0xcc, 0xdd]);
        assert!(is_disc_form(&file) && decode(&file).is_err());
        let sco = decode(&to_plain(&file, &key).unwrap()).unwrap();
        assert_eq!((sco.code_size, sco.statics, sco.globals), (20, 1, 0));
        assert_eq!(&sco.data[..20], &code[..]);
        assert_eq!(&sco.data[20..], &[0xaa, 0xbb, 0xcc, 0xdd]);
        // a size that disagrees with the header is refused
        assert!(to_plain(&file[..file.len() - 1], &key).is_err());
    }
}

#[cfg(test)]
mod verify_small_tests {
    //! FORMATS_TOOLS verification pass (docs/reference/formats/verify_small_evidence.md): F-92179, the
    //! instruction length, run offline by work/verify-small/FORMATS_TOOLS/stage2.py.
    use super::*;

    /// The original's instruction-length function on all 65,536 (opcode, next byte) pairs against
    /// `op_len`; the outputs are held as an FNV-1a 64 hash of the u32 results (values 1 .. 2042).
    #[test]
    fn op_len_matches_the_original() {
        let mut h = 0xcbf29ce484222325u64;
        for op in 0..=255u8 {
            for b in 0..=255u8 {
                let code = [op, b];
                for x in (op_len(&code, 0) as u32).to_le_bytes() {
                    h = (h ^ x as u64).wrapping_mul(0x100000001b3);
                }
            }
        }
        assert_eq!(h, 0x5d06879d5c8e1b89);
    }
}
