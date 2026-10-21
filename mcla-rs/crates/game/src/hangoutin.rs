//! Pulling into a hangout as the original does it: `Game/Hangouts/local_hangout_core` (0xe166126d) and
//! `wager_hangout_core` (0x16e89a34), started by each hangout's set-up script with its block (the local
//! ones from +3280 of it), then `<key>_enter_cine` with the hangout's `_ent` scene. Decoded 2026-10-21
//! (docs/reference/cutscenes/hangout_entry.md):
//!
//! 1. The entry is a line trigger (`TriggerArray_AddTriggerLine`: `oPosStart` +28, `oPosEnd` +40, `rHeight`
//!    20), checked every loop (`Trigger_CheckTrigger`, the line's vtable[3] `sub_826881A8`): the player's
//!    move of the frame (racer+400 -> the matrix position) crosses the line in x/z one way only, and the
//!    crossing point's height is within [-2, +20) of the line's. No speed test; not with the police
//!    activity above 1 (the barrier is up then), only in the offline cruise.
//! 2. "Ease to pause": form 108, `PerspectiveForm_SetSlowMotion(0, 1 / 0.2, 0, 1, 1)`, WAITUNWARPED(200) -
//!    the global time scale to 0 in 0.2 s (sin ease), which the main loop holds at 0.1.
//! 3. `Warper_TransitionTo('STORY_LOCATION_CUT_SIMPLE', +8, +108, +120, +132, ..)` - the garages' warp
//!    (`garagein`): Warp_Target_Close (1 s) to the wait camera (eye +108, up +120 - +108, looking at
//!    +132 = the `_ent` scene's first camera) and ShowLocationName with the place's strings
//!    (`mcCareer_RegisterLocation(uid, <key>, <key>_desc)`: the location's +4 / +132).
//! 4. IsReadyToLoadStuff (polled every 10 ms): `localHangout` / `wagerHangout` thrown, `<key>_state` 3,
//!    the map element removed, `Racer_Deactivate`, "ease from pause" (`SetSlowMotion(1, 1 / 0.5, 0, 1, 1)`,
//!    form 108 hidden); the people and the set dressing streamed; LOCATION_EVENT; MISSION_EVENT +100
//!    (`<KEY>_ENTER`: a mission's event script may play its story scene, else `<key>_enter_cine` plays
//!    the `_ent` scene and sets the state 4).
//! 5. The core polls the state every 100 ms; at 4 it brings its panel up (form 62) over the scene's
//!    last frame.

use glam::Vec3;

use crate::hudmap::CamRows;

/// One hangout's places from its set-up script.
pub struct Setup {
    /// the location's key (its name string; `<key>_desc` the district) and the registry's `<key>_state`
    pub key: &'static str,
    /// +8, the location (mcCareer_RegisterLocation)
    pub uid: u32,
    /// +100, the event the core posts as MISSION_EVENT
    pub enter: &'static str,
    /// +28, +40: the entry line
    pub line: [Vec3; 2],
    /// +108, +120, +132: the wait camera's eye, the point above it, the point it looks at
    pub cam: [Vec3; 3],
    /// the scene `<key>_enter_cine` plays (its generated script's name)
    pub scene: &'static str,
}

