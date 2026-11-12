//! The game's own Vulkan pipeline cache (GitHub #48: the first start built the scene pipelines for 7 - 15 s on 4 cores
//! whenever the driver's own shader cache had dropped them - NVIDIA's is 1 GB for every program on the machine).
//!
//! One file, `<user folder>\shadercache\vk-<version>\<wgpu's key: vendor and device>.bin`: 8 bytes `MCLAPC01`, the data's
//! FNV-1a 64 hash (little-endian), then the data `wgpu::PipelineCache::get_data` gave. wgpu's own header inside the data
//! holds the adapter and the driver's cache UUID, and `fallback: true` turns a cache made by another driver into an empty
//! one; the hash catches a file cut short or damaged on disk, which is then not handed to the driver at all. The folders
//! of other versions are removed. Written next to the old file and renamed over it, so a crash mid-write keeps the old one.
//! Only on Vulkan (wgpu 24 has no pipeline cache on DX12, which keeps its compiled programs itself - `dxil_cache_dir`).
//! `MCLA_PIPELINE_CACHE=0`: no cache, as before.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

const MAGIC: &[u8; 8] = b"MCLAPC01";

struct Pc {
    cache: wgpu::PipelineCache,
    path: PathBuf,
    /// the data's length at the last write (or the read), so an unchanged cache is not written again
    saved: AtomicUsize,
    writing: Mutex<()>,
}

static CACHE: OnceLock<Pc> = OnceLock::new();

fn off() -> bool {
    std::env::var("MCLA_PIPELINE_CACHE").is_ok_and(|v| v.trim() == "0")
}

/// The device feature to ask for: the pipeline cache where the adapter has it (Vulkan).
pub fn feature(info: &wgpu::AdapterInfo, feats: wgpu::Features) -> wgpu::Features {
    if !off() && info.backend == wgpu::Backend::Vulkan && feats.contains(wgpu::Features::PIPELINE_CACHE) {
        wgpu::Features::PIPELINE_CACHE
    } else {
        wgpu::Features::empty()
    }
}

fn fnv64(d: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in d {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The file's data if it is whole (magic and hash), else why not.
fn unwrap(f: &[u8]) -> Result<&[u8], &'static str> {
    if f.len() < 16 || &f[..8] != MAGIC {
        return Err("not a cache file");
    }
    let h = u64::from_le_bytes(f[8..16].try_into().unwrap());
    let d = &f[16..];
    if fnv64(d) != h {
        return Err("damaged (hash)");
    }
    Ok(d)
}

fn wrap(d: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(d.len() + 16);
    f.extend_from_slice(MAGIC);
    f.extend_from_slice(&fnv64(d).to_le_bytes());
    f.extend_from_slice(d);
    f
}

fn dir() -> PathBuf {
    let root = mcla_config::paths::user_data_dir().join("shadercache");
    let name = concat!("vk-", env!("CARGO_PKG_VERSION"));
    if let Ok(list) = std::fs::read_dir(&root) {
        for e in list.flatten() {
            let n = e.file_name();
            let n = n.to_string_lossy();
            if n.starts_with("vk-") && n != name {
                match std::fs::remove_dir_all(e.path()) {
                    Ok(()) => log::info!("pipeline cache: removed the old {n}"),
                    Err(err) => log::warn!("pipeline cache: could not remove {}: {err}", e.path().display()),
                }
            }
        }
    }
    root.join(name)
}

/// Makes the cache from its file, once, after the device (asked for [`feature`]).
pub fn init(device: &wgpu::Device, info: &wgpu::AdapterInfo) {
    if off() || !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
        return;
    }
    let Some(key) = wgpu::util::pipeline_cache_key(info) else { return };
    let path = dir().join(format!("{key}.bin"));
    let file = std::fs::read(&path).ok();
    let data = match file.as_deref().map(unwrap) {
        Some(Ok(d)) => Some(d),
        Some(Err(why)) => {
            log::warn!("pipeline cache: {} {why}, not used", path.display());
            None
        }
        None => None,
    };
    // SAFETY: the data is what `get_data` of this backend wrote (the magic and the hash checked); wgpu checks its own
    // header (adapter, driver cache UUID, size) and with `fallback` starts empty when it does not fit.
    let cache = unsafe { device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor { label: Some("mcla pipelines"), data, fallback: true }) };
    let n = cache.get_data().map_or(0, |d| d.len());
    log::info!("pipeline cache: {} - {} bytes read, {} kept", path.display(), data.map_or(0, |d| d.len()), n);
    let _ = CACHE.set(Pc { cache, path, saved: AtomicUsize::new(n), writing: Mutex::new(()) });
}

/// For the pipelines' `cache` field.
pub fn get() -> Option<&'static wgpu::PipelineCache> {
    CACHE.get().map(|p| &p.cache)
}

/// Writes the cache when it has grown since the last write (after the pipeline builds, and at the end).
pub fn save() {
    let Some(p) = CACHE.get() else { return };
    let Ok(_w) = p.writing.lock() else { return };
    let Some(d) = p.cache.get_data() else { return };
    if d.len() == p.saved.load(Ordering::Relaxed) {
        return;
    }
    match write(&p.path, &wrap(&d)) {
        Ok(()) => {
            p.saved.store(d.len(), Ordering::Relaxed);
            log::info!("pipeline cache: {} bytes written", d.len());
        }
        Err(e) => log::warn!("pipeline cache: could not write {}: {e}", p.path.display()),
    }
}

fn write(path: &Path, f: &[u8]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, f)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_whole_file_reads_back_and_a_damaged_one_does_not() {
        let d: Vec<u8> = (0..1000u32).map(|i| (i * 7) as u8).collect();
        let f = wrap(&d);
        assert_eq!(unwrap(&f), Ok(&d[..]));
        let mut bad = f.clone();
        bad[500] ^= 1;
        assert_eq!(unwrap(&bad), Err("damaged (hash)"));
        assert_eq!(unwrap(&f[..f.len() - 1]), Err("damaged (hash)"));
        assert_eq!(unwrap(&f[..10]), Err("not a cache file"));
        assert_eq!(unwrap(b"something else entirely"), Err("not a cache file"));
    }
}
