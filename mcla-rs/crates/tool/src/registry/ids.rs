//! Opaque IDs for the decode registry (unpublished data in `docs/registry/ids.tsv` + `ids.meta`).
//!
//! Source code that is published must not carry addresses of the original executable. Every function row gets an ID `F-#####`;
//! an address that is not a function start (inside a function, a vtable, a global) gets `A-#####` on demand. The numbers are a
//! seeded random permutation of 10000..=99999 (the seed lives only in `ids.meta`), assigned in order of allocation and never
//! reused. Commands: `id`, `addr`, `ids-sync`, `cite-scan`, `cite-convert`; `check` resolves the IDs and rejects new raw
//! address citations in tracked files (baseline `cite_baseline.tsv`).

use super::*;

pub const IDS_FILE: &str = "ids.tsv";
pub const META_FILE: &str = "ids.meta";
pub const BASELINE_FILE: &str = "cite_baseline.tsv";
const POOL_LO: u32 = 10_000;
const POOL_HI: u32 = 99_999;

#[derive(Clone, Debug, PartialEq)]
pub struct IdRow {
    pub fid: String,
    /// 'F' (function row of functions.tsv, key = its id) or 'A' (any other address, key = 8 upper-case hex digits)
    pub kind: char,
    pub key: String,
}

