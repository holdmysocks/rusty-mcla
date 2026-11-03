//! The pipeline: preflight -> validate -> extract -> ffmpeg -> audio -> video -> stamp -> verify.
//! Every stage is resumable (finished files are skipped), cancellable and reports progress through the [`Ctx`].

use crate::audio;
use crate::error::{ErrorKind, InstallError, IoCtx, Result};
use crate::extract::{self, ExtractOptions, FileRecord};
use crate::ffmpeg;
use crate::layout::{list_hash, list_text, scan, write_file_atomic, FfmpegNote, Layout, LocalManifest, StageSummary, State};
use crate::manifest::Manifest;
use crate::preflight;
use crate::shaders;
use crate::progress::{Cancel, Ctx, Event, Stage, Unit};
use crate::validate::{hash_iso_with_stop, Disc, DumpState, HashResult};
use crate::verify;
use mcla_formats::names::Names;
use mcla_formats::rpf3::Rpf3;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

const NAMES: &str = include_str!("../../../names.txt");

/// The data version at which each stage's output last changed. When the manifest's `data_version` moves, the finished flags of the stages
/// that did not change since the data was built are kept, so a rebuild runs only the new or changed stages (version 2 added "shaders":
/// existing data keeps its extraction, audio and video and only translates the shader programs; version 3 joins the streams' blocks without
/// the packet each block repeats, so the streams are converted again; version 4 adds each wave's loop start and headroom to the
/// banks' `_index.tsv` and folds the streams by speaker mask with each channel's headroom, so the streams are converted again; version 5 also extracts the `resources/` folders of the audio archive into the game data (the Police Car Pack's vehicles and driver animations), so the game data is extracted again).
const STAGE_SINCE: [(&str, u32); 4] = [("extract", 5), ("audio", 4), ("video", 1), ("shaders", 2)];

#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub iso: PathBuf,
    /// The portable folder (`data/`, `tools/` and `manifest.local.json` go below it).
    pub root: PathBuf,
    pub manifest: Manifest,
    /// An ffmpeg the user chose (file or folder).
    pub ffmpeg_path: Option<PathBuf>,
    /// Download the pinned ffmpeg when none is found.
    pub allow_download: bool,
    /// ffmpeg processes at once; 0 = by CPU count.
    pub jobs: usize,
    /// Hash the whole image in the background (verified / unrecognised dump badge).
    pub full_checksum: bool,
    /// A checksum result the caller already has for this very file (the launcher hashes while the user picks the disc); no second pass.
    pub known_checksum: Option<HashResult>,
}

impl InstallOptions {
    pub fn new(iso: impl Into<PathBuf>, root: impl Into<PathBuf>) -> Self {
        InstallOptions { iso: iso.into(), root: root.into(), manifest: Manifest::embedded(), ffmpeg_path: None, allow_download: true, jobs: 0, full_checksum: true, known_checksum: None }
    }
}

#[derive(Debug)]
pub struct InstallOutcome {
    pub local: LocalManifest,
    pub warnings: Vec<String>,
    /// Audio files / waves that could not be converted (also in `local.audio_problems`).
    pub audio_problems: Vec<String>,
    pub extract_written: usize,
    pub extract_skipped: usize,
}

pub fn names() -> Names {
    let mut n = Names::new();
    n.load_list(NAMES);
    n
}

fn jobs_for(opts: &InstallOptions) -> usize {
    if opts.jobs > 0 {
        return opts.jobs;
    }
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 16)
}

