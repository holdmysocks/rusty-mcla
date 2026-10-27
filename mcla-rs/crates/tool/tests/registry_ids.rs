//! End-to-end test of the opaque-ID commands on a small made-up registry in a temp git repository (made-up addresses only).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const HEADER: &str = "id\tname\ttopic\tstatus\tverification\tport\tdoc\tnote\tdeviation\tclaimed_by\tclaimed_date\tattrs";

fn make_root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mcla_regids_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("docs/registry")).unwrap();
    std::fs::create_dir_all(d.join("mcla-rs/crates/game/src")).unwrap();
    let mut s = format!("{HEADER}\n");
    for (i, st) in ["partly-decoded", "partly-decoded"].iter().enumerate() {
        s.push_str(&format!("{:08X}\tf{i}\tAI\t{st}\t\t\t\tnote\t\t\t\tsize=8\n", 0x8225_0001u32 + i as u32 * 0x100));
    }
    std::fs::write(d.join("docs/registry/functions.tsv"), s).unwrap();
    std::fs::write(d.join("docs/registry/topics.tsv"), "topic\tdoc\nAI\tdocs/AI.md\n").unwrap();
    let git = |a: &[&str]| {
        Command::new("git").arg("-C").arg(&d).args(a).output().unwrap();
    };
    git(&["init", "-q"]);
    d
}

fn tool(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mcla-tool")).arg("registry").args(args).arg("--root").arg(root).output().unwrap()
}
fn out(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn ids_cite_scan_convert_check() {
    let root = make_root("a");
    let src = root.join("mcla-rs/crates/game/src/x.rs");
    std::fs::write(&src, "// old note sub_82250001\nfn a() {}\n").unwrap();
    // no IDs yet: check fails
    assert!(!tool(&root, &["check"]).status.success());
    assert!(tool(&root, &["ids-sync"]).status.success());
    let id = out(&tool(&root, &["id", "82250001"])).trim().to_string();
    assert!(id.starts_with("F-") && id.len() == 7, "{id}");
    assert_eq!(out(&tool(&root, &["id", "sub_82250001"])).trim(), id);
    assert_eq!(out(&tool(&root, &["id", "f0"])).trim(), id);
    assert!(out(&tool(&root, &["addr", &id])).contains("82250001"));
    assert!(out(&tool(&root, &["find", &id])).contains("[functions] 82250001"));
    // baseline holds the old citation; check passes
    assert!(tool(&root, &["cite-scan", "--write-baseline"]).status.success());
    let c = tool(&root, &["check"]);
    assert!(c.status.success(), "{}", out(&c));
    // a new citation fails, an ID citation passes, an unknown ID fails
    std::fs::write(&src, "// old note sub_82250001\nfn a() {}\n// new 0x82250101\n").unwrap();
    let c = tool(&root, &["check"]);
    assert!(!c.status.success() && out(&c).contains("x.rs:3"), "{}", out(&c));
    // dry run changes nothing, --write converts
    let d = tool(&root, &["cite-convert", "mcla-rs/crates/game/src/x.rs"]);
    assert!(d.status.success() && out(&d).contains("dry run"), "{}", out(&d));
    assert!(std::fs::read_to_string(&src).unwrap().contains("0x82250101"));
    let w = tool(&root, &["cite-convert", "--write", "mcla-rs/crates/game/src/x.rs"]);
    assert!(w.status.success(), "{}", out(&w));
    let text = std::fs::read_to_string(&src).unwrap();
    assert!(!text.contains("8225010") && text.contains("// new F-"), "{text}");
    // the old line is edited too (it is in the file): its citation became an ID as well
    assert!(!text.contains("sub_8225"), "{text}");
    let c = tool(&root, &["check"]);
    assert!(c.status.success(), "{}", out(&c));
    std::fs::write(&src, format!("{text}// typo F-10000\n")).unwrap();
    assert!(!tool(&root, &["check"]).status.success());
    // the baseline only shrinks
    std::fs::write(&src, "// again sub_82250001\n").unwrap();
    assert!(!tool(&root, &["cite-scan", "--write-baseline"]).status.success());
    let _ = std::fs::remove_dir_all(&root);
}
