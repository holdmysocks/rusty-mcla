//! A draw list's clip-depth masks resolved into geometry: the original's stencil (`F-92070`, `F-90239`, `F-71617`)
//! as clipped triangles, for hosts that draw without a stencil buffer.
//!
//! The original marks a mask's pixels with a new stencil reference (`F-90239`: a 6-bit counter + 128, op REPLACE,
//! function ALWAYS, colour writes off) and then draws with function EQUAL on that reference (`F-71617`), so the
//! content shows only where the last mask drew. [`Item::MaskOff`] turns the test off. Here the last mask is the union
//! of its triangles: a content triangle is cut by each mask triangle in turn, the part inside kept and the parts
//! outside carried on to the next, so the kept pieces do not overlap. Coverage of pixel centres
//! is the stencil's, except for centres lying exactly on a mask edge; a vertex made by a cut takes its colour rounded
//! to a byte (the GPU interpolates it unrounded).

use crate::draw::{DrawList, Item, Vertex};
use std::borrow::Cow;

/// A convex polygon with the vertex attributes interpolated along its edges.
type Poly = Vec<Vertex>;

/// The triangles of `list` that the original shows, with masks cut out (only `Item::Triangles` in the result; a list
/// without masks as it is).
pub fn resolve(list: &DrawList) -> Cow<'_, DrawList> {
    if !list.items.iter().any(|i| matches!(i, Item::MaskBegin | Item::MaskEnd | Item::MaskOff)) {
        return Cow::Borrowed(list);
    }
    let mut out = DrawList::default();
    // colour writes (off while a mask marks), the stencil test / mark enabled, the last mask's pieces
    let (mut colour, mut on) = (true, false);
    let mut region: Vec<Poly> = Vec::new();
    let mut remaining: Vec<Poly> = Vec::new();
    for item in &list.items {
        match item {
            Item::MaskBegin => {
                colour = false;
                on = true;
                region.clear();
            }
            Item::MaskEnd => {
                colour = true;
                on = true;
            }
            Item::MaskOff => on = false,
            Item::Triangles { tex, kind, verts } => {
                if !colour {
                    if on {
                        region.extend(verts.chunks_exact(3).filter(|t| area(t).abs() > 1e-9).map(|t| t.to_vec()));
                    }
                    continue;
                }
                if !on {
                    out.items.push(item.clone());
                    continue;
                }
                let mut kept = Vec::new();
                for t in verts.chunks_exact(3) {
                    remaining.clear();
                    remaining.push(t.to_vec());
                    for m in &region {
                        if remaining.is_empty() {
                            break;
                        }
                        let mb = bbox(m);
                        let mut next = Vec::with_capacity(remaining.len());
                        for r in remaining.drain(..) {
                            let rb = bbox(&r);
                            if rb[0] >= mb[2] || rb[2] <= mb[0] || rb[1] >= mb[3] || rb[3] <= mb[1] {
                                next.push(r);
                                continue;
                            }
                            // the part outside each edge of m goes on; what is left at the end is inside m
                            let mut rest = r;
                            for (a, b) in edges(m) {
                                let out = cut_side(&rest, a, b, false);
                                if out.len() >= 3 && area(&out).abs() > 1e-9 {
                                    next.push(out);
                                }
                                rest = cut_side(&rest, a, b, true);
                                if rest.len() < 3 || area(&rest).abs() <= 1e-9 {
                                    rest.clear();
                                    break;
                                }
                            }
                            if rest.len() >= 3 {
                                for k in 1..rest.len() - 1 {
                                    kept.extend_from_slice(&[rest[0], rest[k], rest[k + 1]]);
                                }
                            }
                        }
                        remaining = next;
                    }
                }
                if !kept.is_empty() {
                    out.items.push(Item::Triangles { tex: *tex, kind: *kind, verts: kept });
                }
            }
        }
    }
    Cow::Owned(out)
}

fn area(p: &[Vertex]) -> f32 {
    let mut a = 0.0;
    for i in 0..p.len() {
        let (u, v) = (p[i].pos, p[(i + 1) % p.len()].pos);
        a += u[0] * v[1] - v[0] * u[1];
    }
    a * 0.5
}

/// The edges of a convex polygon as (a, b) with the inside on the left when the polygon is counter-clockwise; a
/// clockwise one is walked backwards.
fn edges(p: &[Vertex]) -> Vec<([f32; 2], [f32; 2])> {
    let ccw = area(p) >= 0.0;
    // (an edge of no length, where a cut ran through a corner, bounds nothing: left out)
    (0..p.len())
        .map(|i| (p[i].pos, p[(i + 1) % p.len()].pos))
        .filter(|(a, b)| (b[0] - a[0]).abs() + (b[1] - a[1]).abs() > 1e-6)
        .map(|(a, b)| if ccw { (a, b) } else { (b, a) })
        .collect()
}

/// Signed side of `q` against the line a -> b (> 0 on the left).
fn side(a: [f32; 2], b: [f32; 2], q: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0])
}

fn lerp(u: &Vertex, v: &Vertex, t: f32) -> Vertex {
    let l = |x: f32, y: f32| x + (y - x) * t;
    let c = |x: u8, y: u8| l(x as f32, y as f32).round().clamp(0.0, 255.0) as u8;
    Vertex {
        pos: [l(u.pos[0], v.pos[0]), l(u.pos[1], v.pos[1])],
        uv: [l(u.uv[0], v.uv[0]), l(u.uv[1], v.uv[1])],
        color: [c(u.color[0], v.color[0]), c(u.color[1], v.color[1]), c(u.color[2], v.color[2]), c(u.color[3], v.color[3])],
    }
}

