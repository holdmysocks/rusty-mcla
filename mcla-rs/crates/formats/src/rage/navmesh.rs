//! The AI navigation mesh of the city, `resources/city/sc/0x69d17d57.r26` (`city.xsm`,
//! `rage::ai::navLevelSimpleMesh`): 35 chunks of triangles and quads that cover roads,
//! sidewalks, alleys, plazas and parks. Layout (work/map/unknown_files.md, "Puzzle 1"):
//!
//! ```text
//! root        +08 -> material manager {+04 -> material pointers, +08 u16 count}; material +08 u32 flags
//!             +0c -> level
//! level       +28 -> chunk-data table {+04 -> pointers}     +2c -> span-data table {+04 -> pointers, +14 u32 count}
//!             +30 -> chunk pointers, +34 u16 count           +38 -> spans (12 bytes), +3c u16 count
//! chunk       +10 matrix (four vec4 rows; row 3 = origin)    +50 u16 chunk-data index   +56 u16 polygons
//! chunk data  +08 -> local links (12 bytes) n u16 +20        +0c -> normals (vec4, a coarse table) n u16 +22
//!             +10 -> polygons (28 bytes) n u16 +24           +14 -> vertices (3 x i16 / 20 m) n u16 +26
//! polygon     +00 3 x i16 centre   +06 u8 material   +07 u8 has external connections   +08 f32 area
//!             +10 -> u16 link indices   +14 -> u16 vertex indices   +18 u8 normal   +19 u8 links   +1a u8 vertices
//! local link  +04 u16 polygon 1, +06 u16 polygon 2, +08 u16 vertex 1, +0a u16 vertex 2 (the shared edge)
//! span        +00 u16 chunk A, +02 u16 chunk B, +04 u16 span-data index, +06 3 x i16 centre (local to A)
//! span data   +04 -> records (14 bytes: a local link, polygon in A at +04, polygon in B at +06,
//!             the edge's two vertices - of chunk A - at +08 / +0a), +08 u16 count
//! ```
//!
//! Adjacency is the file's own: every polygon lists its local links (to polygons of the same
//! chunk) and the spans join polygons of two chunks. Nothing is rebuilt from positions.

use super::Rd;
use crate::rsc::Resource;
use glam::Vec3;
use hashbrown::HashMap;

/// i16 units per metre (the game sets its quantisation scale to 20 before loading the mesh).
pub const SCALE: f32 = 20.0;
/// Material flag bits, named as the game registers them.
pub const AI_SIDEWALK: u32 = 1;
pub const AI_ROAD: u32 = 2;
pub const AI_GRASS: u32 = 4;
pub const AI_SHORTCUT: u32 = 8;

const GRID: f32 = 32.0;

/// A way from one polygon into a neighbour: the shared edge, in world space.
#[derive(Debug, Clone, Copy)]
pub struct NavLink {
    pub poly: u32,
    pub a: Vec3,
    pub b: Vec3,
    /// stored length of a local link (+2, i16 / 20; 0 on a span record's entry)
    pub len: f32,
    /// the portal ends as the original's path build orders them when leaving the owning
    /// polygon (vertex +8 / +10 when leaving the link's polygon +4, swapped for +6), with the
    /// clearance beyond each end
    pub pa: Vec3,
    pub pb: Vec3,
    pub ca: f32,
    pub cb: f32,
    /// index into `NavMesh::span_recs` for a cross-chunk record, `u32::MAX` for a local link
    pub span: u32,
}

/// A cross-chunk span record (a node of the original's search graph).
#[derive(Debug, Clone, Copy)]
pub struct SpanRec {
    /// polygons (global indices) on the chunk A and chunk B side
    pub pa: u32,
    pub pb: u32,
    /// the edge's two vertices (record +8 / +10), world
    pub a: Vec3,
    pub b: Vec3,
}

impl SpanRec {
    /// The node position: the middle of the edge.
    pub fn mid(&self) -> Vec3 {
        (self.a + self.b) * 0.5
    }
}

/// Chunk-local data of a polygon for the original's point tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct PolyLocal {
    pub verts: [Vec3; 4],
    /// the normal table entry (chunk-local) and the plane's d (polygon +0x0c)
    pub n: Vec3,
    pub d: f32,
}

#[derive(Debug, Clone)]
pub struct NavPoly {
    pub chunk: u16,
    pub material: u8,
    /// flags of its material (bit 0 sidewalk, bit 1 road, ...)
    pub flags: u32,
    /// stored area, m2
    pub area: f32,
    /// stored centre, world space
    pub center: Vec3,
    /// from the chunk's normal table: approximate (see [`NavPoly::height`])
    pub normal: Vec3,
    /// 3 or 4
    pub nv: u8,
    pub verts: [Vec3; 4],
    /// its links: `NavMesh::links[first_link .. first_link + n_links]`
    pub first_link: u32,
    pub n_links: u8,
}

impl NavPoly {
    pub fn sidewalk(&self) -> bool {
        self.flags & AI_SIDEWALK != 0
    }

    pub fn points(&self) -> &[Vec3] {
        &self.verts[..self.nv as usize]
    }

    /// Height of the surface at a place in plan view, from the vertices (the quads are not
    /// flat and the stored normals come from a small table, 25 or so per chunk): the triangle
    /// of the fan about vertex 0 the place lies in, or the nearest of them outside.
    pub fn height(&self, x: f32, z: f32) -> f32 {
        let p = self.points();
        let mut best = (f32::MAX, self.center.y);
        for i in 1..p.len().saturating_sub(1) {
            let (a, b, c) = (p[0], p[i], p[i + 1]);
            let det = (b.z - c.z) * (a.x - c.x) + (c.x - b.x) * (a.z - c.z);
            if det.abs() < 1e-6 {
                continue;
            }
            let u = ((b.z - c.z) * (x - c.x) + (c.x - b.x) * (z - c.z)) / det;
            let v = ((c.z - a.z) * (x - c.x) + (a.x - c.x) * (z - c.z)) / det;
            let w = 1.0 - u - v;
            // how far outside (0 inside)
            let out = (-u).max(-v).max(-w).max(0.0);
            if out < best.0 {
                let (u, v, w) = (u.clamp(-0.5, 1.5), v.clamp(-0.5, 1.5), w.clamp(-0.5, 1.5));
                best = (out, a.y * u + b.y * v + c.y * w);
            }
        }
        best.1
    }

