//! `mcla-tool registry hits <file.tsv> [--top N] [--dry-run]`: import the function hit counter's result.
//!
//! The file (made by `work/recomp-hits/make_hits.py`) has the header `id<TAB>hits<TAB>scenes`: the function's id (address),
//! the number of times the recompiled original entered it over all recorded scenes, and the scenes it ran in (`a/b/c`).
//! Every listed `functions` row gets `hits=<n>` and, when it ran, `hit_in=<scenes>` in `attrs` (an older value is replaced);
//! `hits=0` is only a candidate for dead code (a scene that was not recorded may use it), never evidence by itself.
//! Prints the totals and, with `--top N`, the N most-hit rows that are not yet ported / closed.

use super::*;

const UNPORTED: [&str; 3] = ["unknown", "partly-decoded", "decoded-not-ported"];

fn strip(attrs: &str) -> Vec<String> {
    attrs.split(';').filter(|kv| !kv.starts_with("hits=") && !kv.starts_with("hit_in=") && !kv.is_empty()).map(|s| s.to_string()).collect()
}

pub fn cmd_hits(root: &Path, o: &Opts) -> Result<()> {
    let file = o.pos.first().context("registry hits <file.tsv>")?;
    let text = std::fs::read_to_string(file).with_context(|| format!("reading {file}"))?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let head: Vec<&str> = lines.next().context("hits file is empty")?.split('\t').map(|h| h.trim()).collect();
    if head != ["id", "hits", "scenes"] {
        bail!("hits file needs the header id<TAB>hits<TAB>scenes");
    }
    let mut funcs = Table::load(root, "functions")?;
    let dry = o.flags.contains_key("dry-run");
    let (mut set, mut missing) = (0usize, 0usize);
    for (n, l) in lines.enumerate() {
        let f: Vec<&str> = l.split('\t').collect();
        let hits: u64 = f.get(1).and_then(|v| v.trim().parse().ok()).with_context(|| format!("hits row {}: bad count", n + 2))?;
        let scenes = f.get(2).map(|s| s.trim()).unwrap_or("");
        if scenes.contains(';') || scenes.contains('=') {
            bail!("hits row {}: scene names may not hold ; or =", n + 2);
        }
        let Some(i) = funcs.find_id(f[0].trim()) else {
            missing += 1;
            continue;
        };
        let mut attrs = strip(&funcs.rows[i][ATTRS]);
        attrs.push(format!("hits={hits}"));
        if hits > 0 && !scenes.is_empty() {
            attrs.push(format!("hit_in={scenes}"));
        }
        funcs.rows[i][ATTRS] = attrs.join(";");
        set += 1;
    }
    // totals
    let mut by_status: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for r in &funcs.rows {
        let Some(h) = attr(&r[ATTRS], "hits").and_then(|v| v.parse::<u64>().ok()) else { continue };
        let e = by_status.entry(r[STATUS].clone()).or_default();
        e.0 += 1;
        if h > 0 {
            e.1 += 1;
        }
    }
    println!("hits: {set} rows set{}{}", if missing > 0 { format!(", {missing} ids not in functions.tsv") } else { String::new() }, if dry { " (dry run, nothing written)" } else { "" });
    println!("{:<20} {:>7} {:>7} {:>7}", "status", "rows", "hit", "never");
    for (s, (n, h)) in &by_status {
        println!("{:<20} {:>7} {:>7} {:>7}", s, n, h, n - h);
    }
    if let Some(top) = o.flags.get("top").and_then(|v| v.parse::<usize>().ok()) {
        let ids = Ids::load(root).ok();
        let mut v: Vec<(u64, &Vec<String>)> = funcs
            .rows
            .iter()
            .filter(|r| UNPORTED.contains(&r[STATUS].as_str()))
            .filter_map(|r| attr(&r[ATTRS], "hits").and_then(|h| h.parse::<u64>().ok()).filter(|h| *h > 0).map(|h| (h, r)))
            .collect();
        v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1[ID].cmp(&b.1[ID])));
        println!("top {top} hit functions that are not ported:");
        for (h, r) in v.into_iter().take(top) {
            let fid = ids.as_ref().and_then(|i| i.get('F', &r[ID])).unwrap_or("-");
            println!("{h:>12}  {fid}  {}  {}  {}  [{}]", r[ID], r[NAME], r[STATUS], r[TOPIC]);
        }
    }
    if !dry {
        funcs.save()?;
    }
    Ok(())
}
