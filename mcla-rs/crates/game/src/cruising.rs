//! mcBrainCruising (docs/reference/ai/brain_cruising.md): the brain of the scripted cruising cars. Normally the car is
//! handed to the ambient traffic as a ghost on the lane rails and the physical car follows it; far off its ghost or off
//! the rails it drives with the racing brain to the nearest rail.

use crate::aiv::{ang_xz, ang_xz_unit, norm_or};
use crate::physics::exact::{fma, guest_atan2};
use glam::Vec3;

/// The brain's own fields the scripts set (brain_cruising.md section 1): the destinations (+560, 16 B each, count
/// +1584), the current one (+1656, -1: none), looping (+1660), the offset in the rail's frame (+1632), SetSpeed
/// (+1664) and SetDestSpeed (+1668), and the state (+1708).
#[derive(Clone, Debug)]
pub struct Cruising {
    pub dests: [Vec3; 64],
    pub count: i32,
    pub idx: i32,
    pub looping: bool,
    pub offset: Vec3,
    pub speed: f32,
    pub dest_speed: f32,
    pub state: u32,
    pub follow: Follow,
}

impl Default for Cruising {
    /// The constructor's values (+1664 = +1668 = -1, the index -1).
    fn default() -> Self {
        Cruising { dests: [Vec3::ZERO; 64], count: 0, idx: -1, looping: false, offset: Vec3::ZERO, speed: -1.0, dest_speed: -1.0, state: 0, follow: Follow::default() }
    }
}

impl Cruising {
    /// BrainCruising_AppendDest (F-37550): onto the end (no bound: the script never passes 64), the current one at
    /// least the first.
    pub fn append_dest(&mut self, p: Vec3) {
        self.dests[self.count as usize & 63] = p;
        self.count += 1;
        if self.idx < 0 {
            self.idx = 0;
        }
    }
    /// BrainCruising_SetCurrentDest (F-51609): the list is this one point, the current one it.
    pub fn set_current_dest(&mut self, p: Vec3) {
        self.count = 1;
        self.dests[0] = p;
        self.idx = 0;
    }
    /// BrainCruising_SetLooping (F-92434)
    pub fn set_looping(&mut self, on: i32) {
        self.looping = on != 0;
    }
    /// BrainCruising_SetOffset (F-18127)
    pub fn set_offset(&mut self, v: Vec3) {
        self.offset = v;
    }
    /// BrainCruising_SetSpeed (F-47943)
    pub fn set_speed(&mut self, v: f32) {
        self.speed = v;
    }
    /// BrainCruising_SetDestSpeed (F-20377)
    pub fn set_dest_speed(&mut self, v: f32) {
        self.dest_speed = v;
    }
    /// BrainCruising_HasReachedGoal (F-67268): state 6 (past the last destination).
    pub fn has_reached_goal(&self) -> bool {
        self.state == 6
    }
    /// BrainCruising_GetCurrentDestinationIndex (F-76171; -1 without a brain).
    pub fn current_dest(b: Option<&Cruising>) -> i32 {
        b.map_or(-1, |b| b.idx)
    }
}

/// The physical car as state 5 reads it: its matrix rows a and c and its place (the matrix block, F-95017's car
/// branch), its velocity (F-98427: the sim's +272), the sim's yaw rate (+292) and the vehicle tune's +48 (x 0.25:
/// the slowest ghost it follows).
#[derive(Clone, Copy, Debug)]
pub struct FollowCar {
    pub a: Vec3,
    pub c: Vec3,
    pub pos: Vec3,
    pub vel: Vec3,
    pub yaw_rate: f32,
    pub tune48: f32,
}

/// What state 5 keeps between updates: the ghost's flat way (+1616), its speed (+1672), the rail's curvature
/// (+1688) and the demands to the vehicle controller (F-28541, aiv.rs::pedal_map): curvature +1680, speed +1676.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Follow {
    pub way: Vec3,
    pub speed: f32,
    pub curvature: f32,
    pub steer: f32,
    pub demand: f32,
}

impl Default for Follow {
    fn default() -> Self {
        Follow { way: Vec3::ZERO, speed: 0.0, curvature: 0.0, steer: 0.0, demand: -100.0 }
    }
}

