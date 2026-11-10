//! Small routines of the loose-object simulation in the form they have in the original, on words of guest memory: the
//! port keeps these structures as Rust vectors and fields, and these are the word-level statements of the routines that
//! its code stands for, compared with the generated code run offline (`loose_small_tests`, tools/loose_small_reference.py).

use super::gjk::world_matrix;
use super::loose::{BODY_MAX_SPEED, BODY_MAX_SPIN};
use glam::Vec3;

/// Guest memory by address: one word at a time.
pub trait Words {
    fn get(&self, a: u32) -> u32;
    fn set(&mut self, a: u32, v: u32);

    /// The byte at `a` (the words are the guest's big-endian ones).
    fn get_u8(&self, a: u32) -> u8 {
        (self.get(a & !3) >> (24 - 8 * (a & 3))) as u8
    }

    fn set_u8(&mut self, a: u32, v: u8) {
        let shift = 24 - 8 * (a & 3);
        let w = self.get(a & !3) & !(0xFF << shift) | (v as u32) << shift;
        self.set(a & !3, w);
    }

    /// The halfword at `a` (even).
    fn set_u16(&mut self, a: u32, v: u16) {
        let shift = 16 - 8 * (a & 2);
        let w = self.get(a & !3) & !(0xFFFF << shift) | (v as u32) << shift;
        self.set(a & !3, w);
    }

    /// Four words (a vector register's store).
    fn set4(&mut self, a: u32, v: [f32; 4]) {
        for (k, x) in v.iter().enumerate() {
            self.set(a + 4 * k as u32, x.to_bits());
        }
    }
}

/// F-19844: a contact's other side - the manifold's side B where side A is the instance, else side A.
pub fn other_side(a: u32, b: u32, inst: u32) -> u32 {
    if a == inst { b } else { a }
}

/// F-59304: node `node` goes after the list's tail (head +0, tail +4, count +8; a node's prev +0, next +4). An empty
/// list (head 0) takes it as head and tail; otherwise it is linked behind the tail and takes over the tail's next.
pub fn list_append<W: Words>(w: &mut W, list: u32, node: u32) {
    let head = w.get(list);
    if head == 0 {
        w.set(list, node);
        w.set(list + 4, node);
        w.set(node + 4, 0);
        w.set(node, 0);
    } else {
        let tail = w.get(list + 4);
        let next = w.get(tail + 4);
        w.set(node + 4, next);
        if next != 0 {
            w.set(next, node);
            w.set(tail + 4, node);
            w.set(node, tail);
        } else {
            w.set(list + 4, node);
            w.set(tail + 4, node);
            w.set(node, tail);
        }
    }
    w.set(list + 8, w.get(list + 8).wrapping_add(1));
}

/// F-69979: node `node` goes in front of the list's head.
pub fn list_prepend<W: Words>(w: &mut W, list: u32, node: u32) {
    let head = w.get(list);
    if head == 0 {
        w.set(list, node);
        w.set(list + 4, node);
        w.set(node + 4, 0);
        w.set(node, 0);
    } else {
        let before = w.get(head);
        w.set(node, before);
        if before != 0 {
            w.set(before + 4, node);
            w.set(head, node);
            w.set(node + 4, head);
        } else {
            w.set(list, node);
            w.set(head, node);
            w.set(node + 4, head);
        }
    }
    w.set(list + 8, w.get(list + 8).wrapping_add(1));
}

/// F-68557: node `node` leaves the list: its neighbours are joined (the head or the tail follows), its links are cleared
/// and the count falls by one. Gives the node's old next.
pub fn list_unlink<W: Words>(w: &mut W, list: u32, node: u32) -> u32 {
    let (prev, next) = (w.get(node), w.get(node + 4));
    if prev == 0 {
        w.set(list, next);
        let n = w.get(node + 4);
        if n != 0 {
            w.set(n, 0);
        } else {
            w.set(list + 4, 0);
        }
    } else {
        w.set(prev + 4, next);
        let n = w.get(node + 4);
        if n != 0 {
            let p = w.get(node);
            w.set(n, p);
        } else {
            let p = w.get(node);
            w.set(list + 4, p);
        }
    }
    w.set(node + 4, 0);
    w.set(node, 0);
    w.set(list + 8, w.get(list + 8).wrapping_sub(1));
    next
}

