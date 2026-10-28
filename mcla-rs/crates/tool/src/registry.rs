//! `mcla-tool registry` - the decode registry (docs/registry/*.tsv, schema in docs/registry/SCHEMA.md).
//!
//! ```text
//! mcla-tool registry find <address|name|keyword|topic> [--registry R] [--status S] [--limit N]
//! mcla-tool registry claim <id> <session> [--registry R] [--force]
//! mcla-tool registry release <id> [--registry R]
//! mcla-tool registry set <id[,id..]> <status> [--verify how@evidence] [--port p] [--doc d] [--note n]
//!                        [--topic t] [--deviation Dn] [--name n] [--registry R]
//!                        [--rbd-class C --reads "what the port reads instead" --missing "proof still missing"]  (status replaced-by-data)
//! mcla-tool registry coverage [topic]
//! mcla-tool registry check [--all]
//! mcla-tool registry report
//! mcla-tool registry hits <file.tsv> [--top N] [--dry-run]   (function hit counter import, registry/hits.rs)
//! ```
//! The registry is a set of tab-separated files with one shared column set, one row per item. The tool
//! rewrites only the file it changes, keeping the row order, so diffs stay small.

use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

mod hits;
mod ids;
use ids::Ids;

pub const COLS: [&str; 12] = [
    "id", "name", "topic", "status", "verification", "port", "doc", "note", "deviation", "claimed_by",
    "claimed_date", "attrs",
];
const ID: usize = 0;
const NAME: usize = 1;
const TOPIC: usize = 2;
const STATUS: usize = 3;
const VERIFY: usize = 4;
const PORT: usize = 5;
const DOC: usize = 6;
const NOTE: usize = 7;
const DEVIATION: usize = 8;
const CLAIMED_BY: usize = 9;
const CLAIMED_DATE: usize = 10;
const ATTRS: usize = 11;

pub const STATUSES: [&str; 8] = [
    "ported-verified",
    "ported-unverified",
    "decoded-not-ported",
    "partly-decoded",
    "not-needed",
    "unknown",
    "deviation",
    "replaced-by-data",
];
/// Statuses that close an item (nothing more to do for 1:1): verified, deliberately different, or not needed.
/// `replaced-by-data` is KNOWN but NOT closed: it is open until the proof of D34 / R-D34 exists (then the row becomes `deviation` D34).
pub const CLOSED: [&str; 3] = ["ported-verified", "deviation", "not-needed"];
/// Classes of `replaced-by-data` (rule R18 in docs/registry/SCHEMA.md); the class is `rbd=<class>` in `attrs`.
pub const RBD_CLASSES: [&str; 7] = [
    "resource-fixup",
    "class-registration",
    "tune-defaults",
    "data-loader",
    "data-ctor",
    "data-dtor",
    "array-alloc",
];
/// Statuses a row may leave to become `replaced-by-data` (ported-* and closed rows are never pulled back).
const RBD_FROM: [&str; 4] = ["unknown", "decoded-not-ported", "partly-decoded", "replaced-by-data"];
pub const REGISTRIES: [&str; 11] = [
    "functions", "natives", "tune", "formats", "datafiles", "shaders", "movies", "sounds", "cutscenes", "scripts",
    "savefields",
];
const VERIFY_HOW: [&str; 4] = ["bit-exact", "numbers", "read-only", "none"];
/// First and one-past-last address of the recompiled code (REX_CODE_BASE, REX_CODE_SIZE of the static recomp).
pub const CODE_START: u32 = 0x8213_0000;
pub const CODE_END: u32 = 0x827C_D054;

pub struct Table {
    pub name: String,
    pub path: PathBuf,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    pub fn parse(name: &str, path: &Path, text: &str) -> Result<Table> {
        let mut lines = text.lines();
        let head = lines.next().unwrap_or("");
        let cols: Vec<&str> = head.split('\t').collect();
        if cols != COLS {
            bail!("{}: header is not the registry header ({})", path.display(), COLS.join("\t"));
        }
        let mut rows = Vec::new();
        for (i, l) in lines.enumerate() {
            if l.is_empty() {
                continue;
            }
            let mut f: Vec<String> = l.split('\t').map(|s| s.to_string()).collect();
            if f.len() > COLS.len() {
                bail!("{}:{}: {} fields, expected {}", path.display(), i + 2, f.len(), COLS.len());
            }
            f.resize(COLS.len(), String::new());
            rows.push(f);
        }
        Ok(Table { name: name.to_string(), path: path.to_path_buf(), rows })
    }
    pub fn load(root: &Path, name: &str) -> Result<Table> {
        let path = root.join("docs/registry").join(format!("{name}.tsv"));
        let text = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        Table::parse(name, &path, &text)
    }
    pub fn to_text(&self) -> String {
        let mut s = COLS.join("\t");
        s.push('\n');
        for r in &self.rows {
            s.push_str(&r.join("\t"));
            s.push('\n');
        }
        s
    }
    /// Atomic (temp file, fsync, rename) and skipped when the file already has this text. Call only while holding the registry lock.
    pub fn save(&self) -> Result<()> {
        let text = self.to_text();
        if std::fs::read(&self.path).map(|old| old == text.as_bytes()).unwrap_or(false) {
            return Ok(());
        }
        write_atomic(&self.path, text.as_bytes())
    }
    pub fn find_id(&self, id: &str) -> Option<usize> {
        let id = norm_id(&self.name, id);
        self.rows.iter().position(|r| r[ID].eq_ignore_ascii_case(&id))
    }
}

/// Address-like ids are stored as 8 upper-case hex digits (functions) - accept `sub_8225ABCD`, `0x8225abcd`.
fn norm_id(registry: &str, id: &str) -> String {
    if registry == "functions" {
        let t = id.strip_prefix("sub_").or_else(|| id.strip_prefix("0x")).unwrap_or(id);
        if t.len() == 8 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
            return t.to_ascii_uppercase();
        }
    }
    id.to_string()
}

pub fn attr<'a>(attrs: &'a str, key: &str) -> Option<&'a str> {
    attrs.split(';').find_map(|kv| kv.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
}

pub fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (y, m, d) = civil(secs as i64 / 86400);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 to (year, month, day) - Howard Hinnant's algorithm.
pub fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn find_root(explicit: Option<&str>) -> Result<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Some(r) = explicit {
        cands.push(PathBuf::from(r));
    }
    if let Ok(r) = std::env::var("MCLA_ROOT") {
        cands.push(PathBuf::from(r));
    }
    if let Ok(mut d) = std::env::current_dir() {
        loop {
            cands.push(d.clone());
            if !d.pop() {
                break;
            }
        }
    }
    cands.push(PathBuf::from(r"D:\MCLA"));
    for c in cands {
        if c.join("docs/registry/functions.tsv").exists() {
            return Ok(c);
        }
    }
    bail!("docs/registry/functions.tsv not found (run from inside the repository or pass --root <dir> / set MCLA_ROOT)")
}

/// Address -> function row, by the `size=` attribute (instruction count) of each function.
pub struct FuncIndex {
    starts: Vec<(u32, u32, usize)>, // start, end (exclusive), row
}

impl FuncIndex {
    pub fn new(t: &Table) -> FuncIndex {
        let mut starts = Vec::new();
        for (i, r) in t.rows.iter().enumerate() {
            if r[ID].len() == 8 {
                if let Ok(a) = u32::from_str_radix(&r[ID], 16) {
                    let n: u32 = attr(&r[ATTRS], "size").and_then(|v| v.parse().ok()).unwrap_or(1);
                    starts.push((a, a + n.max(1) * 4, i));
                }
            }
        }
        starts.sort();
        FuncIndex { starts }
    }
    pub fn resolve(&self, a: u32) -> Option<usize> {
        let i = self.starts.partition_point(|s| s.0 <= a);
        if i == 0 {
            return None;
        }
        let s = self.starts[i - 1];
        (a < s.1).then_some(s.2)
    }
}

/// Addresses (8 hex digits starting 0x82..0x827C) cited in `text`, with a 1-based line number.
/// A citation is `sub_XXXXXXXX` (strong: names a function), `0xXXXXXXXX` or a bare token that is not part of a longer word
/// (these may also be data words inside the code range - jump tables, literals - so they only count when a function contains them).
pub fn cited_addresses(text: &str) -> Vec<(u32, usize, bool)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut line = 1usize;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if c == b'8' && i + 8 <= b.len() && b[i + 1] == b'2' {
            let before_ok = i == 0
                || !b[i - 1].is_ascii_alphanumeric()
                || (i >= 2 && (b[i - 1] == b'x' || b[i - 1] == b'X') && b[i - 2] == b'0' && (i == 2 || !b[i - 3].is_ascii_alphanumeric()));
            let tok = &b[i..i + 8];
            let after_ok = i + 8 == b.len() || !b[i + 8].is_ascii_alphanumeric();
            if before_ok && after_ok && tok.iter().all(|x| x.is_ascii_hexdigit()) {
                let a = u32::from_str_radix(std::str::from_utf8(tok).unwrap(), 16).unwrap();
                if (CODE_START..CODE_END).contains(&a) {
                    out.push((a, line, i >= 4 && &b[i - 4..i] == b"sub_"));
                }
                i += 8;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn walk(dir: &Path, exts: &[&str], skip: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if !skip.contains(&name.as_str()) {
                walk(&p, exts, skip, out);
            }
        } else if let Some(x) = p.extension().and_then(|x| x.to_str()) {
            if exts.contains(&x) {
                out.push(p);
            }
        }
    }
}

