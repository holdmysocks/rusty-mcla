//! The night ambient map (class pvsSpace_AO of the original, loaded by sub_822C79A0): the colour
//! of the night fill light by place. `tune/city/sc/grid.aogrid` is text (`m_ext_Min x y z`,
//! `m_cellsize`, `m_numCells_x`, `m_numCells_z`); `ambientnight.raw` holds one palette index a
//! cell, row by row along x, rows along z; `ambientnight.act` 256 RGB entries.
//!
//! The original looks it up on the CPU, nearest cell, never as a texture (sub_82689EE8):
//! props take a quarter of it (stored at instance +148, uploaded as gPropNFLColor.rgb =
//! c / 255 x 1.5 m_NFLFade), traffic cars, pedestrians and drivers add map / 255 x 0.25 x
//! m_NFLFade to their ambient, plants carry it in their own records. work/map/render_rule.md.
use glam::Vec3;
use std::path::Path;

pub struct AoMap {
    pub min: Vec3,
    pub max: Vec3,
    pub cell: f32,
    pub nx: usize,
    pub nz: usize,
    raw: Vec<u8>,
    palette: Vec<u8>,
}

impl AoMap {
    /// `dir`: the folder of the three files (`<data>/tune/city/sc`).
    pub fn load(dir: &Path) -> Option<AoMap> {
        let text = std::fs::read_to_string(dir.join("grid.aogrid")).ok()?;
        let key = |k: &str| text.lines().find_map(|l| l.strip_prefix(k).map(|r| r.split_whitespace().filter_map(|v| v.parse::<f32>().ok()).collect::<Vec<f32>>()));
        let min = key("m_ext_Min ")?;
        let max = key("m_ext_Max ")?;
        let (cell, nx, nz) = (*key("m_cellsize ")?.first()?, *key("m_numCells_x ")?.first()? as usize, *key("m_numCells_z ")?.first()? as usize);
        let raw = std::fs::read(dir.join("ambientnight.raw")).ok()?;
        let palette = std::fs::read(dir.join("ambientnight.act")).ok()?;
        if min.len() < 3 || max.len() < 3 || cell <= 0.0 || raw.len() < nx * nz || palette.len() < 768 {
            return None;
        }
        Some(AoMap { min: Vec3::new(min[0], min[1], min[2]), max: Vec3::new(max[0], max[1], max[2]), cell, nx, nz, raw, palette })
    }

    /// The cell a place lies in, as F-36356 finds it: outside the map's rectangle (min / max, both edges
    /// inside) none; then `(p - min) x (1 / cell)` cut towards zero, and none beyond the cell counts.
    pub fn index(&self, x: f32, z: f32) -> Option<usize> {
        if x < self.min.x || z < self.min.z || x > self.max.x || z > self.max.z {
            return None;
        }
        let inv = 1.0 / self.cell;
        // (fctiwz: cut towards zero, a NaN gives the most negative int)
        let cut = |v: f32| if v.is_nan() { i32::MIN } else { v as i32 };
        let (ix, iz) = (cut((x - self.min.x) * inv), cut((z - self.min.z) * inv));
        if ix < 0 || ix as usize >= self.nx || iz < 0 || iz as usize >= self.nz {
            return None;
        }
        Some(iz as usize * self.nx + ix as usize)
    }

    /// The fill colour at a place: the cell's palette entry; black outside the map.
    pub fn color_at(&self, x: f32, z: f32) -> [u8; 3] {
        let Some(c) = self.index(x, z) else { return [0; 3] };
        let i = self.raw[c] as usize * 3;
        [self.palette[i], self.palette[i + 1], self.palette[i + 2]]
    }

    /// What a prop takes: a quarter of the map's value (as the original stores it), 0..1.
    pub fn prop_fill(&self, x: f32, z: f32) -> Vec3 {
        let c = self.color_at(x, z);
        Vec3::new((c[0] / 4) as f32, (c[1] / 4) as f32, (c[2] / 4) as f32) / 255.0
    }