    /// Plan-view test, either winding.
    pub fn contains(&self, x: f32, z: f32) -> bool {
        let p = self.points();
        let mut s = 0;
        for i in 0..p.len() {
            let (a, b) = (p[i], p[(i + 1) % p.len()]);
            let c = (b.x - a.x) * (z - a.z) - (b.z - a.z) * (x - a.x);
            if c > 1e-5 {
                s |= 1;
            } else if c < -1e-5 {
                s |= 2;
            }
        }
        s != 3
    }

    /// A point inside: bilinear on a quad, folded barycentric on a triangle (u, v in 0..1).
    pub fn point(&self, u: f32, v: f32) -> Vec3 {
        let p = &self.verts;
        if self.nv >= 4 {
            p[0].lerp(p[1], u).lerp(p[3].lerp(p[2], u), v)
        } else {
            let (u, v) = if u + v > 1.0 { (1.0 - u, 1.0 - v) } else { (u, v) };
            p[0] + (p[1] - p[0]) * u + (p[2] - p[0]) * v
        }
    }
}

#[derive(Debug, Default)]
pub struct NavMesh {
    pub polys: Vec<NavPoly>,
    pub links: Vec<NavLink>,
    /// flags of the materials (8 in the shipped file)
    pub materials: Vec<u32>,
    /// index of each chunk's first polygon
    pub chunk_first: Vec<u32>,
    /// links between polygons of one chunk, as listed by the polygons
    pub local_links: usize,
    /// links between chunks (each is entered on both of its polygons)
    pub spans: usize,
    /// per chunk: its polygon quadtree (chunk data +0x04; 27 of the 35 chunks have one)
    pub quads: Vec<Option<NavQuad>>,
    /// cross-chunk span records (nodes of the quickest-path search)
    pub span_recs: Vec<SpanRec>,
    /// per polygon: chunk-local vertices, normal and plane d
    pub local: Vec<PolyLocal>,
    /// per chunk: matrix rows (right, up, forward, position)
    pub chunk_rows: Vec<[Vec3; 4]>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

/// A chunk's polygon quadtree as stored in the file: bounds and splits in the chunk's own
/// x / z, every node listing the polygons (chunk-local indices) filed there.
#[derive(Debug, Clone)]
pub struct NavQuad {
    pub min: [f32; 2],
    pub max: [f32; 2],
    /// the chunk's matrix rows (right, up, forward, position): local = rows^T (world - pos)
    pub rows: [Vec3; 4],
    /// node 0 is the root
    pub nodes: Vec<QuadNode>,
}

#[derive(Debug, Clone)]
pub struct QuadNode {
    /// children (index into `nodes`, `u32::MAX` none): 0 low x low z, 1 high x low z,
    /// 2 low x high z, 3 high x high z
    pub child: [u32; 4],
    pub polys: Vec<u16>,
    pub split: [f32; 2],
}

impl NavQuad {
    /// The polygons of the nodes a square (local x / z) reaches, as the original's walk
    /// reports them (sub_825538B8 / 825530C0 / 82553190): depth first, a node's own polygons
    /// before its children 0..3, every polygon once.
    pub fn walk(&self, lo: [f32; 2], hi: [f32; 2], n_polys: usize, out: &mut Vec<u32>) {
        if lo[0] > self.max[0] || lo[1] > self.max[1] || hi[0] < self.min[0] || hi[1] < self.min[1] || self.nodes.is_empty() {
            return;
        }
        let mut seen = vec![false; n_polys];
        let mut stack = vec![0u32];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n as usize];
            for &p in &node.polys {
                if (p as usize) < n_polys && !seen[p as usize] {
                    seen[p as usize] = true;
                    out.push(p as u32);
                }
            }
            // pushed in reverse so that child 0 is visited first
            for c in (0..4).rev() {
                let ch = node.child[c];
                if ch == u32::MAX {
                    continue;
                }
                let xok = if c & 1 != 0 { hi[0] >= node.split[0] } else { lo[0] <= node.split[0] };
                let zok = if c & 2 != 0 { hi[1] >= node.split[1] } else { lo[1] <= node.split[1] };
                if xok && zok {
                    stack.push(ch);
                }
            }
        }
    }
}

fn parse_quad(rd: &Rd, q: usize, rows: [Vec3; 4]) -> Option<NavQuad> {
    if !rd.has(q, 52) {
        return None;
    }
    let mut nodes: Vec<QuadNode> = Vec::new();
    // (file offset, index of the parent's child slot to fill)
    let mut todo: Vec<(usize, Option<(usize, usize)>)> = vec![(q + 20, None)];
    while let Some((o, parent)) = todo.pop() {
        if nodes.len() > 1 << 16 || !rd.has(o, 32) {
            return None;
        }
        let i = nodes.len();
        let mut polys = Vec::new();
        if let Some(pp) = rd.ptr(o + 16) {
            let n = rd.u16(o + 20) as usize;
            if !rd.has(pp, 2 * n) {
                return None;
            }
            polys = (0..n).map(|k| rd.u16(pp + 2 * k)).collect();
        }
        nodes.push(QuadNode { child: [u32::MAX; 4], polys, split: [rd.f32(o + 24), rd.f32(o + 28)] });
        if let Some((pi, c)) = parent {
            nodes[pi].child[c] = i as u32;
        }
        for c in 0..4 {
            if let Some(ch) = rd.ptr(o + 4 * c) {
                todo.push((ch, Some((i, c))));
            }
        }
    }
    Some(NavQuad { min: [rd.f32(q + 4), rd.f32(q + 8)], max: [rd.f32(q + 12), rd.f32(q + 16)], rows, nodes })
}

fn cell(x: f32, z: f32) -> (i32, i32) {
    ((x / GRID).floor() as i32, (z / GRID).floor() as i32)
}

