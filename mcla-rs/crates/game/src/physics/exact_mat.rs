//! Rotation and matrix routines of the original written out operation for operation, in single precision and in its
//! order (the scalar fused operations of `exact`), so that the matrices the port draws from carry the bits the
//! original's carry. Each was run whole in the generated code offline and compared word for word: tests below, data
//! in testdata/ (tools/engine_quat_reference.py, tools/inverse4_reference.py, tools/skeleton_fk_reference.py).
//!
//! The guest's 4 x 4 is row-vector (rows 0-2 the axes, row 3 the place); a glam `Mat4` of the same memory has the rows
//! as its columns, so `Mat4::from_cols(row0, row1, row2, row3)` is the guest's matrix and no transpose is made.

use super::artic::V4;
use super::artic_pose::mmul;
use super::exact::{fms, fnms};
use glam::{Mat4, Quat, Vec3, Vec4};

/// The root of two the original scales a quaternion by (F-40856) so that its products come out as `2 a b`.
const SQRT2: f32 = f32::from_bits(0x3fb5_04f3);

/// F-40856: the three rows (3 words each) of the rotation of the quaternion `q` = (x, y, z, w), not normalised.
pub fn quat_rows(q: [f32; 4]) -> [[f32; 3]; 3] {
    let (x, y, z, w) = (q[0] * SQRT2, q[1] * SQRT2, q[2] * SQRT2, q[3] * SQRT2);
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (xy, xz, wy, wz, yz, wx) = (y * x, z * x, w * y, w * z, z * y, w * x);
    let (a, b, c) = (yy + xx, zz + yy, zz + xx);
    [[1.0 - b, xy + wz, xz - wy], [xy - wz, 1.0 - c, wx + yz], [wy + xz, yz - wx, 1.0 - a]]
}

/// F-22579: the matrix of a pose, a quaternion `q` and a position `p` (four lanes, copied as they are): rows 0-2 are
/// [`quat_rows`] (their w lanes are not written by the original and are 0 here), row 3 the position.
pub fn pose_rows(q: [f32; 4], p: [f32; 4]) -> [[f32; 4]; 4] {
    let r = quat_rows(q);
    [[r[0][0], r[0][1], r[0][2], 0.0], [r[1][0], r[1][1], r[1][2], 0.0], [r[2][0], r[2][1], r[2][2], 0.0], p]
}

/// F-22579 for the port's `Mat4`: the guest's matrix of rotation `q` and place `t` (w lane of the place 1).
pub fn pose_matrix(q: Quat, t: Vec3) -> Mat4 {
    let r = pose_rows([q.x, q.y, q.z, q.w], [t.x, t.y, t.z, 1.0]);
    Mat4::from_cols(Vec4::from(r[0]), Vec4::from(r[1]), Vec4::from(r[2]), Vec4::from(r[3]))
}

/// F-50812 (under F-16377): the quaternion (x, y, z, w) of a rotation given by its rows `m`. The largest of the trace and
/// the diagonal picks the branch (a NaN compares false, as `fcmpu` + `blt`); no normalisation.
pub fn quat_of_rows(m: &[[f32; 3]; 3]) -> [f32; 4] {
    let (m00, m11, m22) = (m[0][0], m[1][1], m[2][2]);
    let t = m00 + m11 + m22;
    // (the original's tests, in its order: 0 trace, 1 x, 2 y, 3 z)
    let case = if t < m00 {
        if m00 < m11 {
            if m11 < m22 { 3 } else { 2 }
        } else if m00 < m22 {
            3
        } else {
            1
        }
    } else if t < m11 {
        if m11 < m22 { 3 } else { 2 }
    } else if t < m22 {
        3
    } else {
        0
    };
    let k = 0.5f32;
    let q = 0.25f32;
    match case {
        0 => {
            let w = (t + 1.0).sqrt() * k;
            let r = q / w;
            [(m[1][2] - m[2][1]) * r, (m[2][0] - m[0][2]) * r, (m[0][1] - m[1][0]) * r, w]
        }
        1 => {
            let x = (fms(m00, 2.0, t) + 1.0).sqrt() * k;
            let r = q / x;
            [x, (m[1][0] + m[0][1]) * r, (m[2][0] + m[0][2]) * r, (m[1][2] - m[2][1]) * r]
        }
        2 => {
            let y = (fms(m11, 2.0, t) + 1.0).sqrt() * k;
            let r = q / y;
            [(m[1][0] + m[0][1]) * r, y, (m[2][1] + m[1][2]) * r, (m[2][0] - m[0][2]) * r]
        }
        _ => {
            let z = (fms(m22, 2.0, t) + 1.0).sqrt() * k;
            let r = q / z;
            [(m[2][0] + m[0][2]) * r, (m[2][1] + m[1][2]) * r, z, (m[0][1] - m[1][0]) * r]
        }
    }
}