fn stage_done(ctx: &Ctx, stage: Stage, message: &str) {
    ctx.emit(Event { stage, done: 1, total: 1, unit: Unit::Items, message: message.to_string(), check: None, finished: true });
}

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Run the whole pipeline. Safe to call again after a cancel or a failure: finished work is skipped.
pub fn install(opts: &InstallOptions, ctx: &Ctx) -> Result<InstallOutcome> {
    let m = &opts.manifest;
    let l = Layout::new(&opts.root);
    let mut warnings = Vec::new();

    // 1. preflight
    preflight::preflight(&opts.root, Some(&opts.iso), m, ctx)?;
    ctx.check()?;
    stage_done(ctx, Stage::Preflight, "Folder checked");

    // 2. validate
    let mut disc = Disc::open(&opts.iso, m, ctx)?;
    warnings.extend(disc.warnings.iter().cloned());
    stage_done(ctx, Stage::Validate, "The disc image is supported");
    ctx.check()?;

    let mut state = State::load(&l);
    if state.data_version == 0 {
        // no work state (an older launcher removed it, or the data came from another tool): a finished build says what it contains
        if let Some(lm) = LocalManifest::load(&l).filter(|lm| lm.iso.size == disc.identity.size && lm.iso.file_name == disc.identity.file_name) {
            let done = ["extract", "audio", "video"].iter().map(|s| (s.to_string(), true)).collect();
            state = State { data_version: lm.data_version, iso_size: lm.iso.size, iso_name: lm.iso.file_name.clone(), done };
        }
    }
    let mut migrated = false;
    if state.iso_size != disc.identity.size || state.iso_name != disc.identity.file_name {
        state = State { data_version: m.data_version, iso_size: disc.identity.size, iso_name: disc.identity.file_name.clone(), done: BTreeMap::new() };
    } else if state.data_version != m.data_version {
        let old = state.data_version;
        state.done.retain(|k, _| STAGE_SINCE.iter().any(|(n, since)| n == k && old >= *since));
        if old < 4 {
            // the stream conversions are skipped when their files exist: remove the ones made with the old block joining
            audio::remove_streams(&l.audio_pc());
        }
        state.data_version = m.data_version;
        migrated = !state.done.is_empty();
    }
    state.save(&l)?;

    // full checksum beside the other stages; stopped when the install ends
    let stop = Cancel::new();
    let mut hash = if opts.full_checksum && opts.known_checksum.is_none() {
        let (path, mm, c, st) = (opts.iso.clone(), m.clone(), ctx.clone(), stop.clone());
        Some(std::thread::spawn(move || hash_iso_with_stop(&path, &mm, &c, Some(&st))))
    } else {
        None
    };
    let r = run_stages(opts, ctx, &l, &mut disc, &mut state, &mut hash, migrated);
    stop.cancel();
    if let Some(h) = hash.take() {
        let _ = h.join();
    }
    let mut out = r?;
    out.warnings = warnings;
    Ok(out)
}