impl NavMesh {
    pub fn parse(res: &Resource) -> Option<NavMesh> {
        let rd = Rd::new(res);
        let i16at = |o: usize| rd.u16(o) as i16 as f32 / SCALE;
        let v3 = |o: usize| Vec3::new(rd.f32(o), rd.f32(o + 4), rd.f32(o + 8));
        let mut mesh = NavMesh::default();
        if let Some(mm) = rd.ptr(0x08) {
            if let Some(arr) = rd.ptr(mm + 4) {
                for k in 0..(rd.u16(mm + 8) as usize).min(256) {
                    mesh.materials.push(rd.ptr(arr + 4 * k).map_or(0, |m| rd.u32(m + 8)));
                }
            }
        }
        let lvl = rd.ptr(0x0c)?;
        let cds = rd.ptr(rd.ptr(lvl + 0x28)? + 4)?;
        let (chunks, n_chunks) = (rd.ptr(lvl + 0x30)?, rd.u16(lvl + 0x34) as usize);
        // per chunk: world vertices (kept for the spans)
        let mut chunk_verts: Vec<Vec<Vec3>> = Vec::with_capacity(n_chunks);
        for ci in 0..n_chunks {
            mesh.chunk_first.push(mesh.polys.len() as u32);
            chunk_verts.push(Vec::new());
            mesh.quads.push(None);
            mesh.chunk_rows.push([Vec3::ZERO; 4]);
            let Some(c) = rd.ptr(chunks + 4 * ci) else { continue };
            let Some(d) = rd.ptr(cds + 4 * rd.u16(c + 0x50) as usize) else { continue };
            let rows = [v3(c + 0x10), v3(c + 0x20), v3(c + 0x30), v3(c + 0x40)];
            if !rows.iter().all(|r| r.is_finite()) {
                continue;
            }
            mesh.chunk_rows[ci] = rows;
            mesh.quads[ci] = rd.ptr(d + 0x04).and_then(|q| parse_quad(&rd, q, rows));
            let world = |v: Vec3| rows[0] * v.x + rows[1] * v.y + rows[2] * v.z + rows[3];
            let (n_links, n_normals, n_polys, n_verts) = (rd.u16(d + 0x20) as usize, rd.u16(d + 0x22) as usize, rd.u16(d + 0x24) as usize, rd.u16(d + 0x26) as usize);
            let (Some(lk), Some(po), Some(ve)) = (rd.ptr(d + 0x08).or((n_links == 0).then_some(0)), rd.ptr(d + 0x10), rd.ptr(d + 0x14)) else { continue };
            let no = rd.ptr(d + 0x0c);
            if !rd.has(po, n_polys * 28) || !rd.has(ve, n_verts * 6) || !rd.has(lk, n_links * 12) {
                continue;
            }
            let lverts: Vec<Vec3> = (0..n_verts).map(|k| Vec3::new(i16at(ve + 6 * k), i16at(ve + 6 * k + 2), i16at(ve + 6 * k + 4))).collect();
            let verts: Vec<Vec3> = (0..n_verts).map(|k| world(Vec3::new(i16at(ve + 6 * k), i16at(ve + 6 * k + 2), i16at(ve + 6 * k + 4)))).collect();
            let base = mesh.polys.len() as u32;
            for k in 0..n_polys {
                let o = po + 28 * k;
                let material = rd.u8(o + 6);
                let (nl, nv) = (rd.u8(o + 0x19) as usize, (rd.u8(o + 0x1a) as usize).min(4));
                let mut pv = [Vec3::ZERO; 4];
                let mut plv = [Vec3::ZERO; 4];
                if let Some(vp) = rd.ptr(o + 0x14) {
                    for a in 0..nv {
                        plv[a] = lverts.get(rd.u16(vp + 2 * a) as usize).copied().unwrap_or(Vec3::ZERO);
                        pv[a] = verts.get(rd.u16(vp + 2 * a) as usize).copied().unwrap_or(Vec3::ZERO);
                    }
                }
                let ni = rd.u8(o + 0x18) as usize;
                let lnormal = no.filter(|_| ni < n_normals).map_or(Vec3::Y, |n| v3(n + 16 * ni));
                let mut normal = lnormal;
                normal = (rows[0] * normal.x + rows[1] * normal.y + rows[2] * normal.z).normalize_or(Vec3::Y);
                let first_link = mesh.links.len() as u32;
                if let Some(lp) = rd.ptr(o + 0x10) {
                    for a in 0..nl {
                        let l = lk + 12 * rd.u16(lp + 2 * a) as usize;
                        if !rd.has(l, 12) {
                            continue;
                        }
                        let (p1, p2) = (rd.u16(l + 4) as usize, rd.u16(l + 6) as usize);
                        let other = if p1 == k { p2 } else if p2 == k { p1 } else { continue };
                        let (Some(&a), Some(&b)) = (verts.get(rd.u16(l + 8) as usize), verts.get(rd.u16(l + 10) as usize)) else { continue };
                        if other < n_polys {
                            let len = rd.u16(l + 2) as i16 as f32 / SCALE;
                            let (c0, c1) = (rd.u8(l) as i8 as f32 * 0.2, rd.u8(l + 1) as i8 as f32 * 0.2);
                            // (byte +0 is the clearance beyond vertex +8, +1 beyond vertex +10: sub_827635B0,
                            // each end kept with its own through sub_82765820 / sub_82765740 / sub_8276C0E0)
                            let (pa, pb, ca, cb) = if p1 == k { (a, b, c0, c1) } else { (b, a, c1, c0) };
                            mesh.links.push(NavLink { poly: base + other as u32, a, b, len, pa, pb, ca, cb, span: u32::MAX });
                        }
                    }
                }
                mesh.local.push(PolyLocal { verts: plv, n: lnormal, d: rd.f32(o + 0x0c) });
                mesh.local_links += mesh.links.len() - first_link as usize;
                mesh.polys.push(NavPoly {
                    chunk: ci as u16,
                    material,
                    flags: mesh.materials.get(material as usize).copied().unwrap_or(0),
                    area: rd.f32(o + 8),
                    center: world(Vec3::new(i16at(o), i16at(o + 2), i16at(o + 4))),
                    normal,
                    nv: nv as u8,
                    verts: pv,
                    first_link,
                    n_links: (mesh.links.len() - first_link as usize) as u8,
                });
            }
            chunk_verts[ci] = verts;
        }
        // links between chunks: appended behind the local ones, so the polygons' link ranges
        // are rebuilt at the end
        let mut extra: Vec<(u32, NavLink)> = Vec::new();
        if let (Some(sp), Some(sds)) = (rd.ptr(lvl + 0x38), rd.ptr(lvl + 0x2c).and_then(|t| rd.ptr(t + 4))) {
            let count = |ci: usize| -> u32 { mesh.chunk_first.get(ci + 1).copied().unwrap_or(mesh.polys.len() as u32) - mesh.chunk_first[ci] };
            for k in 0..rd.u16(lvl + 0x3c) as usize {
                let o = sp + 12 * k;
                let (ca, cb) = (rd.u16(o) as usize, rd.u16(o + 2) as usize);
                let Some(sd) = rd.ptr(sds + 4 * rd.u16(o + 4) as usize) else { continue };
                let Some(recs) = rd.ptr(sd + 4) else { continue };
                if ca >= n_chunks || cb >= n_chunks {
                    continue;
                }
                for j in 0..rd.u16(sd + 8) as usize {
                    let r = recs + 14 * j;
                    let (pa, pb) = (rd.u16(r + 4) as u32, rd.u16(r + 6) as u32);
                    if pa >= count(ca) || pb >= count(cb) {
                        continue;
                    }
                    let (pa, pb) = (mesh.chunk_first[ca] + pa, mesh.chunk_first[cb] + pb);
                    let (Some(&a), Some(&b)) = (chunk_verts[ca].get(rd.u16(r + 8) as usize), chunk_verts[ca].get(rd.u16(r + 10) as usize)) else { continue };
                    // clearances record +0 (beyond vertex +8) / +1 (beyond vertex +10), i8 x 0.2,
                    // negative = unlimited (1e8): sub_82764150, the fsel in sub_82765820
                    let cl = |o: usize| { let v = rd.u8(o) as i8 as f32 * 0.2; if v < 0.0 { 1e8 } else { v } };
                    let (c0, c1) = (cl(r), cl(r + 1));
                    let sid = mesh.span_recs.len() as u32;
                    mesh.span_recs.push(SpanRec { pa, pb, a, b });
                    extra.push((pa, NavLink { poly: pb, a, b, len: 0.0, pa: a, pb: b, ca: c0, cb: c1, span: sid }));
                    extra.push((pb, NavLink { poly: pa, a, b, len: 0.0, pa: b, pb: a, ca: c1, cb: c0, span: sid }));
                    mesh.spans += 1;
                }
            }
        }
        if !extra.is_empty() {
            let mut add: HashMap<u32, Vec<NavLink>> = HashMap::new();
            for (p, l) in extra {
                add.entry(p).or_default().push(l);
            }
            let old = std::mem::take(&mut mesh.links);
            for (i, p) in mesh.polys.iter_mut().enumerate() {
                let first = mesh.links.len() as u32;
                mesh.links.extend_from_slice(&old[p.first_link as usize..p.first_link as usize + p.n_links as usize]);
                if let Some(v) = add.get(&(i as u32)) {
                    mesh.links.extend_from_slice(v);
                }
                p.first_link = first;
                p.n_links = (mesh.links.len() as u32 - first).min(255) as u8;
            }
        }
        for (i, p) in mesh.polys.iter().enumerate() {
            let (mut lo, mut hi) = (p.verts[0], p.verts[0]);
            for v in p.points() {
                lo = lo.min(*v);
                hi = hi.max(*v);
            }
            let ((x0, z0), (x1, z1)) = (cell(lo.x, lo.z), cell(hi.x, hi.z));
            if x1 - x0 > 64 || z1 - z0 > 64 {
                continue;
            }
            for x in x0..=x1 {
                for z in z0..=z1 {
                    mesh.grid.entry((x, z)).or_default().push(i as u32);
                }
            }
        }
        (!mesh.polys.is_empty()).then_some(mesh)
    }