/// F-51875: node `node` (parent at +28, children's list at +16) is given the parent `parent`: it leaves its old parent's
/// children (or the root list `root`), is appended to the new parent's children, or - no parent - put in front of the
/// root list unless it is in it already (a link, or the head).
pub fn list_move<W: Words>(w: &mut W, root: u32, node: u32, parent: u32) {
    let old = w.get(node + 28);
    if old == parent {
        return;
    }
    if old != 0 {
        list_unlink(w, old + 16, node);
    } else if parent != 0 {
        list_unlink(w, root, node);
    }
    w.set(node + 28, parent);
    if parent != 0 {
        list_append(w, parent + 16, node);
        return;
    }
    let in_list = w.get(node + 4) != 0 || w.get(node) != 0 || w.get(root) == node;
    if !in_list {
        list_prepend(w, root, node);
    }
}

/// F-21340 / F-17349: the matrix of one side of a manifold: the instance's rows, those of the composite bound's part
/// (a type 12 bound) carried by them, the identity where there is no instance. (Three rows' xyz: the port's `Vec3`.)
pub fn side_matrix(inst: Option<&[Vec3; 4]>, part: Option<&[Vec3; 4]>) -> [Vec3; 4] {
    match (inst, part) {
        (None, _) => [Vec3::X, Vec3::Y, Vec3::Z, Vec3::ZERO],
        (Some(m), None) => *m,
        (Some(m), Some(p)) => world_matrix(p, m),
    }
}

/// F-33655: a body's constructor sets (`vtable` is the class's table, a pointer the port replaces by a type): mass and its
/// inverse 1, inertia and its inverse (1, 1, 1; the fourth lane 1 too), the speed and spin limits, the three rows of its
/// frame as the identity (and a zero fourth), a zero position (+16 .. +24), and zero for the velocities, momenta, force
/// and torque sums, the pending impulses and pushes (vectors from +272 to +336, +400, +432) and the three flag bytes at +658.
/// Then its slot 4 (an initialisation) is called on it.
pub fn body_defaults<W: Words>(w: &mut W, obj: u32, vtable: u32) {
    use super::loose::set_inertia;
    w.set(obj, vtable);
    w.set(obj + 192, 1.0f32.to_bits());
    w.set(obj + 196, 1.0f32.to_bits());
    let (i, inv) = set_inertia([1.0, 1.0, 1.0]);
    w.set4(obj + 160, [i[0], i[1], i[2], 1.0]);
    w.set4(obj + 176, [inv[0], inv[1], inv[2], 1.0]);
    w.set(obj + 200, BODY_MAX_SPEED.to_bits());
    w.set(obj + 204, BODY_MAX_SPIN.to_bits());
    w.set4(obj + 208, [1.0, 0.0, 0.0, 0.0]);
    w.set4(obj + 224, [0.0, 1.0, 0.0, 0.0]);
    w.set4(obj + 240, [0.0, 0.0, 1.0, 0.0]);
    w.set4(obj + 256, [0.0; 4]);
    for o in [16, 20, 24] {
        w.set(obj + o, 0);
    }
    for o in [272, 288, 304, 320, 400, 432] {
        w.set4(obj + o, [0.0; 4]);
    }
    for o in [658, 659, 660] {
        w.set_u8(obj + o, 0);
    }
}

