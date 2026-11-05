//! Occlusion culling from the original's occluder file (`city/sc/city.occluder` of the cache).
//!
//! The file lists 5614 closed polyhedra ("Type box": mostly 8-vertex boxes, some prisms with
//! more corners) that stand inside the city's buildings. The rule below is the original's, read
//! from the recompiled executable (docs/MAP.md, "Occluder selection"):
//!
//! * at load every occluder gets a box (its vertices' bounds, F-89837) and a kd-tree is
//!   built over the boxes (F-62761 / F-33503: split the node's bounds at the middle of its
//!   longest axis, partition the boxes by centre, leaves of at most 2);
//! * per view, the tree is walked (F-58127 / F-50889) with the box eye +-400 m (y +-10400):
//!   the range is 0x82007A48 = 40.0 times the scale 0x827D2958, which the city sets to 10.0
//!   (F-20115). The first 128 occluders whose box overlaps it are the candidates of the main
//!   view (512 for the pedestrian spawner's view), in the walk's order: a node's child on the side
//!   of the query centre first. None at all when the camera is above y = 1000;
//! * a candidate is skipped when its bounding sphere is outside the view frustum or lies its
//!   radius behind every plane of a volume already taken (F-38575), when its sphere centre
//!   is more than 400 m from the camera in the ground plane, or when all its vertices are outside
//!   one frustum plane (F-62084); at most 64 volumes are kept per view;
//! * its volume is a set of at most 16 planes: the planes of the polygons that face the camera
//!   (`n . (eye - v) >= 1e-6`), moved 5 m into the occluder (0x82838724 = -5.0, also set by the
//!   city), and, for every edge toggled by an odd number of facing polygons, the plane through the
//!   camera and that edge. No facing polygon (camera inside) or a 17th plane: the occluder is not
//!   used (`Occluder::volume` has the exact rule, F-62084). Every occluder of the file is used so,
//!   convex or not (a polygon's plane is that of its first three vertices in the file's winding);
//! * something is hidden when it lies behind every plane of one volume.
//!
//! What is the port's own: an axis-aligned box is tested by its farthest corner per plane (the
//! original tests spheres and boxes the same way), the test keeps a 2 cm margin, and the frustum
//! is the port's own view's. The original's leaving out of silhouette edges that are outside the
//! frustum (at most 2 per volume) is not done; it only changes what is outside the view.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use glam::{DMat3, DVec3, Mat4, Vec3, Vec4};

use crate::render::Frustum;

/// Ground-plane distance from the camera to an occluder's sphere centre within which it is used,
/// before the scale (0x82007A48 = 40.0).
pub(crate) const RANGE: f32 = 40.0;
/// The range scale 0x827D2958: 1.0 in the image, set to 10.0 (from 0x82003A8C) by the city's
/// init F-20115; the running original reads 10.0.
pub(crate) const RANGE_SCALE: f32 = 10.0;
/// How far the facing planes are moved into the occluder in the main views (0x82838724 = -5.0,
/// set by F-20115 next to the scale; F-47419). The spawner's view does not move them.
const PUSH: f32 = 5.0;
/// Extra height of the tree query box above and below the range (0x8200733C).
const QUERY_Y: f32 = 10000.0;
/// Candidates of a main view (F-38732) and of the spawner's view (F-38127).
const MAIN_CANDIDATES: usize = 128;
const AROUND_CANDIDATES: usize = 512;
/// A node with fewer occluders than this is a leaf (F-89837 passes 2, plus one).
const LEAF_BELOW: usize = 3;
/// No occluders above this camera height (0x82002C64).
const MAX_CAM_Y: f32 = 1000.0;
/// Volumes per view (the view's list holds 64).
const MAX_VOLUMES: usize = 64;
/// Planes per volume.
const MAX_PLANES: usize = 16;
/// A polygon faces the camera when the camera is at least this far in front of its plane.
const FACING_EPS: f32 = 1.0e-6;
/// The w an occluder's vertices are tested with against the view's sides (F-40381): outside beyond it.
const VERTEX_W: f32 = 1.0;
/// Safety margin of the hidden test, metres (the port's own).
const MARGIN: f32 = 0.02;

/// A polygon of an occluder as the file has it: its vertices in order, the file's edge of each side
/// (side k runs from vertex k to vertex k + 1) and the sides whose edge runs the other way (F-45813).
struct Poly {
    verts: Vec<u16>,
    edges: Vec<u16>,
    reversed: u32,
}

struct Occluder {
    centre: Vec3,
    verts: Vec<Vec3>,
    /// the file's edges (vertex, vertex)
    edges: Vec<(u16, u16)>,
    polys: Vec<Poly>,
}

/// One occluder of the file as the original keeps it: bounding sphere, box from the vertices
/// (centre and half size, as F-89837 stores them) and its body (None for broken data only).
struct Entry {
    centre: Vec3,
    radius: f32,
    box_c: Vec3,
    box_e: Vec3,
    body: Option<u32>,
}

/// A kd-tree node (12 bytes in the original: child pointer | axis, then two floats).
#[derive(Clone, Copy)]
enum Node {
    /// `lo` = smallest box minimum of the right child on `axis`, `hi` = largest box maximum of the
    /// left child; the children are `left` and `left + 1`.
    Inner { axis: u8, lo: f32, hi: f32, left: u32 },
    /// `order[start .. start + count]`
    Leaf { start: u32, count: u32 },
}

/// The occluders of the city, loaded once.
pub struct Occluders {
    list: Vec<Occluder>,
    entries: Vec<Entry>,
    nodes: Vec<Node>,
    /// occluder indices in the tree's leaf order
    order: Vec<u32>,
    /// polyhedra in the file, including any dropped as broken data
    pub in_file: usize,
}

/// The occlusion volumes of one view (one frame, one camera).
pub struct Selected {
    volumes: Vec<Vec<Vec4>>,
    /// the occluder (index in the file) of each volume
    ids: Vec<u32>,
}

