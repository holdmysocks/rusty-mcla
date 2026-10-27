//! RPF3 table-of-contents / script payload encryption: AES-256-ECB applied 16 times.
//! The key is not part of this project: it is derived from the user's own disc ([`crate::keys`]).

use aes::Aes256;
use cipher::{BlockDecrypt, KeyInit};

/// Decrypt `data` in place (a trailing partial block is left alone).
pub fn decrypt_toc(data: &mut [u8], key: &[u8; 32]) {
    let cipher = Aes256::new(key.into());
    let whole = data.len() / 16 * 16;
    for _round in 0..16 {
        for chunk in data[..whole].chunks_exact_mut(16) {
            cipher.decrypt_block(chunk.into());
        }
    }
}

#[cfg(test)]
mod verify_small_tests {
    //! FORMATS_TOOLS verification pass (docs/reference/formats/verify_small_evidence.md): the original's
    //! AES context set-up (init from the embedded key), the 16-round block loop and its block decrypt, run
    //! offline by work/verify-small/FORMATS_TOOLS/stage3.py on 14 deterministic buffers (16 .. 16,000
    //! bytes, including sizes that are not a multiple of 16); outputs held as an FNV-1a 64 hash.
    use super::*;

    /// `decrypt_toc` with the key the original embeds (read from the decrypted executable image, 32
    /// bytes at image offset 0x7D8AC4) against the original's output; and that key against the one the install
    /// derives from the disc (`keys::derive`), so the port's key origin is the original's key.
    #[test]
    #[ignore = "requires the user's decrypted executable image (MCLA_IMAGE) and disc image (MCLA_ISO)"]
    fn decrypt_toc_matches_the_original() {
        let image = std::fs::read(std::env::var("MCLA_IMAGE").unwrap_or_else(|_| "D:/MCLA/work/2026-10-10/original-image.bin".into())).unwrap();
        let key: [u8; 32] = image[0x7D8AC4..0x7D8AC4 + 32].try_into().unwrap();
        let (mut ih, mut oh) = (0xcbf29ce484222325u64, 0xcbf29ce484222325u64);
        let (mut state, mut n) = (0x4D434C41414553u64, 0);
        let mut next = move || {
            state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        };
        for size in [16usize, 17, 31, 32, 48, 100, 128, 255, 256, 1000, 2048, 4096, 8192, 16000] {
            let mut buf: Vec<u8> = (0..size).map(|_| (next() >> 56) as u8).collect();
            for &b in &buf {
                ih = (ih ^ b as u64).wrapping_mul(0x100000001b3);
            }
            decrypt_toc(&mut buf, &key);
            for &b in &buf {
                oh = (oh ^ b as u64).wrapping_mul(0x100000001b3);
            }
            n += 1;
        }
        assert_eq!(n, 14);
        assert_eq!(ih, 0x077e26e32bf13388, "case generator");
        assert_eq!(oh, 0x0594248b1859d631, "decrypted buffers");
        let iso_path = std::env::var("MCLA_ISO").unwrap_or_else(|_| "D:/MCLA/Midnight Club - Los Angeles - Complete Edition (USA, Europe).iso".into());
        let iso = crate::iso::Xiso::open(std::path::Path::new(&iso_path)).unwrap();
        assert_eq!(crate::keys::derive(&iso).unwrap(), key, "the key derived from the disc is the original's embedded key");
    }
}