#[derive(Default)]
pub struct Ids {
    pub rows: Vec<IdRow>,
    by_fid: HashMap<String, usize>,
    by_key: HashMap<(char, String), usize>,
    pub seed: u64,
    pool: Vec<u32>,
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The seeded permutation of the ID numbers (Fisher-Yates).
pub fn make_pool(seed: u64) -> Vec<u32> {
    let mut v: Vec<u32> = (POOL_LO..=POOL_HI).collect();
    let mut st = seed;
    for i in (1..v.len()).rev() {
        let j = (splitmix(&mut st) % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
    v
}

pub fn fresh_seed() -> u64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    let mut st = t ^ ((std::process::id() as u64) << 32);
    splitmix(&mut st)
}

/// `F-12345` / `A-12345` -> (kind, number); exactly five digits.
pub fn parse_fid(s: &str) -> Option<(char, u32)> {
    let b = s.as_bytes();
    if b.len() == 7 && (b[0] == b'F' || b[0] == b'A') && b[1] == b'-' && b[2..].iter().all(|c| c.is_ascii_digit()) {
        return Some((b[0] as char, s[2..].parse().ok()?));
    }
    None
}

/// ID citations in text: (byte start, byte end, line, id). Needs a non-alphanumeric byte (and no `-`) before and no digit/letter after.
pub fn id_spans(text: &str) -> Vec<(usize, usize, usize, String)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut line = 1usize;
    for i in 0..b.len() {
        if b[i] == b'\n' {
            line += 1;
            continue;
        }
        if (b[i] == b'F' || b[i] == b'A') && i + 7 <= b.len() && b[i + 1] == b'-' && b[i + 2..i + 7].iter().all(|c| c.is_ascii_digit()) {
            let before_ok = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
            let after_ok = i + 7 == b.len() || !(b[i + 7].is_ascii_alphanumeric() || b[i + 7] == b'_');
            if before_ok && after_ok {
                out.push((i, i + 7, line, text[i..i + 7].to_string()));
            }
        }
    }
    out
}

impl Ids {
    pub fn new(seed: u64) -> Ids {
        Ids { seed, pool: make_pool(seed), ..Default::default() }
    }
    pub fn load(root: &Path) -> Result<Ids> {
        let dir = root.join("docs/registry");
        let p = dir.join(IDS_FILE);
        if !p.exists() {
            return Ok(Ids::default());
        }
        let meta = std::fs::read_to_string(dir.join(META_FILE)).with_context(|| format!("read {} (the seed of the ID permutation)", META_FILE))?;
        let seed: u64 = meta.lines().find_map(|l| l.strip_prefix("seed=")).and_then(|v| v.trim().parse().ok()).context("ids.meta has no seed=")?;
        let mut ids = Ids::new(seed);
        let text = std::fs::read_to_string(&p)?;
        for (n, l) in text.lines().enumerate().skip(1) {
            if l.is_empty() {
                continue;
            }
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() != 3 || parse_fid(f[0]).is_none() || !(f[1] == "func" || f[1] == "addr") {
                bail!("{}:{}: malformed ID row", p.display(), n + 1);
            }
            ids.push(IdRow { fid: f[0].to_string(), kind: if f[1] == "func" { 'F' } else { 'A' }, key: f[2].to_string() });
        }
        Ok(ids)
    }
    fn push(&mut self, r: IdRow) {
        let i = self.rows.len();
        self.by_fid.insert(r.fid.clone(), i);
        self.by_key.insert((r.kind, r.key.clone()), i);
        self.rows.push(r);
    }
    pub fn save(&self, root: &Path) -> Result<()> {
        let dir = root.join("docs/registry");
        let mut s = String::from("fid\tkind\tkey\n");
        for r in &self.rows {
            let _ = writeln!(s, "{}\t{}\t{}", r.fid, if r.kind == 'F' { "func" } else { "addr" }, r.key);
        }
        write_atomic(&dir.join(IDS_FILE), s.as_bytes())?;
        write_atomic(&dir.join(META_FILE), format!("seed={}\n", self.seed).as_bytes())
    }
    pub fn get(&self, kind: char, key: &str) -> Option<&str> {
        self.by_key.get(&(kind, key.to_string())).map(|&i| self.rows[i].fid.as_str())
    }
    pub fn lookup(&self, fid: &str) -> Option<&IdRow> {
        self.by_fid.get(&fid.to_ascii_uppercase()).map(|&i| &self.rows[i])
    }
    /// The next free ID for `key`; the number is the next entry of the permutation, never reused.
    pub fn alloc(&mut self, kind: char, key: &str) -> Result<String> {
        if let Some(f) = self.get(kind, key) {
            return Ok(f.to_string());
        }
        let n = *self.pool.get(self.rows.len()).context("the ID pool (10000..99999) is exhausted")?;
        let fid = format!("{kind}-{n:05}");
        self.push(IdRow { fid: fid.clone(), kind, key: key.to_string() });
        Ok(fid)
    }
    /// Give every row of functions.tsv an ID; returns how many were added (rows are visited in file order).
    pub fn sync_functions(&mut self, funcs: &Table) -> Result<usize> {
        let mut n = 0;
        for r in &funcs.rows {
            if self.get('F', &r[ID]).is_none() {
                self.alloc('F', &r[ID])?;
                n += 1;
            }
        }
        Ok(n)
    }
    /// Integrity: each row's number is the permutation entry of its position, no duplicates, every function row has an ID,
    /// every F entry names an existing row.
    pub fn problems(&self, funcs: Option<&Table>) -> Vec<String> {
        let mut out = Vec::new();
        for (i, r) in self.rows.iter().enumerate() {
            let want = self.pool.get(i).map(|n| format!("{}-{n:05}", r.kind));
            if want.as_deref() != Some(r.fid.as_str()) {
                out.push(format!("ids.tsv row {} ({}) does not follow the seeded permutation (hand edit?)", i + 2, r.fid));
            }
        }
        if self.by_fid.len() != self.rows.len() {
            out.push("ids.tsv has duplicate IDs".into());
        }
        if let Some(f) = funcs {
            let missing = f.rows.iter().filter(|r| self.get('F', &r[ID]).is_none()).count();
            if missing > 0 {
                out.push(format!("{missing} function row(s) have no ID: run `mcla-tool registry ids-sync`"));
            }
            let have: HashSet<&str> = f.rows.iter().map(|r| r[ID].as_str()).collect();
            for r in self.rows.iter().filter(|r| r.kind == 'F') {
                if !have.contains(r.key.as_str()) {
                    out.push(format!("{} names a function row that no longer exists", r.fid));
                }
            }
        }
        out
    }
    /// An ID citation -> the address text to look up (8 hex digits, or the function row id for names like `xstart`).
    pub fn key_of(&self, fid: &str) -> Option<String> {
        self.lookup(fid).map(|r| r.key.clone())
    }
}

/// Address of an ID's key as u32 if it is hex.
fn key_addr(key: &str) -> Option<u32> {
    (key.len() == 8 && key.bytes().all(|b| b.is_ascii_hexdigit())).then(|| u32::from_str_radix(key, 16).unwrap())
}

// ---------------------------------------------------------------- address spans, comments, conversion

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    /// byte range including a `sub_` / `0x` prefix
    pub start: usize,
    pub end: usize,
    pub addr: u32,
    pub line: usize,
    pub strong: bool,
}

