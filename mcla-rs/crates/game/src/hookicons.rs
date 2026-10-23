//! The free-roam 3D icons: the original's `3dicons` movie (`resources/ui/3dicons/0xadc95ef7.r27`)
//! standing over the three nearest cruising hookmen / garages / hangouts / service stations.
//! Decode: `docs/reference/ui/racer_marker.md` section 6; the functions are the icon manager's
//! (`mcUIClass`, vtable 0x82025BDC), the same object as the race markers (racemark.rs):
//!
//! - `sub_82225920`: the candidate list - the racers of icon type 1..9 or 12+, then the HUD map's
//!   elements by type (garage, hangouts, story places, service stations, deliveries, paybacks);
//! - `sub_822252D0`: the three nearest to the camera; `sub_82226FB0`: slots kept by id;
//! - `sub_82225E80` / `sub_82224E68`: each slot's HUD coordinate frame 10 + slot (the movie's meshes
//!   are placed through it: `coord` = 10 + i, `sub_82208058`);
//! - `sub_822269B0` / `sub_82225F70` / `sub_82225428`: the gates and the movie's `_global`s;
//! - `sub_822260A8` / `sub_82225170` -> `sub_82721258(movie, 6)`: the movie's 3D scene drawn in the
//!   world (its `view: "world"` meshes), the stage rendered into the movie's own 256x256 target
//!   being the texture of `player0mesh` (`iconmesh`).
//!
//! The movie's own ActionScript (fade in under 180 m, out over 190 m, the flashing, the ring and
//! arrow by category, the colour) runs in the port's Flash player unchanged.

use crate::hudmap::{Element, Kind};
use crate::racemark::MarkTune;
use glam::{Mat4, Vec3};
use hashbrown::HashMap;
use mcla_flash::draw::TexRef;
use mcla_flash::raster::TexImage;
use mcla_flash::value::Value;
use std::sync::Arc;

/// The movie (one copy per slot: `3DICONSMOVIE0..2`).
pub const MOVIE: &str = "resources/ui/3dicons/0xadc95ef7.r27";
/// The movies' render targets (`[0x8286D7AC + 4 i]`, read live: 256 x 256).
pub const RT: u32 = 256;
/// What the icon's frame stands above a candidate (the constant at 0x827DBDB0).
pub const LIFT: Vec3 = Vec3::new(0.0, 2.0, 0.0);

/// One entry of the candidate list (80 bytes at manager +352, count +5472).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    /// +0: the movie's `icon_type` (the racer's icon type, or by the element's type)
    pub kind: i32,
    /// +16: where it stands (the racer's car matrix, the element's position)
    pub pos: Vec3,
    /// +32: the colour (the racer's `+96`, the element's, GarageColor / GasStationColor)
    pub colour: [f32; 4],
    /// +64: the id the slots are kept by (`racer+140`, the element's UID)
    pub id: u32,
    /// from the racer list (`+48` the racer's index; kind 3 is fed through `sub_82225F70`)
    pub racer: bool,
    /// +56: selected (the GPS target) - `icon_ripple`
    pub ripple: bool,
    /// +60: `icon_category` (1: the square ring and the first arrow, 0: the round ring and the second)
    pub category: i32,
    /// +68: `racer+466` / the element's mission flag - `icon_subtype` 1 (the icon flashes)
    pub flash: bool,
}

/// `sub_82225920`: the racers first, then the HUD map's elements by type (`0x82225B8C`):
/// type 1 subtype 8 (payback target car) -> 10, 6 garage -> 9 in GarageColor, 7 local hangout ->
/// 16, 8 wager hangout -> 6, 9 story place -> 1, 11 pink-slip hangout -> 13, 12 service station ->
/// 14 in GasStationColor, 13 delivery -> 11, 14 payback -> 10 (the last two single player only).
/// Racers of icon type 0, 10 or 11 are not taken (the delivery car, icon 11, comes in as its
/// element).
pub fn candidates(els: &[Element], garage_colour: [f32; 4], gas_colour: [f32; 4]) -> Vec<Candidate> {
    let mut out = Vec::new();
    for e in els {
        if let Kind::Racer(_) = e.kind {
            let k = e.icon as i32;
            if k == 0 || k == 10 || k == 11 {
                continue;
            }
            out.push(Candidate { kind: k, pos: e.pos, colour: e.colour, id: e.uid, racer: true, ripple: e.selected, category: 0, flash: e.mission });
        }
    }
    for e in els {
        let (kind, category, colour) = match e.kind {
            Kind::PaybackCar => (10, 0, e.colour),
            Kind::Garage => (9, 1, garage_colour),
            Kind::Series => (16, 1, e.colour),
            Kind::Wager => (6, 1, e.colour),
            Kind::Story => (1, 0, e.colour),
            Kind::PinkSlip => (13, 1, e.colour),
            Kind::Gas => (14, 1, gas_colour),
            Kind::Delivery => (11, 0, e.colour),
            Kind::Payback => (10, 0, e.colour),
            _ => continue,
        };
        out.push(Candidate { kind, pos: e.pos, colour, id: e.uid, racer: false, ripple: e.selected, category, flash: e.mission });
    }
    out
}

