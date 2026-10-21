//! Roadside parking spots (`resources/city/sc/0x2e3038ce.r62`): root+0x18 -> array of
//! 64-byte records, u16 count at root+0x1c.
//!
//! ```text
//! +28 u8 kind (1 = kerbside, 0 = car park)   +30 vec3 position   +3c f32 heading
//! ```
//! The heading h is in radians; the parked car's z row is (sin h, 0, cos h) and it faces -z =
//! `(-sin h, 0, -cos h)` (sub_826DF700 / sub_826DE7F8). Checked on the data: so placed, 87 % of the
//! kind-1 spots stand side by side with their nearest neighbour 3.3 m away (nose-in rows).

use super::Rd;
use crate::rsc::Resource;
use glam::Vec3;

#[derive(Debug, Clone, Copy)]
pub struct ParkingSpot {
    pub pos: Vec3,
    pub heading: f32,
    pub kind: u8,
}

impl ParkingSpot {
    /// Direction the parked car faces.
    pub fn dir(&self) -> Vec3 {
        Vec3::new(-self.heading.sin(), 0.0, -self.heading.cos())
    }
}

pub fn parse_parking(res: &Resource) -> Vec<ParkingSpot> {
    let rd = Rd::new(res);
    let mut out = Vec::new();
    let Some(arr) = rd.ptr(0x18) else { return out };
    let count = rd.u16(0x1c) as usize;
    for i in 0..count {
        let o = arr + i * 64;
        if !rd.has(o, 64) {
            break;
        }
        let pos = Vec3::new(rd.f32(o + 0x30), rd.f32(o + 0x34), rd.f32(o + 0x38));
        let heading = rd.f32(o + 0x3c);
        if pos.is_finite() && heading.is_finite() {
            out.push(ParkingSpot { pos, heading, kind: rd.u8(o + 0x28) });
        }
    }
    out
}
