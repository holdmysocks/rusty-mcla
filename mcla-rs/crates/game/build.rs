// DXC, linked into the exe for DX12 (wgpu's static-dxc; render/mod.rs, GitHub #33), needs five C runtime formatting
// functions the UCRT mingw has only in its headers: csrc/dxc_crt.c gives them. Passed as an object of its own, not a
// library: the linker reads native libraries before DXC's, when nothing asks for these yet.
//
// The exe's manifest (csrc/mcla.manifest, D59): the mingw default (longPathAware, asInvoker) plus heapType
// SegmentHeap, so the process heap returns freed memory (DX12 held ~0.9 GB of free heap with the old heap; Windows 10
// 2004+, older builds ignore the setting; docs/PERFORMANCE.md 2026-11-18). ld refuses a second manifest beside gcc's
// default-manifest.o, so an empty object of that name is put first in gcc's search path (-B) and ours is linked in.
fn main() {
    println!("cargo:rerun-if-changed=csrc/dxc_crt.c");
    println!("cargo:rerun-if-changed=csrc/mcla.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
        for o in cc::Build::new().file("csrc/dxc_crt.c").compile_intermediates() {
            println!("cargo:rustc-link-arg-bins={}", o.display());
        }
        // (and the UCRT after it, for the entry points it calls)
        println!("cargo:rustc-link-arg-bins=-lucrt");
        manifest();
    }
}

fn manifest() {
    use std::path::PathBuf;
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let src = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("csrc/mcla.manifest");
    let rc = out.join("mcla_manifest.rc");
    std::fs::write(&rc, format!("1 24 \"{}\"\n", src.display().to_string().replace('\\', "/"))).unwrap();
    let obj = out.join("mcla_manifest.o");
    let windres = std::env::var("WINDRES").unwrap_or_else(|_| "windres".into());
    let ok = std::process::Command::new(&windres).args(["-O", "coff", "-o"]).arg(&obj).arg(&rc).status();
    assert!(matches!(ok, Ok(s) if s.success()), "{windres} failed on {}", rc.display());
    // the empty stand-in for gcc's default-manifest.o
    let dir = out.join("nomanifest");
    std::fs::create_dir_all(&dir).unwrap();
    let empty = out.join("empty.c");
    std::fs::write(&empty, "").unwrap();
    let o = cc::Build::new().file(&empty).compile_intermediates();
    std::fs::copy(&o[0], dir.join("default-manifest.o")).unwrap();
    println!("cargo:rustc-link-arg-bins=-B{}/", dir.display().to_string().replace('\\', "/"));
    println!("cargo:rustc-link-arg-bins={}", obj.display());
}
