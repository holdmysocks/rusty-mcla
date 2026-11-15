//! Install pipeline glue: the Tauri commands behind the Install page. All the work is in the `mcla-install` library; this file only
//! owns the run state (one validation + one build at a time), the event log the page polls, and the build log file.
//!
//! The page polls `install_poll` instead of listening to Tauri events, so no capability file is needed.

use mcla_install::{Cancel, Ctx, Disc, DumpState, ErrorKind, Event, HashResult, InstallError, InstallOptions, IsoIdentity, Layout, Manifest};
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::logging;

const KEEP_BUILD_LOGS: usize = 10;
const MAX_EVENTS: usize = 3000;

/// An error as the page shows it: a code for the translated text, the English text, the technical detail.
#[derive(Serialize, Clone, Debug)]
pub struct ErrView {
    pub code: &'static str,
    pub message: &'static str,
    pub detail: String,
}

impl From<&InstallError> for ErrView {
    fn from(e: &InstallError) -> Self {
        ErrView { code: e.code(), message: e.message(), detail: e.detail.clone() }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct HashView {
    pub sha256: String,
    pub verified: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Done {
    /// A validation finished: the identity, or the error.
    Validated { ok: bool, identity: Option<IsoIdentity>, error: Option<ErrView>, warnings: Vec<String> },
    /// The checksum finished (`verified` = matches the supported dump).
    Hashed { hash: Option<HashView>, error: Option<ErrView> },
    /// The build finished.
    Built { ok: bool, cancelled: bool, error: Option<ErrView>, files: usize, bytes: u64, audio_problems: usize, dump: Option<String>, warnings: Vec<String>, seconds: u64 },
    Verified { checked: usize, missing: usize, wrong_size: usize },
}

#[derive(Default)]
struct Shared {
    next_seq: u64,
    events: Vec<(u64, Event)>,
    done: Vec<(u64, Done)>,
    validated: Option<(String, IsoIdentity)>,
    hash: Option<(String, HashResult)>,
    building: bool,
    validating: bool,
    build_cancel: Option<Cancel>,
    hash_cancel: Option<Cancel>,
}

#[derive(Clone, Default)]
pub struct Runner {
    inner: Arc<Mutex<Shared>>,
    /// Supplies the install manifest: the installed game's release manifest overrides the compiled-in one.
    updater: Option<crate::update::Updater>,
}

impl Runner {
    pub fn new(updater: crate::update::Updater) -> Runner {
        Runner { inner: Arc::default(), updater: Some(updater) }
    }
}

#[derive(Serialize)]
pub struct Poll {
    pub events: Vec<Event>,
    pub done: Vec<Done>,
    pub next: u64,
    pub building: bool,
    pub validating: bool,
}

#[derive(Serialize)]
pub struct Info {
    pub root: String,
    pub installed: bool,
    pub free_bytes: u64,
    pub required_bytes: u64,
    pub data_bytes: u64,
    pub iso_size: u64,
    pub ffmpeg_download_bytes: u64,
    pub ffmpeg_version: String,
    pub ffmpeg_license: String,
    pub ffmpeg_found: Option<String>,
    pub building: bool,
    pub validated_iso: Option<String>,
    pub identity: Option<IsoIdentity>,
    pub hash: Option<HashView>,
    pub has_partial: bool,
}

fn lock(r: &Runner) -> std::sync::MutexGuard<'_, Shared> {
    r.inner.lock().unwrap_or_else(|p| p.into_inner())
}

impl Runner {
    fn push_event(&self, e: Event) {
        let mut s = lock(self);
        let seq = s.next_seq;
        s.next_seq += 1;
        s.events.push((seq, e));
        let n = s.events.len();
        if n > MAX_EVENTS {
            s.events.drain(0..n - MAX_EVENTS);
        }
    }

    fn push_done(&self, d: Done) {
        let mut s = lock(self);
        let seq = s.next_seq;
        s.next_seq += 1;
        s.done.push((seq, d));
    }

    pub fn poll(&self, since: u64) -> Poll {
        let s = lock(self);
        Poll {
            events: s.events.iter().filter(|(q, _)| *q >= since).map(|(_, e)| e.clone()).collect(),
            done: s.done.iter().filter(|(q, _)| *q >= since).map(|(_, d)| d.clone()).collect(),
            next: s.next_seq,
            building: s.building,
            validating: s.validating,
        }
    }

    /// A `Ctx` whose events land in the poll log (and the build log when given).
    fn ctx(&self, cancel: Cancel, log: Option<BuildLog>) -> Ctx {
        let me = self.clone();
        let last_progress = Mutex::new(Instant::now());
        let sink: mcla_install::progress::Sink = Arc::new(move |e: &Event| {
            if let Some(l) = &log {
                // everything except the 10-a-second progress ticks; those once every 5 seconds
                let tick = e.total > 0 && !e.finished && e.check.is_none();
                let mut log_it = !tick;
                if tick {
                    if let Ok(mut t) = last_progress.lock() {
                        if t.elapsed().as_secs() >= 5 {
                            *t = Instant::now();
                            log_it = true;
                        }
                    }
                }
                if log_it {
                    l.line(e);
                }
            }
            me.push_event(e.clone());
        });
        Ctx::new(sink, cancel)
    }
}

/// `build-<time>.log` beside the launcher log; the newest KEEP_BUILD_LOGS are kept.
#[derive(Clone)]
struct BuildLog {
    file: Arc<Mutex<std::fs::File>>,
}

impl BuildLog {
    fn open(dir: &Path) -> Option<BuildLog> {
        let _ = std::fs::create_dir_all(dir);
        prune(dir);
        let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        let f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(format!("build-{stamp}.log"))).ok()?;
        Some(BuildLog { file: Arc::new(Mutex::new(f)) })
    }
    fn raw(&self, text: &str) {
        if let Ok(mut f) = self.file.lock() {
            let t = chrono::Local::now().format("%H:%M:%S%.3f");
            let _ = writeln!(f, "[{t}] {}", text.replace('\n', " | "));
        }
    }
    fn line(&self, e: &Event) {
        if let Some(c) = &e.check {
            self.raw(&format!("[{}] check {} {:?}: {}", e.stage.id(), c.id, c.status, c.text));
        } else if e.total > 0 && !e.finished {
            self.raw(&format!("[{}] {} / {}  {}", e.stage.id(), e.done, e.total, e.message));
        } else if !e.message.is_empty() {
            self.raw(&format!("[{}] {}", e.stage.id(), e.message));
        }
    }
}

fn prune(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut logs: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            n.starts_with("build-") && n.ends_with(".log")
        })
        .collect();
    logs.sort();
    while logs.len() >= KEEP_BUILD_LOGS {
        let _ = std::fs::remove_file(logs.remove(0));
    }
}

