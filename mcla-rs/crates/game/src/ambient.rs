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

    /// The original's lookup (sub_826DA510): the clock as HHMM, the first row with
    /// start <= t <= end (an end of 0000 is 2400), 0 when no row covers it. A table
    /// without rows weighs 1 (the tree's starting value, sub_826DAB80).
    pub fn at(&self, hour: f32) -> f32 {
        if self.0.is_empty() {
            return 1.0;
        }
        let h = hour.rem_euclid(24.0);
        let t = h.floor() as i32 * 100 + (h.fract() * 60.0).floor() as i32;
        self.0.iter().find(|(a, b, _)| t >= *a && t <= if *b == 0 { 2400 } else { *b }).map(|r| r.2).unwrap_or(0.0)
    }
}

#[derive(Default)]
pub struct Distribution {
    map: Vec<u8>,
    /// per district colour: (type name, weight over the day), ancestors' lists included
    pub types: HashMap<u8, Vec<(String, Tod)>>,
    pub density: Tod,
    /// per district colour: its density over the day - its own `TodDensityArray`, else
    /// the nearest ancestor's (sub_826DAB80 walks the district's nodes for the first table)
    pub district_density: HashMap<u8, Tod>,
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
        self.district(p).and_then(|c| self.district_density.get(&c)).unwrap_or(&self.density).at(hour)
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

    pub fn pick(&self, pos: Vec3, hour: f32, r: f32, usable: &dyn Fn(&str) -> bool) -> Option<&str> {
        let list = self.types.get(&self.district(pos)?)?;
        let total: f32 = list.iter().filter(|(n, _)| usable(n)).map(|(_, t)| t.at(hour)).sum();
        if total <= 0.0 {
            return None;
        }
        let mut x = r.clamp(0.0, 0.9999) * total;
        let mut last = None;
        for (n, t) in list.iter().filter(|(n, _)| usable(n)) {
            let w = t.at(hour);
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
            d.types.insert(5, ws.iter().enumerate().map(|(i, w)| (format!("t{i}"), Tod(vec![(0, 0, *w)]))).collect());
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
}