/// `sub_822252D0`: the three nearest candidates to the camera (index, id), nearest first; a later
/// one at the same distance does not displace an earlier one; nothing beyond 100000 m.
pub fn nearest3(c: &[Candidate], cam: Vec3) -> [Option<(usize, u32)>; 3] {
    let mut out = [None; 3];
    let mut d = [100000.0f32; 3];
    for (i, k) in c.iter().enumerate() {
        if k.kind == 0 {
            continue;
        }
        let x = k.pos.distance(cam);
        let at = if x < d[0] {
            0
        } else if x < d[1] {
            1
        } else if x < d[2] {
            2
        } else {
            continue;
        };
        for j in (at + 1..3).rev() {
            out[j] = out[j - 1];
            d[j] = d[j - 1];
        }
        out[at] = Some((i, k.id));
        d[at] = x;
    }
    out
}

/// The three slots (manager +300 index, +312 id): a slot keeps its candidate while the candidate
/// is among the three nearest; the free slots take the nearest of the rest in order
/// (`sub_82226FB0` 0x82227060..0x82227190). Returns which slots changed hands (+336).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Slots {
    pub idx: [Option<usize>; 3],
    pub id: [Option<u32>; 3],
}

impl Slots {
    pub fn assign(&mut self, near: [Option<(usize, u32)>; 3]) -> [bool; 3] {
        let mut used = [false; 3];
        let mut kept = [false; 3];
        let mut changed = [false; 3];
        for s in 0..3 {
            let Some(id) = self.id[s] else { continue };
            if let Some(k) = (0..3).find(|&k| near[k].map_or(false, |n| n.1 == id)) {
                self.idx[s] = near[k].map(|n| n.0);
                used[k] = true;
                kept[s] = true;
            }
        }
        for s in 0..3 {
            if kept[s] {
                continue;
            }
            if let Some(k) = (0..3).find(|&k| !used[k]) {
                used[k] = true;
                self.idx[s] = near[k].map(|n| n.0);
                self.id[s] = near[k].map(|n| n.1);
                changed[s] = true;
            }
        }
        changed
    }
}

/// `sub_82224E68`: the icon's coordinate frame (HUD frame 10 + slot) for a candidate at `pos` seen
/// by a camera looking along `fwd` (its matrix's row +96, negated): rows HookmanIconScale x
/// (right, up, forward) with right = |fwd x Y|, up = |right x fwd| (the race markers' basis), the
/// translation `pos + LIFT + HookmanIconTranslatePivot` taken along the scaled rows. Checked against
/// the live frame (`work/hookicons/rec/icons.py`): rows (-1.2320, 0, -0.6649), (-0.0477, 1.3964,
/// 0.0884), (-0.6632, -0.1004, 1.2289), translation (844.8096, 21.7942, -819.5342) for the garage
/// at (844.728, 18.28, -819.383).
pub fn frame(t: &MarkTune, fwd: Vec3, pos: Vec3) -> Mat4 {
    let f = fwd.normalize_or_zero();
    let r = f.cross(Vec3::Y).normalize_or_zero();
    let u = r.cross(f).normalize_or_zero();
    let s = t.icon_scale;
    let (x, y, z) = (r * s, u * s, f * s);
    let p = pos + LIFT + x * t.icon_pivot.x + y * t.icon_pivot.y + z * t.icon_pivot.z;
    Mat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), p.extend(1.0))
}

/// `sub_821D2470` as the feeder uses it: the colour clamped to 0..1, times 255, truncated; the
/// movie gets the RGB word.
pub fn colour_word(c: [f32; 4]) -> i32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as i32;
    (b(c[0]) << 16) | (b(c[1]) << 8) | b(c[2])
}

