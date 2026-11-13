//! The chassis control update F-17997 (the car's pedals and steering into the wheels, the engine and the transmission,
//! each tick), its own pieces as one function: decoded 2026-11-12 from the generated code and checked against it
//! offline (tools/chassis_control_reference.py, 3,000 chassis states with every callee stubbed and recorded;
//! docs/reference/vehicle/chassis_control_check.md, the pieces P0..P36 in docs/reference/vehicle/chassis_control_pieces.md).
//! The callees and the virtual slots are the `Env`'s: the port answers them with its own rows.

use super::artic::V4;

/// The fields of the chassis and of the objects it reaches that F-17997 reads and writes.
#[derive(Clone, Debug, Default)]
pub struct State {
    /// +112..+124 (the centre of mass; +116 / +120 ramped)
    pub cg: [f32; 4],
    /// +132 steer, +136 throttle, +140 brake, +144 handbrake, +148
    pub steer: f32,
    pub throttle: f32,
    pub brake: f32,
    pub handbrake: f32,
    pub f148: f32,
    /// +152 (byte), +224 speed, +260 up.y, +272..+280 velocity, +372, +396, +572, +576, +578 / +579 (bytes), +580
    pub f152: u8,
    pub speed: f32,
    pub up_y: f32,
    pub vel: [f32; 3],
    pub f372: i32,
    pub f396: f32,
    pub f572: f32,
    pub f576: u16,
    pub f578: i8,
    pub f579: u8,
    pub f580: f32,
    /// +592 (0x01, 0x02, 0x04 damaged out, 0x08, 0x10 wiped out), +624 (the input overrides), +656
    pub flags: u32,
    pub f624: u32,
    pub f656: f32,
    /// +736 / +740 / +744 (byte): the rev limiter's state
    pub f736: f32,
    pub f740: f32,
    pub f744: u8,
    /// +816 / +817 / +818 (bytes)
    pub f816: u8,
    pub f817: u8,
    pub f818: u8,
    /// +24 wheels (the array +28)
    pub wheels: i32,
    /// the wheels' +524 (written)
    pub w524: [f32; 4],
    /// the six wheel objects +412..+432 (present) and their +392
    pub slot: [bool; 6],
    pub slot392: [f32; 6],
    /// [+128]: +272..+280, +296
    pub body_v: [f32; 3],
    pub body296: f32,
    /// [+400]: count +20, element 0's +420, +48; the elements' +529 (byte) and +352 (vector)
    pub g400: i32,
    pub g400_420: f32,
    pub g400_48: f32,
    pub g400_el: [(bool, [f32; 4]); 4],
    /// [+404]'s count
    pub g404: i32,
    /// [+8]: +4, +40 (byte)
    pub o8_4: f32,
    pub o8_40: u8,
    /// [+16] the engine: +8, +46 (half), +68, +72, +76
    pub eng8: f32,
    pub eng46: u16,
    pub eng68: f32,
    pub eng72: f32,
    pub eng76: f32,
    /// [+20] the transmission: +20 gear, +24, +32 / +33 (bytes), the ratios from +36
    pub gear: i32,
    pub tr24: i32,
    pub tr32: u8,
    pub tr33: u8,
    pub ratio: [f32; 8],
    /// [+660]: +20 (written), +24; [+664]'s +32 (byte); [+668]'s +72 (None: no object); [+676]'s +16 / +20
    pub q660_20: f32,
    pub q660_24: f32,
    pub q664: bool,
    pub q664_32: u8,
    pub y668: Option<i32>,
    pub p676: Option<(i32, i32)>,
    /// [+4] the vehicle: the input record [+4] (its +8 half, +12..+32; +24 written), [+16]'s +1928, [+48]'s +16 / +18
    /// halves, [+72]'s +48 / +52, and the blend P23 writes: ([[+76]+104] +136)+40, ([[+76]+108]+136)+40 into
    /// ([+92]+136)+40
    pub inp: Option<Input>,
    pub o16: Option<i32>,
    pub s48: Option<(u16, u16)>,
    pub x72: Option<(i32, i32)>,
    pub blend: (f32, f32, f32),
    /// the tune [+368]
    pub tune: Tune,
    /// the image's: the time step and the global word G of the pieces map (tested in P6, P10, P24; not decoded)
    pub dt: f32,
    pub g: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    pub h8: u16,
    pub f12: f32,
    pub f16: f32,
    pub f20: f32,
    pub f24: f32,
    pub f28: f32,
    pub f32: f32,
}

/// The tune's fields F-17997 reads (by offset).
#[derive(Clone, Copy, Debug, Default)]
pub struct Tune {
    pub t52: f32,
    pub t56: f32,
    pub t112: f32,
    pub t132: i32,
    pub t224: f32,
    pub t228: f32,
    pub t232: f32,
    pub t236: f32,
    pub t240: f32,
    pub t248: f32,
    pub t252: f32,
    pub t256: f32,
    pub t260: f32,
    pub t264: f32,
    pub t276: f32,
    pub t292: f32,
    pub t296: f32,
    pub t300: f32,
    pub t304: f32,
    pub t308: f32,
}

/// What F-48988 is called on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum On {
    /// the wheel array's element
    Wheel(usize),
    /// [+400]'s / [+404]'s element
    G400(usize),
    G404(usize),
}