pub const SETUPS: [Setup; 12] = [
    Setup {
        key: "hw_local_hangout1",
        uid: 500,
        enter: "HW_LOCAL_HANGOUT1_ENTER",
        line: [Vec3::new(722.5, 7.9, -289.2), Vec3::new(735.0, 7.9, -288.4)],
        cam: [Vec3::new(731.083, 8.539, -293.005), Vec3::new(730.374, 10.817, -293.258), Vec3::new(748.261, 14.564, -286.889)],
        scene: "hangout/cut_hangout_01_ent_02",
    },
    Setup {
        key: "hw_local_hangout2",
        uid: 501,
        enter: "HW_LOCAL_HANGOUT2_ENTER",
        line: [Vec3::new(-463.6, 3.2, -168.6), Vec3::new(-477.1, 3.2, -169.2)],
        cam: [Vec3::new(-477.232, 8.083, -200.479), Vec3::new(-477.265, 10.48, -200.958), Vec3::new(-469.827, 13.189, -175.405)],
        scene: "hangout/cut_hangout_02_ent_01",
    },
    Setup {
        key: "hw_local_hangout3",
        uid: 502,
        enter: "HW_LOCAL_HANGOUT3_ENTER",
        line: [Vec3::new(-1690.4, -4.9, 258.6), Vec3::new(-1675.0, -4.9, 258.9)],
        cam: [Vec3::new(-1632.13, -2.487, 289.173), Vec3::new(-1631.9611, -0.537, 289.26), Vec3::new(-1672.579, 2.726, 250.904)],
        scene: "hangout/cut_hangout_03_ent_01",
    },
    Setup {
        key: "hw_wager_hangout1",
        uid: 503,
        enter: "HW_WAGER_HANGOUT1_ENTER",
        line: [Vec3::new(-148.03, 7.29, -459.68), Vec3::new(-147.29, 7.18, -475.48)],
        cam: [Vec3::new(-128.651, 9.438, -476.771), Vec3::new(-127.949, 12.161, -476.891), Vec3::new(-146.025, 14.366, -466.529)],
        scene: "hangout/cut_hangout_04_ent_01",
    },
    Setup {
        key: "be_local_hangout1",
        uid: 510,
        enter: "BE_LOCAL_HANGOUT1_ENTER",
        line: [Vec3::new(-2425.6001, -8.7, 330.3), Vec3::new(-2438.3999, -8.7, 341.5)],
        cam: [Vec3::new(-2429.5049, -7.338, 331.313), Vec3::new(-2430.071, -4.63, 330.845), Vec3::new(-2422.8279, -5.822, 336.571)],
        scene: "hangout/cut_hangout_05_ent_01",
    },
    Setup {
        key: "be_wager_hangout1",
        uid: 511,
        enter: "BE_WAGER_HANGOUT1_ENTER",
        line: [Vec3::new(-2752.6001, -3.7, -211.9), Vec3::new(-2741.7, -3.7, -211.3)],
        cam: [Vec3::new(-2753.1201, -2.39, -200.036), Vec3::new(-2753.636, 0.14, -199.837), Vec3::new(-2730.6011, 2.689, -206.211)],
        scene: "hangout/cut_hangout_06_ent_01",
    },
    Setup {
        key: "dt_local_hangout1",
        uid: 530,
        enter: "DT_LOCAL_HANGOUT1_ENTER",
        line: [Vec3::new(2103.7, -2.5, -257.9), Vec3::new(2097.6001, -2.5, -248.3)],
        cam: [Vec3::new(2089.8569, -1.323, -260.159), Vec3::new(2089.4551, 1.457, -259.984), Vec3::new(2106.449, 1.457, -266.135)],
        scene: "hangout/cut_hangout_07_ent_01",
    },
    Setup {
        key: "dt_wager_hangout1",
        uid: 531,
        enter: "DT_WAGER_HANGOUT1_ENTER",
        line: [Vec3::new(1840.7, -9.5, 601.3), Vec3::new(1855.5, -9.5, 602.2)],
        cam: [Vec3::new(1828.9561, -8.888, 626.603), Vec3::new(1824.26, 4.374, 629.292), Vec3::new(1853.833, 2.461, 614.073)],
        scene: "hangout/cut_hangout_08_ent_01",
    },
    Setup {
        key: "hl_local_hangout1",
        uid: 520,
        enter: "HL_LOCAL_HANGOUT1_ENTER",
        line: [Vec3::new(-1007.7, 30.7, -1844.6), Vec3::new(-1018.8, 30.7, -1852.7)],
        cam: [Vec3::new(-1013.38, 34.933, -1871.809), Vec3::new(-1013.925, 37.736, -1872.002), Vec3::new(-1006.639, 37.673, -1851.105)],
        scene: "hangout/cut_hangout_09_ent_01",
    },
    Setup {
        key: "hl_wager_hangout1",
        uid: 521,
        enter: "HL_WAGER_HANGOUT1_ENTER",
        line: [Vec3::new(690.5, 77.0, -1506.4), Vec3::new(679.4, 77.0, -1501.0)],
        cam: [Vec3::new(680.924, 82.6, -1493.364), Vec3::new(680.456, 85.408, -1493.182), Vec3::new(713.054, 89.308, -1514.269)],
        scene: "hangout/cut_hangout_10_ent_01",
    },
    Setup {
        key: "sc_local_hangout1",
        uid: 540,
        enter: "SC_LOCAL_HANGOUT1_ENTER",
        line: [Vec3::new(-1388.0, -8.2, 1095.4), Vec3::new(-1375.6, -7.6, 1115.9)],
        cam: [Vec3::new(-1384.1479, -5.373, 1125.783), Vec3::new(-1384.1, -3.43, 1125.994), Vec3::new(-1385.679, -0.195, 1078.447)],
        scene: "hangout/sc_cut_hangout_02_ent_01",
    },
    Setup {
        key: "sc_wager_hangout1",
        uid: 541,
        enter: "SC_WAGER_HANGOUT1_ENTER",
        line: [Vec3::new(1381.9, -12.0, 1214.6), Vec3::new(1391.7, -12.3, 1215.2)],
        cam: [Vec3::new(1416.699, -5.289, 1235.765), Vec3::new(1416.771, 4.522, 1235.976), Vec3::new(1369.902, -4.007, 1192.278)],
        scene: "hangout/sc_cut_hangout_01_ent_01",
    },
];

