//! The disc's Bink movies (opening movie, attract movie), converted by
//! `tools/convert_video.py` into a list of JPEG frames (`video_pc/<name>.mcv`) and a
//! stereo `.mp3`. Frames are decoded as the clock reaches them and shown full screen.

use std::path::{Path, PathBuf};
use std::time::Instant;

pub struct Video {
    data: memmap2::Mmap,
    pub width: usize,
    pub height: usize,
    fps: f32,
    frames: usize,
    start: Instant,
    offset: f32,
    shown: Option<usize>,
    tex: Option<egui::TextureHandle>,
    /// (start, end, text)
    pub subs: Vec<(f32, f32, String)>,
    /// any key ends it (attract movie)
    pub any_key: bool,
    /// the sound track next to the frames
    pub audio: PathBuf,
    /// skipped: when the skip came (`skip`)
    skipped: Option<Instant>,
}

impl Video {
    pub fn open(dir: &Path, name: &str) -> Option<Video> {
        let file = std::fs::File::open(dir.join(format!("{name}.mcv"))).ok()?;
        let data = unsafe { memmap2::Mmap::map(&file).ok()? };
        if data.len() < 28 || &data[0..4] != b"MCV1" {
            return None;
        }
        let u = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().unwrap()) as usize;
        let (width, height, fps, frames) = (u(4), u(8), u(12) as f32 / 1000.0, u(16));
        if frames == 0 || fps <= 0.0 || data.len() < 20 + (frames + 1) * 8 {
            return None;
        }
        log::info!("video: {} {}x{} {:.0} fps, {:.1} s", name, width, height, fps, frames as f32 / fps);
        Some(Video { data, width, height, fps, frames, start: Instant::now(), offset: 0.0, shown: None, tex: None, subs: Vec::new(), any_key: false, audio: dir.join(format!("{name}.mp3")), skipped: None })
    }

    /// Restart the clock (call when the sound starts); `at` skips ahead.
    pub fn restart(&mut self, at: f32) {
        self.start = Instant::now();
        self.offset = at;
    }

    pub fn time(&self) -> f32 {
        self.start.elapsed().as_secs_f32() + self.offset
    }

    pub fn duration(&self) -> f32 {
        self.frames as f32 / self.fps
    }

    pub fn subtitle(&self) -> Option<&str> {
        let t = self.time();
        self.subs.iter().find(|s| t >= s.0 && t < s.1).map(|s| s.2.as_str())
    }

    fn frame(&self, i: usize) -> &[u8] {
        let off = |k: usize| u64::from_le_bytes(self.data[20 + k * 8..28 + k * 8].try_into().unwrap()) as usize;
        let (a, b) = (off(i).min(self.data.len()), off(i + 1).min(self.data.len()));
        &self.data[a..b.max(a)]
    }

    /// The opening film skipped (CarSelection 0xa02): the movie's tint alpha from 1 down by 0.1 every 30 ms
    /// while it plays on (the first step at once; the eleventh, below zero, ends the loop: 0.33 s), then
    /// MoviePlayer_StopMovie.
    pub fn skip(&mut self) {
        self.skipped.get_or_insert_with(Instant::now);
    }

    /// Draw the frame for the current time over the whole screen; false when the movie is over.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let i = (self.time() * self.fps) as usize;
        let steps = self.skipped.map(|s| 1 + (s.elapsed().as_secs_f32() / 0.03) as u32);
        if i >= self.frames || steps.map_or(false, |n| n > 11) {
            return false;
        }
        let tint = steps.map_or(1.0, |n| (1.0 - 0.1 * n as f32).max(0.0));
        if self.shown != Some(i) {
            if let Ok(img) = mcla_formats::jpeg::decode(self.frame(i)) {
                let ci = egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.rgba);
                match self.tex.as_mut() {
                    Some(t) => t.set(ci, egui::TextureOptions::LINEAR),
                    None => self.tex = Some(ctx.load_texture("movie", ci, egui::TextureOptions::LINEAR)),
                }
                self.shown = Some(i);
            }
        }
        let screen = ctx.screen_rect();
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("movie")));
        p.rect_filled(screen, 0.0, egui::Color32::BLACK);
        if let Some(t) = self.tex.as_ref() {
            // fit, keeping the picture's shape
            let s = (screen.width() / self.width as f32).min(screen.height() / self.height as f32);
            let rect = egui::Rect::from_center_size(screen.center(), egui::vec2(self.width as f32 * s, self.height as f32 * s));
            crate::hud::image(&p, t, rect, egui::Color32::from_white_alpha((tint * 255.0).round() as u8));
        }
        ctx.request_repaint();
        true
    }
}