/// `p` cut to the left of a -> b (Sutherland-Hodgman, one edge); `keep_left` false keeps the right side.
fn cut_side(p: &[Vertex], a: [f32; 2], b: [f32; 2], keep_left: bool) -> Poly {
    let mut out = Vec::with_capacity(p.len() + 1);
    let s = |v: &Vertex| if keep_left { side(a, b, v.pos) } else { -side(a, b, v.pos) };
    for i in 0..p.len() {
        let (u, v) = (&p[i], &p[(i + 1) % p.len()]);
        let (su, sv) = (s(u), s(v));
        if su >= 0.0 {
            out.push(*u);
        }
        if (su >= 0.0) != (sv >= 0.0) {
            out.push(lerp(u, v, su / (su - sv)));
        }
    }
    out
}

fn bbox(p: &[Vertex]) -> [f32; 4] {
    let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for v in p {
        r = [r[0].min(v.pos[0]), r[1].min(v.pos[1]), r[2].max(v.pos[0]), r[3].max(v.pos[1])];
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::TexRef;

    fn v(x: f32, y: f32) -> Vertex {
        Vertex { pos: [x, y], uv: [x / 10.0, y / 10.0], color: [255, 255, 255, 255] }
    }

    fn tris(t: &[[f32; 2]]) -> Item {
        Item::Triangles { tex: TexRef::None, kind: 0, verts: t.iter().map(|p| v(p[0], p[1])).collect() }
    }

    fn covered(list: &DrawList, x: f32, y: f32) -> bool {
        list.items.iter().any(|it| match it {
            Item::Triangles { verts, .. } => verts.chunks_exact(3).any(|t| {
                let (a, b, c) = (t[0].pos, t[1].pos, t[2].pos);
                let (d1, d2, d3) = (side(a, b, [x, y]), side(b, c, [x, y]), side(c, a, [x, y]));
                (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0)
            }),
            _ => false,
        })
    }

    /// The resolved list against a stencil kept per point, on a grid, for overlapping masks, an L-shaped mask
    /// and the test turned off and on again.
    #[test]
    fn resolve_matches_a_stencil_on_a_grid() {
        let quad = |x0: f32, y0: f32, x1: f32, y1: f32| tris(&[[x0, y0], [x1, y0], [x0, y1], [x1, y0], [x1, y1], [x0, y1]]);
        let list = DrawList {
            items: vec![
                quad(0.0, 0.0, 50.0, 50.0),
                Item::MaskBegin,
                // an L of two overlapping quads, one twice, and a slanted triangle
                quad(10.0, 10.0, 30.0, 20.0),
                quad(10.0, 10.0, 20.0, 40.0),
                quad(10.0, 10.0, 20.0, 40.0),
                tris(&[[25.0, 25.0], [45.0, 30.0], [30.0, 45.0]]),
                Item::MaskEnd,
                quad(0.0, 0.0, 50.0, 50.0),
                Item::MaskOff,
                quad(60.0, 0.0, 70.0, 10.0),
                Item::MaskEnd,
                quad(60.0, 0.0, 70.0, 10.0),
            ],
        };
        let r = resolve(&list);
        let inside_mask = |x: f32, y: f32| {
            (x > 10.0 && x < 30.0 && y > 10.0 && y < 20.0)
                || (x > 10.0 && x < 20.0 && y > 10.0 && y < 40.0)
                || {
                    let (a, b, c) = ([25.0, 25.0], [45.0, 30.0], [30.0, 45.0]);
                    let (d1, d2, d3) = (side(a, b, [x, y]), side(b, c, [x, y]), side(c, a, [x, y]));
                    (d1 > 0.0 && d2 > 0.0 && d3 > 0.0) || (d1 < 0.0 && d2 < 0.0 && d3 < 0.0)
                }
        };
        let mut n = 0;
        for i in 0..80 {
            for j in 0..50 {
                let (x, y) = (i as f32 + 0.37, j as f32 + 0.61);
                // triangles over (x, y): the first quad always, the masked quad inside the mask; at 60 the quad after
                // MaskOff unmasked, the one after the second MaskEnd tested against the same mask (none of it inside)
                let mut want = 0;
                if x < 50.0 && y < 50.0 {
                    want += 1 + inside_mask(x, y) as i32;
                }
                if x > 60.0 && x < 70.0 && y < 10.0 {
                    want += 1;
                }
                let got: i32 = r
                    .items
                    .iter()
                    .map(|it| match it {
                        Item::Triangles { verts, .. } => verts
                            .chunks_exact(3)
                            .filter(|t| {
                                let (a, b, c) = (t[0].pos, t[1].pos, t[2].pos);
                                let (d1, d2, d3) = (side(a, b, [x, y]), side(b, c, [x, y]), side(c, a, [x, y]));
                                (d1 > 0.0 && d2 > 0.0 && d3 > 0.0) || (d1 < 0.0 && d2 < 0.0 && d3 < 0.0)
                            })
                            .count() as i32,
                        _ => 0,
                    })
                    .sum();
                assert_eq!(got, want, "at ({x}, {y})");
                n += 1;
            }
        }
        assert_eq!(n, 4000);
        assert!(covered(&r, 15.0, 35.0) && !covered(&r, 75.0, 35.0));
    }
}
