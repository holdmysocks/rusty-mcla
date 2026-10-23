//! Loading behind the start-up movie (docs/reference/ui/boot.md, "Loading behind the movie").
//!
//! The original plays its `legals` movie while it loads: the movie runs its own timeline, holds on LOADING (its frame
//! 662 loop) until the press-start script calls `Game_SetLoadingDone`, and only then fades its black background off
//! the city. The port does the same: the window, the device and the surface, the UI's renderer, the audio output and
//! the movie are made first (a second or two), then everything else - the renderer's programs (`Gpu::build`), the
//! world, the cars, the scripts, speech, props, the UI's art and movies - is made on a loading thread while the main
//! thread draws the movie at its 30 frames a second. When the thread is done the game takes over the window and the
//! movie (`App::update` says loading is done once no sector is waiting to be read).
//!
//! The intro OFF (skip), headless runs (`--screenshot`) and `--auto` / `--race-select` / `--garage` load everything
//! at once before the first frame, as before (`App::start_at_once`). `MCLA_BOOT_LOAD=0` does that with the movie too.

use crate::{audio, boot, render, sound, App, Egui};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

/// Hands a value to another thread that does not move it back while it is there: the game (`App`) goes to the
/// loading thread whole and comes back whole; nothing on the main thread touches it in between, and what it holds
/// that is tied to a thread (the audio output, the window's UI state) stays on the main thread (`Loading`).
struct SendBox<T>(T);
// SAFETY: see above - the value is used by one thread at a time, and the parts of `App` that must stay on the main
// thread (audio output, egui-winit state, the window) are taken out of it before it is sent.
unsafe impl<T> Send for SendBox<T> {}

/// The event loop's handler: the game, or - while it loads on its thread - the start-up movie.
pub struct Runner {
    app: Option<Box<App>>,
    loading: Option<Loading>,
}

struct Loading {
    window: Arc<Window>,
    egui: Egui,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: Arc<wgpu::Surface<'static>>,
    config: wgpu::SurfaceConfiguration,
    depth: wgpu::TextureView,
    boot: boot::Boot,
    audio: Option<audio::Audio>,
    /// the sound library, read on a thread of its own (the loading music plays through it)
    sound: Option<sound::Preloaded>,
    sound_rx: Option<Receiver<SendBox<sound::Preloaded>>>,
    music: Option<crate::sndobj::Handle>,
    job: Option<std::thread::JoinHandle<SendBox<Box<App>>>>,
    started: Instant,
    last: Instant,
    frames: u32,
    /// the longest frame while loading (s) and the frames over 50 ms
    worst: f32,
    long: u32,
}

impl Runner {
    pub fn new(app: App) -> Runner {
        Runner { app: Some(Box::new(app)), loading: None }
    }

    fn start(&mut self, event_loop: &ActiveEventLoop) {
        let t0 = Instant::now();
        let mut app = self.app.take().expect("app");
        let window = app.open_window(event_loop);
        let base = pollster::block_on(render::Gpu::base(window.clone())).expect("gpu init");
        log::info!("startup: device and surface at {} ms", t0.elapsed().as_millis());
        let egui = App::make_egui(&window, &base.device, &base.queue, base.config.format);
        let data_dir = app.opt.data_dir.clone();
        let audio = audio::Audio::new(&data_dir);
        let root = audio.as_ref().map(|a| a.root().to_path_buf());
        // the sound library (the loading music, then the game's): on a thread of its own
        let sound_rx = audio.as_ref().map(|a| {
            let (tx, rx) = channel();
            let (root, cfg) = (a.root().to_path_buf(), a.cfg.clone());
            std::thread::spawn(move || {
                let t = Instant::now();
                let p = sound::Preloaded::load(&root, &cfg, true);
                log::info!("startup: sound library read in {} ms", t.elapsed().as_millis());
                let _ = tx.send(SendBox(p));
            });
            rx
        });
        let mut boot = boot::Boot::new(app.boot_mode()).expect("intro on");
        boot.load(&data_dir, &boot::shared(&data_dir));
        let depth = depth_view(&base.device, base.config.width, base.config.height);
        let (device, queue, surface, config) = (base.device.clone(), base.queue.clone(), base.surface.clone(), base.config.clone());
        let ctx = egui.ctx.clone();
        let sent = SendBox(app);
        let job = std::thread::Builder::new()
            .name("loading".into())
            .stack_size(64 << 20)
            .spawn(move || {
                let sent = sent;
                let SendBox(mut app) = sent;
                let t = Instant::now();
                let gpu = render::Gpu::build(base);
                log::info!("startup: renderer programs made in {} ms (loading thread)", t.elapsed().as_millis());
                app.load_game(gpu, root.as_deref(), Some(&ctx));
                log::info!("startup: loading thread done in {} ms", t.elapsed().as_millis());
                SendBox(app)
            })
            .expect("loading thread");
        log::info!("startup: start-up movie up at {} ms, loading behind it", t0.elapsed().as_millis());
        let now = Instant::now();
        self.loading = Some(Loading { window, egui, device, queue, surface, config, depth, boot, audio, sound: None, sound_rx, music: None, job: Some(job), started: now, last: now, frames: 0, worst: 0.0, long: 0 });
    }

