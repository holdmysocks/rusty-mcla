//! The map's roads, as the original draws them.
//!
//! The game's map (minimap, GPS map) is not built from the road network: it draws
//! five flat meshes of `resources/ui/meshes` in world coordinates, each in one colour
//! of `tune/ui` `HudMap` (`0x822365B8` loads them, `0x822371E0` draws them in this
//! order): `map_roads_small` (MinorRoadColor), `map_roads_main` (MainRoadColor),
//! `map_roads_large` (MajorRoadColor), `map_shortcuts` (ShortcutColor) and
//! `map_freeways` (FreewayColor). Their names take the city's prefix; the Complete
//! Edition's map with South Central is `sc`.

use glam::{Vec2, Vec3};
use hashbrown::HashMap;
use std::path::Path;

/// (mesh name, `HudMap` colour key, the colour if the tune lacks it)
pub const LAYERS: [(&str, &str, [u8; 4]); 5] = [
    ("map_roads_small", "MinorRoadColor", [24, 30, 33, 255]),
    ("map_roads_main", "MainRoadColor", [24, 30, 33, 255]),
    ("map_roads_large", "MajorRoadColor", [24, 30, 33, 255]),
    ("map_shortcuts", "ShortcutColor", [209, 250, 255, 190]),
    ("map_freeways", "FreewayColor", [0, 0, 0, 255]),
];

const CELL: f32 = 96.0;

#[derive(Default)]
pub struct Layer {
    /// triangles on the ground plane (x, z)
    pub tris: Vec<[Vec2; 3]>,
    /// the corners' heights (the meshes are drawn in world space as they are)
    pub ys: Vec<[f32; 3]>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

impl Layer {
    fn from_tris(tris: Vec<[Vec2; 3]>, ys: Vec<[f32; 3]>) -> Layer {
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, t) in tris.iter().enumerate() {
            let mn = t[0].min(t[1]).min(t[2]);
            let mx = t[0].max(t[1]).max(t[2]);
            let (x0, z0) = ((mn.x / CELL).floor() as i32, (mn.y / CELL).floor() as i32);
            let (x1, z1) = ((mx.x / CELL).floor() as i32, (mx.y / CELL).floor() as i32);
            // (a few freeway triangles are long; cap what one may spread over)
            if (x1 - x0 + 1) * (z1 - z0 + 1) > 400 {
                continue;
            }
            for x in x0..=x1 {
                for z in z0..=z1 {
                    grid.entry((x, z)).or_default().push(i as u32);
                }
            }
        }
        Layer { tris, ys, grid }
    }

    /// Triangles that may reach within `r` of `p` (each once).
    pub fn near(&self, p: Vec2, r: f32, out: &mut Vec<u32>) {
        self.near_cells(Self::cells(p, r), out);
    }

    /// The grid cells `near` reads for `p` and `r`: x0, z0, x1, z1 (D59: the same cells give the same list)
    pub fn cells(p: Vec2, r: f32) -> [i32; 4] {
        [((p.x - r) / CELL).floor() as i32, ((p.y - r) / CELL).floor() as i32, ((p.x + r) / CELL).floor() as i32, ((p.y + r) / CELL).floor() as i32]
    }