    /// Polygons of chunk `ci` (global indices).
    pub fn chunk_polys(&self, ci: usize) -> std::ops::Range<u32> {
        let a = self.chunk_first.get(ci).copied().unwrap_or(0);
        let b = self.chunk_first.get(ci + 1).copied().unwrap_or(self.polys.len() as u32);
        a..b.max(a)
    }

    /// The original's sphere query over the level (navlevel 0x827644F8 / 0x82764220): the
    /// chunks in order; within a chunk the polygons its quadtree reports for the square of
    /// `r` around `pos` (all of them, in order, without a tree); those `accept` takes, the
    /// first `cap` kept, then sorted by (chunk, polygon) as sub_82761F20 does.
    pub fn sphere_query(&self, pos: Vec3, r: f32, cap: usize, mut accept: impl FnMut(u32, &NavPoly) -> bool) -> Vec<u32> {
        let mut out: Vec<u32> = Vec::new();
        let mut seq: Vec<u32> = Vec::new();
        for ci in 0..self.chunk_first.len() {
            let range = self.chunk_polys(ci);
            let n = (range.end - range.start) as usize;
            if n == 0 {
                continue;
            }
            seq.clear();
            match self.quads.get(ci).and_then(|q| q.as_ref()) {
                Some(q) => {
                    let d = pos - q.rows[3];
                    let (lx, lz) = (q.rows[0].dot(d), q.rows[2].dot(d));
                    q.walk([lx - r, lz - r], [lx + r, lz + r], n, &mut seq);
                }
                None => seq.extend(0..n as u32),
            }
            for &p in &seq {
                let gi = range.start + p;
                if out.len() < cap && accept(gi, &self.polys[gi as usize]) {
                    out.push(gi);
                }
            }
        }
        out.sort_unstable();
        out
    }

    pub fn links_of(&self, poly: u32) -> &[NavLink] {
        let p = &self.polys[poly as usize];
        &self.links[p.first_link as usize..p.first_link as usize + p.n_links as usize]
    }

    /// Polygons whose bounding box reaches into the square around a place.
    pub fn near(&self, pos: Vec3, radius: f32, mut f: impl FnMut(u32, &NavPoly)) {
        let ((x0, z0), (x1, z1)) = (cell(pos.x - radius, pos.z - radius), cell(pos.x + radius, pos.z + radius));
        for x in x0..=x1 {
            for z in z0..=z1 {
                let Some(list) = self.grid.get(&(x, z)) else { continue };
                for &i in list {
                    let p = &self.polys[i as usize];
                    // listed in several cells: report it from the first of them only
                    let lo = p.points().iter().fold(p.verts[0], |a, v| a.min(*v));
                    let (cx, cz) = cell(lo.x, lo.z);
                    if (cx.max(x0), cz.max(z0)) == (x, z) {
                        f(i, p);
                    }
                }
            }
        }
    }

    /// The polygon under a point: of those with one of the `flags` that contain it in plan
    /// view, the one whose plane is nearest in height (within `dy`).
    pub fn find(&self, pos: Vec3, flags: u32, dy: f32) -> Option<u32> {
        let list = self.grid.get(&cell(pos.x, pos.z))?;
        let mut best: Option<(f32, u32)> = None;
        for &i in list {
            let p = &self.polys[i as usize];
            if p.flags & flags == 0 || !p.contains(pos.x, pos.z) {
                continue;
            }
            let d = (p.height(pos.x, pos.z) - pos.y).abs();
            if d <= dy && best.map_or(true, |b| d < b.0) {
                best = Some((d, i));
            }
        }
        best.map(|b| b.1)
    }