    /// The loading thread is done: the game takes the window, the UI, the audio and the movie.
    fn finish(&mut self) {
        let Some(mut l) = self.loading.take() else { return };
        let SendBox(mut app) = l.job.take().expect("job").join().expect("loading thread");
        if l.sound.is_none() {
            l.sound = l.sound_rx.take().and_then(|rx| rx.recv().ok()).map(|b| b.0);
        }
        if let Some(p) = l.sound.take() {
            app.sound.adopt(p);
            if let Some(h) = l.music {
                app.sound.keep_loop(boot::LOADING_MUSIC, h);
            }
        }
        app.audio = l.audio.take();
        if let Some(a) = app.audio.as_mut() {
            a.set_music_catalog(&app.game.career.strings);
        }
        if let Some(g) = app.gpu.as_mut() {
            let s = l.window.inner_size();
            if (s.width, s.height) != (g.config.width, g.config.height) {
                g.resize(s.width, s.height);
            }
        }
        log::info!(
            "startup: loading done after {:.1} s behind the movie (movie timer {}); {} movie frames, longest {:.0} ms, {} over 50 ms",
            l.started.elapsed().as_secs_f32(),
            boot::movie_timer(&l.boot),
            l.frames,
            l.worst * 1000.0,
            l.long
        );
        app.game.boot = Some(l.boot);
        app.egui = Some(l.egui);
        app.window = Some(l.window.clone());
        app.last = Instant::now();
        l.window.request_redraw();
        self.app = Some(app);
    }
}

impl Loading {
    fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
        self.depth = depth_view(&self.device, w, h);
    }

    /// One frame of the movie (and the loading music).
    fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32();
        // (without vsync: no faster than 120 frames a second)
        if dt < 1.0 / 120.0 {
            std::thread::sleep(std::time::Duration::from_secs_f32(1.0 / 120.0 - dt));
        }
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32();
        self.last = now;
        if self.frames > 0 {
            if dt > self.worst {
                self.worst = dt;
            }
            if dt > 0.05 {
                self.long += 1;
                log::info!("startup: movie frame {} took {:.0} ms", self.frames, dt * 1000.0);
            }
        }
        self.frames += 1;
        // the movie: loading is not done while the thread runs
        self.boot.update(dt, false, false);
        // the loading music from the movie's frame 340 (`FSCommand:StartLoadingMusic`), as the game plays it
        if let (Some(a), Some(lib)) = (self.audio.as_mut(), self.sound.as_mut().and_then(|p| p.lib.as_mut())) {
            if self.boot.music() && self.music.is_none() {
                self.music = lib.play(boot::LOADING_MUSIC, crate::sndobj::Req::default());
            }
            lib.update(dt.min(0.25), a);
        }
        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                let (w, h) = (self.config.width, self.config.height);
                self.resize(w, h);
                return;
            }
            Err(e) => {
                log::warn!("surface: {:?}", e);
                return;
            }
        };
        // (the UI blends in gamma space through the surface's plain view, as `App::render` draws it)
        let ui_view = frame.texture.create_view(&wgpu::TextureViewDescriptor { format: Some(self.config.format.remove_srgb_suffix()), ..Default::default() });
        let eg = &mut self.egui;
        let raw = eg.state.take_egui_input(&self.window);
        let boot = &mut self.boot;
        let out = eg.ctx.run(raw, |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("menu")));
            boot.paint(ctx, &painter);
        });
        eg.state.handle_platform_output(&self.window, out.platform_output);
        let jobs = eg.ctx.tessellate(out.shapes, out.pixels_per_point);
        for (id, delta) in &out.textures_delta.set {
            eg.renderer.update_texture(&self.device, &self.queue, *id, delta);
        }
        let desc = egui_wgpu::ScreenDescriptor { size_in_pixels: [self.config.width, self.config.height], pixels_per_point: out.pixels_per_point };
        let mut enc = self.device.create_command_encoder(&Default::default());
        let cmds = eg.renderer.update_buffers(&self.device, &self.queue, &mut enc, &jobs, &desc);
        {
            let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("start-up movie"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &ui_view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &self.depth, depth_ops: None, stencil_ops: None }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            let mut pass = pass.forget_lifetime();
            eg.renderer.render(&mut pass, &jobs, &desc);
        }
        self.queue.submit(cmds.into_iter().chain(std::iter::once(enc.finish())));
        frame.present();
        for id in &out.textures_delta.free {
            eg.renderer.free_texture(id);
        }
    }

    /// The loading thread and the sound library's thread are done.
    fn done(&mut self) -> bool {
        if self.sound.is_none() {
            match self.sound_rx.as_ref().map(|rx| rx.try_recv()) {
                Some(Ok(b)) => self.sound = Some(b.0),
                // (its thread failed: the game reads the library itself at its first sound update)
                Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => self.sound_rx = None,
                _ => {}
            }
        }
        let sound_ready = self.sound.is_some() || self.sound_rx.is_none();
        sound_ready && self.job.as_ref().map_or(true, |j| j.is_finished())
    }
}

fn depth_view(device: &wgpu::Device, w: u32, h: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("start-up movie depth"),
            size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

impl ApplicationHandler for Runner {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.loading.is_some() {
            return;
        }
        let Some(app) = self.app.as_mut() else { return };
        if app.window.is_some() || !app.loads_behind_movie() {
            app.resumed(event_loop);
            return;
        }
        self.start(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let Some(app) = self.app.as_mut() {
            app.window_event(event_loop, id, event);
            return;
        }
        let Some(l) = self.loading.as_mut() else { return };
        let _ = l.egui.state.on_window_event(&l.window, &event);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(s) => l.resize(s.width, s.height),
            WindowEvent::RedrawRequested => {
                l.frame();
                if l.done() {
                    self.finish();
                } else {
                    l.window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, device_id: winit::event::DeviceId, event: DeviceEvent) {
        if let Some(app) = self.app.as_mut() {
            app.device_event(event_loop, device_id, event);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(app) = self.app.as_mut() {
            app.about_to_wait(event_loop);
        } else if let Some(l) = self.loading.as_ref() {
            l.window.request_redraw();
        }
    }
}
