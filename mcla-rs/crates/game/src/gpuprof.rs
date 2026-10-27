//! The GPU time of each part of the frame's passes (main.rs `render`), from timestamps written between them, averaged
//! over 240 frames and logged as `gputimes:` (docs/PERFORMANCE.md, issue #12: which pass a slow card spends its time in).
//! Read back a few frames later without waiting, so the frame is not held up. `MCLA_GPU_TIMES=0`: off.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

const MAX: u32 = 32;
/// read-back buffers in turn: a frame's stamps are read while the next frames run
const RING: usize = 3;

struct Read {
    buf: wgpu::Buffer,
    /// the stamps' names of the frame it holds; empty: free
    names: Vec<&'static str>,
    done: Arc<AtomicBool>,
    mapping: bool,
}

/// (shared: a worker thread writes the stamp `slot` gave it into its own command buffer)
static SET: OnceLock<wgpu::QuerySet> = OnceLock::new();

struct Prof {
    resolve: wgpu::Buffer,
    reads: Vec<Read>,
    names: Vec<&'static str>,
    /// the read-back buffer this frame's stamps go to (`end`), mapped after the submit
    pending: Option<usize>,
    sums: Vec<(&'static str, f64)>,
    frames: u32,
    period: f32,
}

thread_local! {
    static P: RefCell<Option<Prof>> = const { RefCell::new(None) };
}

/// The features the device needs for it (none when the adapter has not got them: no `gputimes:` then).
pub fn features(adapter: wgpu::Features) -> wgpu::Features {
    let f = wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
    if std::env::var("MCLA_GPU_TIMES").map_or(false, |v| v.trim() == "0") || !adapter.contains(f) {
        return wgpu::Features::empty();
    }
    f | (adapter & wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES)
}

/// A frame's stamps start (what a frame left unended is dropped).
pub fn begin() {
    P.with(|p| {
        if let Some(p) = p.borrow_mut().as_mut() {
            p.names.clear();
        }
    });
}

/// The part `name` starts here, in `enc` (the stamps of a frame in the order the GPU runs them).
pub fn stamp(device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, name: &'static str) {
    write(enc, slot(device, queue, name));
}

/// The part `name` starts where `write` puts the stamp - in a command buffer a worker records, submitted where the part
/// runs among the frame's others (the stamps' order is the order of `slot` / `stamp` calls).
pub fn slot(device: &wgpu::Device, queue: &wgpu::Queue, name: &'static str) -> Option<u32> {
    if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS) {
        return None;
    }
    SET.get_or_init(|| device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("gpu times"), ty: wgpu::QueryType::Timestamp, count: MAX }));
    P.with(|p| {
        let mut p = p.borrow_mut();
        let p = p.get_or_insert_with(|| Prof {
            resolve: device.create_buffer(&wgpu::BufferDescriptor { label: Some("gpu times"), size: MAX as u64 * 8, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false }),
            reads: (0..RING)
                .map(|_| Read {
                    buf: device.create_buffer(&wgpu::BufferDescriptor { label: Some("gpu times read"), size: MAX as u64 * 8, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false }),
                    names: Vec::new(),
                    done: Arc::new(AtomicBool::new(false)),
                    mapping: false,
                })
                .collect(),
            names: Vec::new(),
            pending: None,
            sums: Vec::new(),
            frames: 0,
            period: queue.get_timestamp_period(),
        });
        if (p.names.len() as u32) < MAX {
            p.names.push(name);
            Some(p.names.len() as u32 - 1)
        } else {
            None
        }
    })
}

/// The stamp `slot` gave, into `enc` (any thread).
pub fn write(enc: &mut wgpu::CommandEncoder, slot: Option<u32>) {
    if let (Some(i), Some(set)) = (slot, SET.get()) {
        enc.write_timestamp(set, i);
    }
}

/// The same inside a render pass (`stamp` made the query set before it); nothing where the device cannot.
pub fn stamp_pass(device: &wgpu::Device, pass: &mut wgpu::RenderPass<'_>, name: &'static str) {
    if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES) {
        return;
    }
    P.with(|p| {
        if let (Some(p), Some(set)) = (p.borrow_mut().as_mut(), SET.get()) {
            if (p.names.len() as u32) < MAX {
                pass.write_timestamp(set, p.names.len() as u32);
                p.names.push(name);
            }
        }
    });
}

/// The frame's last stamp, into the encoder submitted last: adds the frames read back since, and resolves this
/// frame's stamps into a free read-back buffer (none free: this frame is not counted).
pub fn end(device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder) {
    stamp(device, queue, enc, "");
    P.with(|p| {
        let mut p = p.borrow_mut();
        let Some(p) = p.as_mut() else { return };
        for i in 0..RING {
            if p.reads[i].mapping && p.reads[i].done.load(Ordering::Acquire) {
                let r = &mut p.reads[i];
                let n = r.names.len();
                let t: Vec<u64> = bytemuck::cast_slice(&r.buf.slice(..n as u64 * 8).get_mapped_range()).to_vec();
                r.buf.unmap();
                r.mapping = false;
                let names = std::mem::take(&mut r.names);
                p.add(&names, &t);
            }
        }
        let names = std::mem::take(&mut p.names);
        let n = names.len() as u32;
        if n < 2 {
            return;
        }
        if let Some(i) = (0..RING).find(|&i| p.reads[i].names.is_empty()) {
            enc.resolve_query_set(SET.get().expect("made by the first stamp"), 0..n, &p.resolve, 0);
            enc.copy_buffer_to_buffer(&p.resolve, 0, &p.reads[i].buf, 0, n as u64 * 8);
            p.reads[i].names = names;
            p.pending = Some(i);
        }
    });
}

/// After the frame's submit: its read-back buffer is mapped (read by a later `end`).
pub fn after_submit() {
    P.with(|p| {
        let mut p = p.borrow_mut();
        let Some(p) = p.as_mut() else { return };
        if let Some(i) = p.pending.take() {
            let r = &mut p.reads[i];
            r.done.store(false, Ordering::Release);
            r.mapping = true;
            let done = r.done.clone();
            let n = r.names.len() as u64;
            r.buf.slice(..n * 8).map_async(wgpu::MapMode::Read, move |res| done.store(res.is_ok(), Ordering::Release));
        }
    });
}

impl Prof {
    fn add(&mut self, names: &[&'static str], t: &[u64]) {
        for i in 0..names.len() - 1 {
            let ms = t[i + 1].saturating_sub(t[i]) as f64 * self.period as f64 / 1e6;
            match self.sums.iter_mut().find(|s| s.0 == names[i]) {
                Some(s) => s.1 += ms,
                None => self.sums.push((names[i], ms)),
            }
        }
        self.frames += 1;
        if self.frames == 240 {
            let total: f64 = self.sums.iter().map(|s| s.1).sum::<f64>() / 240.0;
            let line: Vec<String> = self.sums.iter().map(|(n, s)| format!("{n} {:.2}", s / 240.0)).collect();
            log::info!("gputimes: {} = {:.2} ms (mean of 240 frames)", line.join(", "), total);
            self.sums.clear();
            self.frames = 0;
        }
    }
}
