//! Jenkins one-at-a-time hash, as used by RAGE (`atStringHash`).
//! File names in MC:LA RPF3 archives are hashed after lower-casing.

/// Hash raw bytes (no case folding).
pub fn joaat_bytes(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &c in bytes {
        h = h.wrapping_add(c as u32);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h = h.wrapping_add(h << 15);
    h
}

/// Hash a string the way the game hashes file names: ASCII lower-cased,
/// backslashes normalised to forward slashes.
pub fn joaat(s: &str) -> u32 {
    let mut h: u32 = 0;
    for b in s.bytes() {
        let c = match b {
            b'A'..=b'Z' => b + 32,
            b'\\' => b'/',
            _ => b,
        };
        h = h.wrapping_add(c as u32);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h = h.wrapping_add(h << 15);
    h
}

/// The name hash the archive and particle lookups call (`F-58929`) with its `seed`: a leading `"` is skipped and the string
/// ends at the next `"` (or at a NUL), A-Z are lower-cased, a backslash becomes a slash, and each byte is added as a
/// signed char (bytes from 0x80 on subtract). For names of plain ASCII without quotes it equals [`joaat`] (seed 0).
pub fn at_string_hash(bytes: &[u8], seed: u32) -> u32 {
    let quoted = bytes.first() == Some(&b'"');
    let mut h = seed;
    for &b in bytes.iter().skip(quoted as usize) {
        if b == 0 || (quoted && b == b'"') {
            break;
        }
        let c = match b {
            b'A'..=b'Z' => b + 32,
            b'\\' => b'/',
            _ => b,
        };
        h = h.wrapping_add(c as i8 as i32 as u32);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h.wrapping_add(h << 15)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_values() {
        // Recovered by matching executable strings against the archive TOC.
        assert_eq!(joaat("textures"), 0xa26a72dd);
        assert_eq!(joaat("ui"), 0x1b2d32db);
        assert_eq!(joaat("galaxy.dds"), 0xe9b2daa3);
    }
}
