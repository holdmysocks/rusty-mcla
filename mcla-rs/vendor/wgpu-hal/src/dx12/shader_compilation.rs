use crate::auxil::dxgi::result::HResult;
use std::ffi::CStr;
use std::path::PathBuf;
use thiserror::Error;
use windows::{
    core::{Interface, PCSTR, PCWSTR},
    Win32::Graphics::Direct3D::{Dxc, Fxc},
};

// Currently this will use Dxc if it is chosen as the dx12 compiler at `Instance` creation time, and will
// fallback to FXC if the Dxc libraries (dxil.dll and dxcompiler.dll) are not found, or if Fxc is chosen at'
// `Instance` creation time.

pub(super) fn compile_fxc(
    device: &super::Device,
    source: &str,
    source_name: Option<&CStr>,
    raw_ep: &str,
    stage_bit: wgt::ShaderStages,
    full_stage: &str,
) -> Result<super::CompiledShader, crate::PipelineError> {
    profiling::scope!("compile_fxc");
    let mut shader_data = None;
    let mut compile_flags = Fxc::D3DCOMPILE_ENABLE_STRICTNESS;
    if device
        .private_caps
        .instance_flags
        .contains(wgt::InstanceFlags::DEBUG)
    {
        compile_flags |= Fxc::D3DCOMPILE_DEBUG | Fxc::D3DCOMPILE_SKIP_OPTIMIZATION;
    }

    let raw_ep = std::ffi::CString::new(raw_ep).unwrap();
    let full_stage = std::ffi::CString::new(full_stage).unwrap();

    // If no name has been set, D3DCompile wants the null pointer.
    let source_name = source_name
        .map(|cstr| cstr.as_ptr().cast())
        .unwrap_or(core::ptr::null());

    let mut error = None;
    let hr = unsafe {
        profiling::scope!("Fxc::D3DCompile");
        Fxc::D3DCompile(
            // TODO: Update low-level bindings to accept a slice here
            source.as_ptr().cast(),
            source.len(),
            PCSTR(source_name),
            None,
            None,
            PCSTR(raw_ep.as_ptr().cast()),
            PCSTR(full_stage.as_ptr().cast()),
            compile_flags,
            0,
            &mut shader_data,
            Some(&mut error),
        )
    };

    match hr {
        Ok(()) => {
            let shader_data = shader_data.unwrap();
            Ok(super::CompiledShader::Fxc(shader_data))
        }
        Err(e) => {
            let mut full_msg = format!("FXC D3DCompile error ({e})");
            if let Some(error) = error {
                use std::fmt::Write as _;
                let message = unsafe {
                    std::slice::from_raw_parts(
                        error.GetBufferPointer().cast(),
                        error.GetBufferSize(),
                    )
                };
                let _ = write!(full_msg, ": {}", String::from_utf8_lossy(message));
            }
            Err(crate::PipelineError::Linkage(stage_bit, full_msg))
        }
    }
}

trait DxcObj: Interface {
    const CLSID: windows::core::GUID;
}
impl DxcObj for Dxc::IDxcCompiler3 {
    const CLSID: windows::core::GUID = Dxc::CLSID_DxcCompiler;
}
impl DxcObj for Dxc::IDxcUtils {
    const CLSID: windows::core::GUID = Dxc::CLSID_DxcUtils;
}
impl DxcObj for Dxc::IDxcValidator {
    const CLSID: windows::core::GUID = Dxc::CLSID_DxcValidator;
}

#[derive(Debug)]
struct DxcLib {
    lib: crate::dx12::DynLib,
}

impl DxcLib {
    fn new_dynamic(lib_path: PathBuf) -> Result<Self, libloading::Error> {
        unsafe { crate::dx12::DynLib::new(lib_path).map(|lib| Self { lib }) }
    }

    pub fn create_instance<T: DxcObj>(&self) -> Result<T, crate::DeviceError> {
        unsafe {
            type DxcCreateInstanceFn = unsafe extern "system" fn(
                rclsid: *const windows_core::GUID,
                riid: *const windows_core::GUID,
                ppv: *mut *mut core::ffi::c_void,
            )
                -> windows_core::HRESULT;

            let func: libloading::Symbol<DxcCreateInstanceFn> =
                self.lib.get(b"DxcCreateInstance\0")?;
            dxc_create_instance::<T>(|clsid, iid, ppv| func(clsid, iid, ppv))
        }
    }
}