/// F-44424: four rows from a quaternion: rows 0-2 `1 - 2 a a` on the diagonal and `-2 a b` off it (a, b in x y z), row 3
/// `-2 w (x y z)` (the w lanes are stack leftovers in the original and are not returned).
pub fn reflect_rows(q: [f32; 4]) -> [[f32; 3]; 4] {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (xy, xz, yz) = (y * x, z * x, z * y);
    // (the diagonal is fnmsubs(a a, 2, 1), the others products by -2)
    [
        [fnms(xx, 2.0, 1.0), xy * -2.0, xz * -2.0],
        [xy * -2.0, fnms(yy, 2.0, 1.0), yz * -2.0],
        [xz * -2.0, yz * -2.0, fnms(zz, 2.0, 1.0)],
        [x * w * -2.0, y * w * -2.0, z * w * -2.0],
    ]
}

fn v4_of(c: Vec4) -> V4 {
    V4(c.to_array())
}

fn mat_rows(m: &Mat4) -> [V4; 4] {
    [v4_of(m.x_axis), v4_of(m.y_axis), v4_of(m.z_axis), v4_of(m.w_axis)]
}

fn mat_from_rows(r: [V4; 4]) -> Mat4 {
    Mat4::from_cols(Vec4::from(r[0].0), Vec4::from(r[1].0), Vec4::from(r[2].0), Vec4::from(r[3].0))
}

/// F-97374's product for the port's `Mat4`: the guest's `local` x `parent` (a bone's global from its local and its
/// parent's), the vector unit's unfused multiply-add chain ([`super::artic_pose::mmul`]); in the glam order this is
/// `parent * local`. Checked against F-97374 run offline (`artic_pose::tests::fk_matches_the_original`).
pub fn chain(local: &Mat4, parent: &Mat4) -> Mat4 {
    mat_from_rows(mmul(&mat_rows(local), &mat_rows(parent)))
}

/// F-96265's draw matrix of a bone (the path the port draws: bone rotation matrix `m` [the identity when the bind pose has
/// none], bind position `bind`): rows 0-2 = `m` x `g` (row i: m_i0 g0 + m_i1 g1 + m_i2 g2, each product rounded, summed left
/// to right), row 3 = `g`'s row 3 - the three-lane dots of `bind` with the columns of those rows. In the port's
/// `world * translate(-bind)` this is the translation column. Lane w of every row is `g`'s own.
pub fn bind_skin_with(g: &Mat4, m: &Mat4, bind: Vec3) -> Mat4 {
    let (gr, mr) = (mat_rows(g), mat_rows(m));
    let row = |i: usize| gr[0].mul(mr[i].lane(0)).add(gr[1].mul(mr[i].lane(1))).add(gr[2].mul(mr[i].lane(2)));
    let (r0, r1, r2) = (row(0), row(1), row(2));
    let b = V4::new(bind.x, bind.y, bind.z, 0.0);
    let col = |c: usize| V4::new(r0.0[c], r1.0[c], r2.0[c], 0.0);
    let d = V4::new(b.dot3(col(0)).x(), b.dot3(col(1)).x(), b.dot3(col(2)).x(), 0.0);
    let r3 = gr[3].sub(d);
    mat_from_rows([V4([r0.0[0], r0.0[1], r0.0[2], gr[0].0[3]]), V4([r1.0[0], r1.0[1], r1.0[2], gr[1].0[3]]), V4([r2.0[0], r2.0[1], r2.0[2], gr[2].0[3]]), V4([r3.0[0], r3.0[1], r3.0[2], gr[3].0[3]])])
}