struct Tokens<'a> {
    it: std::str::SplitAsciiWhitespace<'a>,
}

impl<'a> Tokens<'a> {
    fn next(&mut self) -> Result<&'a str, String> {
        self.it.next().ok_or_else(|| "unexpected end of file".to_string())
    }
    fn expect(&mut self, what: &str) -> Result<(), String> {
        let t = self.next()?;
        if t == what { Ok(()) } else { Err(format!("expected `{what}`, found `{t}`")) }
    }
    fn num<T: std::str::FromStr>(&mut self) -> Result<T, String> {
        let t = self.next()?;
        t.parse().map_err(|_| format!("bad number `{t}`"))
    }
    /// `<key> <count> {`
    fn counted(&mut self, key: &str) -> Result<usize, String> {
        self.expect(key)?;
        let n = self.num()?;
        self.expect("{")?;
        Ok(n)
    }
}

impl Occluders {
    /// Parse the text of an occluder file.
    ///
    /// ```text
    /// version: 4
    /// Occluders <n>
    /// {
    ///     Occluder <i>
    ///     {
    ///         Type box
    ///         Name <name>
    ///         BoundingSphere <x> <y> <z> <r>
    ///         Vertices <nv> { <x> <y> <z> ... }
    ///         Edges <ne> { <a> <b> ... }
    ///         Polygons <np> { Vertices <k> { <vertex> ... } Edges <k> { <edge> ... } ... }
    ///     }
    ///     ...
    /// }
    /// ```
    pub fn parse(text: &str) -> Result<Occluders, String> {
        let mut t = Tokens { it: text.split_ascii_whitespace() };
        t.expect("version:")?;
        let version: u32 = t.num()?;
        if version != 4 {
            return Err(format!("occluder file version {version}, expected 4"));
        }
        let count = t.counted("Occluders")?;
        let mut list = Vec::with_capacity(count);
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            t.expect("Occluder")?;
            let _index: u32 = t.num()?;
            t.expect("{")?;
            t.expect("Type")?;
            let kind = t.next()?;
            t.expect("Name")?;
            let _name = t.next()?;
            t.expect("BoundingSphere")?;
            let centre = Vec3::new(t.num()?, t.num()?, t.num()?);
            let radius: f32 = t.num()?;
            let nv = t.counted("Vertices")?;
            let mut verts = Vec::with_capacity(nv);
            for _ in 0..nv {
                verts.push(Vec3::new(t.num()?, t.num()?, t.num()?));
            }
            t.expect("}")?;
            let ne = t.counted("Edges")?;
            let mut edges = Vec::with_capacity(ne);
            for _ in 0..ne {
                let (a, b): (u16, u16) = (t.num()?, t.num()?);
                if a as usize >= nv || b as usize >= nv {
                    return Err(format!("edge vertex {a} / {b} out of range"));
                }
                edges.push((a, b));
            }
            t.expect("}")?;
            let np = t.counted("Polygons")?;
            let mut polys: Vec<(Vec<u16>, Vec<u16>)> = Vec::with_capacity(np);
            for _ in 0..np {
                let k = t.counted("Vertices")?;
                let mut p = Vec::with_capacity(k);
                for _ in 0..k {
                    let v: u16 = t.num()?;
                    if v as usize >= nv {
                        return Err(format!("polygon vertex {v} out of range"));
                    }
                    p.push(v);
                }
                t.expect("}")?;
                let k = t.counted("Edges")?;
                let mut pe = Vec::with_capacity(k);
                for _ in 0..k {
                    let e: u16 = t.num()?;
                    if e as usize >= ne {
                        return Err(format!("polygon edge {e} out of range"));
                    }
                    pe.push(e);
                }
                t.expect("}")?;
                polys.push((p, pe));
            }
            t.expect("}")?;
            t.expect("}")?;
            // the box F-89837 makes: bounds of the vertices, centre and half size
            let mut mn = Vec3::splat(f32::MAX);
            let mut mx = Vec3::splat(-f32::MAX);
            for v in &verts {
                mn = mn.min(*v);
                mx = mx.max(*v);
            }
            let mut entry = Entry { centre, radius, box_c: (mx + mn) * 0.5, box_e: (mx - mn) * 0.5, body: None };
            if kind == "box" {
                if let Some(o) = Occluder::build(centre, verts, edges, polys) {
                    entry.body = Some(list.len() as u32);
                    list.push(o);
                }
            }
            entries.push(entry);
        }
        t.expect("}")?;
        let (nodes, order) = build_tree(&entries);
        Ok(Occluders { list, entries, nodes, order, in_file: count })
    }

    pub fn load(data_dir: &str) -> Option<Occluders> {
        let path = std::path::Path::new(data_dir).join("city/sc/city.occluder");
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("occluders: {}: {e}", path.display());
                return None;
            }
        };
        match Occluders::parse(&text) {
            Ok(o) => {
                log::info!("occluders: {} of {} usable", o.list.len(), o.in_file);
                Some(o)
            }
            Err(e) => {
                log::warn!("occluders: {}: {e}", path.display());
                None
            }
        }
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// The occlusion volumes for the camera of `view_proj` (a perspective view).
    pub fn select(&self, view_proj: Mat4, frustum: &Frustum) -> Selected {
        match eye_of(view_proj) {
            Some(eye) => self.select_at(eye, frustum),
            None => Selected::none(),
        }
    }

    /// The occlusion volumes for a camera at `eye` (F-38732).
    pub fn select_at(&self, eye: Vec3, frustum: &Frustum) -> Selected {
        let mut sel = Selected::none();
        if !(eye.y <= MAX_CAM_Y) || !eye.is_finite() {
            return sel;
        }
        let range = RANGE * RANGE_SCALE;
        for i in self.query(eye, range, MAIN_CANDIDATES) {
            if sel.volumes.len() >= MAX_VOLUMES {
                break;
            }
            let e = &self.entries[i as usize];
            // F-38575: outside the frustum, or hidden by a volume already taken
            if !frustum.intersects_sphere(e.centre, e.radius) || sel.hides_sphere(e.centre, e.radius) {
                continue;
            }
            let Some(b) = e.body else { continue };
            let o = &self.list[b as usize];
            // F-62084
            // (F-40381: a vertex is outside a side when it lies more than its w, 1.0, beyond it - F-19231 tests
            // the plane's distance against the vertex's w lane, which F-40381 sets to 1.0)
            if !o.in_range(eye, range) || frustum.points_outside(&o.verts, VERTEX_W) {
                continue;
            }
            if let Some(v) = o.volume(eye, PUSH) {
                sel.volumes.push(v);
                sel.ids.push(i);
            }
        }
        sel
    }

    /// The volumes of a view without a frustum (the pedestrian spawner's view 2, built in
    /// F-75502 by F-38127 / F-62548): the tree's first 512 candidates within `range`,
    /// at most 64 volumes, the facing planes not moved.
    pub fn select_around(&self, eye: Vec3, range: f32) -> Selected {
        let mut sel = Selected::none();
        if !(eye.y <= MAX_CAM_Y) || !eye.is_finite() {
            return sel;
        }
        for i in self.query(eye, range, AROUND_CANDIDATES) {
            if sel.volumes.len() >= MAX_VOLUMES {
                break;
            }
            let Some(b) = self.entries[i as usize].body else { continue };
            let o = &self.list[b as usize];
            if !o.in_range(eye, range) {
                continue;
            }
            if let Some(v) = o.volume(eye, 0.0) {
                sel.volumes.push(v);
                sel.ids.push(i);
            }
        }
        sel
    }

    /// The occluders whose box overlaps the box `eye` +- `range` (y +- `range` + 10000), in the
    /// order of the original's tree walk (F-58127 / F-50889 with the callback
    /// F-35737), at most `cap`.
    fn query(&self, eye: Vec3, range: f32, cap: usize) -> Vec<u32> {
        let mut out = Vec::new();
        if self.nodes.is_empty() || cap == 0 {
            return out;
        }
        let mut qmin = eye - Vec3::splat(range);
        let mut qmax = eye + Vec3::splat(range);
        qmin.y -= QUERY_Y;
        qmax.y += QUERY_Y;
        let q = (qmin + qmax) * 0.5;
        let mut stack: Vec<u32> = Vec::new();
        let mut node = 0u32;
        loop {
            match self.nodes[node as usize] {
                Node::Leaf { start, count } => {
                    for &i in &self.order[start as usize..(start + count) as usize] {
                        let e = &self.entries[i as usize];
                        let (bmin, bmax) = (e.box_c - e.box_e, e.box_c + e.box_e);
                        if !(qmin.cmpgt(bmax).any() || bmin.cmpgt(qmax).any()) {
                            out.push(i);
                        }
                        if out.len() >= cap {
                            return out;
                        }
                    }
                    match stack.pop() {
                        Some(n) => node = n,
                        None => return out,
                    }
                }
                Node::Inner { axis, lo, hi, left } => {
                    let a = axis as usize;
                    let need_left = qmin[a] <= hi;
                    let need_right = qmax[a] > lo;
                    // the child on the query centre's side first; one of the two is entered
                    // even when neither is needed, as in the original
                    let left_first = need_left && q[a] <= (lo + hi) * 0.5;
                    let (first, second) = if left_first { (left, left + 1) } else { (left + 1, left) };
                    if need_left && need_right {
                        stack.push(second);
                    }
                    node = first;
                }
            }
        }
    }
}