/// F-64718 in state 5 with a physical car: follow the ghost at `p` going `fwd` (its matrix's -c, or the rail's
/// tangent) at `s` m/s; `k` the frame's time (the image's word 1/60). None: the ghost is more than 10 m away in XZ
/// (F-12584 DriveToNearestRail takes over; the way and speed are still kept, as the original stores them after it).
pub fn follow(st: &mut Follow, car: &FollowCar, p: Vec3, fwd: Vec3, s: f32, k: f32) -> Option<()> {
    let t = norm_or(Vec3::new(fwd.x, 0.0, fwd.z), 1e-5, Vec3::X);
    let f = Vec3::ZERO - car.c;
    let d = p - car.pos;
    let near = !(fma(d.z, d.z, d.x * d.x) > 100.0);
    if near {
        let v = car.vel;
        let vf = -fma(v.x, car.c.x, fma(car.c.z, v.z, car.c.y * v.y));
        if s > car.tune48 * 0.25 {
            let lm = if 5.0 - s >= 0.0 { 5.0 } else { s };
            let l = lm * 0.5;
            let ks = k * s;
            let a1 = ang_xz(f, st.way);
            let ad = fma(d.x, car.a.x, fma(car.a.y, d.y, car.a.z * d.z));
            let a0 = guest_atan2(-ad as f64, l as f64) as f32;
            let fxz = norm_or(Vec3::new(f.x, 0.0, f.z), 1e-5, Vec3::X);
            let vd = norm_or(Vec3::new(v.x, 0.0, v.z), 1.0, fxz);
            let a2 = ang_xz_unit(vd, st.way);
            let dot = fma(vd.x, fxz.x, fma(vd.z, fxz.z, vd.y * fxz.y));
            let yaw = st.curvature - car.yaw_rate / lm;
            let a3 = ang_xz_unit(st.way, t);
            let kn = a3 / ks;
            st.curvature = kn;
            st.steer = ((a2 * dot + a0) + a1) / l + kn + yaw;
            let fd = fma(d.x, f.x, fma(f.y, d.y, d.z * f.z));
            st.demand = fma(fd, 2.0, (s - st.speed) / k) + (st.speed - vf);
        } else {
            st.demand = -100.0;
        }
    }
    st.speed = s;
    st.way = t;
    near.then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-64718's state 5 run offline (work/racerai/cfollow): 20,000 cars and ghosts within 10 m (ghost without a rail:
    /// its matrix's place and -c), ghost speeds under and over the car's least, frame times 1/60, 1/30 and others,
    /// a degenerate way and an upright ghost for the fallbacks. Steer, speed demand, curvature, speed and way bit for bit.
    #[test]
    fn follow_is_the_originals() {
        let Ok(b) = std::fs::read("D:/MCLA/work/racerai/cfollow/cases.bin") else { return };
        let w = |r: &[u8], i: usize| f32::from_bits(u32::from_le_bytes(r[4 * i..4 * i + 4].try_into().unwrap()));
        let v = |r: &[u8], i: usize| Vec3::new(w(r, i), w(r, i + 1), w(r, i + 2));
        let mut bad = 0;
        for r in b.chunks_exact(52 * 4) {
            let car = FollowCar { a: v(r, 0), c: v(r, 8), pos: v(r, 12), vel: v(r, 38), yaw_rate: w(r, 41), tune48: w(r, 42) };
            let mut st = Follow { way: v(r, 33), speed: w(r, 36), curvature: w(r, 37), steer: f32::from_bits(0x7fc0_1234), demand: 0.0 };
            // (the steer is kept when the ghost is slow: the case's own output stands in for the old value)
            let slow = !(w(r, 32) > w(r, 42) * 0.25);
            if slow {
                st.steer = w(r, 44);
            }
            follow(&mut st, &car, v(r, 28), Vec3::ZERO - v(r, 24), w(r, 32), w(r, 43));
            let got = [st.steer, st.demand, st.curvature, st.speed, st.way.x, st.way.y, st.way.z].map(f32::to_bits);
            let want: [u32; 7] = std::array::from_fn(|i| w(r, 44 + i).to_bits());
            if got != want {
                if bad < 5 {
                    println!("port {got:08x?}\noriginal {want:08x?}");
                }
                bad += 1;
            }
        }
        assert!(b.len() / 208 >= 20000 && bad == 0, "{bad} differ");
    }

    /// The natives run offline on 400 brains, 50 random calls each (work/racerai/cnat): every field and result after
    /// every call equal.
    #[test]
    fn natives_are_the_originals() {
        let Ok(b) = std::fs::read("D:/MCLA/work/racerai/cnat/cases.bin") else { return };
        let w = |r: &[u8], i: usize| u32::from_le_bytes(r[4 * i..4 * i + 4].try_into().unwrap());
        let f = |r: &[u8], i: usize| f32::from_bits(w(r, i));
        let mut bad = 0;
        let mut br = Cruising::default();
        for (n, r) in b.chunks_exact(16 * 4 + 64 * 12).enumerate() {
            if n % 50 == 0 {
                br = Cruising::default();
            }
            let v = Vec3::new(f(r, 1), f(r, 2), f(r, 3));
            br.state = w(r, 5);
            let mut res = 0xdead_beef_u32;
            match w(r, 0) {
                0 => br.append_dest(v),
                1 => br.set_current_dest(v),
                2 => br.set_looping(w(r, 4) as i32),
                3 => br.set_offset(v),
                4 => br.set_speed(f(r, 4)),
                5 => br.set_dest_speed(f(r, 4)),
                6 => res = br.has_reached_goal() as u32,
                _ => res = Cruising::current_dest(Some(&br)) as u32,
            }
            let got = [res, br.count as u32, br.idx as u32, br.looping as u32, br.speed.to_bits(), br.dest_speed.to_bits(), br.offset.x.to_bits(), br.offset.y.to_bits(), br.offset.z.to_bits()];
            let want: [u32; 9] = std::array::from_fn(|i| w(r, 6 + i));
            let list = (0..64).all(|i| [br.dests[i].x, br.dests[i].y, br.dests[i].z].map(f32::to_bits) == [w(r, 16 + 3 * i), w(r, 17 + 3 * i), w(r, 18 + 3 * i)]);
            if got != want || !list {
                if bad < 5 {
                    println!("call {n} op {}: port {got:08x?} original {want:08x?} list {list}", w(r, 0));
                }
                bad += 1;
            }
        }
        assert!(b.len() / 832 >= 20000 && bad == 0, "{bad} differ");
    }
}