/// the line's `rHeight`, and the 2 m below the line it also takes (0x82040E94)
pub const HEIGHT: f32 = 20.0;
pub const BELOW: f32 = 2.0;
/// the line test's epsilon (0x82007A3C)
const EPS: f32 = 1e-5;
/// "ease to pause": SetSlowMotion's rate 1 / 0.2, then WAITUNWARPED(200)
pub const SLOW_IN: f32 = 0.2;
/// "ease from pause" after the warp: rate 1 / 0.5
pub const SLOW_OUT: f32 = 0.5;

/// `sub_8261B020` with the one-way flag (+100 of the line, 1): where the move `p -> q` meets the line
/// `a -> b` in x/z, as (s along the move, t along the line); None when parallel or crossed the other way.
fn meet(p: [f32; 2], q: [f32; 2], a: [f32; 2], b: [f32; 2]) -> Option<(f32, f32)> {
    let dx_ab = b[0] - a[0];
    let dz_pq = q[1] - p[1];
    let dx_pq = q[0] - p[0];
    let dz_ab = b[1] - a[1];
    let den = dz_ab * dx_pq - dx_ab * dz_pq;
    if den < EPS || den.abs() < EPS {
        return None;
    }
    if dx_ab.abs() < EPS {
        if dx_pq.abs() < EPS {
            return None;
        }
        let s = (a[0] - p[0]) / dx_pq;
        return Some((s, (s * dz_pq + p[1] - a[1]) / dz_ab));
    }
    if dz_ab.abs() < EPS {
        if dz_pq.abs() < EPS {
            return None;
        }
        let s = (a[1] - p[1]) / dz_pq;
        return Some((s, (s * dx_pq + p[0] - a[0]) / dx_ab));
    }
    let s = ((p[1] - a[1]) * dx_ab + (a[0] - p[0]) * dz_ab) / den;
    Some((s, (s * dx_pq + p[0] - a[0]) / dx_ab))
}

/// `Trigger_CheckTrigger` of the entry line (`sub_826881A8`): the player moved from `prev` to `cur`.
pub fn crossed(s: &Setup, prev: Vec3, cur: Vec3) -> bool {
    let [a, b] = s.line;
    let xz = |v: Vec3| [v.x, v.z];
    let Some((u, t)) = meet(xz(prev), xz(cur), xz(a), xz(b)) else { return false };
    // (`sub_8261B168`: both within [0, 1])
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&t) {
        return false;
    }
    let on_line = a.y + (b.y - a.y) * t;
    let at = prev.y + (cur.y - prev.y) * u;
    at < on_line + HEIGHT && at >= on_line - BELOW
}

/// The hangout whose entry line the move `prev -> cur` crossed.
pub fn crossed_any(prev: Vec3, cur: Vec3) -> Option<usize> {
    SETUPS.iter().position(|s| crossed(s, prev, cur))
}

/// The wait camera (the Warper's destination matrix from +108, +120, +132).
pub fn wait_rows(s: &Setup) -> CamRows {
    CamRows::look_at(s.cam[0], s.cam[2], s.cam[1] - s.cam[0])
}

/// Where the entry is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
    /// the ease to pause before the warp (`SLOW_IN`)
    Slow,
    /// the warp to the wait camera
    Warp,
}

pub struct Entry {
    /// `SETUPS` index
    pub setup: usize,
    /// the port's hangout (`App::hangouts`) it is
    pub hangout: usize,
    pub stage: Stage,
    /// time in the stage (real seconds)
    pub t: f32,
    pub warp: Option<crate::hudmap::CineWarp>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One way only: driven in across Hollywood's first hangout's line it fires, out across it not, beside
    /// it or above the height not.
    #[test]
    fn line_one_way() {
        let s = &SETUPS[0];
        let m = (s.line[0] + s.line[1]) * 0.5;
        let d = s.line[1] - s.line[0];
        let n = Vec3::new(-d.z, 0.0, d.x).normalize();
        // (the inside: cross(b - a, x - a) < 0 - see `meet`)
        let inside = m - n;
        let outside = m + n;
        assert!(crossed(s, outside, inside));
        assert!(!crossed(s, inside, outside));
        assert!(!crossed(s, outside + d, inside + d));
        assert!(!crossed(s, outside + Vec3::Y * 21.0, inside + Vec3::Y * 21.0));
        assert!(!crossed(s, outside - Vec3::Y * 2.5, inside - Vec3::Y * 2.5));
        assert!(crossed(s, outside + Vec3::Y * 19.0, inside + Vec3::Y * 19.0));
    }
}
