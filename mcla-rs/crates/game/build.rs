// DXC, linked into the exe for DX12 (wgpu's static-dxc; render/mod.rs, GitHub #33), needs five C runtime formatting
// functions the UCRT mingw has only in its headers: csrc/dxc_crt.c gives them. Passed as an object of its own, not a
// library: the linker reads native libraries before DXC's, when nothing asks for these yet.
fn main() {
    println!("cargo:rerun-if-changed=csrc/dxc_crt.c");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
        for o in cc::Build::new().file("csrc/dxc_crt.c").compile_intermediates() {
            println!("cargo:rustc-link-arg-bins={}", o.display());
        }
        // (and the UCRT after it, for the entry points it calls)
        println!("cargo:rustc-link-arg-bins=-lucrt");
    }
}
