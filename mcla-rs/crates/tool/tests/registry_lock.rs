//! Concurrency tests of `mcla-tool registry`: the lock, atomic writes, stale locks, a crash between temp write and rename,
//! and `set --from`. They spawn the built tool as separate processes on a small registry in a temp directory.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const HEADER: &str = "id\tname\ttopic\tstatus\tverification\tport\tdoc\tnote\tdeviation\tclaimed_by\tclaimed_date\tattrs";

fn make_root(tag: &str, rows: usize) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mcla_reglock_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("docs/registry")).unwrap();
    let mut s = format!("{HEADER}\n");
    for i in 0..rows {
        s.push_str(&format!("{:08X}\tf{i}\tAI\tunknown\t\t\t\t\t\t\t\tsize=4\n", 0x8213_0000u32 + i as u32 * 4));
    }
    std::fs::write(d.join("docs/registry/functions.tsv"), s).unwrap();
    d
}

fn tool(root: &Path, envs: &[(&str, &str)], args: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_mcla-tool"));
    c.arg("registry").args(args).arg("--root").arg(root);
    for (k, v) in envs {
        c.env(k, v);
    }
    c.output().unwrap()
}

fn id(i: usize) -> String {
    format!("{:08X}", 0x8213_0000u32 + i as u32 * 4)
}

/// Parses the file strictly: header, 12 fields per row. Returns rows.
fn parse(root: &Path) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(root.join("docs/registry/functions.tsv")).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next().unwrap(), HEADER);
    lines
        .map(|l| {
            let f: Vec<String> = l.split('\t').map(|s| s.to_string()).collect();
            assert_eq!(f.len(), 12, "torn row: {l:?}");
            f
        })
        .collect()
}

#[test]
fn concurrent_sets_lose_nothing() {
    const WORKERS: usize = 6;
    const EACH: usize = 200;
    let root = make_root("stress", WORKERS * EACH);
    let stop = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|s| {
        let reader = s.spawn(|| {
            let mut n = 0;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                parse(&root); // a plain reader must never see a half-written file
                let o = tool(&root, &[], &["find", "f1", "--limit", "1"]);
                assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
                n += 1;
            }
            n
        });
        let ws: Vec<_> = (0..WORKERS)
            .map(|w| {
                let root = &root;
                s.spawn(move || {
                    for k in 0..EACH {
                        let i = w * EACH + k;
                        let o = tool(root, &[], &["set", &id(i), "partly-decoded", "--note", &format!("w{w}-{k}")]);
                        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
                    }
                })
            })
            .collect();
        for w in ws {
            w.join().unwrap();
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(reader.join().unwrap() > 0);
    });
    let rows = parse(&root);
    assert_eq!(rows.len(), WORKERS * EACH);
    for (i, r) in rows.iter().enumerate() {
        assert_eq!(r[3], "partly-decoded", "row {i} lost");
        assert_eq!(r[7], format!("w{}-{}", i / EACH, i % EACH), "row {i} lost");
    }
    assert!(!root.join("docs/registry/.registry.lock").exists());
    assert!(std::fs::read_dir(root.join("docs/registry")).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().contains(".tmp.")));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn stale_lock_is_broken_and_fresh_lock_times_out() {
    let root = make_root("stale", 4);
    let lock = root.join("docs/registry/.registry.lock");
    // fresh lock of a live-looking owner: bounded wait, clear error, nothing written
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    std::fs::write(&lock, format!("pid=4242\ntime={now}\n")).unwrap();
    let o = tool(&root, &[("MCLA_REGISTRY_LOCK_WAIT_SECS", "1")], &["set", &id(0), "partly-decoded", "--note", "x"]);
    assert!(!o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("locked by pid 4242"), "{err}");
    assert_eq!(parse(&root)[0][3], "unknown");
    // old lock of a dead process: broken, command succeeds, lock gone
    std::fs::write(&lock, "pid=4242\ntime=1000\n").unwrap();
    let o = tool(&root, &[], &["set", &id(0), "partly-decoded", "--note", "x"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(parse(&root)[0][3], "partly-decoded");
    assert!(!lock.exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn crash_between_temp_write_and_rename_changes_nothing() {
    let root = make_root("crash", 4);
    let before = std::fs::read(root.join("docs/registry/functions.tsv")).unwrap();
    let o = tool(&root, &[("MCLA_REGISTRY_CRASH_AFTER_TEMP", "1")], &["set", &id(1), "partly-decoded", "--note", "x"]);
    assert_eq!(o.status.code(), Some(101));
    assert_eq!(std::fs::read(root.join("docs/registry/functions.tsv")).unwrap(), before, "target must be untouched");
    parse(&root);
    // the dead writer left its lock and a temp file; once the lock is stale the next writer cleans up and proceeds
    assert!(root.join("docs/registry/.registry.lock").exists());
    let o = tool(&root, &[("MCLA_REGISTRY_STALE_SECS", "0")], &["set", &id(1), "partly-decoded", "--note", "y"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(parse(&root)[1][7], "y");
    assert!(std::fs::read_dir(root.join("docs/registry")).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().contains(".tmp.")));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn set_from_file_is_one_write_and_all_or_nothing() {
    let root = make_root("from", 50);
    let mut f = String::from("id\tstatus\tnote\n");
    for i in 0..50 {
        f.push_str(&format!("{}\tpartly-decoded\tbatch {i}\n", id(i)));
    }
    let file = root.join("batch.tsv");
    std::fs::write(&file, &f).unwrap();
    let o = tool(&root, &[], &["set", "--from", file.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let rows = parse(&root);
    assert!(rows.iter().enumerate().all(|(i, r)| r[3] == "partly-decoded" && r[7] == format!("batch {i}")));
    // a bad row (ported-verified without --verify) rejects the whole file
    let before = std::fs::read(root.join("docs/registry/functions.tsv")).unwrap();
    std::fs::write(&file, format!("id\tstatus\tnote\n{}\tunknown\tz\n{}\tported-verified\tz\n", id(0), id(1))).unwrap();
    let o = tool(&root, &[], &["set", "--from", file.to_str().unwrap()]);
    assert!(!o.status.success());
    assert_eq!(std::fs::read(root.join("docs/registry/functions.tsv")).unwrap(), before);
    // --keep-going applies the good rows
    let o = tool(&root, &[], &["set", "--from", file.to_str().unwrap(), "--keep-going"]);
    assert!(o.status.success());
    let rows = parse(&root);
    assert_eq!(rows[0][3], "unknown");
    assert_eq!(rows[1][3], "partly-decoded");
    let _ = std::fs::remove_dir_all(&root);
}