/// The original's kd-tree over the occluder boxes (F-38922 / F-33503); returns the nodes
/// (root first) and the occluder indices in leaf order.
fn build_tree(entries: &[Entry]) -> (Vec<Node>, Vec<u32>) {
    let mut order: Vec<u32> = (0..entries.len() as u32).collect();
    if entries.is_empty() {
        return (Vec::new(), order);
    }
    let bmin_of = |i: u32| entries[i as usize].box_c - entries[i as usize].box_e;
    let bmax_of = |i: u32| entries[i as usize].box_c + entries[i as usize].box_e;
    // bounds of the boxes of a range (F-99954)
    let bounds = |order: &[u32]| {
        let (mut mn, mut mx) = (Vec3::splat(f32::MAX), Vec3::splat(-f32::MAX));
        for &i in order {
            mn = mn.min(bmin_of(i));
            mx = mx.max(bmax_of(i));
        }
        (mn, mx)
    };
    // the longest axis; ties go to the later one
    let axis_of = |mn: Vec3, mx: Vec3| {
        let d = mx - mn;
        if d.x > d.y {
            if d.x > d.z { 0 } else { 2 }
        } else if d.y > d.z {
            1
        } else {
            2
        }
    };
    // std::partition by box centre < split (F-64234 with F-50788)
    let partition = |order: &mut [u32], axis: usize, split: f32| {
        let below = |i: u32| ((bmin_of(i) + bmax_of(i)) * 0.5)[axis] < split;
        let (mut f, mut l) = (0usize, order.len());
        loop {
            if f == l {
                return f;
            }
            if below(order[f]) {
                f += 1;
                continue;
            }
            loop {
                l -= 1;
                if f == l {
                    return f;
                }
                if below(order[l]) {
                    break;
                }
            }
            order.swap(f, l);
            f += 1;
        }
    };
    let mut nodes = vec![Node::Leaf { start: 0, count: 0 }];
    let (mn, mx) = bounds(&order);
    // (node, begin, end, bounds min, bounds max); the left child first, the right one waits
    let mut stack: Vec<(usize, usize, usize, Vec3, Vec3)> = Vec::new();
    let mut cur = (0usize, 0usize, order.len(), mn, mx);
    loop {
        let (node, begin, end, mut mn, mut mx) = cur;
        if end - begin < LEAF_BELOW {
            nodes[node] = Node::Leaf { start: begin as u32, count: (end - begin) as u32 };
            match stack.pop() {
                Some(c) => {
                    cur = c;
                    continue;
                }
                None => break,
            }
        }
        let mut axis = axis_of(mn, mx);
        let mut split = (mn[axis] + mx[axis]) * 0.5;
        let mut mid = begin + partition(&mut order[begin..end], axis, split);
        if mid == begin || mid == end {
            // all on one side: split the boxes' own bounds instead
            (mn, mx) = bounds(&order[begin..end]);
            axis = axis_of(mn, mx);
            split = (mn[axis] + mx[axis]) * 0.5;
            mid = begin + partition(&mut order[begin..end], axis, split);
        }
        if mid == begin {
            mid += 1;
        } else if mid == end {
            mid -= 1;
        }
        // F-78996 / F-24677: the left boxes' largest maximum, the right boxes' smallest minimum
        let hi = order[begin..mid].iter().map(|&i| bmax_of(i)[axis]).fold(f32::NEG_INFINITY, |a, b| if a < b { b } else { a });
        let lo = order[mid..end].iter().map(|&i| bmin_of(i)[axis]).fold(f32::INFINITY, |a, b| if b < a { b } else { a });
        let left = nodes.len();
        nodes.push(Node::Leaf { start: 0, count: 0 });
        nodes.push(Node::Leaf { start: 0, count: 0 });
        nodes[node] = Node::Inner { axis: axis as u8, lo, hi, left: left as u32 };
        let (mut lmax, mut rmin) = (mx, mn);
        lmax[axis] = split;
        rmin[axis] = split;
        stack.push((left + 1, mid, end, rmin, mx));
        cur = (left, begin, mid, mn, lmax);
    }
    (nodes, order)
}