    /// Connected pieces of the sidewalk (polygons with flag bit 0, joined by the file's
    /// links): the piece number of every polygon (`u32::MAX` on the others) and their count.
    pub fn sidewalk_islands(&self) -> (Vec<u32>, usize) {
        let mut id = vec![u32::MAX; self.polys.len()];
        let mut n = 0;
        let mut stack = Vec::new();
        for s in 0..self.polys.len() {
            if id[s] != u32::MAX || !self.polys[s].sidewalk() {
                continue;
            }
            id[s] = n;
            stack.push(s as u32);
            while let Some(x) = stack.pop() {
                for l in self.links_of(x) {
                    if id[l.poly as usize] == u32::MAX && self.polys[l.poly as usize].sidewalk() {
                        id[l.poly as usize] = n;
                        stack.push(l.poly);
                    }
                }
            }
            n += 1;
        }
        (id, n as usize)
    }
}

// ---------------------------------------------------------------------------------
// Ways for cars over the mesh (the port's own query: the original's `navLevelSimple`
// path handlers are not decoded - work/map/unknown_files.md, "AI shortcuts on the
// navigation mesh").

pub const AI_JUMP: u32 = 0x10;
pub const AI_JUMP_UP: u32 = 0x20;
/// What a car may drive on: roads and everything marked as a shortcut (alleys, car parks,
/// plazas; the pavement and grass shortcuts too).
pub const AI_DRIVE: u32 = AI_ROAD | AI_SHORTCUT;

impl NavPoly {
    /// A car may drive here.
    pub fn drivable(&self) -> bool {
        self.flags & AI_DRIVE != 0 && self.nv >= 3
    }

    /// The point of the polygon nearest to a place in plan view (the place itself when it
    /// lies inside), at the surface's height.
    pub fn closest(&self, x: f32, z: f32) -> Vec3 {
        let p = self.points();
        if p.len() < 3 {
            return self.center;
        }
        if self.contains(x, z) {
            return Vec3::new(x, self.height(x, z), z);
        }
        let mut best = (f32::MAX, self.center);
        for i in 0..p.len() {
            let (a, b) = (p[i], p[(i + 1) % p.len()]);
            let (dx, dz) = (b.x - a.x, b.z - a.z);
            let t = (((x - a.x) * dx + (z - a.z) * dz) / (dx * dx + dz * dz).max(1e-9)).clamp(0.0, 1.0);
            let q = a.lerp(b, t);
            let d = (q.x - x) * (q.x - x) + (q.z - z) * (q.z - z);
            if d < best.0 {
                best = (d, q);
            }
        }
        best.1
    }
}

// ---------------------------------------------------------------------------------
// navQueryQuickestPath, 1:1 (work/racerai/navquery.md): the search the computer racers use.

/// One portal element of the result path (the original's 56-byte type-2 element).
#[derive(Debug, Clone, Copy)]
pub struct Portal56 {
    /// left end (+4) and right end (+16) in the direction of travel
    pub a: Vec3,
    pub b: Vec3,
    /// clearance beyond A (+32) and beyond B (+36)
    pub clear_a: f32,
    pub clear_b: f32,
    /// the polygon left
    pub left: u32,
    /// the node entered (+44): a polygon index, or `polys.len() + span record` for a span node
    pub entered: u32,
    /// material flags and normal of the entered polygon (of the polygon left when a span
    /// node is entered - approx.: the original's choice for a span node is not decoded)
    pub flags: u32,
    pub normal: Vec3,
}

/// Result of [`NavMesh::quickest_path`].
#[derive(Debug, Clone)]
pub struct QuickestPath {
    /// one portal per polygon left, start to goal
    pub portals: Vec<Portal56>,
    /// the final point (the goal point, or the position of the closest node when not reached)
    pub goal: Vec3,
    /// false: the goal was not reachable, the path leads to the closed node nearest it
    pub reached: bool,
}

/// The racers' cost object (brain rule): 1.0 for the flags of the start or goal polygon and
/// for anything with road, jump or jump-up bits (0x32), else 5.0.
pub fn racer_cost(start_flags: u32, goal_flags: u32) -> impl Fn(u32) -> f32 {
    move |f| if f == start_flags || f == goal_flags || f & 0x32 != 0 { 1.0 } else { 5.0 }
}

#[derive(Clone, Copy, PartialEq)]
enum Edge {
    None,
    Link(usize),
    FromSpan,
}

struct Rec {
    g: f32,
    h: f32,
    parent: u32,
    edge: Edge,
    /// position in the heap (0 = not open)
    heap: usize,
}

const NONE: u32 = u32::MAX;
const MAX_RECORDS: usize = 4096;

struct Search {
    recs: Vec<Rec>,
    map: HashMap<u32, usize>,
    /// 1-based, entry 0 unused: (key, node, record)
    heap: Vec<(f32, u32, usize)>,
}

impl Search {
    fn swap(&mut self, i: usize, j: usize) {
        self.heap.swap(i, j);
        let (ri, rj) = (self.heap[i].2, self.heap[j].2);
        self.recs[ri].heap = i;
        self.recs[rj].heap = j;
    }
    fn sift_up(&mut self, mut i: usize) {
        while i > 1 && self.heap[i / 2].0 > self.heap[i].0 {
            self.swap(i / 2, i);
            i /= 2;
        }
    }
    fn sift_down(&mut self, mut i: usize) {
        let n = self.heap.len() - 1;
        loop {
            let mut c = i;
            if 2 * i <= n && self.heap[2 * i].0 < self.heap[c].0 {
                c = 2 * i;
            }
            if 2 * i + 1 <= n && self.heap[2 * i + 1].0 < self.heap[c].0 {
                c = 2 * i + 1;
            }
            if c == i {
                break;
            }
            self.swap(i, c);
            i = c;
        }
    }
    fn push(&mut self, key: f32, node: u32, rec: usize) {
        self.heap.push((key, node, rec));
        let i = self.heap.len() - 1;
        self.recs[rec].heap = i;
        self.sift_up(i);
    }
    fn pop(&mut self) -> Option<(u32, usize)> {
        if self.heap.len() <= 1 {
            return None;
        }
        let top = self.heap[1];
        let last = self.heap.pop().unwrap();
        self.recs[top.2].heap = 0;
        if self.heap.len() > 1 {
            self.heap[1] = last;
            self.recs[last.2].heap = 1;
            self.sift_down(1);
        }
        Some((top.1, top.2))
    }
    /// get-or-create (sub_827727E0): `None` when 4096 records exist.
    fn rec(&mut self, node: u32) -> Option<usize> {
        if let Some(&r) = self.map.get(&node) {
            return Some(r);
        }
        if self.recs.len() >= MAX_RECORDS {
            return None;
        }
        self.recs.push(Rec { g: 0.0, h: 0.0, parent: NONE, edge: Edge::None, heap: 0 });
        self.map.insert(node, self.recs.len() - 1);
        Some(self.recs.len() - 1)
    }
}