/// `migrated`: the data was built with an older data version and its finished stages are kept (nothing to extract again).
fn run_stages(opts: &InstallOptions, ctx: &Ctx, l: &Layout, disc: &mut Disc, state: &mut State, hash: &mut Option<std::thread::JoinHandle<Result<HashResult>>>, migrated: bool) -> Result<InstallOutcome> {
    let m = &opts.manifest;
    let names = names();
    // 3. extract
    let (extract_written, extract_skipped);
    if migrated && state.is_done("extract") {
        extract_written = 0;
        extract_skipped = 0;
        stage_done(ctx, Stage::Extract, "Game data already extracted");
    } else {
        ctx.message(Stage::Extract, "Extracting the game data");
        let rpf = Rpf3::open(Arc::new(disc.iso.file(&m.iso.game_archive).map_err(|e| InstallError::new(ErrorKind::MissingFile, format!("{e:#}")))?), &disc.key).map_err(|e| InstallError::new(ErrorKind::UnsupportedEdition, format!("{e:#}")))?;
        let rep = extract::extract_archive(&rpf, &l.cache(), &names, ExtractOptions::default(), Stage::Extract, ctx)?;
        if !rep.failed.is_empty() {
            let first: Vec<String> = rep.failed.iter().take(5).map(|(p, e)| format!("{p}: {e}")).collect();
            return Err(InstallError::new(ErrorKind::DataDamaged, format!("{} archive entries could not be extracted: {}", rep.failed.len(), first.join("; "))));
        }
        let (mut written, mut skipped) = (rep.written, rep.skipped);
        // the audio archive also holds the Police Car Pack's vehicle folders and driver animations (resources/), which belong to the game data
        for a in &m.iso.audio_archives {
            let f = match disc.iso.file(a) {
                Ok(f) => f,
                Err(_) => continue,
            };
            let Ok(arc) = Rpf3::open(Arc::new(f), &disc.key) else { continue };
            let opts = ExtractOptions { only_below: Some("resources/"), ..Default::default() };
            let rep = extract::extract_archive(&arc, &l.cache(), &names, opts, Stage::Extract, ctx)?;
            if !rep.failed.is_empty() {
                let first: Vec<String> = rep.failed.iter().take(5).map(|(p, e)| format!("{p}: {e}")).collect();
                return Err(InstallError::new(ErrorKind::DataDamaged, format!("{} entries of {a} could not be extracted: {}", rep.failed.len(), first.join("; "))));
            }
            written += rep.written;
            skipped += rep.skipped;
        }
        state.done.insert("extract".into(), true);
        state.save(l)?;
        stage_done(ctx, Stage::Extract, &format!("{written} files written, {skipped} already there"));
        extract_written = written;
        extract_skipped = skipped;
    }
    ctx.check()?;

    // 3b. translate the sky / road / sea shader programs of the original (a derivative of the game's data, so made here and never shipped)
    let srep = shaders::run(&l.cache(), Some(&disc.iso), ctx)?;
    state.done.insert("shaders".into(), true);
    state.save(l)?;
    stage_done(ctx, Stage::Shaders, &format!("{} shader programs translated ({} written, {} already there)", srep.files.len(), srep.written(), srep.files.len() - srep.written()));
    ctx.check()?;

    // 4. ffmpeg
    let need_audio = !state.is_done("audio");
    let need_video = !state.is_done("video");
    let mut note = None;
    let mut ff_path = None;
    if need_audio || need_video {
        let ff = ffmpeg::ensure(&opts.root, opts.ffmpeg_path.as_deref(), opts.allow_download, m, ctx)?;
        note = Some(FfmpegNote { version: ff.version.clone(), source: ff.source });
        ff_path = Some(ff.path);
    }
    stage_done(ctx, Stage::Ffmpeg, if ff_path.is_some() { "ffmpeg is ready" } else { "ffmpeg not needed" });
    ctx.check()?;

    // 5. audio
    let mut audio_problems = Vec::new();
    if let Some(ff) = ff_path.as_ref().filter(|_| need_audio) {
        ctx.message(Stage::Audio, "Reading the sound banks");
        let mut rpfs = Vec::new();
        // the game archive carries a few banks of its own (menus, collisions, ...) next to the two audio archives
        for a in std::iter::once(&m.iso.game_archive).chain(m.iso.audio_archives.iter()) {
            let f = disc.iso.file(a).map_err(|e| InstallError::new(ErrorKind::MissingFile, format!("{e:#}")))?;
            rpfs.push(Rpf3::open(Arc::new(f), &disc.key).map_err(|e| InstallError::new(ErrorKind::UnsupportedEdition, format!("{a}: {e:#}")))?);
        }
        let refs: Vec<&Rpf3<_>> = rpfs.iter().collect();
        let items = audio::list_items(&refs, &names);
        let wave_names = audio::build_names(&l.cache().join("audio").join("X360").join("config"));
        ctx.message(Stage::Audio, format!("{} names recovered, {} sound files", wave_names.len(), items.len()));
        let read = |it: &audio::Item| -> Result<Vec<u8>> { rpfs[it.archive].read(it.entry).map_err(|e| InstallError::new(ErrorKind::Unreadable, format!("{e:#}"))) };
        let rep = audio::convert(&items, &read, ff, &l.audio_pc(), &wave_names, jobs_for(opts), ctx)?;
        audio_problems = rep.problems;
        state.done.insert("audio".into(), true);
        state.save(l)?;
        stage_done(ctx, Stage::Audio, &format!("{} sound files converted ({} banks, {} streams), {} problems", rep.files, rep.banks, rep.streams, audio_problems.len()));
    } else {
        stage_done(ctx, Stage::Audio, "Audio already built");
    }
    ctx.check()?;

    // 6. video
    if let Some(ff) = ff_path.as_ref().filter(|_| need_video) {
        let open = |name: &str| disc.iso.file(name).map_err(|e| InstallError::new(ErrorKind::MissingFile, format!("{name}: {e:#}")));
        crate::video::convert_all(&m.movies, &open, ff, &l.video_pc(), ctx)?;
        state.done.insert("video".into(), true);
        state.save(l)?;
        stage_done(ctx, Stage::Video, "Videos converted");
    } else {
        stage_done(ctx, Stage::Video, "Video already built");
    }
    ctx.check()?;

    // 7. stamp
    ctx.message(Stage::Stamp, "Writing the file list and manifest");
    let mut files: Vec<FileRecord> = Vec::new();
    let data = l.data();
    let mut stages = BTreeMap::new();
    for (dir, stage) in [("cache", "extract"), ("audio_pc", "audio"), ("video_pc", "video")] {
        let mut v = Vec::new();
        scan(&data, &data.join(dir), &mut v).ctx(dir)?;
        stages.insert(stage.to_string(), StageSummary { files: v.len(), bytes: v.iter().map(|f| f.size).sum() });
        files.extend(v);
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let text = list_text(&files);
    std::fs::create_dir_all(l.install_dir()).ctx(l.install_dir().display())?;
    write_file_atomic(&l.files_list(), text.as_bytes())?;
    // the checksum result goes into the identity (the stage needs the whole image read once)
    if let Some(h) = hash.take() {
        ctx.message(Stage::Checksum, "Waiting for the disc image checksum");
        if let Ok(Ok(r)) = h.join() {
            disc.identity.sha256 = Some(r.sha256);
            disc.identity.dump = if r.state == DumpState::Verified { "verified".into() } else { "unrecognised".into() };
        }
        ctx.check()?;
    } else if let Some(r) = &opts.known_checksum {
        disc.identity.sha256 = Some(r.sha256.clone());
        disc.identity.dump = if r.state == DumpState::Verified { "verified".into() } else { "unrecognised".into() };
    }
    let mut paths = BTreeMap::new();
    paths.insert("cache".to_string(), "data/cache".to_string());
    paths.insert("audio_pc".to_string(), "data/audio_pc".to_string());
    paths.insert("video_pc".to_string(), "data/video_pc".to_string());
    let local = LocalManifest {
        schema: 1,
        data_version: m.data_version,
        installed_unix: now_unix(),
        iso: disc.identity.clone(),
        ffmpeg: note,
        paths,
        stages,
        files_list: "data/.install/files.tsv".into(),
        files_count: files.len(),
        files_bytes: files.iter().map(|f| f.size).sum(),
        files_list_sha256: list_hash(&text),
        audio_problems: audio_problems.len(),
        shaders: shaders::hashes(&l.cache())?,
    };
    local.save(l)?;
    state.done.insert("stamp".into(), true);
    state.save(l)?;
    stage_done(ctx, Stage::Stamp, &format!("{} files, {:.1} GB", local.files_count, local.files_bytes as f64 / 1e9));

    // 8. verify
    let rep = verify::verify(l, ctx)?;
    if !rep.ok() {
        return Err(InstallError::new(ErrorKind::DataDamaged, format!("{} files are missing or have the wrong size right after the build", rep.damaged())));
    }
    stage_done(ctx, Stage::Verify, &format!("{} files checked", rep.checked));
    Ok(InstallOutcome { local, warnings: Vec::new(), audio_problems, extract_written, extract_skipped })
}

/// Verify an existing install and rebuild what is missing (needs the disc image again). `None` when everything was fine.
pub fn repair(opts: &InstallOptions, ctx: &Ctx) -> Result<Option<InstallOutcome>> {
    let l = Layout::new(&opts.root);
    let rep = verify::verify(&l, ctx)?;
    if rep.ok() {
        return Ok(None);
    }
    ctx.message(Stage::Verify, format!("{} files are missing or damaged; rebuilding them", rep.damaged()));
    verify::prepare_repair(&l, &rep)?;
    install(opts, ctx).map(Some)
}
