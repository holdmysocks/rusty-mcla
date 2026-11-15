//! Tests of the pipeline pieces that need no disc image: verify / repair bookkeeping, preflight, cancel, checksum, and extraction of a
//! small synthetic (unencrypted) RPF3 archive including resume. The real-disc check is `tests/real_disc.rs` (ignored by default).

use mcla_formats::hash::joaat;
use mcla_formats::names::Names;
use mcla_formats::rpf3::Rpf3;
use mcla_formats::{ReadAt, Result as FResult};
use mcla_install::extract::{extract_archive, ExtractOptions};
use mcla_install::layout::{list_text, write_file_atomic, Layout, State};
use mcla_install::progress::{Cancel, Ctx, Stage};
use mcla_install::{ErrorKind, Manifest};
use std::path::PathBuf;
use std::sync::Arc;

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mcla_install_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Mem(Vec<u8>);
impl ReadAt for Mem {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, off: u64, buf: &mut [u8]) -> FResult<()> {
        buf.copy_from_slice(&self.0[off as usize..off as usize + buf.len()]);
        Ok(())
    }
}

/// An unencrypted archive: root -> dir "d" -> files "a.bin" and "junk ". Names are the given strings.
fn archive(files: &[(&str, &[u8])]) -> (Mem, Names) {
    let mut names = Names::new();
    names.add("d");
    for (n, _) in files {
        names.add(n);
    }
    // entries: 0 root, 1 dir d, 2.. files
    let count = 2 + files.len();
    let mut toc: Vec<u32> = Vec::new();
    toc.extend([0, 1, 0x8000_0001, 1]); // root: one child (entry 1)
    toc.extend([joaat("d"), 0, 0x8000_0000 | 2, files.len() as u32]);
    let mut data = Vec::new();
    let data_start = 0x800 + count * 16;
    let data_start = (data_start + 0x7FF) & !0x7FF;
    for (n, d) in files {
        let off = data_start + data.len();
        toc.extend([joaat(n), d.len() as u32, off as u32, 0]);
        data.extend_from_slice(d);
    }
    let mut a = vec![0u8; 0x800];
    a[0..4].copy_from_slice(b"RPF3");
    a[4..8].copy_from_slice(&((count * 16) as u32).to_le_bytes());
    a[8..12].copy_from_slice(&(count as u32).to_le_bytes());
    for w in toc {
        a.extend_from_slice(&w.to_le_bytes());
    }
    a.resize(data_start, 0);
    a.extend_from_slice(&data);
    (Mem(a), names)
}