/// Files whose address citations the registry must know: docs (not the generated ones), port source and tools.
pub fn cite_files(root: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    walk(&root.join("docs"), &["md", "txt", "tsv"], &["registry", "_original", "_cleanup", "images"], &mut v);
    walk(&root.join("mcla-rs/crates"), &["rs"], &["target"], &mut v);
    walk(&root.join("mcla-rs/tools"), &["py"], &["__pycache__"], &mut v);
    // DECODED.md is generated; this file's tests hold made-up addresses
    v.retain(|p| p.file_name().map(|n| n != "DECODED.md").unwrap_or(true) && !p.to_string_lossy().replace('\\', "/").contains("crates/tool/src/registry"));
    v.sort();
    v
}

// ---------------------------------------------------------------- locking, atomic writes, snapshots
//
// Every command that changes a registry file holds `docs/registry/.registry.lock` (created with create_new; it holds the owner's pid
// and unix time) for its whole read-modify-write, and writes each file as temp + fsync + rename, so a reader never sees half a file.
// `.registry.gen` is a counter bumped under the lock after every write; readers re-read when it changed or the lock is held.

const LOCK_NAME: &str = ".registry.lock";
const GEN_NAME: &str = ".registry.gen";

fn env_secs(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// `(pid, unix time)` written in the lock file; the time falls back to the file's mtime while the owner has not written it yet.
fn lock_owner(path: &Path) -> (Option<u32>, u64) {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let get = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)).and_then(|v| v.trim().parse::<u64>().ok());
    let time = get("time=").unwrap_or_else(|| {
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0)
    });
    (get("pid=").map(|p| p as u32), time)
}

pub struct RegistryLock {
    path: PathBuf,
    token: String,
}

impl Drop for RegistryLock {
    fn drop(&mut self) {
        // only remove the lock when it is still ours (it may have been broken as stale and re-taken)
        if std::fs::read_to_string(&self.path).map(|t| t == self.token).unwrap_or(false) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Take the exclusive registry lock, waiting up to `MCLA_REGISTRY_LOCK_WAIT_SECS` (default 120). A lock older than
/// `MCLA_REGISTRY_STALE_SECS` (default 180; no command holds it that long) belongs to a dead process and is broken.
pub fn acquire_lock(root: &Path) -> Result<RegistryLock> {
    use std::io::Write;
    let dir = root.join("docs/registry");
    let path = dir.join(LOCK_NAME);
    let wait = env_secs("MCLA_REGISTRY_LOCK_WAIT_SECS", 120);
    let stale = env_secs("MCLA_REGISTRY_STALE_SECS", 180);
    let start = std::time::Instant::now();
    let token = format!("pid={}\ntime={}\n", std::process::id(), unix_now());
    loop {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                f.write_all(token.as_bytes())?;
                f.sync_all()?;
                // temp files left by a crashed writer (we are the only writer now)
                if let Ok(rd) = std::fs::read_dir(&dir) {
                    for e in rd.flatten() {
                        if e.file_name().to_string_lossy().contains(".tmp.") {
                            let _ = std::fs::remove_file(e.path());
                        }
                    }
                }
                return Ok(RegistryLock { path, token });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists || e.kind() == std::io::ErrorKind::PermissionDenied => {
                let (pid, t) = lock_owner(&path);
                let age = unix_now().saturating_sub(t);
                let who = pid.map(|p| p.to_string()).unwrap_or_else(|| "?".into());
                if age >= stale {
                    eprintln!("registry: breaking stale lock (owner pid {who}, {age} s old)");
                    let gone = path.with_file_name(format!("{LOCK_NAME}.stale.{}.{}", std::process::id(), start.elapsed().as_nanos()));
                    // the rename succeeds for one waiter only; the others see the new lock
                    if std::fs::rename(&path, &gone).is_ok() {
                        let _ = std::fs::remove_file(&gone);
                    }
                    continue;
                }
                if start.elapsed().as_secs() >= wait {
                    bail!(
                        "registry is locked by pid {who} for {age} s (waited {wait} s): another session is writing it, retry in a moment. If that process is gone, delete {} (or wait until the lock is {stale} s old)",
                        path.display()
                    );
                }
                std::thread::sleep(std::time::Duration::from_millis(25 + (std::process::id() as u64 % 40)));
            }
            Err(e) => return Err(e).with_context(|| format!("create {}", path.display())),
        }
    }
}

/// Temp file in the same folder, fsync, rename over the target (retried: on Windows a reader can briefly block the rename).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = path.with_file_name(format!("{name}.tmp.{}", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    if std::env::var("MCLA_REGISTRY_CRASH_AFTER_TEMP").is_ok() {
        std::process::exit(101); // test hook: die between the temp write and the rename
    }
    let mut last = None;
    for i in 0..40 {
        match std::fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(10 + i * 5));
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Err(last.unwrap()).with_context(|| format!("rename {} over {}", tmp.display(), path.display()))
}

fn read_gen(root: &Path) -> String {
    std::fs::read_to_string(root.join("docs/registry").join(GEN_NAME)).unwrap_or_default()
}

fn bump_gen(root: &Path) -> Result<()> {
    let g: u64 = read_gen(root).trim().parse().unwrap_or(0);
    write_atomic(&root.join("docs/registry").join(GEN_NAME), format!("{}\n", g + 1).as_bytes())
}

/// Run a read-modify-write under the exclusive lock; the generation counter is bumped when `f` succeeds.
fn locked<T>(root: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let _lock = acquire_lock(root)?;
    let out = f()?;
    bump_gen(root)?;
    Ok(out)
}

