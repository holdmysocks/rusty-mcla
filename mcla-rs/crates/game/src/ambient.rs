//! The game's ambient population tables (`tune/ambients`): a district map
//! (`sc.dds`, one grey level per district), a tree of `mcAmbientDistribution.xml` files
//! listing which vehicle / pedestrian types appear in every district at which time of
//! day, and the overall density over the day. One tree holds traffic, the other
//! pedestrians.

use glam::Vec3;
use hashbrown::HashMap;
use std::path::Path;

pub fn xml_value(text: &str, tag: &str) -> Option<f32> {
    let i = text.find(&format!("<{tag} "))?;
    let rest = &text[i..];
    let v = rest.find("value=\"")? + 7;
    let e = rest[v..].find('"')?;
    rest[v..v + e].trim().parse().ok()
}

/// A `<tag value="true|false"/>` of a tune file.
pub fn xml_bool(text: &str, tag: &str) -> Option<bool> {
    let i = text.find(&format!("<{tag} "))?;
    let rest = &text[i..];
    let v = rest.find("value=\"")? + 7;
    let e = rest[v..].find('"')?;
    rest[v..v + e].trim().parse().ok()
}

/// Weight of something over the day: (start, end, weight) rows of a `TodArray`, times as
/// written (HHMM).
#[derive(Default, Clone, Debug)]
pub struct Tod(pub(crate) Vec<(i32, i32, f32)>);