impl Occluder {
    /// The occluder as the original keeps it, whatever its shape (convex or not, closed or not): the
    /// sides' edge directions as F-45813 sets them at load. None only for a polygon of fewer than 3
    /// vertices or one whose edge list does not match its vertex list.
    fn build(centre: Vec3, verts: Vec<Vec3>, edges: Vec<(u16, u16)>, polys: Vec<(Vec<u16>, Vec<u16>)>) -> Option<Occluder> {
        if verts.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let mut out = Vec::with_capacity(polys.len());
        for (pv, pe) in polys {
            if pv.len() < 3 || pv.len() != pe.len() || pv.len() > 32 {
                return None;
            }
            // side k: from vertex k to the next; its edge stored the same way clears bit k, the other
            // way sets it, an edge of other vertices leaves it clear
            let mut reversed = 0u32;
            for k in 0..pv.len() {
                let (a, b) = (pv[k], pv[(k + 1) % pv.len()]);
                let e = edges[pe[k] as usize];
                if e == (a, b) {
                    reversed &= !(1 << k);
                } else if e == (b, a) {
                    reversed |= 1 << k;
                }
            }
            out.push(Poly { verts: pv, edges: pe, reversed });
        }
        Some(Occluder { centre, verts, edges, polys: out })
    }

    /// Sphere centre within `range` of the camera in the ground plane (F-62084 / F-62548).
    fn in_range(&self, eye: Vec3, range: f32) -> bool {
        let (dx, dz) = (self.centre.x - eye.x, self.centre.z - eye.z);
        !(dx * dx + dz * dz > range * range)
    }

    /// The plane through three points as F-78843 makes it: `n = normalize((b - a) x (c - a))`, the
    /// offset `n . a`; a zero normal stays zero. Returned in the port's form (`n . p + w`, w = -offset).
    fn plane(a: Vec3, b: Vec3, c: Vec3) -> Vec4 {
        let n = (b - a).cross(c - a);
        let l = n.length();
        let n = if l > 0.0 && l.is_finite() { n / l } else { Vec3::ZERO };
        n.extend(-n.dot(a))
    }

    /// The planes behind which nothing can be seen from `eye` (`n . p + w <= 0` behind), the original's
    /// volume of one occluder (F-62084):
    /// * a polygon's plane is that of its vertices 1, 0, 2 (F-88707); it faces the camera when
    ///   `n . (eye - vertex 2) >= 1e-6`; a facing polygon's plane is taken with its three vertices moved
    ///   `push` metres along `-n` (F-47419), and its sides' edges are toggled, the direction kept of the
    ///   polygon that toggled one on;
    /// * every edge toggled on (an odd number of facing polygons: the silhouette) adds the plane through
    ///   the camera, the edge's end and its start in the facing polygon's direction (F-12644); an edge
    ///   index of 32 or more has no bit (the shift gives 0) and is never one;
    /// * no facing polygon, or a silhouette plane due when 16 planes are taken: not used.
    /// Not done: the original leaves out up to 2 silhouette edges whose ends are both outside the same
    /// side of the view (clip codes of F-40381 / F-19231) - planes that only bound what is outside the view.
    fn volume(&self, eye: Vec3, push: f32) -> Option<Vec<Vec4>> {
        let mut planes = Vec::with_capacity(MAX_PLANES);
        let (mut on, mut back) = (0u32, 0u32);
        let bit = |e: u16| if e < 32 { 1u32 << e } else { 0 };
        for p in &self.polys {
            let v = |k: usize| self.verts[p.verts[k] as usize];
            let pl = Self::plane(v(1), v(0), v(2));
            if pl.truncate().dot(eye - v(2)) < FACING_EPS {
                continue;
            }
            let shift = pl.truncate() * -push;
            planes.push(if push != 0.0 { Self::plane(v(1) + shift, v(0) + shift, v(2) + shift) } else { pl });
            for (k, &e) in p.edges.iter().enumerate() {
                let b = bit(e);
                on ^= b;
                if on & b != 0 {
                    if p.reversed & (1 << k) != 0 {
                        back |= b;
                    } else {
                        back &= !b;
                    }
                }
            }
        }
        if planes.is_empty() {
            return None;
        }
        for (e, &(a, b)) in self.edges.iter().enumerate() {
            if on & bit(e as u16) == 0 {
                continue;
            }
            if planes.len() == MAX_PLANES {
                return None;
            }
            let (a, b) = (self.verts[a as usize], self.verts[b as usize]);
            planes.push(if back & bit(e as u16) != 0 { Self::plane(eye, a, b) } else { Self::plane(eye, b, a) });
        }
        Some(planes)
    }
}