#[test]
fn extract_writes_files_and_resumes() {
    let (mem, names) = archive(&[("a.bin", b"hello"), ("b.bin", b"world!!")]);
    let rpf = Rpf3::open(Arc::new(mem), &[0u8; 32]).unwrap();
    let out = temp("extract");
    let ctx = Ctx::silent();
    let r = extract_archive(&rpf, &out, &names, ExtractOptions::default(), Stage::Extract, &ctx).unwrap();
    assert_eq!((r.written, r.skipped, r.failed.len()), (2, 0, 0));
    assert_eq!(std::fs::read(out.join("d/a.bin")).unwrap(), b"hello");
    assert_eq!(r.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), vec!["d/a.bin", "d/b.bin"]);
    // a second run skips what is there; a file of the wrong size (a cut-off copy) is written again
    std::fs::write(out.join("d/b.bin"), b"wo").unwrap();
    let r = extract_archive(&rpf, &out, &names, ExtractOptions::default(), Stage::Extract, &ctx).unwrap();
    assert_eq!((r.written, r.skipped), (1, 1));
    assert_eq!(std::fs::read(out.join("d/b.bin")).unwrap(), b"world!!");
    assert!(!out.join("d/b.bin.part").exists());
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn extract_cancel_returns_cancelled() {
    let (mem, names) = archive(&[("a.bin", b"hello")]);
    let rpf = Rpf3::open(Arc::new(mem), &[0u8; 32]).unwrap();
    let out = temp("cancel");
    let cancel = Cancel::new();
    cancel.cancel();
    let ctx = Ctx::new(Arc::new(|_| {}), cancel);
    let e = extract_archive(&rpf, &out, &names, ExtractOptions::default(), Stage::Extract, &ctx).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Cancelled);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn junk_name_falls_back_to_the_hash_name() {
    // `?` cannot be in a Windows file name; elsewhere it can, so only check there
    if !cfg!(windows) {
        return;
    }
    let (mem, names) = archive(&[("a?b", b"x")]);
    let rpf = Rpf3::open(Arc::new(mem), &[0u8; 32]).unwrap();
    let out = temp("junk");
    let r = extract_archive(&rpf, &out, &names, ExtractOptions::default(), Stage::Extract, &Ctx::silent()).unwrap();
    assert_eq!(r.failed.len(), 0);
    assert_eq!(r.files.len(), 1);
    assert_eq!(r.files[0].path, format!("d/0x{:08x}", joaat("a?b")));
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn verify_finds_damage_and_repair_clears_the_stage() {
    let root = temp("verify");
    let l = Layout::new(&root);
    std::fs::create_dir_all(l.data().join("cache")).unwrap();
    std::fs::create_dir_all(l.data().join("audio_pc")).unwrap();
    std::fs::write(l.data().join("cache/a"), b"12345").unwrap();
    std::fs::write(l.data().join("audio_pc/b.wav"), b"123").unwrap();
    std::fs::write(l.data().join("audio_pc/other.wav"), b"9").unwrap(); // not in the list: the folder still holds a file
    std::fs::create_dir_all(l.data().join("video_pc")).unwrap();
    std::fs::write(l.data().join("video_pc/v.mp4"), b"1").unwrap();
    let files = vec![mcla_install::extract::FileRecord { path: "audio_pc/b.wav".into(), size: 3 }, mcla_install::extract::FileRecord { path: "cache/a".into(), size: 5 }];
    std::fs::create_dir_all(l.install_dir()).unwrap();
    write_file_atomic(&l.files_list(), list_text(&files).as_bytes()).unwrap();
    let ctx = Ctx::silent();
    assert!(mcla_install::verify::verify(&l, &ctx).unwrap().ok());
    std::fs::remove_file(l.data().join("audio_pc/b.wav")).unwrap();
    std::fs::write(l.data().join("cache/a"), b"1").unwrap();
    let rep = mcla_install::verify::verify(&l, &ctx).unwrap();
    assert_eq!((rep.missing.len(), rep.wrong_size.len()), (1, 1));
    assert_eq!(rep.damaged_stages(), vec!["extract", "audio"]);
    let mut st = State::default();
    st.done.insert("audio".into(), true);
    st.done.insert("extract".into(), true);
    st.done.insert("video".into(), true);
    st.save(&l).unwrap();
    mcla_install::verify::prepare_repair(&l, &rep).unwrap();
    let st = State::load(&l);
    assert!(!st.is_done("audio") && !st.is_done("extract") && st.is_done("video"));
    assert!(!l.data().join("cache/a").exists(), "the wrong-size file is removed so it is written again");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_finished_flag_with_no_media_files_is_damage_and_not_complete() {
    // the state file outlived the folders (issue 56): audio_pc missing, video_pc empty, files.tsv names none of their files
    let root = temp("nomedia");
    let l = Layout::new(&root);
    std::fs::create_dir_all(l.data().join("cache")).unwrap();
    std::fs::create_dir_all(l.data().join("video_pc/sub")).unwrap();
    std::fs::write(l.data().join("cache/a"), b"12345").unwrap();
    std::fs::write(l.data().join("video_pc/sub/x.part"), b"unfinished").unwrap(); // a work file is not a media file
    let files = vec![mcla_install::extract::FileRecord { path: "cache/a".into(), size: 5 }];
    std::fs::create_dir_all(l.install_dir()).unwrap();
    write_file_atomic(&l.files_list(), list_text(&files).as_bytes()).unwrap();
    write_file_atomic(&l.local_manifest(), b"{}").unwrap();
    assert_eq!(l.empty_media_dirs(), vec!["audio_pc", "video_pc"]);
    assert!(!l.is_complete(), "a manifest without the media is not an install");
    let mut st = State::default();
    for s in ["extract", "audio", "video"] {
        st.done.insert(s.into(), true);
    }
    st.save(&l).unwrap();
    let rep = mcla_install::verify::verify(&l, &Ctx::silent()).unwrap();
    assert!(!rep.ok());
    assert_eq!(rep.damaged_stages(), vec!["audio", "video"]);
    mcla_install::verify::prepare_repair(&l, &rep).unwrap();
    let st = State::load(&l);
    assert!(st.is_done("extract") && !st.is_done("audio") && !st.is_done("video"), "repair clears exactly the stages without files");
    // once a file is there the folders count
    std::fs::write(l.data().join("video_pc/sub/x.mp4"), b"1").unwrap();
    std::fs::create_dir_all(l.data().join("audio_pc/bank")).unwrap();
    std::fs::write(l.data().join("audio_pc/bank/w.wav"), b"1").unwrap();
    assert!(l.empty_media_dirs().is_empty() && l.is_complete());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn verify_without_a_list_is_data_damaged() {
    let root = temp("nolist");
    let e = mcla_install::verify::verify(&Layout::new(&root), &Ctx::silent()).unwrap_err();
    assert_eq!(e.kind, ErrorKind::DataDamaged);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn preflight_checks_the_folder() {
    // the folder checks must not depend on how much disk the machine running the test has: ask for almost nothing
    let mut m = Manifest::embedded();
    m.space.data_bytes = 1;
    m.space.headroom_bytes = 1;
    let root = temp("pre");
    let ok = mcla_install::preflight::preflight(&root.join("fresh"), None, &m, &Ctx::silent()).unwrap();
    assert!(ok.free_bytes > 0);
    // a file instead of a folder
    let f = root.join("file.txt");
    std::fs::write(&f, b"x").unwrap();
    assert_eq!(mcla_install::preflight::preflight(&f, None, &m, &Ctx::silent()).unwrap_err().kind, ErrorKind::BadFolder);
    // the disc image must not live inside data/
    let data = root.join("inst").join("data");
    std::fs::create_dir_all(&data).unwrap();
    let iso = data.join("x.iso");
    std::fs::write(&iso, b"x").unwrap();
    assert_eq!(mcla_install::preflight::preflight(&root.join("inst"), Some(&iso), &m, &Ctx::silent()).unwrap_err().kind, ErrorKind::BadFolder);
    // not enough space
    let mut big = m.clone();
    big.space.data_bytes = u64::MAX / 2;
    assert_eq!(mcla_install::preflight::preflight(&root, None, &big, &Ctx::silent()).unwrap_err().kind, ErrorKind::DiskSpace);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn checksum_matches_and_cancels() {
    let m = Manifest::embedded();
    let dir = temp("hash");
    let f = dir.join("abc.iso");
    std::fs::write(&f, b"abc").unwrap();
    let r = mcla_install::hash_iso(&f, &m, &Ctx::silent()).unwrap();
    assert_eq!(r.sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(r.state, mcla_install::DumpState::Unrecognised); // not the supported dump: a warning, not an error
    let cancel = Cancel::new();
    cancel.cancel();
    let e = mcla_install::hash_iso(&f, &m, &Ctx::new(Arc::new(|_| {}), cancel)).unwrap_err();
    assert!(e.is_cancelled());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn local_manifest_round_trips() {
    use mcla_install::layout::{LocalManifest, StageSummary};
    let root = temp("local");
    let l = Layout::new(&root);
    let mut paths = std::collections::BTreeMap::new();
    paths.insert("cache".to_string(), "data/cache".to_string());
    let lm = LocalManifest {
        schema: 1,
        data_version: 1,
        installed_unix: 1,
        iso: mcla_install::IsoIdentity { file_name: "x.iso".into(), size: 5, partition_base: 0, title_id: "1".into(), region_mask: "2".into(), rpf_entries: 3, sha256: None, dump: "not_checked".into() },
        ffmpeg: None,
        paths,
        stages: [("extract".to_string(), StageSummary { files: 2, bytes: 9 })].into_iter().collect(),
        files_list: "data/.install/files.tsv".into(),
        files_count: 2,
        files_bytes: 9,
        files_list_sha256: "00".into(),
        audio_problems: 0,
        shaders: Default::default(),
    };
    lm.save(&l).unwrap();
    let back = LocalManifest::load(&l).unwrap();
    assert_eq!(back.files_count, 2);
    assert_eq!(back.paths["cache"], "data/cache");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn shader_files_are_their_own_stage_and_a_changed_file_is_found() {
    use mcla_install::layout::LocalManifest;
    let root = temp("shaders");
    let l = Layout::new(&root);
    let dir = l.cache().join("shaders_wgsl");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(l.install_dir()).unwrap();
    std::fs::write(dir.join("xs_sky.wgsl"), b"fn xs_sky_vs() {}").unwrap();
    std::fs::write(l.cache().join("other"), b"12345").unwrap();
    for f in ["audio_pc/w.wav", "video_pc/v.mp4"] {
        std::fs::create_dir_all(l.data().join(f).parent().unwrap()).unwrap();
        std::fs::write(l.data().join(f), b"1").unwrap(); // the media folders must hold a file
    }
    let files = vec![
        mcla_install::extract::FileRecord { path: "cache/other".into(), size: 5 },
        mcla_install::extract::FileRecord { path: "cache/shaders_wgsl/xs_sky.wgsl".into(), size: 17 },
    ];
    write_file_atomic(&l.files_list(), list_text(&files).as_bytes()).unwrap();
    // the manifest knows the file's hash
    let hashes = mcla_install::shaders::hashes_of(&l.cache(), &["xs_sky.wgsl"]).unwrap();
    let mut lm = {
        let text = serde_json::json!({
            "schema": 1, "data_version": 2, "installed_unix": 1,
            "iso": { "file_name": "x.iso", "size": 5, "partition_base": 0, "title_id": "1", "region_mask": "2", "rpf_entries": 3, "sha256": null, "dump": "not_checked" },
            "ffmpeg": null, "paths": {}, "stages": {}, "files_list": "", "files_count": 2, "files_bytes": 22, "files_list_sha256": "00", "audio_problems": 0
        });
        serde_json::from_value::<LocalManifest>(text).unwrap()
    };
    lm.shaders = hashes;
    lm.save(&l).unwrap();
    let ctx = Ctx::silent();
    assert!(mcla_install::verify::verify(&l, &ctx).unwrap().ok());
    // same size, other content: only the hash tells
    std::fs::write(dir.join("xs_sky.wgsl"), b"fn xs_sky_vs() {;").unwrap();
    let rep = mcla_install::verify::verify(&l, &ctx).unwrap();
    assert_eq!(rep.changed, vec!["cache/shaders_wgsl/xs_sky.wgsl".to_string()]);
    assert!(rep.missing.is_empty() && rep.wrong_size.is_empty());
    assert_eq!(rep.damaged_stages(), vec!["shaders"], "a changed shader file must not send the whole extraction again");
    let mut st = State::default();
    st.done.insert("extract".into(), true);
    st.done.insert("shaders".into(), true);
    st.save(&l).unwrap();
    mcla_install::verify::prepare_repair(&l, &rep).unwrap();
    let st = State::load(&l);
    assert!(st.is_done("extract") && !st.is_done("shaders"));
    assert!(!dir.join("xs_sky.wgsl").exists());
    let _ = std::fs::remove_dir_all(&root);
}