/// A consistent view of all tables: reload when a writer holds the lock or finished while we read.
fn load_snapshot(root: &Path) -> Result<Vec<Table>> {
    let lock = root.join("docs/registry").join(LOCK_NAME);
    let mut tries = 0;
    loop {
        let before = read_gen(root);
        if !lock.exists() {
            let t = load_all(root)?;
            if read_gen(root) == before && !lock.exists() {
                return Ok(t);
            }
        }
        tries += 1;
        if tries >= 200 {
            eprintln!("registry: a writer kept the lock for 10 s; reading the files as they are now");
            return load_all(root);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

// ---------------------------------------------------------------- commands

struct Opts {
    pos: Vec<String>,
    flags: HashMap<String, String>,
}

fn parse_opts(args: &[String], value_flags: &[&str]) -> Result<Opts> {
    let mut pos = Vec::new();
    let mut flags = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(f) = a.strip_prefix("--") {
            if value_flags.contains(&f) {
                i += 1;
                let v = args.get(i).with_context(|| format!("--{f} needs a value"))?;
                flags.insert(f.to_string(), v.clone());
            } else {
                flags.insert(f.to_string(), String::new());
            }
        } else {
            pos.push(a.clone());
        }
        i += 1;
    }
    Ok(Opts { pos, flags })
}

pub fn run(args: &[String]) -> Result<()> {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("");
    let o = parse_opts(
        args.get(1..).unwrap_or(&[]),
        &["registry", "status", "limit", "top", "verify", "port", "doc", "note", "topic", "deviation", "name", "root", "ids", "from", "rbd-class", "reads", "missing"],
    )?;
    let root = find_root(o.flags.get("root").map(|s| s.as_str()))?;
    match sub {
        "find" => cmd_find(&root, &o),
        "claim" => locked(&root, || cmd_claim(&root, &o)),
        "release" => locked(&root, || cmd_release(&root, &o)),
        "set" => locked(&root, || cmd_set(&root, &o)),
        "coverage" => cmd_coverage(&root, &o),
        "check" => cmd_check(&root, &o),
        "report" => locked(&root, || cmd_report(&root)),
        "bulk-set" if o.flags.contains_key("dry-run") => cmd_bulk_set(&root, &o),
        "bulk-set" => locked(&root, || cmd_bulk_set(&root, &o)),
        "hits" if o.flags.contains_key("dry-run") => hits::cmd_hits(&root, &o),
        "hits" => locked(&root, || hits::cmd_hits(&root, &o)),
        "with-lock" => cmd_with_lock(&root, args),
        "id" if o.flags.contains_key("new") => locked(&root, || ids::cmd_id(&root, &o)),
        "id" => ids::cmd_id(&root, &o),
        "addr" => ids::cmd_addr(&root, &o),
        "ids-sync" => locked(&root, || ids::cmd_ids_sync(&root)),
        "cite-scan" if o.flags.contains_key("write-baseline") => ids::cmd_cite_scan(&root, &o),
        "cite-scan" => ids::cmd_cite_scan(&root, &o),
        "cite-convert" => ids::cmd_cite_convert(&root, &o),
        _ => bail!("usage: mcla-tool registry <find|claim|release|set|bulk-set|coverage|check|report|with-lock|hits|id|addr|ids-sync|cite-scan|cite-convert> ... (see crates/tool/src/registry.rs, docs/registry/SCHEMA.md)"),
    }
}

fn load_all(root: &Path) -> Result<Vec<Table>> {
    let mut v = Vec::new();
    for r in REGISTRIES {
        let p = root.join("docs/registry").join(format!("{r}.tsv"));
        if p.exists() {
            v.push(Table::load(root, r)?);
        }
    }
    Ok(v)
}

fn fmt_row(reg: &str, r: &[String]) -> String {
    let mut s = format!("[{reg}] {} {} | {} | {}", r[ID], r[NAME], r[TOPIC], r[STATUS]);
    for (label, i) in [("verify", VERIFY), ("port", PORT), ("doc", DOC), ("dev", DEVIATION), ("claimed", CLAIMED_BY)] {
        if !r[i].is_empty() {
            let _ = write!(s, " | {label}: {}", r[i]);
            if i == CLAIMED_BY && !r[CLAIMED_DATE].is_empty() {
                let _ = write!(s, " ({})", r[CLAIMED_DATE]);
            }
        }
    }
    if !r[NOTE].is_empty() {
        let _ = write!(s, " | {}", r[NOTE]);
    }
    if !r[ATTRS].is_empty() {
        let _ = write!(s, " | {}", r[ATTRS]);
    }
    s
}

fn cmd_find(root: &Path, o: &Opts) -> Result<()> {
    let q = o.pos.first().context("find <address|name|keyword|topic|F-#####>")?;
    let ids = Ids::load(root)?;
    // an opaque ID (F-##### / A-#####) is looked up through its address
    let q_key;
    let q = if ids::parse_fid(q).is_some() {
        q_key = ids.key_of(q).with_context(|| format!("{q}: not an assigned ID"))?;
        &q_key
    } else {
        q
    };
    let limit: usize = o.flags.get("limit").and_then(|v| v.parse().ok()).unwrap_or(40);
    let only = o.flags.get("registry");
    let status = o.flags.get("status");
    let ql = q.to_lowercase();
    let tables = load_snapshot(root)?;
    // an address: exact function or the function that contains it
    let addr = {
        let t = q.strip_prefix("sub_").or_else(|| q.strip_prefix("0x")).unwrap_or(q);
        (t.len() == 8 && t.bytes().all(|b| b.is_ascii_hexdigit())).then(|| u32::from_str_radix(t, 16).unwrap())
    };
    let mut shown = 0usize;
    let mut total = 0usize;
    for t in &tables {
        if only.map(|r| r != &t.name).unwrap_or(false) {
            continue;
        }
        let fi = (t.name == "functions").then(|| FuncIndex::new(t));
        for (i, r) in t.rows.iter().enumerate() {
            if status.map(|s| s != &r[STATUS]).unwrap_or(false) {
                continue;
            }
            let hit = if let (Some(a), Some(fi)) = (addr, &fi) {
                fi.resolve(a) == Some(i)
            } else {
                r[ID].eq_ignore_ascii_case(q)
                    || r[TOPIC].eq_ignore_ascii_case(q)
                    || [NAME, NOTE, DOC, PORT, ATTRS, VERIFY, CLAIMED_BY].iter().any(|&c| r[c].to_lowercase().contains(&ql))
            };
            if hit {
                total += 1;
                if shown < limit {
                    let cite = if t.name == "functions" { ids.get('F', &r[ID]).map(|f| format!(" | cite: {f}")).unwrap_or_default() } else { String::new() };
                    println!("{}{cite}", fmt_row(&t.name, r));
                    shown += 1;
                }
            }
        }
    }
    if total == 0 {
        println!("no row matches '{q}' (nothing is registered for it: decode it, then add it with `registry set`)");
    } else if total > shown {
        println!("... {} more (use --limit N, --registry R, --status S)", total - shown);
    }
    Ok(())
}

fn pick<'a>(tables: &'a mut [Table], reg: Option<&String>, id: &str) -> Result<(&'a mut Table, usize)> {
    let mut found = Vec::new();
    for (ti, t) in tables.iter().enumerate() {
        if reg.map(|r| r != &t.name).unwrap_or(false) {
            continue;
        }
        if let Some(i) = t.find_id(id) {
            found.push((ti, i));
        }
    }
    match found.len() {
        0 => bail!("no row with id '{id}'{}", reg.map(|r| format!(" in registry {r}")).unwrap_or_default()),
        1 => {
            let (ti, i) = found[0];
            Ok((&mut tables[ti], i))
        }
        _ => bail!("id '{id}' is in several registries, pass --registry R"),
    }
}

fn clean(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

pub fn claim_row(r: &mut [String], session: &str, date: &str, force: bool) -> Result<()> {
    if !r[CLAIMED_BY].is_empty() && r[CLAIMED_BY] != session && !force {
        bail!("{} is already claimed by {} since {} (use --force only after asking that session)", r[ID], r[CLAIMED_BY], r[CLAIMED_DATE]);
    }
    r[CLAIMED_BY] = session.to_string();
    r[CLAIMED_DATE] = date.to_string();
    Ok(())
}

fn cmd_claim(root: &Path, o: &Opts) -> Result<()> {
    let id = o.pos.first().context("claim <id> <session>")?;
    let session = o.pos.get(1).context("claim <id> <session>")?;
    let mut tables = load_all(root)?;
    let (t, i) = pick(&mut tables, o.flags.get("registry"), id)?;
    claim_row(&mut t.rows[i], &clean(session), &today(), o.flags.contains_key("force"))?;
    println!("{}", fmt_row(&t.name, &t.rows[i]));
    t.save()
}

fn cmd_release(root: &Path, o: &Opts) -> Result<()> {
    let id = o.pos.first().context("release <id>")?;
    let mut tables = load_all(root)?;
    let (t, i) = pick(&mut tables, o.flags.get("registry"), id)?;
    t.rows[i][CLAIMED_BY].clear();
    t.rows[i][CLAIMED_DATE].clear();
    t.save()
}

/// Apply `set` to a row (validates the combination of fields). Pure, for the tests.
pub fn apply_set(r: &mut [String], status: &str, f: &HashMap<String, String>) -> Result<()> {
    if !STATUSES.contains(&status) {
        bail!("status '{status}' is not one of {}", STATUSES.join(", "));
    }
    let prev = r[STATUS].clone();
    if status == "replaced-by-data" && !RBD_FROM.contains(&prev.as_str()) {
        bail!("replaced-by-data can only follow unknown / decoded-not-ported / partly-decoded (this row is {prev})");
    }
    let from_rbd = prev == "replaced-by-data" && status == "deviation";
    r[STATUS] = status.to_string();
    for (flag, col) in [("verify", VERIFY), ("port", PORT), ("doc", DOC), ("note", NOTE), ("topic", TOPIC), ("deviation", DEVIATION), ("name", NAME)] {
        if let Some(v) = f.get(flag) {
            r[col] = clean(v);
        }
    }
    // a hand change ends the seed: the seeder keeps rows without `seed=1` when it is re-run
    r[ATTRS] = r[ATTRS].split(';').filter(|kv| *kv != "seed=1" && !kv.is_empty()).collect::<Vec<_>>().join(";");
    let flag = |a: &str| f.get(a).or_else(|| f.get(&a.replace('-', "_"))).map(|v| clean(v));
    let old_class = attr(&r[ATTRS], "rbd").map(|s| s.to_string());
    let strip = |attrs: &str| attrs.split(';').filter(|kv| !kv.starts_with("rbd=") && !kv.is_empty()).map(|s| s.to_string()).collect::<Vec<_>>();
    if status == "replaced-by-data" {
        let class = flag("rbd-class").or(old_class);
        let Some(class) = class.filter(|c| RBD_CLASSES.contains(&c.as_str())) else {
            bail!("replaced-by-data needs --rbd-class <{}>", RBD_CLASSES.join("|"));
        };
        let mut attrs = strip(&r[ATTRS]);
        attrs.push(format!("rbd={class}"));
        r[ATTRS] = attrs.join(";");
        // note layout: `RBD <class> | reads: <what the port reads instead> | missing proof: <what would close it>`
        let (reads, missing) = (flag("reads"), flag("missing"));
        if reads.is_some() || missing.is_some() {
            let old = r[NOTE].clone();
            let part = |key: &str| old.split(" | ").find_map(|p| p.strip_prefix(key)).map(|s| s.trim().to_string());
            let reads = reads.or_else(|| part("reads:")).unwrap_or_default();
            let missing = missing.or_else(|| part("missing proof:")).unwrap_or_default();
            // the earlier note (the triage reader's description) is kept after `| prev:`
            let prev_note = if old.starts_with("RBD ") { old.split(" | prev: ").nth(1).unwrap_or("").to_string() } else { old.clone() };
            r[NOTE] = format!("RBD {class} | reads: {reads} | missing proof: {missing}");
            if !prev_note.is_empty() {
                r[NOTE] = format!("{} | prev: {prev_note}", r[NOTE]);
            }
        }
        let filled = |key: &str| r[NOTE].split(" | ").find_map(|p| p.strip_prefix(key)).map(|v| !v.trim().is_empty()).unwrap_or(false);
        if !filled("reads:") || !filled("missing proof:") {
            bail!("replaced-by-data needs --reads <file/converter/parser the port uses instead> and --missing <proof still missing> (or a note with 'reads:' and 'missing proof:')");
        }
        if r[DOC].is_empty() {
            bail!("replaced-by-data needs --doc (the decoded rule)");
        }
    } else if let Some(c) = old_class {
        // leaving replaced-by-data: the class tag goes; a deviation keeps `rbd_from=<class>` so that `check` demands a real comparison
        let mut attrs = strip(&r[ATTRS]);
        if from_rbd {
            attrs.push(format!("rbd_from={c}"));
        }
        r[ATTRS] = attrs.join(";");
    }
    if status == "deviation" && r[DEVIATION] == "D34" {
        if !matches!(r[VERIFY].split_once('@'), Some((how, _)) if VERIFY_HOW[..3].contains(&how)) {
            bail!("deviation D34 needs --verify <bit-exact|numbers|read-only>@<evidence file> (R-D34)");
        }
        if attr(&r[ATTRS], "rbd_from").is_some() && !(r[VERIFY].starts_with("bit-exact@") || r[VERIFY].starts_with("numbers@")) {
            bail!("a replaced-by-data row closes as deviation D34 only with a comparison: --verify bit-exact@.. or numbers@.. (the converted data equals the original's output)");
        }
    }
    if status == "ported-verified" && r[VERIFY].is_empty() {
        bail!("ported-verified needs --verify <bit-exact|numbers>@<evidence file>");
    }
    if status == "deviation" && r[DEVIATION].is_empty() {
        bail!("deviation needs --deviation D<n> (an entry of docs/DECISIONS.md)");
    }
    if status == "not-needed" && r[NOTE].is_empty() {
        bail!("not-needed needs --note <reason>");
    }
    if CLOSED.contains(&status) || status == "ported-unverified" {
        // finished with this decode step: free the claim
        r[CLAIMED_BY].clear();
        r[CLAIMED_DATE].clear();
    }
    Ok(())
}

/// `set --from <file.tsv> [--keep-going]`: many rows under one lock and one write. The file is tab separated with a header line
/// naming columns: `id` and `status` are required; `registry verify port doc note topic deviation name rbd_class reads missing` are optional.
/// All rows are applied or (without `--keep-going`) none.
fn cmd_set_from(root: &Path, o: &Opts, file: &str) -> Result<()> {
    let text = std::fs::read_to_string(file).with_context(|| format!("reading {file}"))?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let head: Vec<&str> = lines.next().context("--from file is empty")?.split('\t').map(|h| h.trim()).collect();
    for need in ["id", "status"] {
        if !head.contains(&need) {
            bail!("--from file needs a header line with the columns id and status (optional: registry verify port doc note topic deviation name rbd_class reads missing)");
        }
    }
    for h in &head {
        if !["id", "status", "registry", "verify", "port", "doc", "note", "topic", "deviation", "name", "rbd_class", "reads", "missing"].contains(h) {
            bail!("--from file: unknown column '{h}'");
        }
    }
    let keep = o.flags.contains_key("keep-going");
    let mut tables = load_all(root)?;
    let (mut ok, mut failed) = (0usize, 0usize);
    for (n, l) in lines.enumerate() {
        let mut f: HashMap<String, String> = HashMap::new();
        for (h, c) in head.iter().zip(l.split('\t')) {
            if !c.is_empty() {
                f.insert(h.to_string(), c.to_string());
            }
        }
        let (Some(id), Some(status)) = (f.get("id").cloned(), f.get("status").cloned()) else {
            bail!("--from row {}: id or status empty", n + 2);
        };
        let reg = f.get("registry").or_else(|| o.flags.get("registry")).cloned();
        // apply to a copy: apply_set changes fields before it validates, and a rejected row must stay as it was
        let r = pick(&mut tables, reg.as_ref(), &id).and_then(|(t, i)| {
            let mut row = t.rows[i].clone();
            apply_set(&mut row, &status, &f)?;
            t.rows[i] = row;
            Ok(())
        });
        match r {
            Ok(()) => ok += 1,
            Err(e) if keep => {
                failed += 1;
                eprintln!("--from row {} ({id}): {e:#}", n + 2);
            }
            Err(e) => return Err(e).with_context(|| format!("--from row {} ({id}); nothing written", n + 2)),
        }
    }
    for t in &tables {
        t.save()?;
    }
    println!("set --from: {ok} rows set{}", if failed > 0 { format!(", {failed} failed (not applied)") } else { String::new() });
    Ok(())
}

/// `with-lock -- <command> [args..]`: run a command (the seeder, a script that edits the files) while holding the registry lock.
fn cmd_with_lock(root: &Path, args: &[String]) -> Result<()> {
    let at = args.iter().position(|a| a == "--").context("with-lock -- <command> [args..]")?;
    let cmd = args.get(at + 1).context("with-lock -- <command> [args..]")?;
    let _lock = acquire_lock(root)?;
    let st = std::process::Command::new(cmd).args(&args[at + 2..]).status().with_context(|| format!("run {cmd}"))?;
    if root.join("docs/registry").join(ids::IDS_FILE).exists() {
        ids::cmd_ids_sync(root)?; // rows added by the command get their IDs
    }
    bump_gen(root)?;
    if !st.success() {
        bail!("{cmd} exited with {st}");
    }
    Ok(())
}

fn cmd_set(root: &Path, o: &Opts) -> Result<()> {
    if let Some(f) = o.flags.get("from") {
        return cmd_set_from(root, o, f);
    }
    let ids = o.pos.first().context("set <id[,id..]> <status> [--verify ..]")?;
    let status = o.pos.get(1).context("set <id[,id..]> <status>")?;
    let mut tables = load_all(root)?;
    for id in ids.split(',') {
        let (t, i) = pick(&mut tables, o.flags.get("registry"), id)?;
        apply_set(&mut t.rows[i], status, &o.flags)?;
        println!("{}", fmt_row(&t.name, &t.rows[i]));
    }
    for t in &tables {
        t.save()?;
    }
    Ok(())
}

/// Apply a recorded rule to one row: only a row that is still `unknown` and not claimed changes (to `not-needed`, topic and
/// note from the rule, `rule=<id>` in `attrs`). Returns false when the row was left alone. Pure, for the tests.
pub fn apply_rule(r: &mut [String], rule: &str, note: &str, topic: &str) -> Result<bool> {
    if r[STATUS] != "unknown" || !r[CLAIMED_BY].is_empty() {
        return Ok(false);
    }
    let mut f = HashMap::new();
    f.insert("note".to_string(), format!("{rule}: {note}"));
    f.insert("topic".to_string(), topic.to_string());
    apply_set(r, "not-needed", &f)?;
    let attrs = r[ATTRS].split(';').filter(|kv| !kv.starts_with("rule=") && !kv.starts_with("topic_src=") && !kv.is_empty()).map(|s| s.to_string());
    r[ATTRS] = attrs.chain(std::iter::once(format!("rule={rule}"))).collect::<Vec<_>>().join(";");
    Ok(true)
}

/// `bulk-set <RULE> --note "<reason>" [--ids <file>] [--topic ENGINE] [--registry functions] [--dry-run]`: marks the ids listed in
/// `docs/registry/rules/<RULE>.txt` (one id per line or whitespace separated, `#` starts a comment) `not-needed` by that rule.
/// Rows that are not `unknown` (a status somebody set with evidence) or are claimed are never touched; they are listed.
fn cmd_bulk_set(root: &Path, o: &Opts) -> Result<()> {
    let rule = o.pos.first().context("bulk-set <RULE> --note <reason> [--ids file]")?;
    let note = o.flags.get("note").context("bulk-set needs --note <reason shared by the rule>")?;
    let topic = o.flags.get("topic").map(|s| s.as_str()).unwrap_or("ENGINE");
    let reg = o.flags.get("registry").map(|s| s.as_str()).unwrap_or("functions");
    let path = match o.flags.get("ids") {
        Some(p) => PathBuf::from(p),
        None => root.join("docs/registry/rules").join(format!("{rule}.txt")),
    };
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut ids = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("");
        ids.extend(line.split_whitespace().map(|s| s.to_string()));
    }
    let mut tables = if o.flags.contains_key("dry-run") { load_snapshot(root)? } else { load_all(root)? };
    let t = tables.iter_mut().find(|t| t.name == reg).with_context(|| format!("no registry '{reg}'"))?;
    let (mut changed, mut skipped) = (0usize, Vec::new());
    for id in &ids {
        let i = t.find_id(&norm_id(reg, id)).with_context(|| format!("{reg}: no row '{id}'"))?;
        if apply_rule(&mut t.rows[i], rule, note, topic)? {
            changed += 1;
        } else {
            skipped.push(format!("{id}({})", t.rows[i][STATUS]));
        }
    }
    println!("{rule}: {} ids, {changed} set not-needed, {} left alone (not unknown or claimed){}", ids.len(), skipped.len(), if o.flags.contains_key("dry-run") { " [dry run, nothing written]" } else { "" });
    if !skipped.is_empty() {
        println!("left alone: {}", skipped.join(" "));
    }
    if !o.flags.contains_key("dry-run") {
        for t in &tables {
            t.save()?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- coverage

#[derive(Default, Clone)]
pub struct Counts {
    pub by_status: BTreeMap<String, usize>,
    pub total: usize,
}
impl Counts {
    pub fn add(&mut self, status: &str) {
        *self.by_status.entry(status.to_string()).or_default() += 1;
        self.total += 1;
    }
    pub fn n(&self, s: &str) -> usize {
        self.by_status.get(s).copied().unwrap_or(0)
    }
    pub fn closed(&self) -> usize {
        CLOSED.iter().map(|s| self.n(s)).sum()
    }
    pub fn known(&self) -> usize {
        self.total - self.n("unknown")
    }
    /// Rows with a real decode behind them: everything except `unknown` and `partly-decoded` (a triage placeholder).
    pub fn decoded(&self) -> usize {
        self.total - self.n("unknown") - self.n("partly-decoded")
    }
}

pub fn pct(a: usize, b: usize) -> String {
    if b == 0 {
        "-".into()
    } else {
        format!("{:.1}%", a as f64 * 100.0 / b as f64)
    }
}

pub fn count_by<'a>(t: &'a Table, topic: Option<&str>) -> Counts {
    let mut c = Counts::default();
    for r in &t.rows {
        if topic.map(|x| !x.eq_ignore_ascii_case(&r[TOPIC])).unwrap_or(false) {
            continue;
        }
        c.add(&r[STATUS]);
    }
    c
}

fn status_header() -> String {
    format!(
        "{:<11} {:>7} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} | {:>7} {:>7}",
        "", "total", "verified", "unverif", "dec-nopt", "repl-dat", "partly", "not-need", "deviat", "UNKNOWN", "closed", "decoded"
    )
}
fn status_line(label: &str, c: &Counts) -> String {
    format!(
        "{:<11} {:>7} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} | {:>7} {:>7}",
        label,
        c.total,
        c.n("ported-verified"),
        c.n("ported-unverified"),
        c.n("decoded-not-ported"),
        c.n("replaced-by-data"),
        c.n("partly-decoded"),
        c.n("not-needed"),
        c.n("deviation"),
        c.n("unknown"),
        pct(c.closed(), c.total),
        pct(c.decoded(), c.total)
    )
}

fn cmd_coverage(root: &Path, o: &Opts) -> Result<()> {
    let tables = load_snapshot(root)?;
    let topic = o.pos.first().map(|s| s.as_str());
    println!("closed = ported-verified + deviation + not-needed; decoded = everything except unknown and partly-decoded (partly-decoded is triage, not a decode); repl-dat (replaced-by-data) is decoded but NOT closed until proven (then deviation D34)\n");
    println!("{}", status_header());
    let mut all = Counts::default();
    for t in &tables {
        let c = count_by(t, topic);
        println!("{}", status_line(&t.name, &c));
        for (k, v) in &c.by_status {
            *all.by_status.entry(k.clone()).or_default() += v;
        }
        all.total += c.total;
    }
    println!("{}", status_line("ALL", &all));
    if topic.is_none() {
        println!("\nper topic (all registries):\n{}", status_header());
        for (tp, c) in per_topic(&tables) {
            println!("{}", status_line(&tp, &c));
        }
    } else {
        println!("\nunknown rows of topic {} per registry are the work list: `registry find {} --status unknown`", topic.unwrap(), topic.unwrap());
    }
    Ok(())
}

pub fn per_topic(tables: &[Table]) -> BTreeMap<String, Counts> {
    let mut m: BTreeMap<String, Counts> = BTreeMap::new();
    for t in tables {
        for r in &t.rows {
            m.entry(r[TOPIC].clone()).or_default().add(&r[STATUS]);
        }
    }
    m
}

// ---------------------------------------------------------------- check

pub struct Problem {
    pub registry: String,
    pub id: String,
    pub msg: String,
}

fn load_topics(root: &Path) -> HashSet<String> {
    let mut s = HashSet::new();
    s.insert("UNASSIGNED".to_string());
    if let Ok(t) = std::fs::read_to_string(root.join("docs/registry/topics.tsv")) {
        for l in t.lines().skip(1) {
            if let Some(f) = l.split('\t').next() {
                if !f.is_empty() {
                    s.insert(f.to_string());
                }
            }
        }
    }
    s
}

fn deviations_known(root: &Path) -> HashSet<String> {
    let mut s = HashSet::new();
    if let Ok(t) = std::fs::read_to_string(root.join("docs/DECISIONS.md")) {
        for l in t.lines() {
            if let Some(rest) = l.strip_prefix("| D") {
                let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if !n.is_empty() {
                    s.insert(format!("D{n}"));
                }
            }
        }
    }
    s
}

fn path_exists(root: &Path, p: &str) -> bool {
    let p = p.split('#').next().unwrap_or(p).trim();
    if p.is_empty() {
        return false;
    }
    [root.join(p), root.join("mcla-rs").join(p), root.join("docs").join(p)].iter().any(|c| c.exists())
}

fn looks_like_path(s: &str) -> bool {
    s.contains('/') || s.contains('\\') || s.ends_with(".md") || s.ends_with(".rs") || s.ends_with(".txt") || s.ends_with(".py")
}

fn port_ok(root: &Path, entry: &str) -> bool {
    let (file, func) = match entry.split_once("::") {
        Some((f, n)) => (f, Some(n)),
        None => (entry, None),
    };
    let found = [root.join(file), root.join("mcla-rs").join(file)].into_iter().find(|p| p.is_file());
    let Some(p) = found else { return false };
    match func {
        None => true,
        Some(f) => {
            let f = f.trim_end_matches("()");
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            let pat = format!("fn {f}");
            text.match_indices(&pat).any(|(i, _)| {
                let after = text[i + pat.len()..].chars().next();
                !after.map(|c| c.is_alphanumeric() || c == '_').unwrap_or(false)
            })
        }
    }
}

pub fn check_rows(root: &Path, t: &Table, topics: &HashSet<String>, devs: &HashSet<String>) -> Vec<Problem> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |id: &str, msg: String| out.push(Problem { registry: t.name.clone(), id: id.to_string(), msg });
    for r in &t.rows {
        let id = r[ID].as_str();
        if id.is_empty() || !seen.insert(id.to_string()) {
            add(id, "empty or duplicate id".into());
        }
        let st = r[STATUS].as_str();
        if !STATUSES.contains(&st) {
            add(id, format!("bad status '{st}'"));
            continue;
        }
        if !topics.contains(&r[TOPIC]) {
            add(id, format!("unknown topic '{}' (docs/registry/topics.tsv)", r[TOPIC]));
        }
        if st == "ported-verified" {
            let v = &r[VERIFY];
            match v.split_once('@') {
                Some((how, ev)) if VERIFY_HOW[..2].contains(&how) && path_exists(root, ev) => {}
                Some((how, ev)) if VERIFY_HOW[..2].contains(&how) => add(id, format!("ported-verified: evidence '{ev}' does not exist")),
                _ => add(id, format!("ported-verified needs verification '<bit-exact|numbers>@<evidence file>', has '{v}'")),
            }
        } else if !r[VERIFY].is_empty() {
            match r[VERIFY].split_once('@') {
                Some((how, ev)) if VERIFY_HOW.contains(&how) && (ev.is_empty() || path_exists(root, ev)) => {}
                None if VERIFY_HOW.contains(&r[VERIFY].as_str()) => {}
                _ => add(id, format!("bad verification '{}' (<how>@<evidence file>)", r[VERIFY])),
            }
        }
        if st == "deviation" {
            if !devs.contains(&r[DEVIATION]) {
                add(id, format!("deviation '{}' has no entry in docs/DECISIONS.md", r[DEVIATION]));
            }
        } else if !r[DEVIATION].is_empty() {
            add(id, "deviation tag on a row that is not status deviation".into());
        }
        if st == "not-needed" && r[NOTE].is_empty() {
            add(id, "not-needed without a reason in note".into());
        }
        if st == "replaced-by-data" {
            match attr(&r[ATTRS], "rbd") {
                Some(c) if RBD_CLASSES.contains(&c) => {}
                other => add(id, format!("replaced-by-data needs attrs rbd=<class> of {}, has {:?}", RBD_CLASSES.join("|"), other)),
            }
            let filled = |key: &str| r[NOTE].split(" | ").find_map(|p| p.strip_prefix(key)).map(|v| !v.trim().is_empty()).unwrap_or(false);
            if !filled("reads:") || !filled("missing proof:") {
                add(id, "replaced-by-data needs a note with 'reads:' (what the port reads instead) and 'missing proof:'".into());
            }
            if r[DOC].is_empty() {
                add(id, "replaced-by-data without a doc".into());
            }
        } else if attr(&r[ATTRS], "rbd").is_some() {
            add(id, "attrs rbd= on a row that is not replaced-by-data".into());
        }
        if st == "deviation" && r[DEVIATION] == "D34" {
            let ev_ok = matches!(r[VERIFY].split_once('@'), Some((how, ev)) if VERIFY_HOW[..3].contains(&how) && path_exists(root, ev));
            if !ev_ok {
                add(id, "deviation D34 needs verification '<bit-exact|numbers|read-only>@<existing evidence file>' (R-D34)".into());
            }
            if !r[NOTE].contains("D34 E") {
                add(id, "deviation D34 needs a note 'D34 E1|E2|E3|E4: ..' naming the evidence kind (R-D34)".into());
            }
            if attr(&r[ATTRS], "rbd_from").is_some() && !(r[VERIFY].starts_with("bit-exact@") || r[VERIFY].starts_with("numbers@")) {
                add(id, "deviation D34 from replaced-by-data needs bit-exact@ or numbers@ evidence".into());
            }
        }
        if (st == "ported-verified" || st == "ported-unverified") && r[PORT].is_empty() {
            add(id, format!("{st} without a port file/function"));
        }
        if st == "decoded-not-ported" && r[DOC].is_empty() {
            add(id, "decoded-not-ported without a doc".into());
        }
        for p in r[PORT].split(';').filter(|p| !p.is_empty()) {
            if !port_ok(root, p) {
                add(id, format!("port '{p}': file or function does not exist"));
            }
        }
        for d in r[DOC].split(';').filter(|d| !d.is_empty()) {
            if looks_like_path(d) && !path_exists(root, d) {
                add(id, format!("doc '{d}' does not exist"));
            }
        }
        if !r[CLAIMED_BY].is_empty() && r[CLAIMED_DATE].is_empty() {
            add(id, "claimed_by without claimed_date".into());
        }
    }
    out
}

/// Citations of recompiled addresses in docs / source that the functions registry does not cover or leaves unknown.
pub fn check_citations(root: &Path, funcs: &Table) -> Vec<Problem> {
    let fi = FuncIndex::new(funcs);
    let mut out = Vec::new();
    let mut seen: HashSet<u32> = HashSet::new();
    for f in cite_files(root) {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let rel = f.strip_prefix(root).unwrap_or(&f).to_string_lossy().replace('\\', "/");
        for (a, line, strong) in cited_addresses(&text) {
            match fi.resolve(a) {
                None => {
                    if strong && seen.insert(a) {
                        out.push(Problem { registry: "functions".into(), id: format!("{a:08X}"), msg: format!("cited at {rel}:{line} but no row in functions.tsv") })
                    }
                }
                Some(i) if funcs.rows[i][STATUS] == "unknown" => {
                    if seen.insert(a) {
                        out.push(Problem {
                            registry: "functions".into(),
                            id: funcs.rows[i][ID].clone(),
                            msg: format!("cited at {rel}:{line} but the row is unknown (registry set {} <status> --doc/--port ..)", funcs.rows[i][ID]),
                        })
                    }
                }
                _ => {}
            }
        }
    }
    out
}

fn cmd_check(root: &Path, o: &Opts) -> Result<()> {
    let tables = load_snapshot(root)?;
    let topics = load_topics(root);
    let devs = deviations_known(root);
    let mut problems = Vec::new();
    for t in &tables {
        problems.extend(check_rows(root, t, &topics, &devs));
    }
    if let Some(f) = tables.iter().find(|t| t.name == "functions") {
        problems.extend(check_citations(root, f));
        let ids = Ids::load(root)?;
        if ids.rows.is_empty() {
            problems.push(Problem { registry: "ids".into(), id: "-".into(), msg: "no IDs yet: run `mcla-tool registry ids-sync`".into() });
        } else {
            for m in ids.problems(Some(f)) {
                problems.push(Problem { registry: "ids".into(), id: "-".into(), msg: m });
            }
        }
        problems.extend(ids::check_ids(root, f, &ids));
    }
    let limit = if o.flags.contains_key("all") { usize::MAX } else { 60 };
    for p in problems.iter().take(limit) {
        println!("FAIL [{}] {}: {}", p.registry, p.id, p.msg);
    }
    if problems.len() > limit {
        println!("... {} more (--all)", problems.len() - limit);
    }
    let rows: usize = tables.iter().map(|t| t.rows.len()).sum();
    if problems.is_empty() {
        println!("registry check OK ({} registries, {} rows)", tables.len(), rows);
        Ok(())
    } else {
        bail!("registry check FAILED: {} problem(s)", problems.len())
    }
}

// ---------------------------------------------------------------- report

fn cmd_report(root: &Path) -> Result<()> {
    let tables = load_all(root)?;
    let text = render_report(&tables);
    let out = root.join("docs/DECODED.md");
    write_atomic(&out, text.as_bytes())?;
    println!("wrote {}", out.display());
    // the status images in docs/status (unpublished); best effort, python may be missing
    let script = root.join("docs/registry/status_svg.py");
    if script.exists() {
        match std::process::Command::new("python").arg("-I").arg(&script).output() {
            Ok(o) if o.status.success() => print!("{}", String::from_utf8_lossy(&o.stdout)),
            Ok(o) => eprintln!("status images not updated: {}", String::from_utf8_lossy(&o.stderr).trim()),
            Err(e) => eprintln!("status images not updated (python not found): {e}"),
        }
    }
    Ok(())
}

pub fn render_report(tables: &[Table]) -> String {
    let mut s = String::new();
    s.push_str("# Decoded: what of the original is decoded, ported and verified\n\n");
    s.push_str("<!-- GENERATED by `mcla-tool registry report` from docs/registry/*.tsv. Do not edit by hand: change the registry with `mcla-tool registry set`. -->\n\n");
    s.push_str("Source of truth: `docs/registry/*.tsv` (schema: `docs/registry/SCHEMA.md`). Find before you decode: `mcla-tool registry find <address|name|keyword|topic>`.\n");
    s.push_str("**closed** = ported-verified + deviation + not-needed. **decoded** = everything except `unknown` and `partly-decoded` (a partly-decoded row is only triaged: read, not yet decoded). **replaced-by-data** is decoded but not closed: it waits for the proof of D34 (then it becomes `deviation`).\n\n");
    s.push_str("## Coverage per registry\n\n");
    let head = "| | total | ported-verified | ported-unverified | decoded-not-ported | replaced-by-data | partly-decoded | not-needed | deviation | **unknown** | closed | decoded |\n|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|\n";
    let line = |label: &str, c: &Counts| {
        format!(
            "| {label} | {} | {} | {} | {} | {} | {} | {} | {} | **{}** | {} | {} |\n",
            c.total,
            c.n("ported-verified"),
            c.n("ported-unverified"),
            c.n("decoded-not-ported"),
            c.n("replaced-by-data"),
            c.n("partly-decoded"),
            c.n("not-needed"),
            c.n("deviation"),
            c.n("unknown"),
            pct(c.closed(), c.total),
            pct(c.decoded(), c.total)
        )
    };
    s.push_str(head);
    let mut all = Counts::default();
    for t in tables {
        let c = count_by(t, None);
        s.push_str(&line(&t.name, &c));
        for (k, v) in &c.by_status {
            *all.by_status.entry(k.clone()).or_default() += v;
        }
        all.total += c.total;
    }
    s.push_str(&line("**ALL**", &all));
    s.push_str("\n## Coverage per topic (all registries)\n\n");
    s.push_str(head);
    let topics = per_topic(tables);
    for (tp, c) in &topics {
        s.push_str(&line(tp, c));
    }
    let funcs = tables.iter().find(|t| t.name == "functions");
    s.push_str("\n## Per topic\n");
    for tp in topics.keys() {
        let _ = writeln!(s, "\n### {tp}\n");
        for t in tables {
            let c = count_by(t, Some(tp));
            if c.total == 0 {
                continue;
            }
            let _ = writeln!(
                s,
                "- **{}**: {} rows, closed {}, decoded {}, unknown {}",
                t.name,
                c.total,
                pct(c.closed(), c.total),
                pct(c.decoded(), c.total),
                c.n("unknown")
            );
        }
        let mut tab = String::new();
        for t in tables {
            for r in &t.rows {
                if r[TOPIC] != *tp {
                    continue;
                }
                let show = match r[STATUS].as_str() {
                    "ported-verified" | "deviation" | "decoded-not-ported" => true,
                    "ported-unverified" | "partly-decoded" | "replaced-by-data" => t.name != "functions",
                    _ => false,
                };
                if show {
                    let _ = writeln!(tab, "| {} | {} | {} | {} | {} | {} |", t.name, r[ID], r[NAME], r[STATUS], md(&r[PORT]), md(&r[VERIFY]));
                }
            }
        }
        if !tab.is_empty() {
            s.push_str("\n| registry | id | name | status | port | verification |\n|---|---|---|---|---|---|\n");
            s.push_str(&tab);
        }
        if let Some(f) = funcs {
            let mut list = |status: &str| {
                let v: Vec<&str> = f.rows.iter().filter(|r| r[TOPIC] == *tp && r[STATUS] == status).map(|r| r[ID].as_str()).collect();
                if !v.is_empty() {
                    let _ = writeln!(s, "\n{} functions ({}): {}", status, v.len(), v.join(" "));
                }
            };
            list("ported-unverified");
            list("replaced-by-data");
            list("partly-decoded");
        }
    }
    s
}

fn md(s: &str) -> String {
    s.replace('|', "/")
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, status: &str) -> Vec<String> {
        let mut r = vec![String::new(); COLS.len()];
        r[ID] = id.into();
        r[TOPIC] = "AI".into();
        r[STATUS] = status.into();
        r
    }
    fn table(name: &str, rows: Vec<Vec<String>>) -> Table {
        Table { name: name.into(), path: PathBuf::from("x.tsv"), rows }
    }
    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mcla_reg_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("docs/registry")).unwrap();
        d
    }
    fn topics() -> HashSet<String> {
        ["AI", "UNASSIGNED"].iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn tsv_round_trip() {
        let mut t = table("functions", vec![row("8225ABCD", "unknown")]);
        t.rows[0][NOTE] = "a note".into();
        t.rows[0][ATTRS] = "size=12;callers=3".into();
        let text = t.to_text();
        let p = Table::parse("functions", Path::new("x"), &text).unwrap();
        assert_eq!(p.rows, t.rows);
        assert_eq!(attr(&p.rows[0][ATTRS], "size"), Some("12"));
        assert_eq!(attr(&p.rows[0][ATTRS], "siz"), None);
        assert!(Table::parse("functions", Path::new("x"), "id\tname\n").is_err());
    }

    #[test]
    fn dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_386), (2025, 10, 25));
        assert_eq!(civil(11_016), (2000, 2, 29));
    }

    #[test]
    fn address_citations() {
        let t = "calls sub_8225ABCD and 0x82558678, bare 827C0000 (hash 0x8053a397) not 18225ABCD or 8225ABCDE\nline2 sub_827CD054 82130000.";
        let c = cited_addresses(t);
        assert_eq!(c, vec![(0x8225ABCD, 1, true), (0x82558678, 1, false), (0x827C0000, 1, false), (0x82130000, 2, false)]);
    }

    #[test]
    fn function_resolve() {
        let mut a = row("82130000", "unknown");
        a[ATTRS] = "size=4".into();
        let mut b = row("82130010", "unknown");
        b[ATTRS] = "size=2".into();
        let t = table("functions", vec![a, b]);
        let fi = FuncIndex::new(&t);
        assert_eq!(fi.resolve(0x82130000), Some(0));
        assert_eq!(fi.resolve(0x8213000C), Some(0));
        assert_eq!(fi.resolve(0x82130010), Some(1));
        assert_eq!(fi.resolve(0x82130014), Some(1));
        assert_eq!(fi.resolve(0x82130018), None);
        assert_eq!(fi.resolve(0x8212FFFC), None);
        assert_eq!(norm_id("functions", "sub_8225abcd"), "8225ABCD");
        assert_eq!(norm_id("natives", "0x1c2391d3"), "0x1c2391d3");
    }

    #[test]
    fn claim_and_set() {
        let mut r = row("8225ABCD", "unknown");
        claim_row(&mut r, "ai2", "2026-10-25", false).unwrap();
        assert!(claim_row(&mut r, "cams", "2026-10-25", false).is_err());
        claim_row(&mut r, "ai2", "2026-10-26", false).unwrap();
        claim_row(&mut r, "cams", "2026-10-26", true).unwrap();
        assert_eq!(r[CLAIMED_BY], "cams");
        let mut f = HashMap::new();
        assert!(apply_set(&mut r, "ported-verified", &f).is_err());
        assert!(apply_set(&mut r, "bogus", &f).is_err());
        assert!(apply_set(&mut r, "deviation", &f).is_err());
        assert!(apply_set(&mut r, "not-needed", &f).is_err());
        f.insert("verify".into(), "bit-exact@docs/x.md".into());
        f.insert("note".into(), "a\tb".into());
        apply_set(&mut r, "ported-verified", &f).unwrap();
        assert_eq!(r[CLAIMED_BY], "", "a closed row frees its claim");
        assert_eq!(r[NOTE], "a b");
        assert_eq!(r[STATUS], "ported-verified");
        r[ATTRS] = "size=4;seed=1".into();
        apply_set(&mut r, "ported-verified", &f).unwrap();
        assert_eq!(r[ATTRS], "size=4", "a hand change drops the seed marker");
    }

    #[test]
    fn rule_application() {
        let mut r = row("8225ABCD", "unknown");
        r[ATTRS] = "size=4;topic_src=T1;seed=1".into();
        assert!(apply_rule(&mut r, "R4", "library code", "ENGINE").unwrap());
        assert_eq!(r[STATUS], "not-needed");
        assert_eq!(r[NOTE], "R4: library code");
        assert_eq!(r[TOPIC], "ENGINE");
        assert_eq!(r[ATTRS], "size=4;rule=R4");
        // a second run changes nothing
        assert!(!apply_rule(&mut r, "R5", "x", "ENGINE").unwrap());
        assert_eq!(r[NOTE], "R4: library code");
        // evidence-based statuses and claims are never overwritten
        for s in ["ported-verified", "ported-unverified", "partly-decoded", "decoded-not-ported", "deviation"] {
            let mut r = row("8225ABCE", s);
            assert!(!apply_rule(&mut r, "R4", "x", "ENGINE").unwrap());
            assert_eq!(r[STATUS], s);
        }
        let mut c = row("8225ABCF", "unknown");
        claim_row(&mut c, "ai2", "2026-10-25", false).unwrap();
        assert!(!apply_rule(&mut c, "R4", "x", "ENGINE").unwrap());
        assert_eq!(c[STATUS], "unknown");
    }

    fn flags(v: &[(&str, &str)]) -> HashMap<String, String> {
        v.iter().map(|(k, x)| (k.to_string(), x.to_string())).collect()
    }

    #[test]
    fn replaced_by_data_transitions() {
        let ok = flags(&[("rbd-class", "resource-fixup"), ("reads", "swf.rs parser"), ("missing", "relocated dump vs parse"), ("doc", "docs/X.md")]);
        // from unknown / decoded-not-ported / partly-decoded
        for from in ["unknown", "decoded-not-ported", "partly-decoded"] {
            let mut r = row("8225ABCD", from);
            apply_set(&mut r, "replaced-by-data", &ok).unwrap();
            assert_eq!(r[STATUS], "replaced-by-data");
            assert_eq!(r[NOTE], "RBD resource-fixup | reads: swf.rs parser | missing proof: relocated dump vs parse");
            assert_eq!(attr(&r[ATTRS], "rbd"), Some("resource-fixup"));
        }
        // never from a ported / closed status
        for from in ["ported-verified", "ported-unverified", "not-needed", "deviation"] {
            let mut r = row("8225ABCD", from);
            assert!(apply_set(&mut r, "replaced-by-data", &ok).is_err(), "{from}");
        }
        // class, reads, missing, doc are required
        let mut r = row("8225ABCD", "unknown");
        assert!(apply_set(&mut r, "replaced-by-data", &flags(&[("reads", "a"), ("missing", "b"), ("doc", "d")])).is_err());
        let mut r = row("8225ABCD", "unknown");
        assert!(apply_set(&mut r, "replaced-by-data", &flags(&[("rbd-class", "bogus"), ("reads", "a"), ("missing", "b"), ("doc", "d")])).is_err());
        let mut r = row("8225ABCD", "unknown");
        assert!(apply_set(&mut r, "replaced-by-data", &flags(&[("rbd-class", "data-ctor"), ("reads", "a"), ("doc", "d")])).is_err());
        let mut r = row("8225ABCD", "unknown");
        assert!(apply_set(&mut r, "replaced-by-data", &flags(&[("rbd-class", "data-ctor"), ("reads", "a"), ("missing", "b")])).is_err());
        // a re-set keeps class and note
        let mut r = row("8225ABCD", "unknown");
        apply_set(&mut r, "replaced-by-data", &ok).unwrap();
        apply_set(&mut r, "replaced-by-data", &flags(&[("missing", "new proof")])).unwrap();
        assert_eq!(r[NOTE], "RBD resource-fixup | reads: swf.rs parser | missing proof: new proof");
        // closing as deviation D34 needs a real comparison
        let mut c = r.clone();
        let dev = flags(&[("deviation", "D34"), ("verify", "read-only@docs/X.md"), ("note", "D34 E3: x")]);
        assert!(apply_set(&mut c, "deviation", &dev).is_err());
        let mut c = r.clone();
        let dev = flags(&[("deviation", "D34"), ("verify", "bit-exact@docs/X.md"), ("note", "D34 E1: dump equals parse")]);
        apply_set(&mut c, "deviation", &dev).unwrap();
        assert_eq!(attr(&c[ATTRS], "rbd"), None);
        assert_eq!(attr(&c[ATTRS], "rbd_from"), Some("resource-fixup"));
        // a plain D34 row (E3 / E4 evidence) is read-only evidence and needs no comparison
        let mut p = row("8225ABCE", "decoded-not-ported");
        apply_set(&mut p, "deviation", &flags(&[("deviation", "D34"), ("verify", "read-only@docs/X.md"), ("note", "D34 E4: release only")])).unwrap();
        assert!(apply_set(&mut row("8225ABCF", "decoded-not-ported"), "deviation", &flags(&[("deviation", "D34")])).is_err());
        // leaving back to decoded-not-ported drops the class
        let mut b = r.clone();
        apply_set(&mut b, "decoded-not-ported", &flags(&[("doc", "docs/X.md")])).unwrap();
        assert_eq!(attr(&b[ATTRS], "rbd"), None);
    }

    #[test]
    fn replaced_by_data_check_and_counts() {
        let root = tmp("rbd");
        std::fs::write(root.join("docs/DECISIONS.md"), "| D34 | x |\n").unwrap();
        std::fs::write(root.join("docs/evidence.md"), "e").unwrap();
        let devs = deviations_known(&root);
        let mut ok = row("00000001", "replaced-by-data");
        ok[NOTE] = "RBD data-ctor | reads: a | missing proof: b".into();
        ok[DOC] = "docs/evidence.md".into();
        ok[ATTRS] = "rbd=data-ctor".into();
        let mut no_class = ok.clone();
        no_class[ID] = "00000002".into();
        no_class[ATTRS].clear();
        let mut no_note = ok.clone();
        no_note[ID] = "00000003".into();
        no_note[NOTE] = "RBD data-ctor".into();
        let mut stray = row("00000004", "decoded-not-ported");
        stray[DOC] = "docs/evidence.md".into();
        stray[ATTRS] = "rbd=data-ctor".into();
        let mut d34_ok = row("00000005", "deviation");
        d34_ok[DEVIATION] = "D34".into();
        d34_ok[VERIFY] = "read-only@docs/evidence.md".into();
        d34_ok[NOTE] = "D34 E3: nothing on the disc".into();
        let mut d34_noev = d34_ok.clone();
        d34_noev[ID] = "00000006".into();
        d34_noev[VERIFY].clear();
        let mut d34_nonote = d34_ok.clone();
        d34_nonote[ID] = "00000007".into();
        d34_nonote[NOTE] = "closed".into();
        let mut d34_weak = d34_ok.clone();
        d34_weak[ID] = "00000008".into();
        d34_weak[ATTRS] = "rbd_from=data-ctor".into();
        let t = table("functions", vec![ok, no_class, no_note, stray, d34_ok, d34_noev, d34_nonote, d34_weak]);
        let p = check_rows(&root, &t, &topics(), &devs);
        let has = |id: &str, frag: &str| p.iter().any(|x| x.id == id && x.msg.contains(frag));
        assert!(!p.iter().any(|x| x.id == "00000001" || x.id == "00000005"), "{:?}", p.iter().map(|x| (&x.id, &x.msg)).collect::<Vec<_>>());
        assert!(has("00000002", "rbd=<class>"));
        assert!(has("00000003", "reads:"));
        assert!(has("00000004", "not replaced-by-data"));
        assert!(has("00000006", "D34 needs verification"));
        assert!(has("00000007", "D34 E"));
        assert!(has("00000008", "bit-exact@ or numbers@"));
        // counted as known, not as closed
        let t = table("functions", vec![row("1", "unknown"), row("2", "replaced-by-data"), row("3", "deviation"), row("4", "decoded-not-ported")]);
        let c = count_by(&t, None);
        assert_eq!((c.total, c.closed(), c.known(), c.n("replaced-by-data")), (4, 1, 3, 1));
        let r = render_report(&[t]);
        assert!(r.contains("replaced-by-data"));
        assert!(status_header().contains("repl-dat") && status_line("x", &c).contains("   25.0%"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn check_rules() {
        let root = tmp("rules");
        std::fs::write(root.join("docs/DECISIONS.md"), "| D7 | x |\n| D26 | y |\n").unwrap();
        std::fs::write(root.join("docs/evidence.md"), "e").unwrap();
        std::fs::create_dir_all(root.join("mcla-rs/crates/game/src")).unwrap();
        std::fs::write(root.join("mcla-rs/crates/game/src/ai.rs"), "pub fn brain_step() {}\nfn brain_step_two() {}\n").unwrap();
        let devs = deviations_known(&root);
        assert!(devs.contains("D7") && devs.contains("D26") && !devs.contains("D8"));

        let mut ok_v = row("00000001", "ported-verified");
        ok_v[VERIFY] = "bit-exact@docs/evidence.md".into();
        ok_v[PORT] = "crates/game/src/ai.rs::brain_step".into();
        let mut no_ev = row("00000002", "ported-verified");
        no_ev[PORT] = "crates/game/src/ai.rs".into();
        let mut bad_ev = row("00000003", "ported-verified");
        bad_ev[VERIFY] = "numbers@docs/missing.md".into();
        bad_ev[PORT] = "crates/game/src/ai.rs".into();
        let mut weak = row("00000004", "ported-verified");
        weak[VERIFY] = "read-only@docs/evidence.md".into();
        weak[PORT] = "crates/game/src/ai.rs".into();
        let mut dev_ok = row("00000005", "deviation");
        dev_ok[DEVIATION] = "D7".into();
        let mut dev_bad = row("00000006", "deviation");
        dev_bad[DEVIATION] = "D99".into();
        let mut dev_none = row("00000007", "deviation");
        dev_none[DEVIATION].clear();
        let mut port_file = row("00000008", "ported-unverified");
        port_file[PORT] = "crates/game/src/nothere.rs".into();
        let mut port_fn = row("00000009", "ported-unverified");
        port_fn[PORT] = "crates/game/src/ai.rs::brain".into(); // only a prefix of brain_step
        let mut port_fn_ok = row("0000000A", "ported-unverified");
        port_fn_ok[PORT] = "crates/game/src/ai.rs::brain_step_two".into();
        let mut nn = row("0000000B", "not-needed");
        nn[NOTE].clear();
        let mut topic = row("0000000C", "unknown");
        topic[TOPIC] = "NOPE".into();
        let dup = row("0000000C", "unknown");
        let mut unp = row("0000000D", "ported-unverified");
        unp[PORT].clear();
        let t = table("functions", vec![ok_v, no_ev, bad_ev, weak, dev_ok, dev_bad, dev_none, port_file, port_fn, port_fn_ok, nn, topic, dup, unp]);
        let p = check_rows(&root, &t, &topics(), &devs);
        let has = |id: &str, frag: &str| p.iter().any(|x| x.id == id && x.msg.contains(frag));
        assert!(!p.iter().any(|x| x.id == "00000001"), "{:?}", p.iter().map(|x| &x.msg).collect::<Vec<_>>());
        assert!(has("00000002", "needs verification"));
        assert!(has("00000003", "does not exist"));
        assert!(has("00000004", "needs verification"));
        assert!(!p.iter().any(|x| x.id == "00000005"));
        assert!(has("00000006", "DECISIONS.md"));
        assert!(has("00000007", "DECISIONS.md"));
        assert!(has("00000008", "does not exist"));
        assert!(has("00000009", "does not exist"));
        assert!(!p.iter().any(|x| x.id == "0000000A"));
        assert!(has("0000000B", "reason"));
        assert!(has("0000000C", "topic"));
        assert!(has("0000000C", "duplicate"));
        assert!(has("0000000D", "without a port"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn check_citations_rules() {
        let root = tmp("cites");
        std::fs::create_dir_all(root.join("docs/registry")).unwrap();
        std::fs::write(root.join("docs/AI.md"), "see sub_82130004 (inside the first function), sub_82130100, 0x82130110 (data word, ignored), 0x82130040 and hash 0x8053a397").unwrap();
        std::fs::write(root.join("docs/registry/skip.md"), "sub_82130200").unwrap();
        let mut a = row("82130000", "partly-decoded");
        a[ATTRS] = "size=8".into();
        let mut b = row("82130040", "unknown");
        b[ATTRS] = "size=4".into();
        let t = table("functions", vec![a, b]);
        let p = check_citations(&root, &t);
        let ids: Vec<&str> = p.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, vec!["82130100", "82130040"], "{:?}", p.iter().map(|x| &x.msg).collect::<Vec<_>>());
        assert!(p[0].msg.contains("no row"));
        assert!(p[1].msg.contains("unknown"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn coverage_counts() {
        let mut t = table("functions", vec![row("1", "unknown"), row("2", "ported-verified"), row("3", "not-needed"), row("4", "partly-decoded")]);
        t.rows[3][TOPIC] = "MAP".into();
        let c = count_by(&t, None);
        assert_eq!((c.total, c.closed(), c.known(), c.n("unknown")), (4, 2, 3, 1));
        assert_eq!(count_by(&t, Some("map")).total, 1);
        assert_eq!(pct(1, 4), "25.0%");
        let r = render_report(&[t]);
        assert!(r.contains("GENERATED") && r.contains("### MAP") && r.contains("**unknown**"));
    }
}
