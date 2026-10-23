//! The scene's multisampling, as the original's renderer has it (LIGHTING.md "Scene MSAA", UI.md "UI edge
//! smoothing"): the scene (shadow collector target, scene, fog, blobs, particles) is drawn into a multisampled
//! 1280 x 720 target - 2 samples while driving, 4 while the renderer's menu flag (+373) or garage flag (+393) is
//! set (sub_822E5ED8 -> BeginTiledRendering sub_8217A470; count -> D3DMULTISAMPLE at 0x82185704) - and resolved
//! before anything reads it: colour = the samples averaged, depth / stencil = sample 0 (resolve flags 0x14).
//! The count to use each frame is `crate::aamode::scene_samples(menu)`.
//!
//! wgpu fixes a pipeline's sample count and depth format, so every pipeline that draws into the scene is a
//! [`MsPipe`]: one pipeline per [`Variant`], built when first used (or ahead by [`warm`]) and kept. The variant
//! in force is a global the frame sets around its scene passes ([`set_scene`] / [`set_plain`]); `Deref` hands out
//! that variant's pipeline, so `pass.set_pipeline(&gpu.pipeline_opaque)` stays as it was.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

/// What a pipeline is built for: the plain 1-sample Depth32Float passes (environment cube, reflections) or the
/// scene pass with `samples` samples and the scene's depth-stencil format (stencil = the original's ids: 0 the
/// world, 1 the sky and ground dome, 4 k the k-th registered vehicle - PSStreakMotionBlur's matrices).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Variant {
    pub samples: u32,
    pub scene: bool,
}

pub const PLAIN: Variant = Variant { samples: 1, scene: false };

pub fn scene(samples: u32) -> Variant {
    Variant { samples: samples.clamp(1, 4), scene: true }
}

static DS_FORMAT: OnceLock<wgpu::TextureFormat> = OnceLock::new();

/// The scene's depth-stencil format: Depth32FloatStencil8 where the device has it, else Depth24PlusStencil8.
pub fn scene_ds_format() -> wgpu::TextureFormat {
    *DS_FORMAT.get_or_init(|| wgpu::TextureFormat::Depth24PlusStencil8)
}

/// Set once by `Gpu::base` from the device's features (before any pipeline is built).
pub fn set_scene_ds_format(f: wgpu::TextureFormat) {
    let _ = DS_FORMAT.set(f);
}

impl Variant {
    pub fn slot(self) -> usize {
        if !self.scene {
            return 0;
        }
        match self.samples {
            1 => 1,
            2 => 2,
            _ => 3,
        }
    }

    pub fn ms(self) -> wgpu::MultisampleState {
        wgpu::MultisampleState { count: self.samples, mask: !0, alpha_to_coverage_enabled: false }
    }

    pub fn depth_format(self) -> wgpu::TextureFormat {
        if self.scene {
            scene_ds_format()
        } else {
            wgpu::TextureFormat::Depth32Float
        }
    }

    /// The pipeline's depth state in this variant; `stencil_write`: the draw writes its stencil reference (the
    /// original's ALWAYS / REPLACE where the depth test passes), else the stencil is left alone.
    pub fn ds(self, depth_write_enabled: bool, depth_compare: wgpu::CompareFunction, stencil_write: bool, bias: wgpu::DepthBiasState) -> wgpu::DepthStencilState {
        let stencil = if self.scene && stencil_write {
            let face = wgpu::StencilFaceState { compare: wgpu::CompareFunction::Always, fail_op: wgpu::StencilOperation::Keep, depth_fail_op: wgpu::StencilOperation::Keep, pass_op: wgpu::StencilOperation::Replace };
            wgpu::StencilState { front: face, back: face, read_mask: 0xff, write_mask: 0xff }
        } else {
            Default::default()
        };
        wgpu::DepthStencilState { format: self.depth_format(), depth_write_enabled, depth_compare, stencil, bias }
    }
}

static CURRENT: AtomicU32 = AtomicU32::new(0);

/// The scene passes of this frame draw with `samples` samples (into `Gpu::scene_color` / `scene_ds`).
pub fn set_scene(samples: u32) {
    CURRENT.store(scene(samples).slot() as u32, Ordering::Relaxed);
}

/// Back to the plain passes (environment cube, reflections, ...).
pub fn set_plain() {
    CURRENT.store(0, Ordering::Relaxed);
}

pub fn current() -> Variant {
    match CURRENT.load(Ordering::Relaxed) {
        0 => PLAIN,
        1 => scene(1),
        2 => scene(2),
        _ => scene(4),
    }
}

type Build = Box<dyn Fn(&wgpu::Device, Variant) -> wgpu::RenderPipeline + Send + Sync>;

struct Inner {
    device: wgpu::Device,
    build: Build,
    slots: [OnceLock<wgpu::RenderPipeline>; 4],
    /// the variants this pipeline is ever drawn in (for `warm`): scene only, or plain too
    plain_too: bool,
}

impl Inner {
    fn at(&self, v: Variant) -> &wgpu::RenderPipeline {
        self.slots[v.slot()].get_or_init(|| (self.build)(&self.device, v))
    }
}

static REGISTRY: Mutex<Vec<Weak<Inner>>> = Mutex::new(Vec::new());

/// A pipeline per [`Variant`], built lazily and cached.
#[derive(Clone)]
pub struct MsPipe {
    inner: Arc<Inner>,
}

impl MsPipe {
    /// A scene pipeline (drawn only in the scene pass).
    pub fn new(device: &wgpu::Device, build: impl Fn(&wgpu::Device, Variant) -> wgpu::RenderPipeline + Send + Sync + 'static) -> MsPipe {
        Self::make(device, Box::new(build), false)
    }

    /// One drawn both in the scene pass and in the plain passes (the city's, the sky's).
    pub fn new_shared(device: &wgpu::Device, build: impl Fn(&wgpu::Device, Variant) -> wgpu::RenderPipeline + Send + Sync + 'static) -> MsPipe {
        Self::make(device, Box::new(build), true)
    }

    fn make(device: &wgpu::Device, build: Build, plain_too: bool) -> MsPipe {
        let inner = Arc::new(Inner { device: device.clone(), build, slots: Default::default(), plain_too });
        if let Ok(mut r) = REGISTRY.lock() {
            r.retain(|w| w.strong_count() > 0);
            r.push(Arc::downgrade(&inner));
        }
        MsPipe { inner }
    }

    pub fn at(&self, v: Variant) -> &wgpu::RenderPipeline {
        self.inner.at(v)
    }
}

impl std::ops::Deref for MsPipe {
    type Target = wgpu::RenderPipeline;
    fn deref(&self) -> &wgpu::RenderPipeline {
        self.inner.at(current())
    }
}

/// Build every registered pipeline's `variants` now (the plain one only for those drawn in plain passes).
pub fn warm_now(variants: &[Variant]) {
    let list: Vec<Arc<Inner>> = REGISTRY.lock().map(|r| r.iter().filter_map(|w| w.upgrade()).collect()).unwrap_or_default();
    for p in list {
        for &v in variants {
            if v.scene || p.plain_too {
                p.at(v);
            }
        }
    }
}

/// The same on a thread of its own (the menus' 4x set while the game drives at 2x), so that the first switch
/// does not wait for the driver's compiler.
pub fn warm(variants: Vec<Variant>) {
    let _ = std::thread::Builder::new().name("msaa warm".into()).spawn(move || warm_now(&variants));
}