impl Selected {
    pub fn none() -> Selected {
        Selected { volumes: Vec::new(), ids: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.volumes.len()
    }

    /// True when the box is entirely behind one occluder (never true for something visible).
    pub fn hides_aabb(&self, mn: Vec3, mx: Vec3) -> bool {
        'volume: for planes in &self.volumes {
            for p in planes {
                let v = Vec3::new(
                    if p.x >= 0.0 { mx.x } else { mn.x },
                    if p.y >= 0.0 { mx.y } else { mn.y },
                    if p.z >= 0.0 { mx.z } else { mn.z },
                );
                if !(p.truncate().dot(v) + p.w <= -MARGIN) {
                    continue 'volume;
                }
            }
            return true;
        }
        false
    }

    /// True when a sphere lies `r` or more behind every plane of one volume (F-38575,
    /// occluder part).
    pub fn hides_sphere(&self, c: Vec3, r: f32) -> bool {
        self.volumes.iter().any(|planes| planes.iter().all(|p| p.truncate().dot(c) + p.w <= -r))
    }

    /// True when all the boxes together are behind one occluder: the test for a sector, which
    /// the original hides or shows as a whole (its parts may be set back behind a wall's face,
    /// like shop windows, and must not be tested one by one).
    pub fn hides_all(&self, boxes: impl Iterator<Item = (Vec3, Vec3)>) -> bool {
        if self.volumes.is_empty() {
            return false;
        }
        let mut mn = Vec3::splat(f32::INFINITY);
        let mut mx = Vec3::splat(f32::NEG_INFINITY);
        for (a, b) in boxes {
            mn = mn.min(a);
            mx = mx.max(b);
        }
        mn.cmple(mx).all() && self.hides_aabb(mn, mx)
    }
}

/// Centre of projection of a perspective view-projection matrix: the point whose clip x, y and w
/// are all zero.
fn eye_of(m: Mat4) -> Option<Vec3> {
    let (r0, r1, r3) = (m.row(0).as_dvec4(), m.row(1).as_dvec4(), m.row(3).as_dvec4());
    let a = DMat3::from_cols(r0.truncate(), r1.truncate(), r3.truncate()).transpose();
    let det = a.determinant();
    let scale = r0.truncate().length() * r1.truncate().length() * r3.truncate().length();
    if !(det.abs() > 1.0e-9 * scale) {
        return None;
    }
    let e = a.inverse() * -DVec3::new(r0.w, r1.w, r3.w);
    e.is_finite().then(|| e.as_vec3())
}

/// The city's occluders, loaded on first use; None when the file is missing or
/// `MCLA_NO_OCCLUDE` is set.
pub fn city(data_dir: &str) -> Option<&'static Occluders> {
    static CITY: OnceLock<Option<Occluders>> = OnceLock::new();
    CITY.get_or_init(|| {
        if std::env::var("MCLA_NO_OCCLUDE").is_ok() {
            log::info!("occluders: off (MCLA_NO_OCCLUDE)");
            return None;
        }
        Occluders::load(data_dir)
    })
    .as_ref()
}