/// The callees and the virtual slots F-17997 calls, in its order.
pub trait Env {
    /// the chassis' slots 11 / 12 / 13
    fn slot11(&mut self) -> i32;
    fn slot12(&mut self) -> bool;
    fn slot13(&mut self) -> f32;
    /// slot 11 of [[[+4]+28]+4]; slot 13 of [[+4]+8]
    fn mass_slot11(&mut self) -> f32;
    fn other_slot13(&mut self) -> f32;
    /// F-90102 (the float in, out), F-86323, F-40603
    fn f90102(&mut self, x: f32, a: f32, b: f32, dt: f32) -> f32;
    fn f86323(&mut self, x: f32) -> f32;
    fn f40603(&mut self, x: f32) -> f32;
    /// F-16736 on the transmission, F-14919 on the vehicle, F-74215 / F-74056 on [+664] / [+668]
    fn f16736(&mut self);
    fn f14919(&mut self) -> bool;
    fn f74215(&mut self);
    fn f74056(&mut self);
    /// F-57353 on wheel object k (+412 + 4k)
    fn f57353(&mut self, k: usize, steer: f32, brake: f32, handbrake: f32);
    /// F-19727 / F-50158 on the engine
    fn f19727(&mut self);
    fn f50158(&mut self, x: f32);
    /// F-57868 on [+676]
    fn f57868(&mut self) -> bool;
    /// F-80284 on the chassis
    fn f80284(&mut self) -> i32;
    /// F-59013 on the transmission
    fn f59013(&mut self, r4: u32);
    fn f48988(&mut self, on: On, x: f32);
    /// F-51861 / F-52728 on the wheel array's element
    fn f51861(&mut self, i: usize, x: f32);
    fn f52728(&mut self, i: usize);
    /// F-82657 on the chassis
    fn f82657(&mut self, r4: u32);
    /// F-14399 on [+128] (the vector)
    fn f14399(&mut self, v: [f32; 4]);
    /// F-12574 on [+12]; F-19385 on [+400]
    fn f12574(&mut self) -> bool;
    fn f19385(&mut self) -> i32;
    /// F-75805 on [[+4]+48] (Some: the found object's +18 half); F-71293 on it: its +128, +132, +136
    fn f75805(&mut self) -> Option<u16>;
    fn f71293(&mut self) -> [f32; 3];
    /// F-48205 on the engine
    fn f48205(&mut self, x: f32);
    /// the text block (F-55794, F-65049, slot 12 of the found object; true: two lines printed - not ported)
    fn diag(&mut self) -> bool;
}

fn fsel(a: f32, b: f32, c: f32) -> f32 {
    if a >= 0.0 { b } else { c }
}

/// x towards `tgt` by dt x k (fused), not past it.
fn ramp(x: f32, tgt: f32, dt: f32, k: f32) -> f32 {
    if x < tgt {
        let y = dt.mul_add(k, x);
        if y > tgt { tgt } else { y }
    } else if x > tgt {
        let y = -dt.mul_add(k, -x);
        if y < tgt { tgt } else { y }
    } else {
        x
    }
}