impl Tod {
    pub fn parse(xml: &str) -> Tod {
        let mut v = Vec::new();
        // the files write the row tag both as <Item> and <item>
        for item in xml.replace("<item>", "<Item>").split("<Item>").skip(1) {
            let (Some(a), Some(b), Some(w)) = (xml_value(&item, "iStartTime"), xml_value(&item, "iEndTime"), xml_value(&item, "fWeight")) else { continue };
            v.push((a as i32, b as i32, w));
        }
        Tod(v)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The original's lookup (F-90692): the clock as HHMM, the first row with
    /// start <= t <= end (an end of 0000 is 2400), 0 when no row covers it. A table
    /// without rows weighs 1 (the tree's starting value, F-83932).
    pub fn at(&self, hour: f32) -> f32 {
        if self.0.is_empty() {
            return 1.0;
        }
        self.at_hhmm(hhmm(hour))
    }

    /// F-90692 itself, on the clock as HHMM: the first row with start <= t <= end (an end of
    /// 0000 is 2400), 0 when no row covers it - a table without rows too.
    pub fn at_hhmm(&self, t: i32) -> f32 {
        self.0.iter().find(|(a, b, _)| t >= *a && t <= if *b == 0 { 2400 } else { *b }).map(|r| r.2).unwrap_or(0.0)
    }
}

/// The clock as HHMM (hours x 100 + whole minutes).
pub fn hhmm(hour: f32) -> i32 {
    let h = hour.rem_euclid(24.0);
    h.floor() as i32 * 100 + (h.fract() * 60.0).floor() as i32
}

/// One node of a distribution tree as F-83932 reads it.
#[derive(Default, Clone, Debug)]
pub struct Node {
    /// its `TypeDistributionArray`: the type's index in the tree's type list (-1: a name the
    /// loader found no type for) and its weight over the day
    pub types: Vec<(i32, Tod)>,
    /// its `TodDensityArray`
    pub density: Tod,
    pub parent: Option<usize>,
}

/// What F-83932 reads of a type: +8 the instances it holds (F-49084 adds one when a
/// request takes the type, F-39893 gives it back), +111 `bIsCab`, +112 `bIsTrailer`.
#[derive(Default, Clone, Copy, Debug)]
pub struct TypeUse {
    pub live: i32,
    pub cab: bool,
    pub trailer: bool,
}

/// A list's weights, their total and the density (the list +8, +16, +20).
#[derive(Default, Clone, Debug)]
pub struct Weights {
    pub w: Vec<f32>,
    pub total: f32,
    pub density: f32,
}

/// The density of the district node `node` at `t` (HHMM): the first node up the chain whose
/// table has rows and gives no less than 0 (a NaN too), else 1 (F-83932).
pub fn chain_density(nodes: &[Node], node: Option<usize>, t: i32) -> f32 {
    let mut k = node;
    while let Some(i) = k {
        if !nodes[i].density.0.is_empty() {
            let v = nodes[i].density.at_hhmm(t);
            if !(v < 0.0) {
                return v;
            }
        }
        k = nodes[i].parent;
    }
    1.0
}

/// F-83932: the weights of a list's types for district node `node` at `t` (HHMM). All weights
/// and the total cleared (F-97900); `uniform` (the manager's byte +248 / +249): every weight
/// 1, the total the count, the density left. Else the density (`chain_density`), then up the
/// chain from the node to the root each type takes the weight of the first node that lists it
/// (the nearest; a type no node lists weighs 0). When 10 or more types hold instances, a type
/// that holds none and is no trailer weighs 0. With `test_trailers` (the `testtrailers`
/// parameter) only cabs and trailers weigh 1, the rest 0. The total: the weights added in order.
pub fn type_weights(out: &mut Weights, nodes: &[Node], node: Option<usize>, types: &[TypeUse], t: i32, uniform: bool, test_trailers: bool) {
    let n = types.len();
    out.w.clear();
    out.w.resize(n, 0.0);
    out.total = 0.0;
    if uniform {
        out.w.fill(1.0);
        out.total = n as f32;
        return;
    }
    out.density = chain_density(nodes, node, t);
    let mut seen = vec![false; n];
    let mut k = node;
    while let Some(i) = k {
        for (ix, tod) in &nodes[i].types {
            if *ix >= 0 && !seen[*ix as usize] {
                seen[*ix as usize] = true;
                out.w[*ix as usize] = tod.at_hhmm(t);
            }
        }
        k = nodes[i].parent;
    }
    if types.iter().filter(|u| u.live > 0).count() >= 10 {
        for (w, u) in out.w.iter_mut().zip(types) {
            if u.live == 0 && *w > 0.0 && !u.trailer {
                *w = 0.0;
            }
        }
    }
    if test_trailers {
        for (w, u) in out.w.iter_mut().zip(types) {
            *w = if u.cab || u.trailer { 1.0 } else { 0.0 };
        }
    }
    out.total = out.w.iter().fold(0.0f32, |a, w| w + a);
}

#[derive(Default)]
pub struct Distribution {
    map: Vec<u8>,
    /// per district colour: (type name, weight over the day), ancestors' lists included
    pub types: HashMap<u8, Vec<(String, Tod)>>,
    pub density: Tod,
    /// per district colour: its density over the day - its own `TodDensityArray`, else
    /// the nearest ancestor's (F-83932 walks the district's nodes for the first table)
    pub district_density: HashMap<u8, Tod>,
    /// F-83932's tree: the type list in the loader's order (the tree in pre-order, a node's
    /// types then its `ChildNames` in order, each name at its first sight), the nodes, and the
    /// node of each district colour
    pub all: Vec<String>,
    pub nodes: Vec<Node>,
    pub node_of: HashMap<u8, usize>,
    /// per type of `all`: the instances it holds (type +8) and `bIsTrailer` (+112), set by the
    /// owner once a frame (`set_use`)
    pub uses: Vec<TypeUse>,
}

fn type_list(xml: &str) -> Vec<(String, Tod)> {
    let body = xml.split("<TodDensityArray>").next().unwrap_or("");
    body.split("<TypeName>").skip(1).map(|item| (item.split('<').next().unwrap_or("").trim().to_ascii_lowercase(), Tod::parse(item))).collect()
}

impl Distribution {
    // world rectangle covered by the 256x256 map (fitted to the parked car lists, which
    // are stored per district)
    const X0: f32 = -3102.5;
    const X1: f32 = 2912.5;
    const Z0: f32 = -2450.0;
    const Z1: f32 = 2060.0;

    pub fn district(&self, p: Vec3) -> Option<u8> {
        if self.map.len() < 65536 {
            return None;
        }
        let u = ((p.x - Self::X0) / (Self::X1 - Self::X0) * 256.0).clamp(0.0, 255.0) as usize;
        let v = ((p.z - Self::Z0) / (Self::Z1 - Self::Z0) * 256.0).clamp(0.0, 255.0) as usize;
        Some(self.map[v * 256 + u])
    }