    /// The triangles of the cells `c` (`cells`), ascending, each once.
    pub fn near_cells(&self, c: [i32; 4], out: &mut Vec<u32>) {
        out.clear();
        let [x0, z0, x1, z1] = c;
        for x in x0..=x1 {
            for z in z0..=z1 {
                if let Some(v) = self.grid.get(&(x, z)) {
                    out.extend_from_slice(v);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
    }
}

pub struct RoadMap {
    pub layers: Vec<Layer>,
}

impl RoadMap {
    pub fn load(data_dir: &str) -> Option<RoadMap> {
        let dir = Path::new(data_dir).join("resources/ui/meshes");
        let mut layers = Vec::new();
        for (name, _, _) in LAYERS {
            let mut tris = Vec::new();
            let mut ys = Vec::new();
            // the Complete Edition's city first, the plain name after it
            let found = ["sc", ""].iter().find_map(|pre| {
                let full = format!("{pre}{name}");
                let plain = dir.join(format!("{full}.r102"));
                let hashed = dir.join(format!("0x{:08x}.r102", mcla_formats::hash::joaat(&format!("{full}.xdr"))));
                [plain, hashed].into_iter().find(|p| p.exists())
            });
            if let Some(path) = found {
                if let Ok(res) = crate::assets::load_resource_file(&path) {
                    if let Ok(d) = mcla_formats::rage::scan::Drawable::parse(&res) {
                        for model in &d.models {
                            for (mesh, _) in &model.parts {
                                let at = |i: u32| mesh.positions.get(i as usize).map(|p: &Vec3| Vec2::new(p.x, p.z));
                                for t in mesh.indices.chunks_exact(3) {
                                    if let (Some(a), Some(b), Some(c)) = (at(t[0]), at(t[1]), at(t[2])) {
                                        tris.push([a, b, c]);
                                        let y = |i: u32| mesh.positions[i as usize].y;
                                        ys.push([y(t[0]), y(t[1]), y(t[2])]);
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                log::warn!("map: road mesh {} not found", name);
            }
            layers.push(Layer::from_tris(tris, ys));
        }
        let n: usize = layers.iter().map(|l| l.tris.len()).sum();
        log::info!("map: {} road triangles in {} layers", n, layers.len());
        if n == 0 { None } else { Some(RoadMap { layers }) }
    }
}

/// Clip a convex polygon to a disc, the disc taken as a polygon of `sides` sides.
pub fn clip_to_disc(poly: &mut Vec<egui::Pos2>, centre: egui::Pos2, radius: f32, sides: usize) {
    let mut tmp: Vec<egui::Pos2> = Vec::with_capacity(poly.len() + 4);
    for k in 0..sides {
        if poly.len() < 3 {
            poly.clear();
            return;
        }
        // the half plane inside this side: n . (p - centre) <= d
        let a = (k as f32 + 0.5) / sides as f32 * std::f32::consts::TAU;
        let n = egui::vec2(a.cos(), a.sin());
        let d = radius * (std::f32::consts::PI / sides as f32).cos();
        let side = |p: egui::Pos2| (p - centre).dot(n) - d;
        if poly.iter().all(|p| side(*p) <= 0.0) {
            continue;
        }
        tmp.clear();
        for i in 0..poly.len() {
            let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
            let (sp, sq) = (side(p), side(q));
            if sp <= 0.0 {
                tmp.push(p);
            }
            if (sp <= 0.0) != (sq <= 0.0) {
                let t = sp / (sp - sq);
                tmp.push(p + (q - p) * t);
            }
        }
        std::mem::swap(poly, &mut tmp);
    }
    if poly.len() < 3 {
        poly.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangles_are_found_by_place() {
        let l = Layer::from_tris(vec![[Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(0.0, 10.0)], [Vec2::new(900.0, 900.0), Vec2::new(910.0, 900.0), Vec2::new(900.0, 910.0)]], vec![[0.0; 3]; 2]);
        let mut out = Vec::new();
        l.near(Vec2::new(5.0, 5.0), 50.0, &mut out);
        assert_eq!(out, vec![0]);
        l.near(Vec2::new(905.0, 905.0), 50.0, &mut out);
        assert_eq!(out, vec![1]);
        l.near(Vec2::new(450.0, 450.0), 2000.0, &mut out);
        assert_eq!(out, vec![0, 1]);
    }

    /// D59: the minimap keeps a layer's list while the cells it reads stay the same (minimap_gpu `NEAR`) - two places
    /// in the same cells give the same list, another cell another
    #[test]
    fn the_same_cells_give_the_same_list() {
        let tris: Vec<[Vec2; 3]> = (0..400).map(|i| {
            let o = Vec2::new((i % 20) as f32 * 37.0 - 300.0, (i / 20) as f32 * 41.0 - 350.0);
            [o, o + Vec2::new(30.0, 5.0), o + Vec2::new(4.0, 33.0)]
        }).collect();
        let l = Layer::from_tris(tris, vec![[0.0; 3]; 400]);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for k in 0..200 {
            let p = Vec2::new((k * 13 % 97) as f32 * 6.1 - 290.0, (k * 7 % 89) as f32 * 7.3 - 320.0);
            let r = 160.0 + (k % 5) as f32 * 20.0;
            l.near(p, r, &mut a);
            l.near_cells(Layer::cells(p, r), &mut b);
            assert_eq!(a, b);
            let q = p + Vec2::splat(0.25);
            if Layer::cells(q, r) == Layer::cells(p, r) {
                l.near(q, r, &mut b);
                assert_eq!(a, b);
            }
        }
    }

    #[test]
    fn a_square_across_the_rim_is_cut_to_the_disc() {
        let c = egui::pos2(0.0, 0.0);
        let mut inside = vec![egui::pos2(-1.0, -1.0), egui::pos2(1.0, -1.0), egui::pos2(1.0, 1.0), egui::pos2(-1.0, 1.0)];
        clip_to_disc(&mut inside, c, 10.0, 48);
        assert_eq!(inside.len(), 4, "untouched well inside");
        let mut across = vec![egui::pos2(8.0, -1.0), egui::pos2(14.0, -1.0), egui::pos2(14.0, 1.0), egui::pos2(8.0, 1.0)];
        clip_to_disc(&mut across, c, 10.0, 48);
        assert!(across.len() >= 3);
        assert!(across.iter().all(|p| p.to_vec2().length() <= 10.01), "{across:?}");
        let mut outside = vec![egui::pos2(20.0, 0.0), egui::pos2(22.0, 0.0), egui::pos2(21.0, 2.0)];
        clip_to_disc(&mut outside, c, 10.0, 48);
        assert!(outside.is_empty());
    }

    /// The disc's road meshes: all five layers load and cover the city.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc"]
    fn the_discs_road_meshes_load() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let m = RoadMap::load(&dir).expect("road meshes");
        assert_eq!(m.layers.len(), 5);
        for (l, (name, _, _)) in m.layers.iter().zip(LAYERS) {
            assert!(l.tris.len() > 1000, "{name}: {}", l.tris.len());
        }
    }
}