impl NavMesh {
    fn node_pos(&self, node: u32) -> Vec3 {
        match self.polys.get(node as usize) {
            Some(p) => p.center,
            None => self.span_recs[node as usize - self.polys.len()].mid(),
        }
    }

    /// Neighbours of a node in the original's order: (node, length, edge).
    fn node_neighbours(&self, node: u32, out: &mut Vec<(u32, f32, Edge)>) {
        out.clear();
        let n = self.polys.len() as u32;
        if node < n {
            let p = &self.polys[node as usize];
            for i in 0..p.n_links as usize {
                let li = p.first_link as usize + i;
                let l = &self.links[li];
                if l.span == NONE {
                    out.push((l.poly, l.len, Edge::Link(li)));
                } else {
                    let nb = n + l.span;
                    out.push((nb, self.node_pos(nb).distance(p.center), Edge::Link(li)));
                }
            }
        } else {
            let s = &self.span_recs[(node - n) as usize];
            let m = s.mid();
            out.push((s.pa, self.polys[s.pa as usize].center.distance(m), Edge::FromSpan));
            out.push((s.pb, self.polys[s.pb as usize].center.distance(m), Edge::FromSpan));
        }
    }

    fn chunk_local(&self, poly: u32, p: Vec3) -> Vec3 {
        let r = &self.chunk_rows[self.polys[poly as usize].chunk as usize];
        let d = p - r[3];
        Vec3::new(r[0].dot(d), r[1].dot(d), r[2].dot(d))
    }

    /// Plan-view inside test of sub_825500A0 (chunk-local, one winding, border included).
    fn plan_inside(&self, poly: u32, lp: Vec3) -> bool {
        let q = &self.local[poly as usize];
        let nv = self.polys[poly as usize].nv as usize;
        (0..nv).all(|i| {
            let (a, b) = (q.verts[i], q.verts[(i + 1) % nv]);
            (lp.z - a.z) * (b.x - a.x) - (lp.x - a.x) * (b.z - a.z) <= 2e-4
        })
    }

    /// Height of a chunk-local point over the polygon's plane along y (sub_82550658).
    fn plane_h(&self, poly: u32, lp: Vec3) -> Option<f32> {
        let q = &self.local[poly as usize];
        (q.n.y > 0.001).then(|| (q.n.dot(lp) + q.d) / q.n.y)
    }

    /// Polygons containing `p` in plan view with `lo <= height over the plane <= hi`, ascending.
    fn polys_containing(&self, p: Vec3, lo: f32, hi: f32, cap: usize) -> Vec<u32> {
        let mut out = Vec::new();
        self.near(p, 0.0, |i, _| {
            if out.len() >= cap || self.local[i as usize].n.y < 0.0 {
                return;
            }
            let lp = self.chunk_local(i, p);
            if self.plan_inside(i, lp) && self.plane_h(i, lp).map_or(false, |h| h >= lo && h <= hi) {
                out.push(i);
            }
        });
        out.sort_unstable();
        out
    }