/// Every address citation of the code range with its byte span (same rules as `cited_addresses`).
pub fn address_spans(text: &str) -> Vec<Span> {
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
            let hex_prefix = i >= 2 && (b[i - 1] == b'x' || b[i - 1] == b'X') && b[i - 2] == b'0' && (i == 2 || !b[i - 3].is_ascii_alphanumeric());
            let before_ok = i == 0 || !b[i - 1].is_ascii_alphanumeric() || hex_prefix;
            let tok = &b[i..i + 8];
            let after_ok = i + 8 == b.len() || !b[i + 8].is_ascii_alphanumeric();
            if before_ok && after_ok && tok.iter().all(|x| x.is_ascii_hexdigit()) {
                let a = u32::from_str_radix(std::str::from_utf8(tok).unwrap(), 16).unwrap();
                if (CODE_START..CODE_END).contains(&a) {
                    let strong = i >= 4 && &b[i - 4..i] == b"sub_";
                    let start = if strong {
                        i - 4
                    } else if hex_prefix {
                        i - 2
                    } else {
                        i
                    };
                    out.push(Span { start, end: i + 8, addr: a, line, strong });
                }
                i += 8;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Per byte: is it inside a comment (or a docstring / prose) for a file of this extension? Prose files are all comment.
pub fn comment_mask(ext: &str, text: &str) -> Vec<bool> {
    let b = text.as_bytes();
    let n = b.len();
    let mut m = vec![false; n];
    match ext {
        "md" | "txt" | "html" => m.fill(true),
        "rs" | "wgsl" | "js" => {
            #[derive(PartialEq)]
            enum S {
                Normal,
                Line,
                Block,
                Str,
            }
            let mut s = S::Normal;
            let mut i = 0;
            while i < n {
                match s {
                    S::Normal => {
                        if b[i] == b'/' && i + 1 < n && b[i + 1] == b'/' {
                            s = S::Line;
                            continue;
                        } else if b[i] == b'/' && i + 1 < n && b[i + 1] == b'*' {
                            s = S::Block;
                            m[i] = true;
                            m[i + 1] = true;
                            i += 2;
                            continue;
                        } else if b[i] == b'"' {
                            s = S::Str;
                        } else if b[i] == b'\'' {
                            if i + 2 < n && b[i + 2] == b'\'' {
                                i += 3;
                                continue;
                            }
                            if i + 3 < n && b[i + 1] == b'\\' && b[i + 3] == b'\'' {
                                i += 4;
                                continue;
                            }
                        }
                        i += 1;
                    }
                    S::Line => {
                        if b[i] == b'\n' {
                            s = S::Normal;
                        } else {
                            m[i] = true;
                        }
                        i += 1;
                    }
                    S::Block => {
                        m[i] = true;
                        if b[i] == b'*' && i + 1 < n && b[i + 1] == b'/' {
                            m[i + 1] = true;
                            s = S::Normal;
                            i += 2;
                            continue;
                        }
                        i += 1;
                    }
                    S::Str => {
                        if b[i] == b'\\' {
                            i += 2;
                            continue;
                        }
                        if b[i] == b'"' {
                            s = S::Normal;
                        }
                        i += 1;
                    }
                }
            }
        }
        "py" | "toml" | "yml" | "yaml" | "ps1" | "cmd" | "bat" => {
            let py = ext == "py";
            let mut i = 0;
            let mut quote: Option<u8> = None;
            let mut triple = false;
            let mut line_comment = false;
            while i < n {
                if line_comment {
                    if b[i] == b'\n' {
                        line_comment = false;
                    } else {
                        m[i] = true;
                    }
                    i += 1;
                    continue;
                }
                if triple {
                    m[i] = true;
                    if i + 2 < n && b[i] == quote.unwrap() && b[i + 1] == quote.unwrap() && b[i + 2] == quote.unwrap() {
                        m[i + 1] = true;
                        m[i + 2] = true;
                        triple = false;
                        quote = None;
                        i += 3;
                        continue;
                    }
                    i += 1;
                    continue;
                }
                match quote {
                    Some(q) => {
                        if b[i] == b'\\' {
                            i += 2;
                            continue;
                        }
                        if b[i] == q || b[i] == b'\n' {
                            quote = None;
                        }
                        i += 1;
                    }
                    None => {
                        if b[i] == b'#' {
                            line_comment = true;
                            continue;
                        }
                        if b[i] == b'"' || b[i] == b'\'' {
                            let q = b[i];
                            if py && i + 2 < n && b[i + 1] == q && b[i + 2] == q {
                                triple = true;
                                quote = Some(q);
                                m[i] = true;
                                m[i + 1] = true;
                                m[i + 2] = true;
                                i += 3;
                                continue;
                            }
                            quote = Some(q);
                        }
                        i += 1;
                    }
                }
            }
        }
        _ => {}
    }
    m
}

pub struct Conversion {
    pub text: String,
    pub converted: usize,
    /// (line, reason) of citations that were left
    pub left: Vec<(usize, &'static str)>,
}

/// Rewrite the address citations of `text` to IDs. `resolve(addr)` returns the ID text for a registered address (None: not
/// registered, hex left alone). Only comments are touched; a citation glued to an identifier is left.
/// Two converted citations joined by `-` (a range) are written `ID..ID`.
pub fn convert_text(ext: &str, text: &str, registered: &dyn Fn(u32) -> bool, resolve: &mut dyn FnMut(u32) -> Result<Option<String>>) -> Result<Conversion> {
    let mask = comment_mask(ext, text);
    let b = text.as_bytes();
    let mut reps: Vec<(usize, usize, String)> = Vec::new();
    let mut left = Vec::new();
    for s in address_spans(text) {
        let glued = (s.start > 0 && b[s.start - 1] == b'_') || (s.end < b.len() && b[s.end] == b'_');
        if !mask[s.start] {
            if registered(s.addr) {
                left.push((s.line, "not in a comment (string or code): edit by hand"));
            }
            continue;
        }
        if glued {
            left.push((s.line, "glued to an identifier"));
            continue;
        }
        if let Some(id) = resolve(s.addr)? {
            reps.push((s.start, s.end, id));
        }
    }
    let converted = reps.len();
    // ranges: "ID-ID" -> "ID..ID"
    let mut seps: Vec<(usize, usize)> = Vec::new();
    for w in reps.windows(2) {
        if w[1].0 == w[0].1 + 1 && b[w[0].1] == b'-' {
            seps.push((w[0].1, w[1].0));
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    let mut all: Vec<(usize, usize, String)> = reps;
    for (a, e) in seps {
        all.push((a, e, "..".to_string()));
    }
    all.sort_by_key(|r| r.0);
    for (s, e, r) in all {
        out.push_str(&text[pos..s]);
        out.push_str(&r);
        pos = e;
    }
    out.push_str(&text[pos..]);
    Ok(Conversion { text: out, converted, left })
}

// ---------------------------------------------------------------- scanning tracked files, baseline

pub fn fnv64(s: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in s.trim().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Files of the repository that are or will be tracked: `git ls-files` plus untracked files that are not ignored.
/// None when `root` is not a git checkout. The registry tool's own source and tests (made-up addresses) are left out.
pub fn tracked_files(root: &Path) -> Option<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = Vec::new();
    for args in [&["ls-files", "-z"][..], &["ls-files", "-z", "--others", "--exclude-standard"][..]] {
        let out = std::process::Command::new("git").arg("-C").arg(root).args(args).output().ok()?;
        if !out.status.success() {
            return None;
        }
        for name in out.stdout.split(|&c| c == 0).filter(|s| !s.is_empty()) {
            let rel = String::from_utf8_lossy(name).to_string();
            if rel.contains("crates/tool/src/registry") || rel.starts_with("mcla-rs/crates/tool/tests/") || rel.ends_with("Cargo.lock") {
                continue;
            }
            let p = root.join(&rel);
            if p.is_file() {
                v.push(p);
            }
        }
    }
    v.sort();
    v.dedup();
    Some(v)
}

pub fn read_text(p: &Path) -> Option<String> {
    let md = std::fs::metadata(p).ok()?;
    if md.len() > 4_000_000 {
        return None;
    }
    String::from_utf8(std::fs::read(p).ok()?).ok()
}

pub fn rel_of(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cite {
    pub file: String,
    pub line: usize,
    pub hash: u64,
    /// number of address citations on the line
    pub n: usize,
}

/// Lines of `text` that cite an address which resolves to a function of the registry.
pub fn scan_text(file: &str, text: &str, fi: &FuncIndex) -> Vec<Cite> {
    let mut by_line: BTreeMap<usize, usize> = BTreeMap::new();
    for s in address_spans(text) {
        if fi.resolve(s.addr).is_some() {
            *by_line.entry(s.line).or_default() += 1;
        }
    }
    if by_line.is_empty() {
        return Vec::new();
    }
    let lines: Vec<&str> = text.lines().collect();
    by_line.into_iter().map(|(l, n)| Cite { file: file.to_string(), line: l, hash: fnv64(lines.get(l - 1).copied().unwrap_or("")), n }).collect()
}

pub fn scan_files(root: &Path, files: &[PathBuf], fi: &FuncIndex) -> Vec<Cite> {
    let mut out = Vec::new();
    for f in files {
        if let Some(t) = read_text(f) {
            out.extend(scan_text(&rel_of(root, f), &t, fi));
        }
    }
    out
}

pub fn load_baseline(root: &Path) -> Vec<Cite> {
    let text = std::fs::read_to_string(root.join("docs/registry").join(BASELINE_FILE)).unwrap_or_default();
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() != 4 {
                return None;
            }
            Some(Cite { file: f[0].into(), line: f[1].parse().ok()?, hash: u64::from_str_radix(f[2], 16).ok()?, n: f[3].parse().ok()? })
        })
        .collect()
}

pub fn baseline_text(c: &[Cite]) -> String {
    let mut s = String::from("file\tline\thash\tcitations\n");
    for x in c {
        let _ = writeln!(s, "{}\t{}\t{:016x}\t{}", x.file, x.line, x.hash, x.n);
    }
    s
}

/// Citations that are not covered by the baseline (matched by file + line hash, as a multiset: moving a line is fine,
/// editing it or adding a copy is new).
pub fn new_citations(current: &[Cite], baseline: &[Cite]) -> Vec<Cite> {
    let mut allowed: HashMap<(&str, u64), usize> = HashMap::new();
    for b in baseline {
        *allowed.entry((b.file.as_str(), b.hash)).or_default() += 1;
    }
    let mut out = Vec::new();
    for c in current {
        match allowed.get_mut(&(c.file.as_str(), c.hash)) {
            Some(k) if *k > 0 => *k -= 1,
            _ => out.push(c.clone()),
        }
    }
    out
}

/// Problems for `check`: ID citations (source, docs, tracked files) that do not resolve or point at an unknown row,
/// and new raw-address citations in tracked files.
pub fn check_ids(root: &Path, funcs: &Table, ids: &Ids) -> Vec<Problem> {
    let mut out = Vec::new();
    let fi = FuncIndex::new(funcs);
    let mut files = cite_files(root);
    let tracked = tracked_files(root);
    if let Some(t) = &tracked {
        files.extend(t.iter().cloned());
    }
    files.sort();
    files.dedup();
    let mut seen: HashSet<String> = HashSet::new();
    for f in &files {
        let rel = rel_of(root, f);
        if rel.contains("crates/tool/src/registry") || rel.starts_with("mcla-rs/crates/tool/tests/") {
            continue;
        }
        let Some(text) = read_text(f) else { continue };
        for (_, _, line, id) in id_spans(&text) {
            let Some(r) = ids.lookup(&id) else {
                if seen.insert(id.clone()) {
                    out.push(Problem { registry: "ids".into(), id: id.clone(), msg: format!("cited at {rel}:{line} but is not an assigned ID (typo? see `registry addr`)") });
                }
                continue;
            };
            let row = if r.kind == 'F' { funcs.find_id(&r.key) } else { key_addr(&r.key).and_then(|a| fi.resolve(a)) };
            match row {
                Some(i) if funcs.rows[i][STATUS] == "unknown" => {
                    if seen.insert(id.clone()) {
                        out.push(Problem {
                            registry: "functions".into(),
                            id: funcs.rows[i][ID].clone(),
                            msg: format!("{id} cited at {rel}:{line} but the row is unknown (registry set {} <status> --doc/--port ..)", funcs.rows[i][ID]),
                        });
                    }
                }
                Some(_) => {}
                None => {
                    if seen.insert(id.clone()) {
                        out.push(Problem { registry: "ids".into(), id: id.clone(), msg: format!("cited at {rel}:{line} but its address is in no function row") });
                    }
                }
            }
        }
    }
    if let Some(t) = tracked {
        let cur = scan_files(root, &t, &fi);
        let base = load_baseline(root);
        for c in new_citations(&cur, &base) {
            out.push(Problem {
                registry: "citations".into(),
                id: format!("{}:{}", c.file, c.line),
                msg: "raw address of the original in a tracked file (not in cite_baseline.tsv): cite the ID instead, `mcla-tool registry cite-convert --write <file>`".into(),
            });
        }
    }
    out
}

// ---------------------------------------------------------------- commands

fn funcs_table(tables: &[Table]) -> Result<&Table> {
    tables.iter().find(|t| t.name == "functions").context("functions.tsv missing")
}

fn addr_of(q: &str) -> Option<u32> {
    let t = q.strip_prefix("sub_").or_else(|| q.strip_prefix("0x")).unwrap_or(q);
    key_addr(&t.to_ascii_uppercase())
}

/// `registry ids-sync` (also run after `with-lock` commands): IDs for new function rows.
pub fn cmd_ids_sync(root: &Path) -> Result<()> {
    let funcs = Table::load(root, "functions")?;
    let mut ids = if root.join("docs/registry").join(IDS_FILE).exists() { Ids::load(root)? } else { Ids::new(fresh_seed()) };
    let n = ids.sync_functions(&funcs)?;
    if n > 0 || !root.join("docs/registry").join(IDS_FILE).exists() {
        ids.save(root)?;
    }
    println!("ids: {} rows ({} new)", ids.rows.len(), n);
    Ok(())
}

/// Resolve one query (address or function name) to its ID; `new` allocates an A- ID for an address inside a function.
fn id_for(funcs: &Table, ids: &mut Ids, q: &str, new: bool) -> Result<String> {
    let fi = FuncIndex::new(funcs);
    if let Some(a) = addr_of(q) {
        let i = fi.resolve(a).with_context(|| format!("{q}: not inside any function row"))?;
        let row_id = &funcs.rows[i][ID];
        if key_addr(row_id) == Some(a) {
            return ids.get('F', row_id).map(|s| s.to_string()).with_context(|| "function has no ID yet: run `registry ids-sync`");
        }
        let key = format!("{a:08X}");
        if let Some(f) = ids.get('A', &key) {
            return Ok(f.to_string());
        }
        if !new {
            bail!("{q} is inside {} but not a function start and has no ID yet (use --new to allocate an A- ID)", funcs.rows[i][NAME]);
        }
        return ids.alloc('A', &key);
    }
    let ql = q.to_ascii_lowercase();
    let hit = funcs.rows.iter().find(|r| r[NAME].to_ascii_lowercase() == ql || r[ID].to_ascii_lowercase() == ql).with_context(|| format!("no function named '{q}'"))?;
    ids.get('F', &hit[ID]).map(|s| s.to_string()).with_context(|| "function has no ID yet: run `registry ids-sync`")
}

pub fn cmd_id(root: &Path, o: &Opts) -> Result<()> {
    if o.pos.is_empty() {
        bail!("id <address|name>...");
    }
    let tables = load_snapshot(root)?;
    let funcs = funcs_table(&tables)?;
    let mut ids = Ids::load(root)?;
    for q in &o.pos {
        println!("{}", id_for(funcs, &mut ids, q, o.flags.contains_key("new"))?);
    }
    if o.flags.contains_key("new") {
        ids.save(root)?;
    }
    Ok(())
}

pub fn cmd_addr(root: &Path, o: &Opts) -> Result<()> {
    if o.pos.is_empty() {
        bail!("addr <ID>...");
    }
    let tables = load_snapshot(root)?;
    let funcs = funcs_table(&tables)?;
    let fi = FuncIndex::new(funcs);
    let ids = Ids::load(root)?;
    for q in &o.pos {
        let r = ids.lookup(q).with_context(|| format!("{q}: not an assigned ID"))?;
        if r.kind == 'F' {
            let name = funcs.find_id(&r.key).map(|i| funcs.rows[i][NAME].clone()).unwrap_or_default();
            println!("{}\t{}\t{}", r.fid, r.key, name);
        } else {
            let inside = key_addr(&r.key)
                .and_then(|a| fi.resolve(a))
                .map(|i| format!("inside {} ({})", funcs.rows[i][ID], ids.get('F', &funcs.rows[i][ID]).unwrap_or("?")))
                .unwrap_or_default();
            println!("{}\t{}\t{}", r.fid, r.key, inside);
        }
    }
    Ok(())
}

fn resolve_paths(root: &Path, pos: &[String]) -> Result<Vec<PathBuf>> {
    let mut v = Vec::new();
    for p in pos {
        let pb = PathBuf::from(p);
        let cands = [pb.clone(), root.join(p), root.join("mcla-rs").join(p)];
        let hit = cands.iter().find(|c| c.is_file()).with_context(|| format!("{p}: file not found (tried cwd, repo root, mcla-rs/)"))?;
        v.push(hit.clone());
    }
    Ok(v)
}

pub fn cmd_cite_scan(root: &Path, o: &Opts) -> Result<()> {
    let tables = load_snapshot(root)?;
    let fi = FuncIndex::new(funcs_table(&tables)?);
    let files = if o.pos.is_empty() { tracked_files(root).context("not a git checkout")? } else { resolve_paths(root, &o.pos)? };
    let cur = scan_files(root, &files, &fi);
    let mut per: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for c in &cur {
        let e = per.entry(c.file.as_str()).or_default();
        e.0 += 1;
        e.1 += c.n;
        if o.flags.contains_key("list") {
            println!("{}:{}", c.file, c.line);
        }
    }
    for (f, (l, n)) in &per {
        println!("{l:5} lines {n:5} citations  {f}");
    }
    let base = load_baseline(root);
    let new = new_citations(&cur, &base);
    println!("total: {} lines, {} citations in {} files; baseline {} lines; {} line(s) not covered by it", cur.len(), cur.iter().map(|c| c.n).sum::<usize>(), per.len(), base.len(), if o.pos.is_empty() { new.len() } else { 0 });
    if o.flags.contains_key("write-baseline") {
        if !o.pos.is_empty() {
            bail!("--write-baseline scans all tracked files; do not pass paths");
        }
        if !new.is_empty() && !base.is_empty() && !o.flags.contains_key("force") {
            bail!("the baseline only shrinks: {} citation line(s) are not in it (convert them with cite-convert, or --force after asking the user)", new.len());
        }
        locked(root, || write_atomic(&root.join("docs/registry").join(BASELINE_FILE), baseline_text(&cur).as_bytes()))?;
        println!("wrote docs/registry/{BASELINE_FILE} ({} lines)", cur.len());
    }
    Ok(())
}

pub fn cmd_cite_convert(root: &Path, o: &Opts) -> Result<()> {
    if o.pos.is_empty() {
        bail!("cite-convert [--write] <files...>");
    }
    let write = o.flags.contains_key("write");
    let files = resolve_paths(root, &o.pos)?;
    let run = || -> Result<()> {
        let tables = load_all(root)?;
        let funcs = funcs_table(&tables)?;
        let fi = FuncIndex::new(funcs);
        let mut ids = if root.join("docs/registry").join(IDS_FILE).exists() { Ids::load(root)? } else { Ids::new(fresh_seed()) };
        if write {
            ids.sync_functions(funcs)?;
        }
        let mut total = 0;
        for f in &files {
            let rel = rel_of(root, f);
            let text = std::fs::read_to_string(f).with_context(|| format!("read {rel} (UTF-8 text only)"))?;
            let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
            let mut resolve = |a: u32| -> Result<Option<String>> {
                let Some(i) = fi.resolve(a) else { return Ok(None) };
                let row_id = &funcs.rows[i][ID];
                if key_addr(row_id) == Some(a) {
                    return Ok(Some(ids.get('F', row_id).unwrap_or("F-?????").to_string()));
                }
                let key = format!("{a:08X}");
                if let Some(f) = ids.get('A', &key) {
                    return Ok(Some(f.to_string()));
                }
                if write {
                    Ok(Some(ids.alloc('A', &key)?))
                } else {
                    Ok(Some("A-?????".to_string()))
                }
            };
            let c = convert_text(ext, &text, &|a| fi.resolve(a).is_some(), &mut resolve)?;
            for (l, why) in &c.left {
                println!("left {rel}:{l}: {why}");
            }
            if c.converted > 0 {
                let (old, new): (Vec<&str>, Vec<&str>) = (text.lines().collect(), c.text.lines().collect());
                for (n, (a, b)) in old.iter().zip(new.iter()).enumerate() {
                    if a != b {
                        println!("{rel}:{}\n  - {}\n  + {}", n + 1, a.trim(), b.trim());
                    }
                }
                if write {
                    write_atomic(f, c.text.as_bytes())?;
                }
            }
            println!("{rel}: {} citation(s) {}", c.converted, if write { "converted" } else { "would be converted (dry run, add --write)" });
            total += c.converted;
        }
        if write && total > 0 {
            ids.save(root)?;
        }
        Ok(())
    };
    if write {
        locked(root, run)
    } else {
        run()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic addresses only: odd low digits are never an instruction address of the executable.
    fn funcs() -> Table {
        let mk = |id: &str, size: &str| {
            let mut r = vec![String::new(); COLS.len()];
            r[ID] = id.into();
            r[NAME] = format!("sub_{id}");
            r[TOPIC] = "AI".into();
            r[STATUS] = "unknown".into();
            r[ATTRS] = format!("size={size}");
            r
        };
        Table { name: "functions".into(), path: PathBuf::from("x.tsv"), rows: vec![mk("82250001", "8"), mk("82250101", "4"), mk("xstart", "2")] }
    }

    #[test]
    fn permutation_and_stability() {
        let a = make_pool(7);
        let b = make_pool(7);
        assert_eq!(a, b);
        assert_ne!(a, make_pool(8));
        assert_eq!(a.len(), 90_000);
        let mut s = a.clone();
        s.sort();
        assert_eq!(s, (POOL_LO..=POOL_HI).collect::<Vec<_>>());
        assert_ne!(&a[..10], &(POOL_LO..POOL_LO + 10).collect::<Vec<_>>()[..], "not in order");
        let f = funcs();
        let mut ids = Ids::new(7);
        assert_eq!(ids.sync_functions(&f).unwrap(), 3);
        assert_eq!(ids.sync_functions(&f).unwrap(), 0);
        let first = ids.get('F', "82250001").unwrap().to_string();
        assert_eq!(first, format!("F-{:05}", a[0]));
        let x = ids.alloc('A', "82250009").unwrap();
        assert_eq!(x, format!("A-{:05}", a[3]));
        assert_eq!(ids.alloc('A', "82250009").unwrap(), x);
        assert!(ids.problems(Some(&f)).is_empty());
        ids.rows[1].fid = "F-10000".into();
        assert!(!ids.problems(Some(&f)).is_empty());
    }

    #[test]
    fn save_load_round_trip() {
        let root = std::env::temp_dir().join(format!("mcla_ids_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs/registry")).unwrap();
        let mut ids = Ids::new(99);
        ids.sync_functions(&funcs()).unwrap();
        ids.alloc('A', "82250005").unwrap();
        ids.save(&root).unwrap();
        let l = Ids::load(&root).unwrap();
        assert_eq!(l.rows, ids.rows);
        assert!(l.problems(Some(&funcs())).is_empty());
        let mut l = l;
        let n = l.alloc('A', "82250007").unwrap();
        assert_eq!(n, format!("A-{:05}", make_pool(99)[4]));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn id_citations() {
        let t = "see F-10432, A-99999. not XF-10432 or F-104321 or F-1043 or F_10432\nF-12345";
        let ids: Vec<(usize, String)> = id_spans(t).into_iter().map(|s| (s.2, s.3)).collect();
        assert_eq!(ids, vec![(1, "F-10432".into()), (1, "A-99999".into()), (2, "F-12345".into())]);
        assert_eq!(parse_fid("F-12345"), Some(('F', 12345)));
        assert_eq!(parse_fid("F-1234"), None);
    }

    #[test]
    fn spans_match_citations() {
        let t = "calls sub_82250001 and 0x82250009, bare 82250101 (hash 0x8053a397) not 182250001 or 822500011\n82250001-82250101.";
        let s = address_spans(t);
        assert_eq!(s.iter().map(|x| x.addr).collect::<Vec<_>>(), vec![0x82250001, 0x82250009, 0x82250101, 0x82250001, 0x82250101]);
        assert_eq!(&t[s[0].start..s[0].end], "sub_82250001");
        assert_eq!(&t[s[1].start..s[1].end], "0x82250009");
        assert_eq!(&t[s[2].start..s[2].end], "82250101");
        assert!(s[0].strong && !s[1].strong);
    }

    fn conv(ext: &str, text: &str) -> Conversion {
        let f = funcs();
        let fi = FuncIndex::new(&f);
        let mut ids = Ids::new(5);
        ids.sync_functions(&f).unwrap();
        let mut res = |a: u32| -> Result<Option<String>> {
            let Some(i) = fi.resolve(a) else { return Ok(None) };
            if key_addr(&f.rows[i][ID]) == Some(a) {
                return Ok(Some(ids.get('F', &f.rows[i][ID]).unwrap().to_string()));
            }
            Ok(Some(ids.alloc('A', &format!("{a:08X}"))?))
        };
        convert_text(ext, text, &|a| fi.resolve(a).is_some(), &mut res).unwrap()
    }

    #[test]
    fn convert_comments_only() {
        let pool = make_pool(5);
        let f1 = format!("F-{:05}", pool[0]);
        let f2 = format!("F-{:05}", pool[1]);
        let a1 = format!("A-{:05}", pool[3]);
        let src = "let a = 0x82250001; // sub_82250001 and 0x82250101 here, mid 82250009, hash 0x8053a397 and 0x82990011\n\
                   /// range 82250001-82250101 done\n\
                   let s = \"sub_82250001\"; /* block 0x82250001\n still block sub_82250101 */ fn sub_82250001_x() {} // sub_82250001_y\n";
        let c = conv("rs", src);
        let want = format!(
            "let a = 0x82250001; // {f1} and {f2} here, mid {a1}, hash 0x8053a397 and 0x82990011\n\
             /// range {f1}..{f2} done\n\
             let s = \"sub_82250001\"; /* block {f1}\n still block {f2} */ fn sub_82250001_x() {{}} // sub_82250001_y\n"
        );
        assert_eq!(c.text, want);
        assert_eq!(c.converted, 7);
        assert!(c.left.iter().any(|l| l.1.contains("not in a comment")));
        assert!(c.left.iter().any(|l| l.1.contains("glued")));
        // second run changes nothing
        assert_eq!(conv("rs", &c.text).converted, 0);
    }

    #[test]
    fn convert_prose_and_python() {
        let pool = make_pool(5);
        let f1 = format!("F-{:05}", pool[0]);
        assert_eq!(conv("md", "see 0x82250001.\r\nnext").text, format!("see {f1}.\r\nnext"));
        let py = "x = 0x82250001  # sub_82250001\n\"\"\"doc 82250001\"\"\"\ny = 'sub_82250001'\n";
        assert_eq!(conv("py", py).text, format!("x = 0x82250001  # {f1}\n\"\"\"doc {f1}\"\"\"\ny = 'sub_82250001'\n"));
    }

    #[test]
    fn baseline_rules() {
        let f = funcs();
        let fi = FuncIndex::new(&f);
        let old = "// sub_82250001 old\nfn a() {}\n// sub_82250001 old\n";
        let base = scan_text("a.rs", old, &fi);
        assert_eq!(base.len(), 2);
        // moved line: fine; third copy and an edited line: new
        let cur = scan_text("a.rs", "fn a() {}\n// sub_82250001 old\n// sub_82250001 old\n// sub_82250001 old\n// sub_82250101 other\n", &fi);
        let new = new_citations(&cur, &base);
        assert_eq!(new.iter().map(|c| c.line).collect::<Vec<_>>(), vec![4, 5]);
        // a different file never uses another file's baseline
        assert_eq!(new_citations(&scan_text("b.rs", old, &fi), &base).len(), 2);
        // hex that is no function is not a citation
        assert!(scan_text("a.rs", "// 0x82990011\n", &fi).is_empty());
        let t = baseline_text(&base);
        assert!(t.starts_with("file\tline\thash\tcitations\n") && t.lines().count() == 3);
    }

    #[test]
    fn check_resolves_ids() {
        let root = std::env::temp_dir().join(format!("mcla_idcheck_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs/registry")).unwrap();
        let mut f = funcs();
        f.rows[1][STATUS] = "partly-decoded".into();
        let mut ids = Ids::new(3);
        ids.sync_functions(&f).unwrap();
        let known = ids.get('F', "82250101").unwrap().to_string();
        let unknown_row = ids.get('F', "82250001").unwrap().to_string();
        std::fs::write(root.join("docs/AI.md"), format!("fine {known}, unknown row {unknown_row}, typo F-10001 and A-10002")).unwrap();
        let p = check_ids(&root, &f, &ids);
        let msgs: Vec<&str> = p.iter().map(|x| x.msg.as_str()).collect();
        assert_eq!(p.len(), 3, "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("row is unknown")));
        assert_eq!(msgs.iter().filter(|m| m.contains("not an assigned ID")).count(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }
}