/// [`bind_skin_with`] for a bind pose without rotations (the port's skeletons): the bone matrix is the identity.
pub fn bind_skin(g: &Mat4, bind: Vec3) -> Mat4 {
    bind_skin_with(g, &Mat4::IDENTITY, bind)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-96265 run offline (tools/skin_matrices_reference.py): 1,500 skeletons of 1 to 8 bones, the bone matrices the
    /// identity or a rotation, bind positions random; the object's flag word has bits 3 and 2 set and no second set
    /// (the path with the bone rotations): xyz of every row equal word for word (lane w of the original's rows is not
    /// written or is a leftover). The flags 0 / 4 / 8 and a second matrix set are other code paths, not compared.
    #[test]
    fn bind_skin_matches_the_original() {
        let mut bones = 0;
        for line in include_str!("../testdata/skin_matrices_ref.txt").lines() {
            let (case, want) = line.split_once(" -> ").unwrap();
            let t: Vec<&str> = case.split(' ').collect();
            let n: usize = t[0].parse().unwrap();
            let w: Vec<f32> = t[2..].iter().map(|x| f32::from_bits(h(x))).collect();
            let want: Vec<u32> = want.split_whitespace().map(h).collect();
            for k in 0..n {
                let m16 = |o: usize| -> Mat4 { Mat4::from_cols_slice(&w[o + 16 * k..o + 16 * k + 16]) };
                let g = m16(0);
                let m = m16(16 * n);
                let b = &w[32 * n + 4 * k..32 * n + 4 * k + 4];
                let got = bind_skin_with(&g, &m, Vec3::new(b[0], b[1], b[2]));
                for (r, row) in [got.x_axis, got.y_axis, got.z_axis, got.w_axis].iter().enumerate() {
                    for l in 0..3 {
                        assert_eq!(nan(row[l].to_bits()), nan(want[16 * k + 4 * r + l]), "{line} bone {k} row {r} lane {l}");
                    }
                }
                bones += 1;
            }
        }
        assert_eq!(bones, 5181);
    }

    fn h(x: &str) -> u32 {
        u32::from_str_radix(x, 16).unwrap()
    }
    /// A NaN is compared only as a NaN (the harness's default NaN is not the console's).
    fn nan(x: u32) -> u32 {
        if f32::from_bits(x).is_nan() { 0x7fc0_0000 } else { x }
    }

    /// F-86566 (rotation of a vector), F-22579 / F-40856 (pose to rows), F-16377 / F-50812 (rows to quaternion) and F-44424
    /// run offline (tools/engine_quat_reference.py): 3,000 + 3,000 + 6,000 + 3,000 cases (unit and not, zero, NaN and
    /// infinite lanes, the identity and axes, matrices a little off square and ties of the trace and diagonal).
    #[test]
    fn quaternion_rows_match_the_original() {
        let mut n = [0usize; 4];
        for line in include_str!("../testdata/engine_quat_ref.txt").lines() {
            let (case, want) = line.split_once(" -> ").unwrap();
            let mut it = case.split(' ');
            let kind = it.next().unwrap();
            let c: Vec<f32> = it.map(|x| f32::from_bits(h(x))).collect();
            let want: Vec<u32> = want.split(' ').map(h).collect();
            let got: Vec<u32> = match kind {
                "rot" => {
                    let v = crate::peds::quat_rotate(Quat::from_xyzw(c[0], c[1], c[2], c[3]), Vec3::new(c[4], c[5], c[6]));
                    n[0] += 1;
                    vec![v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]
                }
                "pose" => {
                    let r = pose_rows([c[0], c[1], c[2], c[3]], [c[4], c[5], c[6], c[7]]);
                    n[1] += 1;
                    r[0][..3].iter().chain(&r[1][..3]).chain(&r[2][..3]).chain(&r[3]).map(|x| x.to_bits()).collect()
                }
                "quat" => {
                    let m = [[c[0], c[1], c[2]], [c[4], c[5], c[6]], [c[8], c[9], c[10]]];
                    n[2] += 1;
                    quat_of_rows(&m).iter().chain(&c[12..16]).map(|x| x.to_bits()).collect()
                }
                _ => {
                    let r = reflect_rows([c[0], c[1], c[2], c[3]]);
                    n[3] += 1;
                    r.iter().flatten().map(|x| x.to_bits()).collect()
                }
            };
            assert_eq!(got.iter().map(|&x| nan(x)).collect::<Vec<_>>(), want.iter().map(|&x| nan(x)).collect::<Vec<_>>(), "{line}");
        }
        assert_eq!(n, [3000, 3000, 6000, 3000]);
    }

    #[test]
    fn pose_matrix_is_the_rows_as_columns() {
        let q = Quat::from_xyzw(0.1, 0.2, 0.3, 0.9).normalize();
        let m = pose_matrix(q, Vec3::new(1.0, 2.0, 3.0));
        let r = quat_rows([q.x, q.y, q.z, q.w]);
        assert_eq!(m.x_axis.truncate().to_array(), r[0]);
        assert_eq!(m.w_axis.to_array(), [1.0, 2.0, 3.0, 1.0]);
    }
}