/// The install manifest in effect: the installed game's own copy of its release manifest when there is one (so the built data always
/// matches the installed game), else the compiled-in one.
fn manifest(r: &Runner) -> Manifest {
    match &r.updater {
        Some(u) => u.install_manifest(),
        None => Manifest::embedded(),
    }
}

pub fn info(r: &Runner, root: &Path) -> Info {
    let m = manifest(r);
    let (free, need) = mcla_install::preflight::space_for(root, &m);
    let s = lock(r);
    let layout = Layout::new(root);
    let dl = mcla_install::ffmpeg::pinned(&m);
    Info {
        root: root.to_string_lossy().into_owned(),
        installed: layout.is_complete(),
        free_bytes: free,
        required_bytes: need,
        data_bytes: m.space.data_bytes,
        iso_size: m.iso.size,
        ffmpeg_download_bytes: dl.map(|d| d.size).unwrap_or(0),
        ffmpeg_version: m.ffmpeg.version.clone(),
        ffmpeg_license: m.ffmpeg.license.clone(),
        ffmpeg_found: mcla_install::ffmpeg::locate(root, None).map(|f| f.version),
        building: s.building,
        validated_iso: s.validated.as_ref().map(|v| v.0.clone()),
        identity: s.validated.as_ref().map(|v| v.1.clone()),
        hash: s.hash.as_ref().map(|(_, h)| HashView { sha256: h.sha256.clone(), verified: h.state == DumpState::Verified }),
        has_partial: layout.state_file().is_file() && !layout.is_complete(),
    }
}

/// Native file picker for the disc image. Blocks until the dialog closes; call it from a worker thread.
pub fn pick_iso() -> Option<String> {
    rfd::FileDialog::new().set_title("Choose your Xbox 360 disc image").add_filter("Disc image", &["iso"]).add_filter("All files", &["*"]).pick_file().map(|p| p.to_string_lossy().into_owned())
}