/// What one slot hands its movie this frame (`sub_82225428`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Feed {
    pub icon_type: i32,
    pub icon_subtype: i32,
    pub icon_color: i32,
    pub icon_ripple: bool,
    pub icon_category: i32,
    pub distance: f32,
}

/// The gates of `sub_822269B0` and the feeder: kind 10 only while no race has checkpoints, kind
/// 14 always, every other kind not while a race runs (a police pursuit is one - `[0x8286EBF8]`);
/// kind 12 never. Kind 3 (a racer) is measured from its car, the rest from 2 m above the
/// candidate; at or beyond IconFadeoutDist nothing is fed (the fade `1 - d / 400` itself only
/// gates - its colour is not handed on). `icon_subtype` 2 (vehicle +113) was 0 on every car read
/// and is not modelled.
pub fn feed(t: &MarkTune, c: &Candidate, cam: Vec3, racing: bool, checkpoints: bool) -> Option<Feed> {
    match c.kind {
        10 if checkpoints => return None,
        10 | 14 => {}
        _ if racing => return None,
        12 => return None,
        _ => {}
    }
    let pos = if c.kind == 3 && c.racer { c.pos } else { c.pos + LIFT };
    let d = pos.distance(cam);
    if 1.0 - (d / t.icon_fadeout).min(1.0) == 0.0 {
        return None;
    }
    Some(Feed {
        icon_type: c.kind,
        icon_subtype: if c.flash { 1 } else { 0 },
        icon_color: colour_word(c.colour),
        icon_ripple: c.ripple,
        icon_category: if c.kind == 3 && c.racer { 0 } else { c.category },
        distance: d,
    })
}

/// A slot shown this frame: its frame, its distance from the camera (the queue's sort key),
/// the stage as the movie drew it, and the movie's meshes (in the movie's scene, before the frame).
pub struct Shown {
    pub slot: usize,
    pub frame: Mat4,
    pub dist: f32,
    pub rgba: Arc<Vec<u8>>,
    pub nodes: Vec<crate::menu3d::Node>,
}

/// The three movies and the slots.
pub struct HookIcons {
    movies: Vec<crate::flash_ui::FlashMovie>,
    pub slots: Slots,
    cache: HashMap<TexRef, Option<Arc<TexImage>>>,
    pub shown: Vec<Shown>,
}

impl HookIcons {
    pub fn new(data_dir: &str, shared: &crate::flash_ui::UiShared) -> Option<HookIcons> {
        let mut movies = Vec::new();
        for _ in 0..3 {
            let mut m = crate::flash_ui::FlashMovie::load(data_dir, MOVIE, shared)?;
            // (the first frame builds the scene: `_global.scenegraph`, the meshes, the defaults;
            // the second runs the per-frame script hidden, so `screen._alpha` starts from 0)
            m.player.tick();
            m.player.set_global("cur_visibility", Value::Int(0));
            m.player.tick();
            movies.push(m);
        }
        Some(HookIcons { movies, slots: Slots::default(), cache: HashMap::new(), shown: Vec::new() })
    }