/// F-17997.
pub fn update(s: &mut State, e: &mut impl Env) {
    let tu = s.tune;
    let dt = s.dt;
    // P0
    let r26 = e.slot11();
    // P1 (the input record into the controls)
    if let Some(i) = s.inp {
        if i.h8 != 0xFFFF {
            s.f148 = i.f28;
            s.steer = i.f24;
            s.throttle = i.f12;
            s.brake = i.f16;
            s.handbrake = i.f20;
        }
    }
    // P2
    if s.g400 != 0 {
        s.f572 = -(s.g400_420 * s.g400_48);
    }
    // P3 (+624's overrides)
    let o = s.f624;
    if o & 0x1 != 0 {
        s.brake = 0.0;
    } else if o & 0x2 != 0 {
        s.brake = 1.0;
    }
    if o & 0x100 != 0 {
        s.handbrake = 0.0;
    } else if o & 0x200 != 0 {
        s.handbrake = 1.0;
    }
    if o & 0x8 != 0 {
        s.throttle = 1.0;
    } else if o & 0x4 != 0 {
        s.throttle = 0.0;
    }
    if o & 0x400 != 0 {
        s.steer = 0.0;
    } else if o & 0x10 != 0 {
        s.steer = -1.0;
    } else if o & 0x20 != 0 {
        s.steer = 1.0;
    } else if o & 0x40 != 0 {
        s.steer = -s.steer;
    }
    if o & 0x800 != 0 {
        s.throttle = 0.0;
        s.handbrake = 0.25;
    } else if o & 0x1000 != 0 {
        s.steer = -s.steer;
        s.handbrake = 0.25;
    }
    // P4 (slot 12: straight to P11)
    if !e.slot12() {
        let f = s.flags;
        if f & 0x1 != 0 {
            // P5
            s.throttle = 0.0;
            s.brake = 0.0;
            s.handbrake = 0.0;
            s.f148 = 0.0;
            s.steer = s.f656;
        } else if f & 0x2 != 0 {
            // P6
            if s.g == 0 && (s.f576 >> 2) & 1 == 0 && s.throttle > 0.0 && s.handbrake > 0.0 {
                e.f16736();
                s.handbrake = 1.0;
                s.steer = 0.0;
            } else {
                e.f16736();
                if !(s.g == 0 && (s.f580 == 0.0 || s.handbrake == 0.0)) {
                    s.throttle = 0.0;
                }
                s.brake = 1.0;
                s.steer = 0.0;
            }
            if let Some(i) = s.inp.as_mut() {
                i.f24 = 0.0;
            }
        } else if f & 0x8 != 0 {
            // P7
            let d = -s.vel[0].mul_add(s.body_v[0], s.vel[2].mul_add(s.body_v[2], s.vel[1] * s.body_v[1]));
            let t = if d > 0.0 { d / s.speed } else { 0.0 };
            let t = fsel(t, t, 0.0);
            s.brake = 1.0;
            s.throttle = 0.0;
            s.handbrake = fsel(t - 1.0, 1.0, t);
            if e.f14919() {
                s.steer = s.f656;
            }
        }
        // P8
        let f = s.flags;
        if f & 0x4 != 0 {
            s.brake = 1.0;
            s.throttle = 0.0;
            s.steer = s.f656;
            s.handbrake = 0.0;
        } else if f & 0x10 != 0 {
            if s.wheels == 2 {
                s.throttle = 0.0;
                s.brake = 0.5;
                let x = s.body296;
                let x = fsel(x - -1.0, x, -1.0);
                let x = fsel(x - 1.0, 1.0, x);
                s.steer = e.f90102(s.steer, x, 2.5, dt);
                s.handbrake = 0.0;
            } else {
                let x = fsel(s.up_y, s.up_y, 0.0);
                let x = fsel(x - 1.0, 1.0, x);
                let t = s.throttle;
                s.throttle = e.f86323(x) * t;
            }
        }
        // P9
        if s.q664 {
            e.f74215();
        }
        if s.y668.is_some() {
            e.f74056();
        }
        // P10
        if s.g != 0 || (s.flags & 0x10 == 0 && s.flags & 0x2 == 0) {
            s.f580 = 0.0;
            s.f578 = 0;
            s.f579 = 0;
        }
    }
    // P11 (a driver state below 4: brake held)
    let held = s.o16.is_some_and(|k| k < 4);
    if held {
        if s.eng46 == 0 {
            s.eng46 = 1;
        }
        e.f16736();
        s.brake = 1.0;
        s.handbrake = 0.0;
        s.throttle = 0.0;
    } else if s.eng46 == 0 {
        s.throttle = 0.0;
    } else if s.q664_32 != 0 && s.f818 == 0 {
        s.throttle = 1.0;
    }
    // P12
    let d = s.vel[0].mul_add(s.body_v[0], s.vel[2].mul_add(s.body_v[2], s.vel[1] * s.body_v[1]));
    let k = e.f40603(d.abs());
    let mut f21 = s.steer * k;
    let f28 = s.brake;
    let f20 = s.handbrake;
    let mut f22 = s.throttle;
    // P13
    if s.wheels == 2 && s.f580 > 0.0 {
        f21 = -(s.f580 * f21).mul_add(0.9, -f21);
    }
    // P14
    e.f57353(0, f21, f28, f20);
    if s.slot[2] {
        e.f57353(2, f21, f28, f20);
    }
    e.f57353(1, -f21, f28, f20);
    for k in 3..6 {
        if s.slot[k] {
            e.f57353(k, -f21, f28, f20);
        }
    }
    // P15 (the transmission's +32)
    if held {
        s.tr32 = 1;
    } else if s.flags & 0x2 != 0 && s.f580 < 0.95 && s.handbrake == 0.0 {
        s.tr32 = 1;
    } else if s.tr24 != 0 && s.gear > 2 {
        e.f19727();
        s.tr32 = 1;
    } else if s.gear == 1 || (s.wheels == 2 && s.gear == 0) {
        s.tr32 = 1;
    } else {
        if s.f816 == 0 && s.tr32 == 1 {
            e.f50158(0.0);
        }
        s.tr32 = s.f152;
    }
    // P16
    if s.f816 == 0 && (s.throttle == 0.0 || s.f578 == 1 || s.f578 == 2) {
        e.f50158(0.0);
    }
    // P17
    let mut f12 = 4.47f32;
    if s.g400 > 0 {
        let r = s.ratio[s.gear as usize];
        if r != 0.0 {
            f12 = (s.g400_420 / r.abs()).mul_add(s.eng76, 2.235);
        }
    }
    // P18 (+396)
    if s.flags & 0x1 != 0 {
        s.f396 = 1.0;
    } else {
        let v = s.throttle;
        if (!(v > 0.1) || s.gear == 1) && s.speed < f12 && s.f572 < f12 {
            if s.f396 == 0.0 {
                s.f396 = 0.25;
            }
            let x = dt.mul_add(0.5, s.f396);
            s.f396 = if x > 1.0 { 1.0 } else { x };
        } else if v > 0.05 {
            s.f396 = 0.0;
        }
    }
    // P19
    if s.f817 == 0 && s.f396 > 0.0 {
        for i in 0..s.wheels.max(0) as usize {
            e.f48988(On::Wheel(i), s.f396);
        }
        s.tr32 = 1;
    }
    // P20
    if s.flags & 0x2 != 0 {
        for i in 0..s.g404.max(0) as usize {
            e.f48988(On::Wheel(i), 10.0);
        }
    }
    // P21 (the scenario object [[+4]+48])
    let mut p22 = true;
    if let Some((m, h18)) = s.s48 {
        let mi = m as i16;
        if mi == 1 {
            mass_tail(s, e, tu.t112);
        } else if mi == -1 {
            if s.cg[2] == tu.t56 {
                s.cg[2] = s.slot392[0] * 0.25;
                mass_tail(s, e, tu.t112);
            }
        } else if h18 != 0 && s.wheels != 2 {
            s.cg[2] = 0.0;
            mass_tail(s, e, tu.t112);
            s.o8_4 = tu.t300;
        }
        if m != 0 || h18 != 0 {
            p22 = false;
        }
    }
    // P22 (+120 towards the tune's +56)
    if p22 {
        if let Some((a, b)) = s.p676 {
            if e.f57868() && (a != 0 || b != 0) {
                p22 = false;
            }
        }
    }
    if p22 && (!(s.f580 > 0.0) || s.handbrake == 0.0) {
        s.cg[2] = ramp(s.cg[2], tu.t56, dt, 1.5);
        for i in 0..s.wheels.max(0) as usize {
            e.f52728(i);
        }
    }
    // P23
    if let Some((_, 0)) = s.s48 {
        let (p0, p1, _) = s.blend;
        s.blend.2 = (p1 - p0).mul_add(0.0, p0);
    }
    // P24..P26
    let r29 = s.s48.is_none_or(|(m, h)| m == 0 && h == 0) && s.g == 0;
    s.tr33 = 0;
    let r30 = e.f80284() >= 0 || s.wheels != 2;
    let lim = if e.f80284() >= 0 { tu.t224 } else { tu.t228 };
    let r11 = f22 > 0.0 && f20 > 0.0 && s.speed < lim && r26 > 0;
    // P27 (+578's states)
    let st = s.f578;
    if st > -1 {
        if (s.f576 >> 2) & 1 == 0 && r29 && r11 && r30 {
            // P27a
            let k = e.f80284();
            e.f59013(if k < 0 { 0 } else { 2 });
            s.tr33 = 1;
            for i in 0..s.g404.max(0) as usize {
                e.f48988(On::G404(i), 2.0);
            }
            for i in 0..s.g400.max(0) as usize {
                e.f48988(On::G400(i), 0.0);
            }
            s.f578 = 1;
            s.f579 = 1;
            let f31 = if s.flags & 0x2 != 0 { 0.75 } else { tu.t248 };
            let a = s.f580 < 1.0 && r26 > 0;
            let x = if tu.t132 != 0 && tu.t132 != 2 {
                if s.f372 == 3 {
                    e.f82657(1);
                }
                s.slot392[1]
            } else {
                if s.f372 == 2 {
                    e.f82657(0);
                }
                s.slot392[0]
            };
            s.cg[1] = 0.0;
            s.cg[2] = x * f31;
            for i in 0..s.wheels.max(0) as usize {
                e.f52728(i);
            }
            if a {
                let y = tu.t236.mul_add(dt, s.f580);
                s.f580 = if y > 1.0 { 1.0 } else { y };
            } else {
                s.f579 = 1;
                s.f578 = 2;
            }
        } else if s.f580 > 0.0 {
            // P27b
            if st != 3 && e.f14919() && s.flags & 0x2 == 0 {
                let _ = e.diag();
            }
            s.f579 = 1;
            s.f578 = 3;
            if s.handbrake == 0.0 && e.slot12() && r26 > 0 && e.slot12() {
                impulse(s, e, f20, f22, f28);
            }
            let a = -tu.t240.mul_add(dt, -s.f580);
            let b = -((1.0 - f22) * tu.t236).mul_add(dt, -a);
            s.f580 = b;
            if b < 0.0 {
                s.f580 = 0.0;
                s.f578 = 0;
                s.f579 = 0;
                if tu.t132 == 2 {
                    e.f82657(2);
                } else if tu.t132 == 3 {
                    e.f82657(3);
                }
            }
        }
    }
    // P28 (+740 down when the engine is not held)
    let p29 = s.tr32 != 0 || s.gear == 1 || (s.gear == 2 && s.f580 > 0.0) || e.slot13() > 0.0;
    if !p29 {
        if s.f740 > 0.0 {
            let x = s.f740 - dt;
            s.f740 = if x < 0.0 { 0.0 } else { x };
        }
    } else {
        // P29 (the rev limiter: +736 / +740 / +744)
        let ratio = (s.eng8 - s.eng72) / (s.eng68 - s.eng72);
        if ratio > tu.t256 {
            s.f740 += dt;
            if s.f740 > tu.t260 && s.f744 == 0 {
                s.f744 = 1;
                s.f736 = if tu.t264 < 0.0 { f22 } else { tu.t264 };
                if s.gear == 1 || s.tr32 != 0 || e.f19385() == 0 {
                    s.f736 = fsel(0.5 - s.f736, s.f736, 0.5);
                }
            }
        } else if ratio < tu.t252 {
            s.f744 = 0;
        } else if !(ratio > 0.0) {
            s.f740 = -dt;
        }
        let x = fsel(s.f740, s.f740, 0.0);
        s.f740 = fsel(x - tu.t260, tu.t260, x);
        if s.f744 != 0 {
            if tu.t264 < 0.0 {
                let y = tu.t264.mul_add(dt, s.f736);
                s.f736 = if y < 0.0 { 0.0 } else { y };
            }
            if s.f736 < f22 {
                f22 = s.f736;
            }
        }
    }
    // P30 ([+8] +4)
    let skip = s.x72.is_some_and(|(a, b)| b == 1 && a == 3);
    if !skip {
        if tu.t304 < 1.0 && (s.flags & 0x10 != 0 || s.flags & 0x4 != 0) {
            if !e.f12574() && (s.wheels == 2 || s.speed > 4.47) {
                s.o8_4 = tu.t304;
            }
        } else if s.wheels != 2 && tu.t308 < 1.0 && s.q660_24 > 0.01 {
            s.o8_4 = (tu.t308 - 1.0).mul_add(s.q660_24, 1.0);
        }
    }
    // P31 ([+8] +40)
    s.o8_40 = if s.wheels != 2 {
        u8::from(s.y668.is_some_and(|y| y > -1))
    } else {
        u8::from(s.inp.is_some_and(|i| i.f32 < -0.5) && e.other_slot13() < 0.5)
    };
    // P32 (the wheels' +524)
    let m = s.o8_4;
    let lerp = (m < 1.0 && !(s.q660_24 > 0.01)) || s.flags & 0x10 != 0 || s.flags & 0x4 != 0;
    let v = if lerp {
        (tu.t296 - tu.t292).mul_add(m, tu.t292)
    } else if s.f624 & 0x80 != 0 {
        10.0
    } else {
        tu.t296
    };
    for i in 0..s.wheels.max(0) as usize {
        s.w524[i] = v;
    }
    // P33
    s.q660_20 = if s.o8_4 > 1.0 { 1.0 } else { 0.0 };
    // P34 (the found object's +18: the steering pulled, the throttle scaled)
    if let Some(h) = e.f75805() {
        let n = h as i16 as i32;
        if n as f32 != 0.0 {
            let n = n.clamp(-1, 1);
            let a = e.f71293()[2];
            let b = e.f71293()[0];
            let q = a / b;
            let w = (1.0 - f21.abs()) * q;
            f21 = -w.mul_add(n as f32, -f21);
            if f22 > 0.5 {
                let c = e.f71293()[1];
                f22 = (f22 - 0.5).mul_add(c, 0.5);
            }
            s.tr33 = 1;
        }
    }
    // P35
    s.steer = f21;
    s.throttle = f22;
    e.f48205(f22);
    // P36 (+116 towards the tune's +52 scaled by the handbrake)
    let k = tu.t276;
    if k > 0.0 && f20 > 0.0 {
        let base = tu.t52;
        let tgt = base.mul_add(k, -base).mul_add(f20, base);
        s.cg[1] = ramp(s.cg[1], tgt, dt, 1.5);
    }
}