    /// Nearest polygon within `max` metres by 3-D point-to-polygon distance (mesh vfunc 56).
    /// (approx.: the edge branch of sub_8254FCA8 is the point-segment distance, not stepped
    /// through; the accept test is area > 0 or any link, for a zero-width actor.)
    fn nearest_poly(&self, p: Vec3, max: f32) -> Option<u32> {
        let mut cand = Vec::new();
        self.near(p, max, |i, _| cand.push(i));
        cand.sort_unstable();
        let mut best = (max, None);
        for i in cand {
            let q = &self.local[i as usize];
            let pl = &self.polys[i as usize];
            if pl.area <= 0.0 && pl.n_links == 0 {
                continue;
            }
            let lp = self.chunk_local(i, p);
            let nl = q.n.length();
            if nl < 1e-6 {
                continue;
            }
            let dp = ((q.n.dot(lp) + q.d) / nl).abs();
            if dp > best.0 {
                continue;
            }
            let dist = if self.plan_inside(i, lp) {
                dp
            } else {
                let nv = pl.nv as usize;
                (0..nv)
                    .map(|k| {
                        let (a, b) = (q.verts[k], q.verts[(k + 1) % nv]);
                        let ab = b - a;
                        let t = if ab.length_squared() > 0.0 { ((lp - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                        (lp - (a + ab * t)).length()
                    })
                    .fold(f32::MAX, f32::min)
            };
            if dist < best.0 {
                best = (dist, Some(i));
            }
        }
        best.1
    }

    /// Start / goal polygons of a point: all containing it (+-1 m of the plane), else the
    /// nearest within 10 m.
    fn point_polys(&self, p: Vec3) -> Vec<u32> {
        let v = self.polys_containing(p, -1.0, 1.0, 1024);
        if !v.is_empty() {
            return v;
        }
        self.nearest_poly(p, 10.0).into_iter().collect()
    }

    /// sub_82764568 (the brain's on-mesh test): inside a polygon in plan view, between 10 m
    /// below and 100 m above its plane.
    pub fn on_mesh(&self, p: Vec3) -> bool {
        !self.polys_containing(p, -10.0, 100.0, 1).is_empty()
    }

    /// Material flags of the polygon `p` lies in (nearest within 10 m, only when `p` is
    /// inside it within +-1 m, else 0), sub_826FD730.
    pub fn point_flags(&self, p: Vec3) -> u32 {
        match self.nearest_poly(p, 10.0) {
            Some(i) if self.polys_containing(p, -1.0, 1.0, 1024).contains(&i) => self.polys[i as usize].flags,
            _ => 0,
        }
    }

    /// sub_826FD508: the brain's off-mesh teleport: the nearest polygon within 10 m; the point
    /// itself when inside it (+-1 m), else the polygon's stored centre. `None` = no polygon.
    pub fn teleport_target(&self, p: Vec3) -> Option<Vec3> {
        let i = self.nearest_poly(p, 10.0)?;
        if self.polys_containing(p, -1.0, 1.0, 1024).contains(&i) {
            Some(p)
        } else {
            Some(self.polys[i as usize].center)
        }
    }

    /// navQueryQuickestPath: weighted A* (h = 1.5 x distance) over polygons and span records.
    /// `cost` gives the multiplier for the material flags of a polygon entered (called only
    /// for non-zero flags). `None` when the start point has no polygon.
    pub fn quickest_path(&self, from: Vec3, to: Vec3, cost: &dyn Fn(u32) -> f32) -> Option<QuickestPath> {
        let n = self.polys.len() as u32;
        let starts = self.point_polys(from);
        if starts.is_empty() {
            return None;
        }
        let goals: hashbrown::HashSet<u32> = self.point_polys(to).into_iter().collect();
        let mult = |nb: u32| -> f32 {
            match self.polys.get(nb as usize) {
                Some(p) if p.flags != 0 => cost(p.flags),
                _ => 1.0,
            }
        };
        let heur = |nb: u32| -> f32 { if goals.contains(&nb) { 0.0 } else { 1.5 * to.distance(self.node_pos(nb)) } };
        let mut s = Search { recs: Vec::new(), map: HashMap::new(), heap: vec![(0.0, 0, 0)] };
        // relax (sub_82772BD8)
        let relax = |s: &mut Search, parent: u32, nb: u32, g: f32, edge: Edge| {
            let Some(r) = s.rec(nb) else { return };
            let rc = &s.recs[r];
            if rc.heap != 0 {
                if rc.g <= g {
                    return;
                }
                let rc = &mut s.recs[r];
                rc.parent = parent;
                rc.g = g;
                rc.edge = edge;
                let (key, i) = (rc.h + g, rc.heap);
                s.heap[i].0 = key;
                s.sift_up(i);
            } else {
                if rc.parent != NONE && rc.g <= g {
                    return;
                }
                let h = heur(nb);
                let rc = &mut s.recs[r];
                rc.g = g;
                rc.parent = parent;
                rc.edge = edge;
                rc.h = h;
                s.push(h + g, nb, r);
            }
        };
        // initialisation (G slot 11): a start polygon that is a goal ends the search at once
        if starts.iter().any(|p| goals.contains(p)) {
            return Some(QuickestPath { portals: Vec::new(), goal: to, reached: true });
        }
        for &sp in &starts {
            let r = s.rec(sp)?;
            let rc = &mut s.recs[r];
            rc.g = 0.0;
            rc.parent = sp;
            rc.h = 0.0;
            rc.edge = Edge::None;
        }
        let mut nbs = Vec::new();
        for &sp in &starts {
            self.node_neighbours(sp, &mut nbs);
            for &(nb, _len, edge) in &nbs {
                let c = if goals.contains(&nb) { from.distance(to) } else { from.distance(self.node_pos(nb)) } * mult(nb);
                relax(&mut s, sp, nb, c, edge);
            }
        }
        // main loop (sub_827735D0)
        let mut found = None;
        while let Some((node, r)) = s.pop() {
            if goals.contains(&node) {
                found = Some(node);
                break;
            }
            let g = s.recs[r].g;
            let pos = self.node_pos(node);
            self.node_neighbours(node, &mut nbs);
            for &(nb, len, edge) in &nbs {
                let c = if goals.contains(&nb) { to.distance(pos) } else { len } * mult(nb);
                if c < 0.0 {
                    continue;
                }
                relax(&mut s, node, nb, g + c, edge);
            }
        }
        let (end, goal, reached) = match found {
            Some(nd) => (nd, to, true),
            None => {
                // the closed record with the smallest h (sub_82772608), first on ties
                let mut best: Option<(f32, u32)> = None;
                let mut by_rec: Vec<(usize, u32)> = s.map.iter().map(|(&nd, &r)| (r, nd)).collect();
                by_rec.sort_unstable();
                for (r, nd) in by_rec {
                    let rc = &s.recs[r];
                    if rc.heap != 0 || rc.parent == NONE || rc.edge == Edge::None || rc.parent == nd {
                        continue;
                    }
                    if best.map_or(true, |b| rc.h < b.0) {
                        best = Some((rc.h, nd));
                    }
                }
                let (_, nd) = best?;
                (nd, self.node_pos(nd), false)
            }
        };
        // path build: walk the parents, a portal per step that leaves a polygon
        let mut portals = Vec::new();
        let mut cur = end;
        loop {
            let rc = &s.recs[s.map[&cur]];
            if rc.parent == cur || rc.parent == NONE || rc.edge == Edge::None {
                break;
            }
            let parent = rc.parent;
            if let (true, Edge::Link(li)) = (parent < n, rc.edge) {
                let l = &self.links[li];
                let ep = if cur < n { cur } else { parent };
                let q = &self.polys[ep as usize];
                portals.push(Portal56 { a: l.pa, b: l.pb, clear_a: l.ca, clear_b: l.cb, left: parent, entered: cur, flags: q.flags, normal: q.normal });
            }
            cur = parent;
        }
        portals.reverse();
        Some(QuickestPath { portals, goal, reached })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> Option<Resource> {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let d = std::fs::read(std::path::Path::new(&root).join("resources/city/sc/0x69d17d57.r26")).ok()?;
        if d.len() < 16 || &d[0..4] != b"MCLR" {
            return None;
        }
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let data = d[16..].to_vec();
        let vs = (u(12) as usize).min(data.len());
        Some(Resource { rtype: u(4), flags: u(8), virtual_size: vs, physical_size: data.len() - vs, data })
    }

    /// Counts of work/map/unknown_files.md (Puzzle 1) and the sanity of the adjacency.
    #[test]
    fn navmesh_counts_and_adjacency() {
        let Some(res) = file() else { return };
        let m = NavMesh::parse(&res).unwrap();
        assert_eq!(m.chunk_first.len(), 35);
        assert_eq!(m.polys.len(), 52_987);
        assert_eq!(m.materials, [0x81, 0x82, 0x09, 0x0a, 0x110, 0x84, 0xa0, 0x0c]);
        // sidewalk 13,762 + sidewalk-and-shortcut 1,350
        assert_eq!(m.polys.iter().filter(|p| p.sidewalk()).count(), 15_112);
        assert_eq!(m.polys.iter().filter(|p| p.material == 0).count(), 13_762);
        assert_eq!(m.polys.iter().filter(|p| p.material == 1).count(), 21_586);
        assert_eq!(m.polys.iter().filter(|p| p.nv == 3).count(), 9_229);
        assert_eq!(m.spans, 1_372);
        // every local link is listed by both of its polygons, give or take the few odd ones
        assert!((m.local_links as i64 - 2 * 84_354).abs() < 1_200, "{}", m.local_links);
        // stored centre = mean of the vertices; a link's edge lies on the polygon's outline
        let (mut on, mut all, mut flat) = (0, 0, 0);
        for (i, p) in m.polys.iter().enumerate() {
            let mean = p.points().iter().copied().sum::<Vec3>() / p.nv as f32;
            assert!(mean.distance(p.center) < 0.12, "{i}: {:?} {:?}", mean, p.center);
            flat += p.points().iter().all(|v| (p.height(v.x, v.z) - v.y).abs() < 0.05) as usize;
            for l in m.links_of(i as u32) {
                assert!((l.poly as usize) < m.polys.len() && l.poly as usize != i);
                all += 1;
                let hit = |q: Vec3| p.points().iter().any(|v| v.distance(q) < 0.12);
                on += (hit(l.a) && hit(l.b)) as usize;
            }
        }
        assert!(on as f32 > all as f32 * 0.985, "{on} of {all}");
        // the height query gives back the vertices (not on the few folded or upright polygons)
        assert!(flat as f32 > m.polys.len() as f32 * 0.99, "{flat}");
        println!("links {all}, edge on the outline {on}, height exact at the vertices {flat}");
        // world extent of the notes
        let lo = m.polys.iter().fold(Vec3::splat(1e9), |a, p| a.min(p.center));
        let hi = m.polys.iter().fold(Vec3::splat(-1e9), |a, p| a.max(p.center));
        assert!(lo.x > -3470.0 && hi.x < 2830.0 && lo.z > -2335.0 && hi.z < 1970.0, "{lo:?} {hi:?}");
        // the pavements are islands (city blocks) between the roads
        let (id, n) = m.sidewalk_islands();
        assert_eq!(n, 351);
        assert_eq!(id.iter().filter(|&&x| x != u32::MAX).count(), 15_112);
        // point query: a sidewalk polygon is found at its own centre
        let k = m.polys.iter().position(|p| p.sidewalk()).unwrap();
        let c = m.polys[k].center;
        let f = m.find(c, AI_SIDEWALK, 1.0).unwrap();
        assert!(m.polys[f as usize].contains(c.x, c.z));
        let mut seen = 0;
        m.near(c, 10.0, |i, _| seen += (i as usize == k) as usize);
        assert_eq!(seen, 1);
    }

    /// Ways for cars between route nodes of real races that lie away from the lane network
    /// (car parks and alleys; from the sweep in game/src/offroad.rs): a way exists, its
    /// polygons are neighbours, and the pulled-tight line stays on polygons a car may use.
    #[test]
    fn link_lengths_and_portal_sides() {
        let Some(res) = file() else { return };
        let m = NavMesh::parse(&res).unwrap();
        let (mut n, mut err, mut left, mut within) = (0usize, 0.0f64, 0usize, 0usize);
        for (i, p) in m.polys.iter().enumerate() {
            for l in m.links_of(i as u32) {
                if l.span != u32::MAX {
                    continue;
                }
                let d = m.polys[l.poly as usize].center - p.center;
                let e = (l.len - d.length()).abs();
                err += e as f64;
                within += (e < 0.06) as usize;
                // y up: the left of the direction d is y x d
                left += (Vec3::Y.cross(d).dot(l.pa - p.center) > 0.0) as usize;
                n += 1;
            }
        }
        println!("{n} links: mean length error {:.4} m, {within} within 0.06 m, A left in {left}", err / n as f64);
        assert!(err / (n as f64) < 0.05);
        assert!(within as f32 > n as f32 * 0.9);
        assert!(left as f32 > n as f32 * 0.99, "A on the left in {left} of {n}");
    }

    #[test]
    fn quickest_path_over_roads() {
        let Some(res) = file() else { return };
        let m = NavMesh::parse(&res).unwrap();
        let n = m.polys.len() as u32;
        let roads: Vec<u32> = (0..n).filter(|&i| m.polys[i as usize].flags == 0x82).collect();
        let start = roads[roads.len() / 3];
        let from = m.polys[start as usize].center;
        let goal = roads
            .iter()
            .copied()
            .find(|&g| {
                let d = m.polys[g as usize].center.distance(from);
                d > 300.0 && d < 500.0
            })
            .unwrap();
        let to = m.polys[goal as usize].center;
        assert!(m.on_mesh(from) && m.teleport_target(from) == Some(from));
        let cost = racer_cost(0x82, 0x82);
        let q = m.quickest_path(from, to, &cost).unwrap();
        assert!(q.reached && q.goal == to, "not reached");
        assert!(q.portals.len() > 5);
        let first = &q.portals[0];
        assert!(m.point_polys(from).contains(&first.left));
        for w in q.portals.windows(2) {
            if w[0].entered < n {
                assert_eq!(w[0].entered, w[1].left);
            } else {
                let s = &m.span_recs[(w[0].entered - n) as usize];
                assert!(w[1].left == s.pa || w[1].left == s.pb);
            }
        }
        assert!(m.point_polys(to).contains(&q.portals.last().unwrap().entered));
        // each portal is an edge of the polygon it leaves
        for pt in &q.portals {
            let p = &m.polys[pt.left as usize];
            let hit = |x: Vec3| p.points().iter().any(|v| v.distance(x) < 0.12);
            assert!(hit(pt.a) && hit(pt.b));
        }
        let road = q.portals.iter().filter(|p| p.flags & 2 != 0).count();
        println!("{} portals, {} on road, {} to goal", q.portals.len(), road, q.portals.len());
        assert!(road as f32 > q.portals.len() as f32 * 0.9);
        // with roads as dear as the rest the way is no cheaper, with roads cheap it is no longer
        let flat = m.quickest_path(from, to, &|_| 1.0).unwrap();
        assert!(flat.reached);
        // a goal on the far side of nothing: the closest node is returned
        let far = Vec3::new(from.x + 20000.0, from.y, from.z);
        let _ = m.quickest_path(from, far, &cost).map(|q| assert!(!q.reached));
        // off-mesh points are put back
        let off = from + Vec3::new(0.0, 5.0, 0.0);
        assert!(m.on_mesh(off));
        assert_eq!(m.teleport_target(off).unwrap(), m.polys[m.nearest_poly(off, 10.0).unwrap() as usize].center);
    }
}