    /// One frame: the candidates, the slots, each fed slot's movie advanced by `dt` and its scene
    /// read. Unfed movies are hidden and not advanced *(I: the HUD movies' update is taken to skip
    /// a movie whose visible byte +68 is clear; each frame `sub_82726FB0` clears it and sets
    /// `cur_visibility` 0, the feeder sets both back)*.
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, t: &MarkTune, els: &[Element], garage_colour: [f32; 4], gas_colour: [f32; 4], cam: Vec3, fwd: Vec3, racing: bool, checkpoints: bool, dt: f32) {
        self.shown.clear();
        let cands = candidates(els, garage_colour, gas_colour);
        let near = nearest3(&cands, cam);
        self.slots.assign(near);
        let log = std::env::var("MCLA_HOOKICON_LOG").is_ok();
        if std::env::var("MCLA_HOOKICON_LOG").map_or(false, |v| v == "all") {
            let f = fwd.normalize_or_zero();
            for c in &cands {
                let d = c.pos - cam;
                let ang = f.cross(Vec3::Y).dot(d).atan2(f.dot(d)).to_degrees();
                log::info!("hook icon candidate kind {} id {} at {:?}: {:.0} m, {:.0} deg right", c.kind, c.id, c.pos, d.length(), ang);
            }
        }
        if log {
            log::info!("hook icons: camera {:?} looking {:?}: {} candidates, nearest {:?}, slots {:?}, racing {} checkpoints {}", cam, fwd.normalize_or_zero(), cands.len(), near.map(|n| n.map(|(i, id)| (cands[i].kind, id, cands[i].pos.distance(cam) as i32))), self.slots.id, racing, checkpoints);
        }
        for s in 0..3 {
            let Some(c) = self.slots.idx[s].and_then(|i| cands.get(i)).copied() else { continue };
            let Some(f) = feed(t, &c, cam, racing, checkpoints) else { continue };
            if log {
                log::info!("hook icon slot {s}: id {} type {} subtype {} ripple {}", c.id, f.icon_type, f.icon_subtype, f.icon_ripple);
            }
            let m = &mut self.movies[s];
            let p = &mut m.player;
            p.set_global("icon_type", Value::Int(f.icon_type));
            p.set_global("icon_subtype", Value::Int(f.icon_subtype));
            p.set_global("icon_color", Value::Int(f.icon_color));
            p.set_global("icon_ripple", Value::Bool(f.icon_ripple));
            p.set_global("icon_category", Value::Int(f.icon_category));
            p.set_global("distance", Value::Float(f.distance));
            p.set_global("bomb_meter", Value::Float(0.0));
            p.set_global("cur_visibility", Value::Int(1));
            p.update(dt);
            let mut nodes = crate::menu3d::read_scene(p).map(|sc| sc.nodes).unwrap_or_default();
            // (`player0mesh` takes the movie as its texture whatever its useMovieAsTexture - 0x82724300)
            let p0 = p.get_path("player0mesh.file").map(|v| v.to_string().to_string()).unwrap_or_default();
            for n in nodes.iter_mut() {
                if n.file == p0 {
                    n.movie = true;
                }
            }
            nodes.retain(|n| n.view == "world" && n.alpha > 0.0);
            let rgba = m.raster(RT, RT, &mut self.cache);
            let fr = frame(t, fwd, c.pos);
            if log {
                log::info!("hook icon slot {}: kind {} id {} dist {:.1} feed {:?} frame t {:?} meshes {:?}", s, c.kind, c.id, f.distance, f, fr.w_axis.truncate(), nodes.iter().map(|n| (n.file.as_str(), n.alpha)).collect::<Vec<_>>());
            }
            self.shown.push(Shown { slot: s, frame: fr, dist: f.distance, rgba: Arc::new(rgba), nodes });
        }
        // (the queue is drawn far first - `sub_82225138`)
        self.shown.sort_by(|a, b| b.dist.total_cmp(&a.dist));
    }
}

/// The movie's mesh files (the names the port's mesh loader keys by).
fn mesh_name(file: &str) -> Option<&'static str> {
    let base = file.rsplit(['/', '\\']).next().unwrap_or(file);
    let base = base.split('.').next().unwrap_or(base);
    Some(match base {
        "iconmesh" => "iconmesh",
        "icon_arrow" => "icon_arrow",
        "icon_arrow2" => "icon_arrow2",
        "target_ripple_ingame" => "target_ripple_ingame",
        "ctf_bomb" => "ctf_bomb",
        "ctf_flag" => "ctf_flag",
        _ => return None,
    })
}