/// F-31401: a distance constraint's defaults: zero for the links (+16 .. +28), the two corner vectors (0, 0, 0, 1) at +48 and
/// +64 and zero from +80 to +156, its length 0, its depth (the port's `rope_depth`), 60 (+168) and zero around them,
/// flags 2 (+200, a halfword) and 3 (+240), the rows (1, 0, 0, 0) at +208 and +224, zero from +256 to +316, a byte 1 at
/// +320 and 0 at +321, and -1 (+328: it never breaks) with 0 behind.
pub fn constraint_defaults<W: Words>(w: &mut W, c: u32, vtable: u32, rope_depth: f32) {
    w.set(c, vtable);
    for o in [16, 20, 24, 28, 188, 192, 204, 332] {
        w.set(c + o, 0);
    }
    w.set4(c + 48, [0.0, 0.0, 0.0, 1.0]);
    w.set4(c + 64, [0.0, 0.0, 0.0, 1.0]);
    for o in [80, 96, 112, 128, 144, 256, 272, 288, 304] {
        w.set4(c + o, [0.0; 4]);
    }
    for o in [160, 172, 176, 180, 184] {
        w.set(c + o, 0);
    }
    w.set(c + 164, rope_depth.to_bits());
    w.set(c + 168, 60.0f32.to_bits());
    w.set_u16(c + 200, 2);
    w.set4(c + 208, [1.0, 0.0, 0.0, 0.0]);
    w.set4(c + 224, [1.0, 0.0, 0.0, 0.0]);
    w.set(c + 240, 3);
    w.set_u8(c + 320, 1);
    w.set_u8(c + 321, 0);
    w.set(c + 328, (-1.0f32).to_bits());
}

/// F-82734: is `collider` one of the manager's (its index - the distance from the manager's base over 672, the signed
/// division, below the count) with the low bit of its flag byte set?
pub fn registered<W: Words>(w: &W, manager: u32, collider: u32) -> bool {
    let idx = (collider.wrapping_sub(w.get(manager + 32)) as i32) / 672;
    if idx as u32 >= w.get(manager + 24) {
        return false;
    }
    w.get_u8(w.get(manager + 36).wrapping_add(idx as u32)) & 1 != 0
}

/// F-50834: do all the colliders of an island - its constraints' (+12 list, count +20) and both of each contact's (+4
/// list, count +52, the manifold at +160 holds them at +8 and +12; a missing one is skipped) - rest: not registered with
/// the manager, with a rest object (+24) in state 2 (+24) whose byte +40 is 0?
pub fn island_rests<W: Words>(w: &W, island: u32, manager: u32) -> bool {
    let rests = |c: u32| {
        let rest = w.get(c + 24);
        !registered(w, manager, c) && rest != 0 && w.get(rest + 24) as i32 == 2 && w.get_u8(rest + 40) == 0
    };
    for i in 0..w.get(island + 20) as i32 {
        if !rests(w.get(w.get(island + 12).wrapping_add(4 * i as u32))) {
            return false;
        }
    }
    for k in 0..w.get(island + 52) as i32 {
        let contact = w.get(w.get(island + 4).wrapping_add(4 * k as u32));
        let manifold = w.get(contact + 160);
        for c in [w.get(manifold + 8), w.get(manifold + 12)] {
            if c != 0 && !rests(c) {
                return false;
            }
        }
    }
    true
}

/// F-49276: the mass an entry holds: each child (mass, damaged mass, group, has a damaged form) whose group has not
/// left, its mass - or, where the group's health is not above 0 and the child has a damaged form, its damaged mass -
/// added one after another.
pub fn entry_mass_health(children: &[(f32, f32, u8, bool)], absent: u64, health: &[f32]) -> f32 {
    let mut m = 0.0f32;
    for &(mass, damaged, group, has_damaged) in children {
        if absent & (1 << group) == 0 {
            let x = if health[group as usize] > 0.0 || !has_damaged { mass } else { damaged };
            m = x + m;
        }
    }
    m
}

/// F-43430 (the limit at +60): the smaller of the two instances' slot 9 answers - a side without an instance gives the
/// largest number - by the difference's sign (a NaN difference keeps A's).
pub fn contact_limit(a: Option<f32>, b: Option<f32>) -> f32 {
    let (x, y) = (a.unwrap_or(f32::MAX), b.unwrap_or(f32::MAX));
    if x - y >= 0.0 { y } else { x }
}