    /// What a traffic car, pedestrian or driver adds to its ambient at full night: map / 255 x 0.25.
    pub fn body_fill(&self, x: f32, z: f32) -> Vec3 {
        let c = self.color_at(x, z);
        Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32) / 255.0 * 0.25
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The grid file's parse (F-39732: min, max, cell size and cell counts) against the object the original's own
    /// parse leaves, read from the running original by the object-dump recorder (docs/reference/map/objdump_signals.md):
    /// `AoMap::load` of the shipped grid gives the same bits for every field the port keeps. Skipped without the data.
    #[test]
    fn grid_parse_matches_the_original() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let Some(m) = AoMap::load(&std::path::Path::new(&root).join("tune/city/sc")) else { return };
        let w: Vec<u32> = include_str!("testdata/aogrid_object_ref.txt").lines().next().unwrap().split(' ').map(|h| u32::from_str_radix(h, 16).unwrap()).collect();
        let f = |i: usize| f32::from_bits(w[i]);
        assert_eq!([m.min.x.to_bits(), m.min.y.to_bits(), m.min.z.to_bits()], [w[0], w[1], w[2]]);
        assert_eq!([m.max.x.to_bits(), m.max.y.to_bits(), m.max.z.to_bits()], [w[4], w[5], w[6]]);
        assert_eq!(m.cell.to_bits(), w[10]);
        assert_eq!((m.nx as u32, m.nz as u32), (w[11], w[12]));
        assert_eq!((w[11], w[12]), (1024, 768));
        // size and total are derived: the original stores max - min and nx * nz
        assert_eq!(f(8), m.max.x - m.min.x);
        assert_eq!(f(9), m.max.z - m.min.z);
        assert_eq!(w[13], m.nx as u32 * m.nz as u32);
    }

    /// The cell lookup against the original's own code run offline (tools/map_aomap_reference.py): four grids
    /// (the city's and three odd ones whose 1 / cell is not exact), places inside, on the cell edges (+-1 ulp), on and
    /// just outside the rectangle.
    #[test]
    fn cell_index_matches_the_original() {
        let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let mut n = 0;
        for l in include_str!("testdata/map_aomap_ref.txt").lines() {
            let t: Vec<&str> = l.split_whitespace().collect();
            let m = AoMap {
                min: Vec3::new(f(t[0]), 0.0, f(t[1])),
                max: Vec3::new(f(t[2]), 0.0, f(t[3])),
                cell: f(t[4]),
                nx: t[5].parse().unwrap(),
                nz: t[6].parse().unwrap(),
                raw: Vec::new(),
                palette: Vec::new(),
            };
            let want: i64 = t[9].parse().unwrap();
            let got = m.index(f(t[7]), f(t[8])).map_or(-1, |i| i as i64);
            assert_eq!(got, want, "{l}");
            n += 1;
        }
        assert!(n > 8000, "{n}");
    }

    /// Every plant carries the map's colour at its place in its own record: the check that
    /// fixes the files' layout, row order and axes (95,192 of the 95,523; 330 lie off the map
    /// and are black there, one differs).
    #[test]
    fn plants_carry_the_maps_colour() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let root = Path::new(&root);
        let Some(map) = AoMap::load(&root.join("tune/city/sc")) else { return };
        assert_eq!((map.nx, map.nz, map.cell), (1024, 768, 6.25));
        let Ok(d) = std::fs::read(root.join("resources/prop/0x982ff5e0.r134")) else { return };
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u(12) as usize).min(data.len());
        let res = crate::rsc::Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data };
        let (mut all, mut same) = (0, 0);
        for t in super::super::props::parse_foliage(&res) {
            for i in &t.instances {
                all += 1;
                same += (map.color_at(i.pos.x, i.pos.z) == i.fill) as usize;
            }
        }
        assert_eq!(all, 95523);
        assert!(same >= 95000, "{same} of {all} plants carry the map's colour");
    }
}