/// The volumes of this frame's main view, from the city's file (loaded on first use).
/// Empty when the file is missing or `MCLA_NO_OCCLUDE` is set.
pub fn frame(data_dir: &str, view_proj: Mat4, frustum: &Frustum) -> Selected {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let Some(city) = city(data_dir) else { return Selected::none() };
    let s = city.select(view_proj, frustum);
    if CALLS.fetch_add(1, Ordering::Relaxed) % 120 == 60 {
        log::info!("occluders: {} volumes in view", s.len());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_text(boxes: &[(Vec3, Vec3)]) -> String {
        let mut s = format!("version: 4\nOccluders {}\n{{\n", boxes.len());
        for (i, (a, b)) in boxes.iter().enumerate() {
            let c = (*a + *b) * 0.5;
            let r = (*b - *a).length() * 0.5;
            s += &format!("\tOccluder {i} \n\t{{\n\t\tType box\n\t\tName test{i}\n\t\tBoundingSphere {}\t{}\t{}\t{} \n", c.x, c.y, c.z, r);
            s += "\t\tVertices 8\n\t\t{\n";
            // the file's own corner order for a box (its second four corners lie towards -z of the first four:
            // the file's winding makes every polygon's plane F-78843 face out)
            for (x, y, z) in [(0, 0, 0), (1, 0, 0), (0, 1, 0), (1, 1, 0), (0, 1, 1), (1, 1, 1), (0, 0, 1), (1, 0, 1)] {
                let p = Vec3::new(if x == 0 { a.x } else { b.x }, if y == 0 { a.y } else { b.y }, if z == 0 { b.z } else { a.z });
                s += &format!("\t\t\t{}\t{}\t{} \n", p.x, p.y, p.z);
            }
            s += "\t\t}\n\t\tEdges 12\n\t\t{\n\t\t\t0 1\n\t\t\t2 3\n\t\t\t4 5\n\t\t\t6 7\n\t\t\t0 2\n\t\t\t1 3\n\t\t\t2 4\n\t\t\t3 5\n\t\t\t4 6\n\t\t\t5 7\n\t\t\t6 0\n\t\t\t7 1\n\t\t}\n";
            s += "\t\tPolygons 6\n\t\t{\n";
            for (v, e) in [("2 3 1 0", "1 5 0 4"), ("4 5 3 2", "2 7 1 6"), ("6 7 5 4", "3 9 2 8"), ("0 1 7 6", "0 11 3 10"), ("3 5 7 1", "7 9 11 5"), ("4 2 0 6", "6 4 10 8")] {
                s += &format!("\t\t\tVertices 4\n\t\t\t{{\n\t\t\t\t{v} \n\t\t\t}}\n\t\t\tEdges 4\n\t\t\t{{\n\t\t\t\t{e} \n\t\t\t}}\n");
            }
            s += "\t\t}\n\t}\n";
        }
        s += "}\n";
        s
    }

    fn view(eye: Vec3, at: Vec3) -> (Mat4, Frustum) {
        let vp = Mat4::perspective_rh(1.0, 16.0 / 9.0, 0.1, 2000.0) * Mat4::look_at_rh(eye, at, Vec3::Y);
        (vp, Frustum::from_view_proj(vp))
    }

    /// a wall 2 m thick, 10 m high, 10 m wide, 10 m in front of the camera along +x
    const WALL: (Vec3, Vec3) = (Vec3::new(10.0, 0.0, -5.0), Vec3::new(12.0, 10.0, 5.0));
    const EYE: Vec3 = Vec3::new(0.0, 2.0, 0.0);

    fn wall_view(eye: Vec3) -> Selected {
        let o = Occluders::parse(&box_text(&[WALL])).unwrap();
        assert_eq!(o.len(), 1);
        let (vp, f) = view(eye, eye + Vec3::X);
        o.select(vp, &f)
    }

    #[test]
    fn eye_from_matrix() {
        let eye = Vec3::new(712.5, 19.25, -803.0);
        let (vp, _) = view(eye, eye + Vec3::new(0.3, -0.1, 0.9));
        assert!(eye_of(vp).unwrap().distance(eye) < 1.0e-3);
        assert!(eye_of(Mat4::orthographic_rh(-1.0, 1.0, -1.0, 1.0, 0.1, 10.0)).is_none());
    }

    #[test]
    fn behind_is_hidden() {
        let s = wall_view(EYE);
        assert_eq!(s.len(), 1);
        assert!(s.hides_aabb(Vec3::new(20.0, 1.0, -1.0), Vec3::new(22.0, 3.0, 1.0)));
        // 5 m or more behind the front face (the face plane is moved 5 m in)
        assert!(s.hides_aabb(Vec3::new(15.1, 1.0, -1.0), Vec3::new(20.0, 3.0, 1.0)));
        assert!(!s.hides_aabb(Vec3::new(14.9, 1.0, -1.0), Vec3::new(20.0, 3.0, 1.0)));
        // far behind, wide, still inside the shadow
        assert!(s.hides_aabb(Vec3::new(100.0, 0.5, -40.0), Vec3::new(110.0, 30.0, 40.0)));
    }

    #[test]
    fn sticking_out_is_not_hidden() {
        let s = wall_view(EYE);
        // sideways, both ways
        assert!(!s.hides_aabb(Vec3::new(20.0, 1.0, 0.0), Vec3::new(22.0, 3.0, 30.0)));
        assert!(!s.hides_aabb(Vec3::new(20.0, 1.0, -30.0), Vec3::new(22.0, 3.0, 0.0)));
        // above
        assert!(!s.hides_aabb(Vec3::new(20.0, 1.0, -1.0), Vec3::new(22.0, 40.0, 1.0)));
        // below the wall's foot (the wall stands on y = 0, the camera is at y = 2)
        assert!(!s.hides_aabb(Vec3::new(20.0, -6.0, -1.0), Vec3::new(22.0, 3.0, 1.0)));
        // in front
        assert!(!s.hides_aabb(Vec3::new(5.0, 1.0, -1.0), Vec3::new(6.0, 3.0, 1.0)));
        // straddling the front face
        assert!(!s.hides_aabb(Vec3::new(9.0, 1.0, -1.0), Vec3::new(11.0, 3.0, 1.0)));
        // straddling the silhouette at the wall's edge
        assert!(!s.hides_aabb(Vec3::new(13.0, 1.0, 4.0), Vec3::new(14.0, 3.0, 8.0)));
        // just touching the back of the front face from outside
        assert!(!s.hides_aabb(Vec3::new(9.99, 1.0, -1.0), Vec3::new(30.0, 3.0, 1.0)));
        // behind the camera
        assert!(!s.hides_aabb(Vec3::new(-22.0, 1.0, -1.0), Vec3::new(-20.0, 3.0, 1.0)));
        // a sector: hidden only as a whole
        let back = (Vec3::new(20.0, 1.0, -1.0), Vec3::new(22.0, 3.0, 1.0));
        let front = (Vec3::new(9.5, 1.0, -1.0), Vec3::new(9.9, 3.0, 1.0));
        assert!(s.hides_all([back, (Vec3::new(16.0, 1.0, 1.0), Vec3::new(17.5, 2.0, 2.0))].into_iter()));
        assert!(!s.hides_all([back, front].into_iter()));
        assert!(!s.hides_all(std::iter::empty()));
    }

    #[test]
    fn camera_inside_or_far_selects_nothing() {
        let s = wall_view(Vec3::new(11.0, 2.0, 0.0));
        assert_eq!(s.len(), 0);
        assert!(!s.hides_aabb(Vec3::new(20.0, 1.0, -1.0), Vec3::new(22.0, 3.0, 1.0)));
        // more than 400 m from the occluder's centre
        let s = wall_view(Vec3::new(-380.0, 2.0, 0.0));
        assert_eq!(s.len(), 1);
        let s = wall_view(Vec3::new(-400.0, 2.0, 0.0));
        assert_eq!(s.len(), 0);
        // looking away: the occluder is outside the frustum
        let o = Occluders::parse(&box_text(&[WALL])).unwrap();
        let (vp, f) = view(EYE, EYE - Vec3::X);
        assert_eq!(o.select(vp, &f).len(), 0);
        // above the height limit
        let (_, f) = view(EYE, EYE + Vec3::X);
        assert_eq!(o.select_at(Vec3::new(0.0, 1500.0, 0.0), &f).len(), 0);
    }

    #[test]
    fn corner_view_uses_two_faces() {
        // seen over a corner: two facing sides, the shadow is still exact
        let eye = Vec3::new(0.0, 2.0, -12.0);
        let o = Occluders::parse(&box_text(&[WALL])).unwrap();
        let (vp, f) = view(eye, Vec3::new(11.0, 2.0, 0.0));
        let s = o.select(vp, &f);
        assert_eq!(s.len(), 1);
        assert_eq!(s.volumes[0].len(), 2 + 6);
        assert!(s.hides_aabb(Vec3::new(20.0, 1.0, 9.0), Vec3::new(21.0, 3.0, 10.0)));
        assert!(!s.hides_aabb(Vec3::new(20.0, 1.0, -9.0), Vec3::new(21.0, 3.0, -8.0)));
    }

    /// Does the segment eye -> p pass through the box?
    fn blocked(eye: Vec3, p: Vec3, b: (Vec3, Vec3)) -> bool {
        let d = p - eye;
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for k in 0..3 {
            if d[k].abs() < 1.0e-9 {
                if eye[k] < b.0[k] || eye[k] > b.1[k] {
                    return false;
                }
                continue;
            }
            let (a, c) = ((b.0[k] - eye[k]) / d[k], (b.1[k] - eye[k]) / d[k]);
            t0 = t0.max(a.min(c));
            t1 = t1.min(a.max(c));
        }
        t0 <= t1
    }

    #[test]
    fn hidden_boxes_are_really_hidden() {
        // random boxes against random cameras: whatever is reported hidden must have every
        // corner and many inner points behind the occluder as seen from the camera
        let mut seed = 0x1234_5678_u32;
        let mut rnd = move |lo: f32, hi: f32| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            lo + (hi - lo) * (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        let o = Occluders::parse(&box_text(&[WALL])).unwrap();
        let (mut hidden, mut total) = (0, 0);
        for _ in 0..400 {
            let eye = Vec3::new(rnd(-20.0, 30.0), rnd(0.5, 25.0), rnd(-30.0, 30.0));
            let (vp, f) = view(eye, Vec3::new(11.0, 5.0, 0.0));
            let s = o.select(vp, &f);
            for _ in 0..200 {
                let c = Vec3::new(rnd(-40.0, 60.0), rnd(-5.0, 30.0), rnd(-50.0, 50.0));
                let h = Vec3::new(rnd(0.1, 6.0), rnd(0.1, 6.0), rnd(0.1, 6.0));
                let (mn, mx) = (c - h, c + h);
                total += 1;
                if !s.hides_aabb(mn, mx) {
                    continue;
                }
                hidden += 1;
                for i in 0..64 {
                    let p = if i < 8 {
                        Vec3::new(if i & 1 == 0 { mn.x } else { mx.x }, if i & 2 == 0 { mn.y } else { mx.y }, if i & 4 == 0 { mn.z } else { mx.z })
                    } else {
                        Vec3::new(rnd(mn.x, mx.x), rnd(mn.y, mx.y), rnd(mn.z, mx.z))
                    };
                    assert!(blocked(eye, p, WALL), "eye {eye} box {mn} {mx} point {p}");
                }
            }
        }
        assert!(hidden > 200, "only {hidden} of {total} hidden: the test does not exercise the rule");
    }

    #[test]
    fn not_convex_is_used() {
        // a box with one corner pulled inwards so far that two faces are no longer planar: the
        // original builds its volume all the same (planes of each polygon's first three vertices)
        let text = box_text(&[WALL]).replace("12	10	5 ", "11	5	0 ");
        assert_ne!(text, box_text(&[WALL]));
        let o = Occluders::parse(&text).unwrap();
        assert_eq!((o.len(), o.in_file), (1, 1));
        let (vp, f) = view(EYE, EYE + Vec3::X);
        let s = o.select(vp, &f);
        assert_eq!(s.len(), 1);
        assert!(s.hides_aabb(Vec3::new(20.0, 1.0, -1.0), Vec3::new(22.0, 3.0, 1.0)));
    }

    #[test]
    fn edge_directions() {
        // the test box's side "2 3 1 0" with edges 1 5 0 4: 2-3 is edge 1 (2 3), 3-1 is edge 5 (1 3)
        // reversed, 1-0 is edge 0 (0 1) reversed, 0-2 is edge 4 (0 2)
        let o = Occluders::parse(&box_text(&[WALL])).unwrap();
        assert_eq!(o.list[0].polys[0].reversed, 0b0110);
    }

    #[test]
    fn city_file() {
        let Ok(text) = std::fs::read_to_string("D:/MCLA/extracted/cache/city/sc/city.occluder") else { return };
        let o = Occluders::parse(&text).unwrap();
        assert_eq!(o.in_file, 5614);
        println!("city.occluder: {} of {} usable", o.len(), o.in_file);
        assert!(o.len() > 5000);
        let range = RANGE * RANGE_SCALE;
        let most = o.list.iter().map(|a| o.list.iter().filter(|b| (b.centre.x - a.centre.x).hypot(b.centre.z - a.centre.z) <= range).count()).max().unwrap();
        println!("most occluder centres within {range} m of one: {most}");
        // the tree holds every occluder once, in leaves of at most 2
        let mut seen = o.order.clone();
        seen.sort();
        assert!(seen.iter().enumerate().all(|(k, &i)| k as u32 == i) && seen.len() == o.in_file);
        assert!(o.nodes.iter().all(|n| !matches!(n, Node::Leaf { count, .. } if *count > 2)));
        // the walk finds what overlaps (it may miss a box only in the original's corner case
        // of a gap between two children), in the walk's order
        let mut missed = 0;
        for (k, e) in o.entries.iter().enumerate().step_by(97) {
            let eye = e.centre + Vec3::new(13.0, 1.5, -7.0);
            let (qmin, qmax) = (eye - Vec3::new(range, range + QUERY_Y, range), eye + Vec3::new(range, range + QUERY_Y, range));
            let want: Vec<u32> = (0..o.entries.len() as u32)
                .filter(|&i| {
                    let b = &o.entries[i as usize];
                    !(qmin.cmpgt(b.box_c + b.box_e).any() || (b.box_c - b.box_e).cmpgt(qmax).any())
                })
                .collect();
            let mut got = o.query(eye, range, usize::MAX);
            assert!(got.iter().all(|i| want.contains(i)), "eye {k}");
            missed += want.len() - got.len();
            got.sort();
            got.dedup();
            assert_eq!(got.len(), o.query(eye, range, usize::MAX).len());
        }
        println!("tree: {} nodes, missed {missed}", o.nodes.len());
        // the same tree as the running original's (dumped from its memory by
        // work/occl10/livetree.py: preorder, "N axis lo hi" / "L items")
        let Ok(live) = std::fs::read_to_string("D:/MCLA/work/occl10/live_tree.txt") else { return };
        let mut mine = Vec::new();
        let mut stack = vec![0u32];
        while let Some(n) = stack.pop() {
            match o.nodes[n as usize] {
                Node::Inner { axis, lo, hi, left } => {
                    mine.push(format!("N {axis} {lo} {hi}"));
                    stack.push(left + 1);
                    stack.push(left);
                }
                Node::Leaf { start, count } => {
                    let items: Vec<String> = o.order[start as usize..(start + count) as usize].iter().map(|i| i.to_string()).collect();
                    mine.push(format!("L {}", items.join(" ")));
                }
            }
        }
        let live: Vec<&str> = live.lines().collect();
        assert_eq!(live.len(), mine.len());
        for (k, (a, b)) in live.iter().zip(&mine).enumerate() {
            let (pa, pb): (Vec<&str>, Vec<&str>) = (a.split(' ').collect(), b.split(' ').collect());
            let same = pa.len() == pb.len()
                && pa.iter().zip(&pb).all(|(x, y)| x == y || matches!((x.parse::<f32>(), y.parse::<f32>()), (Ok(u), Ok(v)) if u == v));
            assert!(same, "node {k}: original `{a}`, port `{b}`");
        }
        println!("tree identical to the original's ({} nodes)", live.len());
    }

    /// The volumes the running original chose (work/occl10/livecap.py: its main view 0 with the
    /// eye and frustum planes, and the spawner's view 2), against the port's choice.
    #[test]
    fn live_views() {
        let Ok(text) = std::fs::read_to_string("D:/MCLA/extracted/cache/city/sc/city.occluder") else { return };
        let Ok(live) = std::fs::read_to_string("D:/MCLA/work/occl10/live_views.txt") else { return };
        let o = Occluders::parse(&text).unwrap();
        for line in live.lines() {
            let parts: Vec<&str> = line.split(" | ").collect();
            let head: Vec<f32> = parts[0].split(' ').skip(1).map(|x| x.parse().unwrap()).collect();
            let pl: Vec<f32> = parts[1].split(' ').map(|x| x.parse().unwrap()).collect();
            let want: Vec<u32> = parts[2].split(' ').filter(|x| !x.is_empty()).map(|x| x.parse().unwrap()).collect();
            let eye = Vec3::new(head[1], head[2], head[3]);
            let sel = if head[0] == 0.0 {
                // planes 0, 1 sides, 2 near, 4, 5 top and bottom, 6 far; the original's are
                // positive outside (F-38575), the port's positive inside
                let planes = [0, 1, 2, 4, 5, 6].map(|j| -Vec4::new(pl[4 * j], pl[4 * j + 1], pl[4 * j + 2], pl[4 * j + 3]));
                o.select_at(eye, &Frustum::from_planes(planes))
            } else {
                o.select_around(eye, RANGE * RANGE_SCALE)
            };
            if head[0] == 0.0 {
                let t = std::time::Instant::now();
                let plane = |j: usize| -Vec4::new(pl[4 * j], pl[4 * j + 1], pl[4 * j + 2], pl[4 * j + 3]);
                let f = Frustum::from_planes([0, 1, 2, 4, 5, 6].map(plane));
                for _ in 0..200 {
                    std::hint::black_box(o.select_at(eye, &f));
                }
                println!("select_at: {:.1} us", t.elapsed().as_secs_f64() * 1e6 / 200.0);
            }
            let dropped: Vec<u32> = want.iter().copied().filter(|&i| o.entries[i as usize].body.is_none()).collect();
            println!("view {} at {eye}: original {} volumes, port {}; not convex in the port: {dropped:?}", head[0], want.len(), sel.ids.len());
            println!("  original {want:?}
  port     {:?}", sel.ids);
            assert!(dropped.is_empty());
            for &i in want.iter().filter(|i| !sel.ids.contains(i)) {
                let e = &o.entries[i as usize];
                let f = Frustum::from_planes([0, 1, 2, 4, 5, 6].map(|j| -Vec4::new(pl[4 * j], pl[4 * j + 1], pl[4 * j + 2], pl[4 * j + 3])));
                let b = &o.list[e.body.unwrap() as usize];
                let pre = Selected { volumes: sel.volumes[..sel.ids.iter().position(|&x| want[..want.iter().position(|&w| w == i).unwrap()].last() == Some(&x)).map_or(0, |k| k + 1)].to_vec(), ids: Vec::new() };
                println!("  missing {i}: in frustum {}, hidden by earlier {}, in range {}, outside {}, volume {:?}", f.intersects_sphere(e.centre, e.radius), pre.hides_sphere(e.centre, e.radius), b.in_range(eye, RANGE * RANGE_SCALE), f.points_outside(&b.verts, VERTEX_W), b.volume(eye, PUSH).map(|v| v.len()));
                for (k, v) in pre.volumes.iter().enumerate() {
                    let worst = v.iter().map(|p| p.truncate().dot(e.centre) + p.w).fold(f32::MIN, f32::max);
                    if worst <= -e.radius + 1.0 {
                        println!("    volume {} ({}): worst plane {worst} vs -r {}", k, sel.ids[k], -e.radius);
                    }
                }
            }
            assert_eq!(sel.ids, want);
        }
    }
}