/// P21's tail: slot 11 of the mass object x the tune's +112 x 9.8 x 2 / wheels into F-51861 on each wheel.
fn mass_tail(s: &State, e: &mut impl Env, t112: f32) {
    let n = s.wheels as f32;
    let f31 = 2.0 / n;
    let f1 = e.mass_slot11();
    let f0 = (f1 * t112) * 9.8;
    if s.wheels > 0 {
        let x = f0 * f31;
        for i in 0..s.wheels as usize {
            e.f51861(i, x);
        }
    }
}

/// P27b's push: the sum of [+400]'s elements' -+352 where +529 is set, made unit (the estimate and one Newton step),
/// x F-80284 x (1 - handbrake) x (1 - brake) x the tune's +232 x the throttle, into F-14399.
/// (The original sets the sum's x, y, z to 0 and leaves its w lane as the stack holds it; here 0.)
fn impulse(s: &State, e: &mut impl Env, f20: f32, f22: f32, f28: f32) {
    let mut acc = V4::splat(0.0);
    for i in 0..s.g400.max(0) as usize {
        let (on, v) = s.g400_el[i];
        if on {
            acc = acc.sub(V4::new(v[0], v[1], v[2], v[3]));
        }
    }
    let l = acc.dot3(acc);
    let r = l.rsqrte();
    let r = r.sel(V4::splat(0.0), r.eq(V4::splat(f32::INFINITY)));
    let r2 = r.mul(r);
    let h = V4::splat(0.5).mul(r);
    let n = l.nmsub(r2, V4::splat(1.0));
    let k = e.f80284();
    let r = n.madd(h, r);
    let scale = (((k as f32 * (1.0 - f20)) * (1.0 - f28)) * s.tune.t232) * f22;
    let v = acc.mul(r).mul(V4::splat(scale));
    e.f14399(v.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fmt::Write;

    // the reference's layout (tools/chassis_control_reference.py)
    const C: u32 = 0x1000;
    const TUNE: u32 = 0x1400;
    const V: u32 = 0x1580;
    const INP: u32 = 0x1600;
    const V8: u32 = 0x1640;
    const V28O: u32 = 0x1680;
    const P0O: u32 = 0x1990;
    const P1O: u32 = 0x19C0;
    const DEST: u32 = 0x1960;
    const C8: u32 = 0x1A00;
    const C12: u32 = 0x1A40;
    const E16: u32 = 0x1A80;
    const T20: u32 = 0x1B00;
    const O128: u32 = 0x1B80;
    const G400: u32 = 0x1D00;
    const G404: u32 = 0x1D40;
    const WARR: u32 = 0x1D80;
    const Q660: u32 = 0x1DA0;
    const Q664: u32 = 0x1DC0;
    const Y668: u32 = 0x1DE0;
    const P676: u32 = 0x1E40;
    const R29: u32 = 0x1E60;
    const OEA: u32 = 0x1E80;
    const DIAG: u32 = 0x1F20;

    #[derive(Default, Clone)]
    struct Mem(HashMap<u32, u8>);
    impl Mem {
        fn b(&self, a: u32) -> u8 {
            *self.0.get(&a).unwrap_or(&0)
        }
        fn w(&self, a: u32) -> u32 {
            u32::from_be_bytes([self.b(a), self.b(a + 1), self.b(a + 2), self.b(a + 3)])
        }
        fn h(&self, a: u32) -> u16 {
            u16::from_be_bytes([self.b(a), self.b(a + 1)])
        }
        fn f(&self, a: u32) -> f32 {
            f32::from_bits(self.w(a))
        }
        fn sw(&mut self, a: u32, v: u32) {
            for (k, x) in v.to_be_bytes().into_iter().enumerate() {
                self.0.insert(a + k as u32, x);
            }
        }
        fn sh(&mut self, a: u32, v: u16) {
            for (k, x) in v.to_be_bytes().into_iter().enumerate() {
                self.0.insert(a + k as u32, x);
            }
        }
        fn sf(&mut self, a: u32, v: f32) {
            self.sw(a, v.to_bits());
        }
    }

    struct Rec<'a> {
        m: &'a Mem,
        ret: [u32; 16],
        log: String,
    }
    impl Rec<'_> {
        fn l(&mut self, t: char, r3: u32, r4: u32, a: f32, b: f32, c: f32) {
            write!(self.log, " {t},{r3:x},{r4:x},{:x},{:x},{:x}", a.to_bits(), b.to_bits(), c.to_bits()).unwrap();
        }
        fn rf(&self, k: usize) -> f32 {
            f32::from_bits(self.ret[k])
        }
        fn wheel(&self, i: usize) -> u32 {
            self.m.w(WARR + 4 * i as u32)
        }
    }
    impl Env for Rec<'_> {
        fn slot11(&mut self) -> i32 {
            self.l('a', C, 0, 0.0, 0.0, 0.0);
            self.ret[0] as i32
        }
        fn slot12(&mut self) -> bool {
            self.l('b', C, 0, 0.0, 0.0, 0.0);
            self.ret[1] as u8 != 0
        }
        fn slot13(&mut self) -> f32 {
            self.l('c', C, 0, 0.0, 0.0, 0.0);
            self.rf(2)
        }
        fn mass_slot11(&mut self) -> f32 {
            self.l('d', V28O, 0, 0.0, 0.0, 0.0);
            self.rf(3)
        }
        fn other_slot13(&mut self) -> f32 {
            self.l('e', V8, 0, 0.0, 0.0, 0.0);
            self.rf(4)
        }
        fn f90102(&mut self, _x: f32, a: f32, b: f32, dt: f32) -> f32 {
            self.l('A', 0x5DF0, 0, a, b, dt);
            self.rf(5)
        }
        fn f86323(&mut self, x: f32) -> f32 {
            self.l('B', 0, 0, x, 0.0, 0.0);
            self.rf(6)
        }
        fn f40603(&mut self, x: f32) -> f32 {
            self.l('C', C, 0, x, 0.0, 0.0);
            self.rf(7)
        }
        fn f16736(&mut self) {
            self.l('D', T20, 0, 0.0, 0.0, 0.0);
        }
        fn f14919(&mut self) -> bool {
            self.l('E', V, 0, 0.0, 0.0, 0.0);
            self.ret[8] as u8 != 0
        }
        fn f74215(&mut self) {
            self.l('F', Q664, 0, 0.0, 0.0, 0.0);
        }
        fn f74056(&mut self) {
            self.l('G', Y668, 0, 0.0, 0.0, 0.0);
        }
        fn f57353(&mut self, k: usize, a: f32, b: f32, c: f32) {
            let p = self.m.w(C + 412 + 4 * k as u32);
            self.l('H', p, 0, a, b, c);
        }
        fn f19727(&mut self) {
            self.l('I', E16, 0, 0.0, 0.0, 0.0);
        }
        fn f50158(&mut self, x: f32) {
            self.l('J', E16, 0, x, 0.0, 0.0);
        }
        fn f57868(&mut self) -> bool {
            self.l('K', P676, 0, 0.0, 0.0, 0.0);
            self.ret[9] as u8 != 0
        }
        fn f80284(&mut self) -> i32 {
            self.l('L', C, 0, 0.0, 0.0, 0.0);
            self.ret[10] as i32
        }
        fn f59013(&mut self, r4: u32) {
            self.l('M', T20, r4, 0.0, 0.0, 0.0);
        }
        fn f48988(&mut self, on: On, x: f32) {
            let p = match on {
                On::Wheel(i) => self.wheel(i),
                On::G400(i) => self.m.w(G400 + 24 + 4 * i as u32),
                On::G404(i) => self.m.w(G404 + 24 + 4 * i as u32),
            };
            self.l('N', p, 0, x, 0.0, 0.0);
        }
        fn f51861(&mut self, i: usize, x: f32) {
            let p = self.wheel(i);
            self.l('O', p, 0, x, 0.0, 0.0);
        }
        fn f52728(&mut self, i: usize) {
            let p = self.wheel(i);
            self.l('P', p, 0, 0.0, 0.0, 0.0);
        }
        fn f82657(&mut self, r4: u32) {
            self.l('Q', C, r4, 0.0, 0.0, 0.0);
        }
        fn f14399(&mut self, v: [f32; 4]) {
            write!(self.log, " R,{O128:x},{:x},{:x},{:x},{:x}", v[0].to_bits(), v[1].to_bits(), v[2].to_bits(), v[3].to_bits()).unwrap();
        }
        fn f12574(&mut self) -> bool {
            self.l('S', C12, 0, 0.0, 0.0, 0.0);
            self.ret[11] as u8 != 0
        }
        fn f19385(&mut self) -> i32 {
            self.l('T', G400, 0, 0.0, 0.0, 0.0);
            self.ret[12] as i32
        }
        fn f75805(&mut self) -> Option<u16> {
            let p = self.m.w(V + 48);
            self.l('U', p, 0, 0.0, 0.0, 0.0);
            (self.ret[13] != 0).then(|| self.m.h(R29 + 18))
        }
        fn f71293(&mut self) -> [f32; 3] {
            self.l('V', R29, 0, 0.0, 0.0, 0.0);
            [self.m.f(OEA + 128), self.m.f(OEA + 132), self.m.f(OEA + 136)]
        }
        fn f48205(&mut self, x: f32) {
            self.l('W', E16, 0, x, 0.0, 0.0);
        }
        fn diag(&mut self) -> bool {
            // (the global the lookup takes holds 0 in the image)
            self.l('X', V, 0, 0.0, 0.0, 0.0);
            self.l('Y', 0, 0, 0.0, 0.0, 0.0);
            self.l('f', DIAG, 0, 0.0, 0.0, 0.0);
            false
        }
    }

    fn opt(m: &Mem, a: u32) -> Option<u32> {
        let p = m.w(a);
        (p != 0).then_some(p)
    }

    fn load(m: &Mem) -> State {
        let mut s = State::default();
        for k in 0..4 {
            s.cg[k] = m.f(C + 112 + 4 * k as u32);
        }
        s.steer = m.f(C + 132);
        s.throttle = m.f(C + 136);
        s.brake = m.f(C + 140);
        s.handbrake = m.f(C + 144);
        s.f148 = m.f(C + 148);
        s.f152 = m.b(C + 152);
        s.speed = m.f(C + 224);
        s.up_y = m.f(C + 260);
        s.vel = [m.f(C + 272), m.f(C + 276), m.f(C + 280)];
        s.f372 = m.w(C + 372) as i32;
        s.f396 = m.f(C + 396);
        s.f572 = m.f(C + 572);
        s.f576 = m.h(C + 576);
        s.f578 = m.b(C + 578) as i8;
        s.f579 = m.b(C + 579);
        s.f580 = m.f(C + 580);
        s.flags = m.w(C + 592);
        s.f624 = m.w(C + 624);
        s.f656 = m.f(C + 656);
        s.f736 = m.f(C + 736);
        s.f740 = m.f(C + 740);
        s.f744 = m.b(C + 744);
        s.f816 = m.b(C + 816);
        s.f817 = m.b(C + 817);
        s.f818 = m.b(C + 818);
        s.wheels = m.w(C + 24) as i32;
        for i in 0..s.wheels as usize {
            s.w524[i] = m.f(m.w(WARR + 4 * i as u32) + 524);
        }
        for k in 0..6 {
            if let Some(p) = opt(m, C + 412 + 4 * k as u32) {
                s.slot[k] = true;
                s.slot392[k] = m.f(p + 392);
            }
        }
        s.body_v = [m.f(O128 + 272), m.f(O128 + 276), m.f(O128 + 280)];
        s.body296 = m.f(O128 + 296);
        s.g400 = m.w(G400 + 20) as i32;
        if let Some(e0) = opt(m, G400 + 24) {
            s.g400_420 = m.f(e0 + 420);
        }
        s.g400_48 = m.f(G400 + 48);
        for i in 0..4 {
            if let Some(p) = opt(m, G400 + 24 + 4 * i as u32) {
                s.g400_el[i] = (m.b(p + 529) != 0, [m.f(p + 352), m.f(p + 356), m.f(p + 360), m.f(p + 364)]);
            }
        }
        s.g404 = m.w(G404 + 20) as i32;
        s.o8_4 = m.f(C8 + 4);
        s.o8_40 = m.b(C8 + 40);
        s.eng8 = m.f(E16 + 8);
        s.eng46 = m.h(E16 + 46);
        s.eng68 = m.f(E16 + 68);
        s.eng72 = m.f(E16 + 72);
        s.eng76 = m.f(E16 + 76);
        s.gear = m.w(T20 + 20) as i32;
        s.tr24 = m.w(T20 + 24) as i32;
        s.tr32 = m.b(T20 + 32);
        s.tr33 = m.b(T20 + 33);
        for g in 0..8 {
            s.ratio[g] = m.f(T20 + 36 + 4 * g as u32);
        }
        s.q660_20 = m.f(Q660 + 20);
        s.q660_24 = m.f(Q660 + 24);
        s.q664 = opt(m, C + 664).is_some();
        s.q664_32 = m.b(Q664 + 32);
        s.y668 = opt(m, C + 668).map(|p| m.w(p + 72) as i32);
        s.p676 = opt(m, C + 676).map(|p| (m.w(p + 16) as i32, m.w(p + 20) as i32));
        s.inp = opt(m, V + 4).map(|p| Input {
            h8: m.h(p + 8),
            f12: m.f(p + 12),
            f16: m.f(p + 16),
            f20: m.f(p + 20),
            f24: m.f(p + 24),
            f28: m.f(p + 28),
            f32: m.f(p + 32),
        });
        s.o16 = opt(m, V + 16).map(|p| m.w(p + 1928) as i32);
        s.s48 = opt(m, V + 48).map(|p| (m.h(p + 16), m.h(p + 18)));
        s.x72 = opt(m, V + 72).map(|p| (m.w(p + 48) as i32, m.w(p + 52) as i32));
        s.blend = (m.f(P0O + 40), m.f(P1O + 40), m.f(DEST + 40));
        let t = |o: u32| m.f(TUNE + o);
        s.tune = Tune {
            t52: t(52),
            t56: t(56),
            t112: t(112),
            t132: m.w(TUNE + 132) as i32,
            t224: t(224),
            t228: t(228),
            t232: t(232),
            t236: t(236),
            t240: t(240),
            t248: t(248),
            t252: t(252),
            t256: t(256),
            t260: t(260),
            t264: t(264),
            t276: t(276),
            t292: t(292),
            t296: t(296),
            t300: t(300),
            t304: t(304),
            t308: t(308),
        };
        s.dt = m.f(0x827D_7508);
        s.g = m.w(0x8288_E614);
        s
    }

    fn store(m: &mut Mem, s: &State) {
        for k in 0..4 {
            m.sf(C + 112 + 4 * k as u32, s.cg[k]);
        }
        m.sf(C + 132, s.steer);
        m.sf(C + 136, s.throttle);
        m.sf(C + 140, s.brake);
        m.sf(C + 144, s.handbrake);
        m.sf(C + 148, s.f148);
        m.sf(C + 396, s.f396);
        m.sf(C + 572, s.f572);
        m.0.insert(C + 578, s.f578 as u8);
        m.0.insert(C + 579, s.f579);
        m.sf(C + 580, s.f580);
        m.sf(C + 736, s.f736);
        m.sf(C + 740, s.f740);
        m.0.insert(C + 744, s.f744);
        for i in 0..s.wheels as usize {
            let p = m.w(WARR + 4 * i as u32);
            m.sf(p + 524, s.w524[i]);
        }
        m.sf(C8 + 4, s.o8_4);
        m.0.insert(C8 + 40, s.o8_40);
        m.sh(E16 + 46, s.eng46);
        m.0.insert(T20 + 32, s.tr32);
        m.0.insert(T20 + 33, s.tr33);
        m.sf(Q660 + 20, s.q660_20);
        if let Some(i) = s.inp {
            m.sf(INP + 24, i.f24);
        }
        m.sf(DEST + 40, s.blend.2);
    }

    /// F-17997 against the original run offline (tools/chassis_control_reference.py): every word it changes and
    /// every call it makes, in order.
    #[test]
    fn chassis_control_matches_the_original() {
        let text = include_str!("testdata/chassis_control_ref.txt");
        let mut n = 0;
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        let head: Vec<u32> = lines.next().unwrap().split_whitespace().map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
        for line in lines {
            let (inp, out) = line.split_once(" -> ").unwrap();
            let (words, calls) = out.split_once('|').unwrap();
            let w: Vec<u32> = inp.split_whitespace().map(|t| u32::from_str_radix(t, 16).unwrap()).collect();
            let mut m = Mem::default();
            let mut ret = [0u32; 16];
            for p in head.chunks(2) {
                m.sw(p[0], p[1]);
            }
            for p in w.chunks(2) {
                let (a, v) = (p[0], p[1]);
                if a >= 0xF000_0000 {
                    ret[(a - 0xF000_0000) as usize] = v;
                } else if a & 0x8000_0000 != 0 {
                    m.sw(a, v);
                } else if a & 0x4000_0000 != 0 {
                    m.0.insert(a & 0x3FFF_FFFF, v as u8);
                } else if a & 0x2000_0000 != 0 {
                    m.sh(a & 0x1FFF_FFFF, v as u16);
                } else {
                    m.sw(a, v);
                }
            }
            let before = m.clone();
            let mut s = load(&m);
            let mut rec = Rec { m: &before, ret, log: String::new() };
            update(&mut s, &mut rec);
            let log = rec.log;
            store(&mut m, &s);
            let mut changed = String::new();
            let mut a = 0x1000;
            while a < 0x5000 {
                if m.w(a) != before.w(a) {
                    write!(changed, "{a:x}={:08x} ", m.w(a)).unwrap();
                }
                a += 4;
            }
            assert_eq!(changed.trim(), words.trim(), "case {n} words: {inp}");
            // (F-14399's w lane: P27b clears the stack vector's x, y, z only - its +92 is what an earlier copy left)
            let w_off = |t: &str| t.split(' ').map(|c| if c.starts_with("R,") { c.rsplit_once(',').unwrap().0.to_string() } else { c.to_string() }).collect::<Vec<_>>().join(" ");
            assert_eq!(w_off(log.trim()), w_off(calls.trim()), "case {n} calls: {inp}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }
}