impl HookIcons {
    /// The shown slots' meshes as draws, far slots first, each slot's meshes in the scene's order.
    pub fn draws(&self) -> Vec<crate::hudmap_gpu::HookDraw> {
        let mut out = Vec::new();
        for sh in &self.shown {
            for n in &sh.nodes {
                let Some(mesh) = mesh_name(&n.file) else { continue };
                let c = n.color;
                out.push(crate::hudmap_gpu::HookDraw {
                    mesh,
                    world: sh.frame * n.srt,
                    colour: [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, n.alpha],
                    movie: n.movie.then(|| (sh.slot, sh.rgba.clone())),
                    ripple: mesh == "target_ripple_ingame",
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(kind: Kind, pos: [f32; 3], uid: u32) -> Element {
        let mut e = Element::at(kind, Vec3::from(pos));
        e.uid = uid;
        e
    }

    #[test]
    fn candidates_by_type_as_read_live() {
        // (the live list at the Hollywood garage: gas stations kind 14 in grey, the garage 9, the
        // delivery 11, local hangouts 16 in yellow, wager hangouts 6; categories 1 / 0)
        let grey = [0.501961, 0.501961, 0.501961, 1.0];
        let mut hang = el(Kind::Series, [729.046, 9.391, -285.351], 500);
        hang.colour = [1.0, 1.0, 0.0, 1.0];
        let mut man = el(Kind::Racer(3), [457.87, 19.87, -730.41], 2000);
        man.icon = 16;
        man.mission = true;
        let mut dcar = el(Kind::Racer(1), [0.0; 3], 2001);
        dcar.icon = 11;
        let els = vec![el(Kind::Gas, [516.6, 20.4, -751.6], 712), el(Kind::Garage, [844.728, 18.28, -819.383], 505), hang, el(Kind::Wager, [-146.544, 8.162, -469.806], 503), el(Kind::Delivery, [834.47, 18.06, -843.32], 504), man, dcar, el(Kind::Cop, [0.0; 3], 0)];
        let c = candidates(&els, [1.0; 4], grey);
        let got: Vec<(i32, u32, i32)> = c.iter().map(|k| (k.kind, k.id, k.category)).collect();
        assert_eq!(got, vec![(16, 2000, 0), (14, 712, 1), (9, 505, 1), (16, 500, 1), (6, 503, 1), (11, 504, 0)]);
        assert!(c[0].flash && c[0].racer);
        assert_eq!(colour_word(grey), 0x808080);
        assert_eq!(colour_word(c[3].colour), 0xFFFF00);
    }

    #[test]
    fn nearest_three_and_slots_as_read_live() {
        // camera (824.29, 19.83, -816.99): slots 505 (20.6 m), 504 (28.3 m), 712 (314.6 m)
        let mk = |kind, p: [f32; 3], id| Candidate { kind, pos: Vec3::from(p), colour: [1.0; 4], id, racer: false, ripple: false, category: 1, flash: false };
        let c = vec![mk(16, [457.87, 19.87, -730.41], 1021), mk(14, [241.0, 25.76, -918.4], 700), mk(14, [516.6, 20.4, -751.6], 712), mk(9, [844.73, 18.28, -819.38], 505), mk(11, [834.47, 18.06, -843.32], 504)];
        let cam = Vec3::new(824.29, 19.83, -816.99);
        let n = nearest3(&c, cam);
        assert_eq!(n, [Some((3, 505)), Some((4, 504)), Some((2, 712))]);
        let mut s = Slots::default();
        assert_eq!(s.assign(n), [true; 3]);
        assert_eq!(s.id, [Some(505), Some(504), Some(712)]);
        // the garage goes (the player drove in): its slot takes the next nearest, the others stay
        let n2 = [Some((4, 504)), Some((2, 712)), Some((0, 1021))];
        assert_eq!(s.assign(n2), [true, false, false]);
        assert_eq!(s.id, [Some(1021), Some(504), Some(712)]);
        assert_eq!(s.idx, [Some(0), Some(4), Some(2)]);
    }

    #[test]
    fn frame_as_read_live() {
        let t = MarkTune::default();
        // camera row +96 (0.474, 0.072, -0.878) at sample 0 of the garage read
        let fwd = -Vec3::new(0.4737, 0.0717, -0.8778);
        let m = frame(&t, fwd, Vec3::new(844.728, 18.28, -819.383));
        let row = |i: usize| m.col(i).truncate();
        assert!((row(0) - Vec3::new(-1.2320, 0.0, -0.6649)).length() < 2e-3, "{:?}", row(0));
        assert!((row(1) - Vec3::new(-0.0477, 1.3964, 0.0884)).length() < 2e-3, "{:?}", row(1));
        assert!((row(2) - Vec3::new(-0.6632, -0.1004, 1.2289)).length() < 2e-3, "{:?}", row(2));
        assert!((row(3) - Vec3::new(844.8096, 21.7942, -819.5342)).length() < 3e-3, "{:?}", row(3));
    }

    /// The movie in the port's player: the garage icon fed 25 m away for two seconds; prints the
    /// meshes and writes the stage (MCLA_HOOKICON_PNG).
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc"]
    fn movie_runs() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let strings = mcla_formats::strtbl::StringTable::parse(&std::fs::read(std::path::Path::new(&dir).join("ui/0xa4c1e8fa")).unwrap()).unwrap();
        let shared = crate::flash_ui::UiShared { textures: Arc::new(crate::flash_ui::load_shared(&dir)), strings: Arc::new(strings), logic: Arc::new(crate::menu3d::UiLogic::load(&dir)) };
        let mut hi = HookIcons::new(&dir, &shared).expect("movie");
        let t = MarkTune::default();
        let mut g = Element::at(Kind::Garage, Vec3::new(0.0, 0.0, 0.0));
        g.uid = 505;
        let cam = Vec3::new(0.0, 3.0, 25.0);
        let mut ring = Vec::new();
        for _ in 0..60 {
            hi.update(&t, &[g.clone()], [1.0; 4], [0.5; 4], cam, -cam, false, false, 1.0 / 30.0);
            // (the ring: a top segment's middle - it fades in from 0, then pulses with `ring_anim`)
            ring.push(hi.shown[0].rgba[(20 * RT as usize + 80) * 4]);
        }
        println!("ring over the first two seconds: {ring:?}");
        assert!(ring[0] < 40 && ring.iter().max() > Some(&240));
        let sh = &hi.shown[0];
        for n in &sh.nodes {
            println!("{} alpha {:.3} colour {:?} movie {} view {} coord {} srt {:?}", n.file, n.alpha, n.color, n.movie, n.view, n.coord, n.srt.to_cols_array());
        }
        let opaque = sh.rgba.chunks(4).filter(|p| p[3] > 0).count();
        println!("stage pixels drawn: {opaque}");
        assert!(opaque > 1000);
        if let Ok(out) = std::env::var("MCLA_HOOKICON_PNG") {
            let f = std::fs::File::create(out).unwrap();
            let mut e = png::Encoder::new(std::io::BufWriter::new(f), RT, RT);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            e.write_header().unwrap().write_image_data(&sh.rgba).unwrap();
        }
    }

    /// Out past 190 m the movie fades its stage to `_alpha` 0; coming back under 180 m it must fade
    /// in again (the port's player skips the frame scripts of an instance at alpha 0 - the stage's
    /// own script must still run).
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc"]
    fn fades_out_and_back_in() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let strings = mcla_formats::strtbl::StringTable::parse(&std::fs::read(std::path::Path::new(&dir).join("ui/0xa4c1e8fa")).unwrap()).unwrap();
        let shared = crate::flash_ui::UiShared { textures: Arc::new(crate::flash_ui::load_shared(&dir)), strings: Arc::new(strings), logic: Arc::new(crate::menu3d::UiLogic::load(&dir)) };
        let mut hi = HookIcons::new(&dir, &shared).expect("movie");
        let t = MarkTune::default();
        let mut g = Element::at(Kind::Garage, Vec3::ZERO);
        g.uid = 505;
        let drawn = |hi: &HookIcons| hi.shown.first().map_or(0, |s| s.rgba.chunks(4).filter(|p| p[3] > 0).count());
        let mut run = |hi: &mut HookIcons, d: f32, n: usize| {
            for _ in 0..n {
                hi.update(&t, &[g.clone()], [1.0; 4], [0.5; 4], Vec3::new(0.0, 2.0, d), Vec3::new(0.0, 0.0, -1.0), false, false, 1.0 / 30.0);
            }
        };
        run(&mut hi, 25.0, 60);
        let near = drawn(&hi);
        run(&mut hi, 300.0, 120);
        let far = drawn(&hi);
        run(&mut hi, 25.0, 60);
        let back = drawn(&hi);
        println!("drawn: near {near}, far {far}, back {back}");
        assert!(near > 1000 && far == 0 && back > 1000);
    }

    #[test]
    fn gates() {
        let t = MarkTune::default();
        let mk = |kind| Candidate { kind, pos: Vec3::ZERO, colour: [1.0; 4], id: 1, racer: false, ripple: false, category: 1, flash: false };
        let cam = Vec3::new(0.0, 0.0, 100.0);
        assert!(feed(&t, &mk(9), cam, false, false).is_some());
        assert!(feed(&t, &mk(9), cam, true, false).is_none());
        assert!(feed(&t, &mk(14), cam, true, true).is_some());
        assert!(feed(&t, &mk(10), cam, true, false).is_some());
        assert!(feed(&t, &mk(10), cam, false, true).is_none());
        assert!(feed(&t, &mk(12), cam, false, false).is_none());
        // 400 m from the lifted point: nothing
        assert!(feed(&t, &mk(9), Vec3::new(0.0, 2.0, 400.0), false, false).is_none());
        assert!(feed(&t, &mk(9), Vec3::new(0.0, 2.0, 399.0), false, false).is_some());
    }
}