/// Invokes the provided library function to create a DXC object.
unsafe fn dxc_create_instance<T: DxcObj>(
    f: impl Fn(
        *const windows_core::GUID,
        *const windows_core::GUID,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
) -> Result<T, crate::DeviceError> {
    let mut result__ = None;
    f(&T::CLSID, &T::IID, <*mut _>::cast(&mut result__))
        .ok()
        .into_device_result("DxcCreateInstance")?;
    result__.ok_or(crate::DeviceError::Unexpected)
}

// Destructor order should be fine since _dxil and _dxc don't rely on each other.
pub(super) struct DxcContainer {
    compiler: Dxc::IDxcCompiler3,
    utils: Dxc::IDxcUtils,
    validator: Option<Dxc::IDxcValidator>,
    // Has to be held onto for the lifetime of the device otherwise shaders will fail to compile.
    // Only needed when using dynamic linking.
    _dxc: Option<DxcLib>,
    // Also Has to be held onto for the lifetime of the device otherwise shaders will fail to validate.
    // Only needed when using dynamic linking.
    _dxil: Option<DxcLib>,
}

#[derive(Debug, Error)]
pub(super) enum GetDynamicDXCContainerError {
    #[error(transparent)]
    Device(#[from] crate::DeviceError),
    #[error("Failed to load {0}: {1}")]
    FailedToLoad(&'static str, libloading::Error),
}

pub(super) fn get_dynamic_dxc_container(
    dxc_path: PathBuf,
    dxil_path: PathBuf,
) -> Result<DxcContainer, GetDynamicDXCContainerError> {
    let dxc = DxcLib::new_dynamic(dxc_path)
        .map_err(|e| GetDynamicDXCContainerError::FailedToLoad("dxcompiler.dll", e))?;

    let dxil = DxcLib::new_dynamic(dxil_path)
        .map_err(|e| GetDynamicDXCContainerError::FailedToLoad("dxil.dll", e))?;

    let compiler = dxc.create_instance::<Dxc::IDxcCompiler3>()?;
    let utils = dxc.create_instance::<Dxc::IDxcUtils>()?;
    let validator = dxil.create_instance::<Dxc::IDxcValidator>()?;

    Ok(DxcContainer {
        compiler,
        utils,
        validator: Some(validator),
        _dxc: Some(dxc),
        _dxil: Some(dxil),
    })
}

/// Creates a [`DxcContainer`] that delegates to the statically-linked version of DXC.
pub(super) fn get_static_dxc_container() -> Result<DxcContainer, crate::DeviceError> {
    #[cfg(static_dxc)]
    {
        unsafe {
            let compiler = dxc_create_instance::<Dxc::IDxcCompiler3>(|clsid, iid, ppv| {
                windows_core::HRESULT(mach_dxcompiler_rs::DxcCreateInstance(
                    clsid.cast(),
                    iid.cast(),
                    ppv,
                ))
            })?;
            let utils = dxc_create_instance::<Dxc::IDxcUtils>(|clsid, iid, ppv| {
                windows_core::HRESULT(mach_dxcompiler_rs::DxcCreateInstance(
                    clsid.cast(),
                    iid.cast(),
                    ppv,
                ))
            })?;

            Ok(DxcContainer {
                compiler,
                utils,
                validator: None,
                _dxc: None,
                _dxil: None,
            })
        }
    }
    #[cfg(not(static_dxc))]
    {
        panic!("Attempted to create a static DXC shader compiler, but the static-dxc feature was not enabled")
    }
}

/// Owned PCWSTR
#[allow(clippy::upper_case_acronyms)]
struct OPCWSTR {
    inner: Vec<u16>,
}

impl OPCWSTR {
    fn new(s: &str) -> Self {
        let mut inner: Vec<_> = s.encode_utf16().collect();
        inner.push(0);
        Self { inner }
    }

    fn ptr(&self) -> PCWSTR {
        PCWSTR(self.inner.as_ptr())
    }
}

fn get_output<T: Interface>(
    res: &Dxc::IDxcResult,
    kind: Dxc::DXC_OUT_KIND,
) -> Result<T, crate::DeviceError> {
    let mut result__: Option<T> = None;
    unsafe { res.GetOutput::<T>(kind, &mut None, <*mut _>::cast(&mut result__)) }
        .into_device_result("GetOutput")?;
    result__.ok_or(crate::DeviceError::Unexpected)
}

fn as_err_str(blob: &Dxc::IDxcBlobUtf8) -> Result<&str, crate::DeviceError> {
    let ptr = unsafe { blob.GetStringPointer() };
    let len = unsafe { blob.GetStringLength() };
    core::str::from_utf8(unsafe { core::slice::from_raw_parts(ptr.0, len) })
        .map_err(|_| crate::DeviceError::Unexpected)
}

pub(super) fn compile_dxc(
    device: &crate::dx12::Device,
    source: &str,
    source_name: Option<&CStr>,
    raw_ep: &str,
    stage_bit: wgt::ShaderStages,
    full_stage: &str,
    dxc_container: &DxcContainer,
) -> Result<crate::dx12::CompiledShader, crate::PipelineError> {
    profiling::scope!("compile_dxc");

    // (MCLA patch: the DXIL disk cache, see vendor/wgpu-hal/MCLA_PATCH.md)
    let cache = dxil_cache::path(
        source,
        raw_ep,
        full_stage,
        device
            .private_caps
            .instance_flags
            .contains(wgt::InstanceFlags::DEBUG),
        dxc_container.validator.is_some(),
    );
    if let Some(bytes) = cache.as_deref().and_then(dxil_cache::load) {
        return Ok(crate::dx12::CompiledShader::Bytes(bytes));
    }

    let source_name = source_name.and_then(|cstr| cstr.to_str().ok());

    let source_name = source_name.map(OPCWSTR::new);
    let raw_ep = OPCWSTR::new(raw_ep);
    let full_stage = OPCWSTR::new(full_stage);

    let mut compile_args = arrayvec::ArrayVec::<PCWSTR, 12>::new_const();

    if let Some(source_name) = source_name.as_ref() {
        compile_args.push(source_name.ptr())
    }

    compile_args.extend([
        windows::core::w!("-E"),
        raw_ep.ptr(),
        windows::core::w!("-T"),
        full_stage.ptr(),
        windows::core::w!("-HV"),
        windows::core::w!("2018"), // Use HLSL 2018, Naga doesn't supported 2021 yet.
        windows::core::w!("-no-warnings"),
        Dxc::DXC_ARG_ENABLE_STRICTNESS,
    ]);

    if dxc_container.validator.is_some() {
        compile_args.push(Dxc::DXC_ARG_SKIP_VALIDATION); // Disable implicit validation to work around bugs when dxil.dll isn't in the local directory.)
    }

    if device
        .private_caps
        .instance_flags
        .contains(wgt::InstanceFlags::DEBUG)
    {
        compile_args.push(Dxc::DXC_ARG_DEBUG);
        compile_args.push(Dxc::DXC_ARG_SKIP_OPTIMIZATIONS);
    }

    let buffer = Dxc::DxcBuffer {
        Ptr: source.as_ptr().cast(),
        Size: source.len(),
        Encoding: Dxc::DXC_CP_UTF8.0,
    };

    let compile_res: Dxc::IDxcResult = unsafe {
        dxc_container
            .compiler
            .Compile(&buffer, Some(&compile_args), None)
    }
    .into_device_result("Compile")?;

    drop(compile_args);
    drop(source_name);
    drop(raw_ep);
    drop(full_stage);

    let err_blob = get_output::<Dxc::IDxcBlobUtf8>(&compile_res, Dxc::DXC_OUT_ERRORS)?;

    let len = unsafe { err_blob.GetStringLength() };
    if len != 0 {
        let err = as_err_str(&err_blob)?;
        return Err(crate::PipelineError::Linkage(
            stage_bit,
            format!("DXC compile error: {err}"),
        ));
    }

    let blob = get_output::<Dxc::IDxcBlob>(&compile_res, Dxc::DXC_OUT_OBJECT)?;

    if let Some(validator) = &dxc_container.validator {
        let err_blob = {
            let res = unsafe { validator.Validate(&blob, Dxc::DxcValidatorFlags_InPlaceEdit) }
                .into_device_result("Validate")?;

            unsafe { res.GetErrorBuffer() }.into_device_result("GetErrorBuffer")?
        };

        let size = unsafe { err_blob.GetBufferSize() };
        if size != 0 {
            let err_blob = unsafe { dxc_container.utils.GetBlobAsUtf8(&err_blob) }
                .into_device_result("GetBlobAsUtf8")?;
            let err = as_err_str(&err_blob)?;
            return Err(crate::PipelineError::Linkage(
                stage_bit,
                format!("DXC validation error: {err}"),
            ));
        }
    }

    if let Some(path) = cache.as_deref() {
        let bytes = unsafe {
            core::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize())
        };
        dxil_cache::store(path, bytes);
    }

    Ok(crate::dx12::CompiledShader::Dxc(blob))
}

/// MCLA patch: compiled DXIL kept on disk, so a second start skips DXC (GitHub #33: the city's general fragment
/// program alone takes 14 s in DXC). On only when `MCLA_DXIL_CACHE` names a folder; the file name is a 128-bit hash
/// of everything that goes into the compile (the HLSL text, entry point, profile, flags, this crate's and DXC's
/// versions); a file that is not a whole DXIL container is ignored and compiled again.
mod dxil_cache {
    use std::path::{Path, PathBuf};

    const VERSION: &str = concat!("mcla-dxil-1 wgpu-hal ", env!("CARGO_PKG_VERSION"), " mach-dxcompiler 0.1.7+2026.10.26-141dd7f.1");

    fn fnv(seed: u64, parts: &[&[u8]]) -> u64 {
        let mut h = seed;
        for p in parts {
            for &b in p.iter().chain((p.len() as u64).to_le_bytes().iter()) {
                h ^= b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        h
    }

    pub(super) fn path(source: &str, ep: &str, stage: &str, debug: bool, validator: bool) -> Option<PathBuf> {
        let dir = std::env::var_os("MCLA_DXIL_CACHE").filter(|d| !d.is_empty())?;
        let flags = [debug as u8, validator as u8];
        let parts: [&[u8]; 5] = [VERSION.as_bytes(), source.as_bytes(), ep.as_bytes(), stage.as_bytes(), &flags];
        let a = fnv(0xcbf2_9ce4_8422_2325, &parts);
        let b = fnv(0x6c62_272e_07bb_0142, &parts);
        Some(PathBuf::from(dir).join(format!("{a:016x}{b:016x}.dxil")))
    }

    /// The file's bytes when they are one DXIL container: "DXBC" and the container's own size (+24) = the length.
    pub(super) fn load(path: &Path) -> Option<Vec<u8>> {
        let bytes = std::fs::read(path).ok()?;
        let whole = bytes.len() >= 32
            && &bytes[..4] == b"DXBC"
            && u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]) as usize == bytes.len();
        if !whole {
            log::warn!("DXIL cache: {} is not a whole container, compiled again", path.display());
            return None;
        }
        Some(bytes)
    }

    /// Written to a file of its own and renamed, so a start that stops half way or a second thread never leaves a part.
    pub(super) fn store(path: &Path, bytes: &[u8]) {
        let Some(dir) = path.parent() else { return };
        let tmp = dir.join(format!(
            "{}.{}.{:?}.tmp",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("x"),
            std::process::id(),
            std::thread::current().id()
        ));
        let res = std::fs::create_dir_all(dir)
            .and_then(|_| std::fs::write(&tmp, bytes))
            .and_then(|_| std::fs::rename(&tmp, path));
        if let Err(e) = res {
            let _ = std::fs::remove_file(&tmp);
            log::warn!("DXIL cache: could not write {}: {e}", path.display());
        }
    }
}
