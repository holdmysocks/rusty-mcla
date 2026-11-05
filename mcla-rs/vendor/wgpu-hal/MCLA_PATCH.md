# wgpu-hal 24.0.4, patched for Rusty MCLA

The crates.io release `wgpu-hal 24.0.4` (git 993fc39b07e8a4418a15043265298d7bc016d059, MIT / Apache-2.0, licence
files kept), used through `[patch.crates-io]` in the workspace `Cargo.toml`. The examples are removed; nothing else
in the crate changes except the following.

## The DXIL disk cache (GitHub #33)

wgpu 24 has no pipeline cache on DX12, so DXC compiles every program again at each start. The city's general
fragment program alone takes 14 s in DXC, and an AMD RX 5500 XT player waited about a minute through the loading.

- `src/dx12/shader_compilation.rs` `compile_dxc` first looks for `<MCLA_DXIL_CACHE>/<hash>.dxil`. The hash is two
  64-bit FNV-1a over the cache version string (this crate's and DXC's versions), the HLSL text, the entry point,
  the profile and the debug / validator flags. A file that is a whole DXIL container ("DXBC", the size at +24 =
  the file length) is used as it is. After a compile (and validation), the DXIL is written to a temporary file and
  renamed.
- `src/dx12/mod.rs` `CompiledShader::Bytes(Vec<u8>)` carries the read file.
- When `MCLA_DXIL_CACHE` is unset or empty, nothing changes. The game sets it to
  `<user folder>\shadercache\dx12-<version>` (`crates/game/src/render/mod.rs` `dxil_cache_dir`).

When wgpu is upgraded: drop this folder and the patch line, and carry the cache over if the new wgpu still has no
DX12 pipeline cache.