    /// Density of the district at `p` at this hour (its `TodDensityArray`; the root's where
    /// the map gives no district).
    pub fn density_at(&self, p: Vec3, hour: f32) -> f32 {
        match self.district(p).and_then(|c| self.node_of.get(&c)) {
            Some(&k) => chain_density(&self.nodes, Some(k), hhmm(hour)),
            None => self.density.at(hour),
        }
    }

    /// The types' use for F-83932's variety rule: `live(name)` the instances of the type in
    /// play, `trailer(name)` its `bIsTrailer`.
    pub fn set_use(&mut self, live: &dyn Fn(&str) -> i32, trailer: &dyn Fn(&str) -> bool) {
        self.uses = self.all.iter().map(|n| TypeUse { live: live(n), cab: false, trailer: trailer(n) }).collect();
    }

    /// F-83932's weights of the list for the district at `pos` at this hour (None: no district).
    pub fn weights(&self, pos: Vec3, hour: f32) -> Option<Weights> {
        let node = *self.node_of.get(&self.district(pos)?)?;
        let uses: Vec<TypeUse> = (0..self.all.len()).map(|i| self.uses.get(i).copied().unwrap_or_default()).collect();
        let mut w = Weights::default();
        type_weights(&mut w, &self.nodes, Some(node), &uses, hhmm(hour), false, false);
        Some(w)
    }

    /// F-83932's tree from `dir` in the loader's order (a node's list, then its children).
    fn load_tree(&mut self, dir: &Path, parent: Option<usize>) {
        let k = self.nodes.len();
        let x = std::fs::read_to_string(dir.join("mcAmbientDistribution.xml")).unwrap_or_default();
        let mut node = Node { parent, ..Default::default() };
        for (name, tod) in type_list(&x) {
            let i = match self.all.iter().position(|n| *n == name) {
                Some(i) => i,
                None => {
                    self.all.push(name);
                    self.all.len() - 1
                }
            };
            node.types.push((i as i32, tod));
        }
        if let Some(i) = x.find("<TodDensityArray>") {
            node.density = Tod::parse(&x[i..]);
        }
        self.nodes.push(node);
        let tree = std::fs::read_to_string(dir.join("mcAmbientDistributionTreeData.xml")).unwrap_or_default();
        if let Some(c) = xml_value(&tree, "iMapColor") {
            self.node_of.insert(c as u8, k);
        }
        let kids = tree.split("<ChildNames>").nth(1).and_then(|t| t.split("</ChildNames>").next()).unwrap_or("");
        let subs: Vec<std::path::PathBuf> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default();
        for item in kids.split("<Item>").skip(1) {
            let name = item.split('<').next().unwrap_or("").trim();
            if let Some(sub) = subs.iter().find(|p| p.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(name))) {
                self.load_tree(sub, Some(k));
            }
        }
    }

    pub fn loaded(&self) -> bool {
        !self.types.is_empty()
    }

    /// Load the tree whose type names start with `prefix` ("ped_" or "va_").
    pub fn load(data_dir: &Path, prefix: &str) -> Distribution {
        let mut d = Distribution::default();
        let amb = data_dir.join("tune/ambients");
        if let Ok(b) = std::fs::read(amb.join("sc.dds")).or_else(|_| std::fs::read(amb.join("LA.dds"))) {
            if b.len() >= 128 + 65536 {
                d.map = b[b.len() - 65536..].to_vec();
            }
        }
        let Ok(rd) = std::fs::read_dir(&amb) else { return d };
        let mut tops: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        tops.sort();
        for top in tops {
            let Ok(root_xml) = std::fs::read_to_string(top.join("mcAmbientDistribution.xml")) else { continue };
            // walk the tree, handing every node the lists (and density table) of its ancestors
            let mut leaves: Vec<(u8, Vec<(String, Tod)>, Tod)> = Vec::new();
            let mut stack = vec![(top.clone(), Vec::<(String, Tod)>::new(), Tod::default())];
            while let Some((dir, inherited, mut density)) = stack.pop() {
                let mut list = inherited;
                if let Ok(x) = std::fs::read_to_string(dir.join("mcAmbientDistribution.xml")) {
                    list.extend(type_list(&x));
                    if let Some(i) = x.find("<TodDensityArray>") {
                        let own = Tod::parse(&x[i..]);
                        if !own.is_empty() {
                            density = own;
                        }
                    }
                }
                if let Ok(tree) = std::fs::read_to_string(dir.join("mcAmbientDistributionTreeData.xml")) {
                    if let Some(c) = xml_value(&tree, "iMapColor") {
                        leaves.push((c as u8, list.clone(), density.clone()));
                    }
                }
                if let Ok(rd) = std::fs::read_dir(&dir) {
                    for c in rd.flatten() {
                        if c.path().is_dir() {
                            stack.push((c.path(), list.clone(), density.clone()));
                        }
                    }
                }
            }
            if !leaves.iter().any(|(_, l, _)| l.iter().any(|(n, _)| n.starts_with(prefix))) {
                continue;
            }
            if let Some(i) = root_xml.find("<TodDensityArray>") {
                d.density = Tod::parse(&root_xml[i..]);
            }
            d.district_density = leaves.iter().map(|(c, _, t)| (*c, t.clone())).collect();
            d.types = leaves.into_iter().map(|(c, l, _)| (c, l)).collect();
            d.load_tree(&top, None);
            break;
        }
        d
    }