/// Run the fast checks on `path`, then start the full checksum in the background. Blocks for the fast checks (a fraction of a second).
pub fn validate(r: &Runner, path: &str) -> Result<IsoIdentity, ErrView> {
    {
        let mut s = lock(r);
        if s.building {
            return Err(ErrView { code: ErrorKind::BadFolder.code(), message: "A build is running.", detail: "validate while building".into() });
        }
        if let Some(c) = s.hash_cancel.take() {
            c.cancel(); // a hash of an earlier choice
        }
        s.validating = true;
        s.validated = None;
        s.hash = None;
        s.events.clear();
        s.done.clear();
    }
    let m = manifest(r);
    let ctx = r.ctx(Cancel::new(), None);
    let res = Disc::open(Path::new(path), &m, &ctx);
    lock(r).validating = false;
    match res {
        Ok(disc) => {
            let identity = disc.identity.clone();
            let warnings = disc.warnings.clone();
            drop(disc);
            lock(r).validated = Some((path.to_string(), identity.clone()));
            r.push_done(Done::Validated { ok: true, identity: Some(identity.clone()), error: None, warnings });
            // the full checksum runs on in the background; a mismatch only warns
            let cancel = Cancel::new();
            lock(r).hash_cancel = Some(cancel.clone());
            let (me, p, mm) = (r.clone(), path.to_string(), m.clone());
            std::thread::spawn(move || {
                let ctx = me.ctx(cancel.clone(), None);
                match mcla_install::hash_iso(Path::new(&p), &mm, &ctx) {
                    Ok(h) => {
                        let view = HashView { sha256: h.sha256.clone(), verified: h.state == DumpState::Verified };
                        logging::info(&format!("disc image checksum {} ({})", h.sha256, if view.verified { "verified" } else { "unrecognised" }));
                        lock(&me).hash = Some((p, h));
                        me.push_done(Done::Hashed { hash: Some(view), error: None });
                    }
                    Err(e) if e.is_cancelled() => {}
                    Err(e) => me.push_done(Done::Hashed { hash: None, error: Some((&e).into()) }),
                }
            });
            Ok(identity)
        }
        Err(e) => {
            logging::warn(&format!("disc image refused: {e}"));
            let view: ErrView = (&e).into();
            r.push_done(Done::Validated { ok: false, identity: None, error: Some(view.clone()), warnings: vec![] });
            Err(view)
        }
    }
}

/// Start (or resume) the build on a worker thread. Returns at once; progress comes through `poll`.
pub fn start(r: &Runner, root: &Path, logs_dir: &Path, iso: &str, ffmpeg_path: Option<String>) -> Result<(), ErrView> {
    let cancel = Cancel::new();
    {
        let mut s = lock(r);
        if s.building {
            return Err(ErrView { code: ErrorKind::BadFolder.code(), message: "A build is already running.", detail: "start while building".into() });
        }
        s.building = true;
        s.build_cancel = Some(cancel.clone());
        s.events.clear();
        s.done.clear();
    }
    let log = BuildLog::open(logs_dir);
    let mut opts = InstallOptions::new(iso, root);
    opts.manifest = manifest(r);
    opts.ffmpeg_path = ffmpeg_path.filter(|p| !p.is_empty()).map(PathBuf::from);
    {
        let mut s = lock(r);
        if let Some((p, h)) = &s.hash {
            if p == iso {
                opts.known_checksum = Some(h.clone());
            }
        }
        if opts.known_checksum.is_none() {
            // the launcher's own checksum has not finished: the pipeline takes over and records the result in the manifest
            if let Some(c) = s.hash_cancel.take() {
                c.cancel();
            }
        }
    }
    if let Some(l) = &log {
        l.raw(&format!("build started: iso {iso}, folder {}", root.display()));
    }
    logging::info(&format!("build started: iso {iso}, folder {}", root.display()));
    let me = r.clone();
    std::thread::spawn(move || {
        let ctx = me.ctx(cancel, log.clone());
        let t0 = Instant::now();
        let res = mcla_install::install(&opts, &ctx);
        let seconds = t0.elapsed().as_secs();
        let done = match res {
            Ok(o) => {
                // a checksum that finished while we ran (the launcher's own) can be stamped later; keep the result we had
                Done::Built { ok: true, cancelled: false, error: None, files: o.local.files_count, bytes: o.local.files_bytes, audio_problems: o.local.audio_problems, dump: Some(o.local.iso.dump.clone()), warnings: o.warnings, seconds }
            }
            Err(e) => Done::Built { ok: false, cancelled: e.is_cancelled(), error: Some((&e).into()), files: 0, bytes: 0, audio_problems: 0, dump: None, warnings: vec![], seconds },
        };
        if let Some(l) = &log {
            l.raw(&format!("build finished: {:?}", done));
        }
        logging::info(&format!("build finished after {seconds} s: ok={}", matches!(&done, Done::Built { ok: true, .. })));
        {
            let mut s = lock(&me);
            s.building = false;
            s.build_cancel = None;
        }
        me.push_done(done);
    });
    Ok(())
}

pub fn cancel(r: &Runner) {
    let s = lock(r);
    if let Some(c) = &s.build_cancel {
        c.cancel();
    }
}

/// Re-check the built files against the list written at the end of the build; damaged stages are cleared so the next build repairs them.
pub fn verify(r: &Runner, root: &Path) -> Result<(), ErrView> {
    let layout = Layout::new(root);
    let ctx = r.ctx(Cancel::new(), None);
    let rep = mcla_install::verify::verify(&layout, &ctx).map_err(|e| ErrView::from(&e))?;
    if !rep.ok() {
        mcla_install::verify::prepare_repair(&layout, &rep).map_err(|e| ErrView::from(&e))?;
    }
    r.push_done(Done::Verified { checked: rep.checked, missing: rep.missing.len(), wrong_size: rep.wrong_size.len() });
    Ok(())
}