    /// Weighted pick for this place and hour among the types `usable` accepts
    /// (`r` in 0..1). `None` when the district has nothing to offer.
    /// The types `pick` can draw at `pos` at this hour.
    pub fn names(&self, pos: Vec3, hour: f32) -> Vec<&str> {
        self.district(pos).and_then(|d| self.types.get(&d)).map_or(Vec::new(), |l| l.iter().filter(|(_, t)| t.at(hour) > 0.0).map(|(n, _)| n.as_str()).collect())
    }

    /// The weights are F-83932's (`weights`): each type the nearest node's weight, the variety
    /// rule on the types' use (`set_use`).
    pub fn pick(&self, pos: Vec3, hour: f32, r: f32, usable: &dyn Fn(&str) -> bool) -> Option<&str> {
        let wt = self.weights(pos, hour)?;
        let list: Vec<(&String, f32)> = self.all.iter().zip(wt.w).filter(|(n, _)| usable(n)).collect();
        let total: f32 = list.iter().map(|(_, w)| *w).sum();
        if total <= 0.0 {
            return None;
        }
        let mut x = r.clamp(0.0, 0.9999) * total;
        let mut last = None;
        for &(n, w) in &list {
            if w <= 0.0 {
                continue;
            }
            last = Some(n.as_str());
            x -= w;
            if x <= 0.0 {
                break;
            }
        }
        last
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture dir of the verification run against the original's recompiled functions
    /// (work/verify-people/peds/gen_and_run.py; docs/reference/people/verify_people_peds.md).
    fn fixture(name: &str) -> Option<String> {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../work/verify-people/peds").join(name)).ok()
    }

    /// F-90692 on 4200 cases: tables of 0..8 rows, every minute of the day at the row edges. The
    /// original takes the clock as HHMM; `at` takes the hour and makes the HHMM itself, so the hour is
    /// built from the HHMM (middle of the minute).
    #[test]
    fn tod_at_matches_original_bits() {
        let (Some(inp), Some(refs)) = (fixture("tod_inputs.txt"), fixture("tod_reference.txt")) else { return };
        let (mut cases, mut empty, mut bad) = (0, 0, 0);
        for (line, want) in inp.lines().zip(refs.lines()) {
            let mut it = line.split_whitespace();
            let n: usize = it.next().unwrap().parse().unwrap();
            let t: i32 = it.next().unwrap().parse().unwrap();
            let rows: Vec<(i32, i32, f32)> = (0..n)
                .map(|_| {
                    let s: i32 = it.next().unwrap().parse().unwrap();
                    let e: i32 = it.next().unwrap().parse().unwrap();
                    (s, e, f32::from_bits(u32::from_str_radix(it.next().unwrap(), 16).unwrap()))
                })
                .collect();
            let want = u32::from_str_radix(want, 16).unwrap();
            let hour = (t / 100) as f32 + ((t % 100) as f32 + 0.5) / 60.0;
            let got = Tod(rows).at(hour).to_bits();
            if n == 0 {
                // the one known difference: no rows is 0.0 in the original, 1.0 here (the tree's start value)
                empty += 1;
                assert_eq!(want, 0.0f32.to_bits());
                assert_eq!(got, 1.0f32.to_bits());
                continue;
            }
            cases += 1;
            if got != want {
                bad += 1;
                eprintln!("tod mismatch: {line} -> port {got:08x} original {want:08x}");
            }
        }
        eprintln!("tod: {cases} non-empty cases, {bad} mismatches; {empty} empty-table cases (differ as documented)");
        assert_eq!(bad, 0);
    }

    /// F-36934 on 4500 cases: the weighted draw from the original's 23-bit random value
    /// (`r` = value * 2^-23, exact), each type's weight a one-row table over the whole day.
    #[test]
    fn pick_matches_original_choice() {
        let (Some(full), Some(refs)) = (fixture("pick_full.txt"), fixture("pick_reference.txt")) else { return };
        let (mut cases, mut bad, mut le, mut clamp) = (0, 0, 0, 0);
        for (line, want) in full.lines().zip(refs.lines()) {
            let tok: Vec<&str> = line.split_whitespace().collect();
            let n: usize = tok[0].parse().unwrap();
            let ws: Vec<f32> = (0..n).map(|i| f32::from_bits(u32::from_str_radix(tok[3 + i], 16).unwrap())).collect();
            let r23 = u32::from_str_radix(tok[3 + n], 16).unwrap();
            let want = u32::from_str_radix(want, 16).unwrap();
            let want = if want == 0 { None } else { Some((want - 0xA000) as usize) };
            let mut d = Distribution::default();
            d.map = vec![5u8; 65536];
            d.all = (0..n).map(|i| format!("t{i}")).collect();
            d.nodes = vec![Node { types: ws.iter().enumerate().map(|(i, w)| (i as i32, Tod(vec![(0, 0, *w)]))).collect(), ..Default::default() }];
            d.node_of.insert(5, 0);
            let r = r23 as f32 * (1.0 / 8388608.0);
            let got = d.pick(Vec3::ZERO, 12.0, r, &|_| true).map(|s| s[1..].parse::<usize>().unwrap());
            cases += 1;
            if got != want {
                bad += 1;
                // classify: the original's strict u < 0 against this port's x <= 0, and the 0.9999 clamp
                let total: f32 = ws.iter().fold(0.0, |a, w| a + w);
                let mut u = r * total;
                let mut exact = false;
                for w in &ws {
                    u -= w;
                    if u == 0.0 {
                        exact = true;
                    }
                    if u < 0.0 {
                        break;
                    }
                }
                if r > 0.9999 {
                    clamp += 1;
                } else if exact {
                    le += 1;
                }
                if bad <= 5 {
                    eprintln!("pick mismatch: {line} -> port {got:?} original {want:?}");
                }
            }
        }
        eprintln!("pick: {cases} cases, {bad} mismatches ({le} on exact zero, {clamp} on r > 0.9999)");
        // known differences (partly-decoded): every mismatch is one of the two, none else
        assert_eq!(bad, le + clamp, "a mismatch outside the two known differences");
    }

    /// F-83932 (with F-97900 and F-90692) run whole offline on the real traffic and people trees
    /// and 40 synthetic ones (tools/ambient_weights_reference.py -> testdata/ambient_weights_ref.txt):
    /// 4,000 cases - every node at clocks on and about the rows' edges, 10 or more types in use
    /// in about half, the cab / trailer flags, the uniform flag and the trailer test. Every
    /// weight, the total and the density to the bit.
    #[test]
    fn type_weights_match_original_bits() {
        let txt = include_str!("testdata/ambient_weights_ref.txt");
        let hex = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let rows = |it: &mut std::str::SplitWhitespace, n: usize| -> Tod { Tod((0..n).map(|_| (hex(it.next().unwrap()) as i32, hex(it.next().unwrap()) as i32, f32::from_bits(hex(it.next().unwrap())))).collect()) };
        let mut trees: Vec<(Vec<Node>, usize)> = Vec::new();
        let (mut cases, mut bad, mut capped, mut real) = (0, 0, 0, 0);
        for l in txt.lines().filter(|l| !l.is_empty()) {
            let mut it = l.split_whitespace();
            match it.next() {
                Some("T") => {
                    it.next();
                    trees.push((Vec::new(), it.next().unwrap().parse().unwrap()));
                }
                Some("N") => {
                    let t: usize = it.next().unwrap().parse().unwrap();
                    it.next();
                    let p: i32 = it.next().unwrap().parse().unwrap();
                    assert_eq!(it.next(), Some("D"));
                    let nd: usize = it.next().unwrap().parse().unwrap();
                    let density = rows(&mut it, nd);
                    assert_eq!(it.next(), Some("E"));
                    let ne: usize = it.next().unwrap().parse().unwrap();
                    let types = (0..ne)
                        .map(|_| {
                            let ix: i32 = it.next().unwrap().parse().unwrap();
                            let nr: usize = it.next().unwrap().parse().unwrap();
                            (ix, rows(&mut it, nr))
                        })
                        .collect();
                    trees[t].0.push(Node { types, density, parent: (p >= 0).then_some(p as usize) });
                }
                Some("C") => {
                    let (head, want) = l.split_once(" -> ").unwrap();
                    let h: Vec<&str> = head.split_whitespace().collect();
                    let (t, node): (usize, usize) = (h[1].parse().unwrap(), h[2].parse().unwrap());
                    let (hm, uniform, test, old) = (hex(h[3]) as i32, h[4] == "1", h[5] == "1", f32::from_bits(hex(h[6])));
                    let (nodes, n) = &trees[t];
                    let mut uses = vec![TypeUse::default(); *n];
                    let fpos = h.iter().position(|x| *x == "F").unwrap();
                    for kv in &h[8..fpos] {
                        let (i, v) = kv.split_once(':').unwrap();
                        uses[hex(i) as usize].live = hex(v) as i32;
                    }
                    for kv in &h[fpos + 1..] {
                        let (i, v) = kv.split_once(':').unwrap();
                        uses[hex(i) as usize].cab = hex(v) & 1 != 0;
                        uses[hex(i) as usize].trailer = hex(v) & 2 != 0;
                    }
                    let mut w = Weights { w: vec![9.25; *n], total: 5.5, density: old };
                    type_weights(&mut w, nodes, Some(node), &uses, hm, uniform, test);
                    let (ws, tail) = want.split_once('|').unwrap();
                    let mut exp = vec![0u32; *n];
                    for kv in ws.split_whitespace() {
                        let (i, v) = kv.split_once(':').unwrap();
                        exp[hex(i) as usize] = hex(v);
                    }
                    let tl: Vec<u32> = tail.split_whitespace().map(hex).collect();
                    let got: Vec<u32> = w.w.iter().map(|x| x.to_bits()).collect();
                    cases += 1;
                    real += (t < 2) as usize;
                    capped += (uses.iter().filter(|u| u.live > 0).count() >= 10) as usize;
                    if got != exp || w.total.to_bits() != tl[0] || w.density.to_bits() != tl[1] {
                        bad += 1;
                        if bad <= 5 {
                            eprintln!("weights mismatch: {head} -> port {got:x?} {:08x} {:08x}, original {exp:x?} {tail}", w.total.to_bits(), w.density.to_bits());
                        }
                    }
                }
                _ => panic!("bad line {l}"),
            }
        }
        eprintln!("type weights: {cases} cases ({real} on the real trees, {capped} with 10 or more types in use), {bad} mismatches");
        assert!(cases >= 4000 && real >= 3000);
        assert_eq!(bad, 0);
        // the port's loader builds the trees the reference ran on (traffic 0, people 1), when the data is here
        let root = std::path::PathBuf::from(std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string()));
        for (t, prefix) in [(0usize, "va_"), (1, "ped_")] {
            let d = Distribution::load(&root, prefix);
            if d.nodes.is_empty() {
                continue;
            }
            let (nodes, n) = &trees[t];
            assert_eq!((d.all.len(), d.nodes.len()), (*n, nodes.len()), "tree {t}");
            for (a, b) in d.nodes.iter().zip(nodes) {
                assert_eq!(a.parent, b.parent);
                assert_eq!(a.types.iter().map(|x| (x.0, x.1 .0.clone())).collect::<Vec<_>>(), b.types.iter().map(|x| (x.0, x.1 .0.clone())).collect::<Vec<_>>());
                assert_eq!(a.density.0, b.density.0);
            }
            eprintln!("tree {t}: the port's loader gives the reference's {} nodes and {} types", nodes.len(), n);
        }
    }
}
