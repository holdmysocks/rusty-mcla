//! The garage's cameras: every car has a file (`tune/garage/<hash of "<car>.garage">`)
//! with some two dozen named views - the turntable view, close-ups of the wheels, the
//! bumpers, the spoiler, the seats - each an orbit around a point of the car:
//! `<View>Height`, `Dist`, `Azimuth`, `Incline`, `FOV` and `InterestOffsetX/Y/Z`.
//!
//! The car's frame in these files is X towards the rear, Y up, Z to the right; azimuth 0
//! looks at the car from behind and turns towards the right-hand side.

use glam::{Quat, Vec3};
use hashbrown::HashMap;

#[derive(Clone, Copy, Debug, Default)]
pub struct View {
    pub height: f32,
    pub dist: f32,
    pub azimuth: f32,
    pub incline: f32,
    pub roll: f32,
    /// vertical field of view (degrees)
    pub fov: f32,
    pub offset: Vec3,
    /// where the menu panel hangs in this view (`<View>MenuOffsetX/Y/Z`, file frame),
    /// its turn about the vertical (`MenuRotation`, radians) and its size
    /// (`MenuScaleX/Y`); the game hands them to the garage movie as `cur_MenuPosition`,
    /// `cur_menuRotation` and `cur_Menuscale` (`0x826B32E0`)
    pub menu_offset: Vec3,
    pub menu_rot: f32,
    pub menu_scale: [f32; 2],
}

#[derive(Default)]
pub struct Views {
    views: HashMap<String, View>,
    /// seconds a move from one view to the next takes
    pub transition: f32,
    /// the performance shop's dyno graph on the screen: left, top, width, height as
    /// fractions of it (`DynoScreenPosHD`, `DynoScreenSizeHD`)
    pub dyno: [f32; 4],
}

const FIELDS: [&str; 8] = ["Height", "Dist", "Azimuth", "Incline", "FOV", "InterestOffsetX", "InterestOffsetY", "InterestOffsetZ"];

impl Views {
    pub fn load(data_dir: &str, car: &str) -> Option<Views> {
        let base = car.to_ascii_lowercase();
        // (the DUB editions use their base car's garage; a mod car: its own file, then its manifest's garage donor -
        // modfs.rs)
        let mut names = vec![base.clone(), base.replace("_dub", "")];
        names.extend(crate::modfs::car_names(&base, crate::modfs::Donor::Garage).into_iter().skip(1));
        let read = |n: &str| crate::modfs::read_text(data_dir, &format!("tune/garage/0x{:08x}", mcla_formats::hash::joaat(&format!("{n}.garage"))));
        let text = names.iter().find_map(|n| read(n))?;
        // (the cut GT-R's file was left unfinished - no WholeCarViewMenuOffsetZ / MenuScaleX / MenuScaleY,
        // no RearLowCloseMenuScaleY: the VEHICLE tab's panel had no size. Its stand-in's file goes first,
        // its own lines after it - catalog::tune_donor)
        let text = match crate::catalog::tune_donor(&base).and_then(|d| read(d)) {
            Some(donor) => format!("{donor}
{text}"),
            None => text,
        };
        let mut v = Views { transition: 0.7, ..Default::default() };
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let (Some(key), Some(val)) = (it.next(), it.next().and_then(|x| x.parse::<f32>().ok())) else { continue };
            if key == "DynoScreenPosHD" || key == "DynoScreenSizeHD" {
                let second = it.next().and_then(|x| x.parse::<f32>().ok()).unwrap_or(0.0);
                let at = if key == "DynoScreenPosHD" { 0 } else { 2 };
                v.dyno[at] = val;
                v.dyno[at + 1] = second;
                continue;
            }
            if key == "camTransDuration" {
                v.transition = val.clamp(0.1, 3.0);
                continue;
            }
            let menu = [("MenuRotation", 0), ("MenuOffsetX", 1), ("MenuOffsetY", 2), ("MenuOffsetZ", 3), ("MenuScaleX", 4), ("MenuScaleY", 5)];
            if let Some((name, k)) = menu.iter().find_map(|(f, k)| key.strip_suffix(f).map(|n| (n, *k))) {
                if !name.is_empty() {
                    let e = v.views.entry(name.to_string()).or_default();
                    match k {
                        0 => e.menu_rot = val,
                        1 => e.menu_offset.x = val,
                        2 => e.menu_offset.y = val,
                        3 => e.menu_offset.z = val,
                        4 => e.menu_scale[0] = val,
                        _ => e.menu_scale[1] = val,
                    }
                }
                continue;
            }
            if let Some(name) = key.strip_suffix("Roll").filter(|n| !n.is_empty()) {
                v.views.entry(name.to_string()).or_default().roll = val;
                continue;
            }
            for (i, f) in FIELDS.iter().enumerate() {
                if let Some(name) = key.strip_suffix(f) {
                    // ("RearWheelsHeight" must not be read as view "RearWheels" + "Height" of "Rear...": the
                    // suffix match is exact, and the longer field names are tried by their own entry)
                    if name.is_empty() || name.ends_with("Interest") {
                        continue;
                    }
                    let e = v.views.entry(name.to_string()).or_default();
                    match i {
                        0 => e.height = val,
                        1 => e.dist = val,
                        2 => e.azimuth = val,
                        3 => e.incline = val,
                        4 => e.fov = val,
                        5 => e.offset.x = val,
                        6 => e.offset.y = val,
                        _ => e.offset.z = val,
                    }
                    break;
                }
            }
        }
        // keep the views that are complete
        v.views.retain(|_, w| w.fov > 5.0);
        if v.views.is_empty() { None } else { Some(v) }
    }

    pub fn get(&self, name: &str) -> Option<View> {
        self.views.get(name).copied()
    }

    pub fn len(&self) -> usize {
        self.views.len()
    }
}

/// The turn of the garage movie's coordinate frame 5, where the menu panel hangs: F-32331 copies frame 6 into it,
/// built from the car's matrix with row 1 = (0, 1, 0), row 2 = the car's z row made square to it (F-49999's
/// orthonormalise: y dropped, normalised) and row 0 = their cross product - the car's heading only, no pitch or
/// roll (the translation is the car matrix's own, `origin` at the caller).
pub fn yaw_frame(rot: Quat) -> Quat {
    let z = rot * Vec3::Z;
    let flat = Vec3::new(z.x, 0.0, z.z);
    if flat.length_squared() < 1e-12 {
        return Quat::IDENTITY;
    }
    let flat = flat.normalize();
    Quat::from_rotation_y(flat.x.atan2(flat.z))
}

/// The views' frame (`coord` 5 of the garage movie) as a matrix into the car's own
/// frame (x right, y up, z rear).
pub fn file_frame() -> glam::Mat4 {
    use glam::{Mat4, Vec4};
    let m = |x: Vec3, z: Vec3| Mat4::from_cols(x.extend(0.0), Vec4::Y, z.extend(0.0), Vec4::W);
    let m = |x: Vec3, z: Vec3| Mat4::from_rotation_y(garage_yaw()) * m(x, z);
    match std::env::var("MCLA_GARAGE_FRAME").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(GARAGE_FRAME) {
        0 => m(Vec3::Z, Vec3::X),
        1 => m(Vec3::Z, -Vec3::X),
        2 => m(-Vec3::Z, Vec3::X),
        3 => m(Vec3::X, Vec3::Z),
        _ => m(-Vec3::X, -Vec3::Z),
    }
}

/// Test hook: a turn between the views' axes and the car (degrees in
/// `MCLA_GARAGE_YAW`). The car stands square in them: with no turn FrontOverviewMed
/// (azimuth 170 degrees about a point 0.91 m to the car's left) looks straight down
/// the car's centre line, as the reference capture shows it.
pub fn garage_yaw() -> f32 {
    std::env::var("MCLA_GARAGE_YAW").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0).to_radians()
}

/// Which of the candidate frames `file_frame` uses (settled against the reference capture).
const GARAGE_FRAME: u32 = 3;

/// The garage camera's own figures at a moment: what the game eases from view to view
/// (0x826B4508) - the target point and Height, Dist, Azimuth, Incline, Roll and FOV, each
/// by (1 - cos(pi t)) / 2 over `camTransDuration` - so that the camera swings round the
/// car instead of cutting through it.
#[derive(Clone, Copy, Debug)]
pub struct CamState {
    pub target: Vec3,
    pub height: f32,
    pub dist: f32,
    /// from the car's heading (the camera's +200: placement mode 1 adds the heading, F-74130)
    pub azimuth: f32,
    pub incline: f32,
    pub roll: f32,
    /// degrees (the camera's +168)
    pub fov: f32,
    /// the car's heading from the world's axes, added to the azimuth when the camera is placed
    pub heading: f32,
}

impl CamState {
    pub fn of(view: &View, pos: Vec3, rot: Quat, turn: f32) -> CamState {
        let back = rot * Vec3::Z;
        let off = if std::env::var("MCLA_GARAGE_OFFYAW").map_or(false, |v| v == "car") { rot * view.offset } else { view.offset };
        CamState { target: pos + off, height: view.height, dist: if view.dist < 0.5 { 0.5 } else { view.dist }, azimuth: view.azimuth + turn, incline: view.incline, roll: view.roll, fov: view.fov, heading: garage_yaw() + back.x.atan2(back.z) }
    }

    /// The figures the interior free look turns.
    pub fn look(&self) -> Look {
        Look { fov: self.fov, azimuth: self.azimuth, incline: self.incline }
    }

    pub fn with_look(mut self, l: Look) -> CamState {
        self.fov = l.fov;
        self.azimuth = l.azimuth;
        self.incline = l.incline;
        self
    }

    /// `self` moved towards `to` by the transition's share `t` (0..1 of its duration).
    /// The azimuth goes the short way round (0x826B3AD0).
    pub fn eased(&self, to: &CamState, t: f32) -> CamState {
        let k = (1.0 - (std::f32::consts::PI * t.clamp(0.0, 1.0)).cos()) * 0.5;
        let mut from_az = self.azimuth;
        while to.azimuth - from_az > std::f32::consts::PI {
            from_az += std::f32::consts::TAU;
        }
        while from_az - to.azimuth > std::f32::consts::PI {
            from_az -= std::f32::consts::TAU;
        }
        let l = |a: f32, b: f32| a + (b - a) * k;
        CamState { target: self.target.lerp(to.target, k), height: l(self.height, to.height), dist: l(self.dist, to.dist), azimuth: l(from_az, to.azimuth), incline: l(self.incline, to.incline), roll: l(self.roll, to.roll), fov: l(self.fov, to.fov), heading: l(self.heading, to.heading) }
    }

    /// Eye, the point looked at, the up vector and the field of view (radians).
    pub fn place(&self) -> (Vec3, Vec3, Vec3, f32) {
        let az = self.azimuth + self.heading;
        let dir = Vec3::new(az.sin() * self.incline.cos(), self.incline.sin(), az.cos() * self.incline.cos());
        let look = self.target + Vec3::Y * self.height;
        let up0 = (Vec3::Y - dir * dir.y).normalize_or(Vec3::Y);
        let right = up0.cross(dir);
        (look + dir * self.dist, look, up0 * self.roll.cos() - right * self.roll.sin(), self.fov.to_radians())
    }
}

/// The garage camera's view numbers (the argument of F-34768 and the global view index; the garage file's order).
pub const VIEW_NAMES: [&str; 25] = [
    "Rotate", "RearHighClose", "RearLowClose", "Interior", "Interior2", "Wheels", "FrontLowClose", "FrontHighClose", "SideOverviewClose", "ShoppingCart", "FrontSeats", "Stereo", "SteeringWheel", "PostGauges", "RearWheels", "TopOverViewMed", "LeftOverviewMed", "RightOverviewMed", "FrontOverviewMed", "RearOverviewMed", "FrontHighFar", "WholeCarView", "RightRearQuarter", "LeftFrontQuarter", "FrontHighMed",
];

pub fn view_number(name: &str) -> Option<u32> {
    VIEW_NAMES.iter().position(|n| *n == name).map(|i| i as u32)
}

/// F-34768's class test: Interior (3) and the close-ups 10..13 (FrontSeats, Stereo, SteeringWheel, PostGauges).
pub fn interior(view: u32) -> bool {
    view == 3 || (10..=13).contains(&view)
}

/// The camera's figures the interior free look turns: FOV (+168, degrees), azimuth (+200, from the car's heading) and
/// incline (+204).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub fov: f32,
    pub azimuth: f32,
    pub incline: f32,
}

/// The move the free look starts (F-57991): F-34768(view, forced) with the camera's figures at the call (`at`: the
/// azimuth already turned, the incline not yet), then the move's end azimuth = `at.azimuth` and its duration 0.4 s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flip {
    pub view: u32,
    pub at: Look,
}

/// The flip's duration (F-57991).
pub const FLIP_TIME: f32 = 0.4;

/// The game frame the garage camera's per-frame step F-53958 runs at.
const GAME_FRAME: f32 = 1.0 / 30.0;

/// The garage camera from view to view (F-53958 / F-34768; the ease is the port's `CamState::eased`) and the interior free
/// look on it (F-28551).
#[derive(Clone, Copy, Debug)]
pub struct Driver {
    pub from: CamState,
    pub to: CamState,
    /// seconds into the move, and its length
    pub timer: f32,
    pub dur: f32,
    /// the global view index and the view saved for the way back from Interior2 (+39240)
    pub view: u32,
    pub saved: u32,
    /// the figures the menu's view last had: when they change (another car) the move starts again
    base: CamState,
    /// game frames' time not run yet
    pending: f32,
}

fn same(a: &CamState, b: &CamState) -> bool {
    (a.target - b.target).length() < 1e-3 && (a.dist - b.dist).abs() < 1e-4 && (a.azimuth - b.azimuth).abs() < 1e-4 && (a.height - b.height).abs() < 1e-4 && (a.heading - b.heading).abs() < 1e-4
}

impl Driver {
    pub fn new(to: CamState, view: u32) -> Driver {
        Driver { from: to, to, timer: 1.0e3, dur: 1.0, view, saved: if interior(view) { view } else { 3 }, base: to, pending: 0.0 }
    }

    pub fn moving(&self) -> bool {
        self.timer < self.dur
    }

    /// The camera's figures now.
    pub fn live(&self) -> CamState {
        if self.moving() { self.from.eased(&self.to, self.timer / self.dur.max(0.01)) } else { self.to }
    }

    fn start(&mut self, to: CamState, dur: f32) {
        self.from = self.live();
        self.to = to;
        self.timer = 0.0;
        self.dur = dur;
    }

    /// The menu's view `view` with figures `to` (F-34768, not forced): the same view, or an interior view while the
    /// camera is in Interior2, starts nothing; an interior view entered from outside them is saved.
    pub fn ask(&mut self, view: u32, to: CamState, dur: f32) {
        let changed = !same(&self.base, &to);
        self.base = to;
        if view == self.view || (interior(view) && self.view == 4) {
            // (the port's own: another car's figures in the same view move the camera to them)
            if changed {
                self.view = view;
                self.start(to, dur);
            }
            return;
        }
        if interior(view) && !interior(self.view) {
            self.saved = view;
        }
        self.view = view;
        self.start(to, dur);
    }

    /// `dt` seconds: the move goes on, and each game frame the camera is still in an interior view or Interior2 the
    /// free look runs on the pad's buttons word and right stick bytes. `table` gives a view number's figures.
    pub fn step(&mut self, dt: f32, pad: Option<(u32, [u8; 2])>, table: impl Fn(u32) -> Option<CamState>) {
        self.timer += dt;
        self.pending = (self.pending + dt).min(0.25);
        while self.pending >= GAME_FRAME {
            self.pending -= GAME_FRAME;
            let Some((buttons, stick)) = pad else { continue };
            if self.moving() || !(interior(self.view) || self.view == 4) {
                continue;
            }
            let mut l = self.to.look();
            match free_look(&mut l, &mut self.view, &mut self.saved, buttons, stick, false) {
                Some(f) => {
                    // the move's start is the camera as it is at the call, its end the new view's figures at the
                    // azimuth reached
                    let mut b = table(f.view).unwrap_or(self.to);
                    b.azimuth = f.at.azimuth;
                    self.from = self.to.with_look(f.at);
                    self.to = b;
                    self.timer = 0.0;
                    self.dur = FLIP_TIME;
                }
                None => self.to = self.to.with_look(l),
            }
        }
    }
}

/// One game frame of the interior free look F-28551 (views 3, 4, 10..13, the camera not moving), the flip F-57991 in it.
/// `buttons` is the pad record's word (bit 1 RT, bit 0 LT: the poll sets them from 64 of the trigger's 255), `stick` its
/// right stick's bytes. `rmr` is F-78526: the garage's online room (+21680, D7) showing, false offline - while it is,
/// the triggers zoom only in views 0 and 3. `view` (the global view index) and `saved` (+39240) change as F-34768 would
/// change them on the flip.
pub fn free_look(look: &mut Look, view: &mut u32, saved: &mut u32, buttons: u32, stick: [u8; 2], rmr: bool) -> Option<Flip> {
    let gate = !rmr || *view == 0 || *view == 3;
    // the triggers: RT narrows the lens by 0.2 degrees down to 35.2, LT widens it up to 69.8 (RT first)
    let (lo, hi, step) = (f32::from_bits(0x420C_CCCD), f32::from_bits(0x428B_999A), f32::from_bits(0x3E4C_CCCD));
    if buttons & 2 != 0 && gate {
        if !(look.fov < lo) {
            look.fov -= step;
        }
    } else if buttons & 1 != 0 && gate && !(look.fov > hi) {
        look.fov += step;
    }
    let axis = |b: u8| ((b as f32 - 127.5) * f32::from_bits(0x3C01_0204)).clamp(-1.0, 1.0);
    let (x, y) = (axis(stick[0]), axis(stick[1]));
    if x <= 0.25 && x >= -0.25 && y <= 0.25 && y >= -0.25 {
        return None;
    }
    let rate = f32::from_bits(0x3D23_D70A);
    look.azimuth += crate::boomcam::dead_zone(x, 0.25) * rate;
    // F-57991: in Interior2 coming back in front of +-90 degrees returns to the saved view; in the others passing it
    // goes to Interior2, the view left saved
    let q = std::f32::consts::FRAC_PI_2;
    let a = look.azimuth;
    let mut flip = None;
    if *view == 4 {
        if !(a >= q) && !(a <= -q) {
            flip = Some(Flip { view: *saved, at: *look });
        }
    } else if a > q || !(a >= -q) {
        *saved = *view;
        flip = Some(Flip { view: 4, at: *look });
    }
    if let Some(f) = flip {
        // (F-34768 forced: an interior view entered from Interior2 is saved again - the same one)
        if interior(f.view) && !interior(*view) {
            *saved = f.view;
        }
        *view = f.view;
    }
    let (min, max) = if *view != 4 && interior(*view) { (f32::from_bits(0x3C75_C28F), f32::from_bits(0x3E8C_CCCD)) } else { (f32::from_bits(0x3D4C_CCCD), f32::from_bits(0x3F66_6666)) };
    let inc = look.incline - crate::boomcam::dead_zone(y, 0.25) * rate;
    // (two fsel: NaN takes the limit)
    let inc = if inc - min >= 0.0 { inc } else { min };
    look.incline = if inc - max >= 0.0 { max } else { inc };
    flip
}

impl View {
    /// The camera's up vector for an eye standing `back` (eye - target) from what it looks
    /// at: the level up turned by the view's roll. (Sign from the running game's camera
    /// matrix, step 34: up = up0 cos r - right sin r, right = up0 x back.)
    pub fn up(&self, back: Vec3) -> Vec3 {
        let z = back.normalize_or(Vec3::Z);
        let up0 = (Vec3::Y - z * z.y).normalize_or(Vec3::Y);
        let right = up0.cross(z);
        up0 * self.roll.cos() - right * self.roll.sin()
    }

    /// Camera position, the point it looks at and the vertical field of view (radians)
    /// for a car standing at `pos` with rotation `rot`. `turn` is added to the azimuth
    /// (the turntable view drifts round the car).
    pub fn place(&self, pos: Vec3, rot: Quat, turn: f32) -> (Vec3, Vec3, f32) {
        // Read from the running game's camera on eleven views (work/ui/s11/camdump.py, step 34):
        // the camera's target is the car's position plus the interest offset in WORLD axes -
        // the offset does not turn with the car (0x826B3A84) - and the camera works in mode 1
        // (0x8271FF98): the azimuth counts from the car's heading (atan2 of its z axis' x and
        // z). The eye stands `dist` along (sin az cos inc, sin inc, cos az cos inc) from the
        // target and is lifted by `height` without being turned; it looks back along that line.
        // The Interior view's distance, 0 in the file, is 0.5 on the camera.
        let back = rot * Vec3::Z;
        let az = self.azimuth + turn + garage_yaw() + back.x.atan2(back.z);
        let dir = Vec3::new(az.sin() * self.incline.cos(), self.incline.sin(), az.cos() * self.incline.cos());
        // (test: MCLA_GARAGE_OFFYAW=car adds the offset in the car's axes, as before step 34)
        let off = if std::env::var("MCLA_GARAGE_OFFYAW").map_or(false, |v| v == "car") { rot * self.offset } else { self.offset };
        let target = pos + off + Vec3::Y * self.height;
        let dist = if self.dist < 0.5 { 0.5 } else { self.dist };
        let cam = target + dir * dist;
        let look = target;
        (cam, look, self.fov.to_radians())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F-28551 (with the flip F-57991 and the dead zone F-93983) against the original's code run offline
    /// (tools/garage_look_reference.py -> testdata/garage_look_ref.txt, 20,000 game frames in views 3, 4, 10..13 with
    /// random buttons, stick bytes, online-room flag, lens, azimuth and incline at and around the limits): the camera's
    /// FOV, azimuth and incline equal word for word, the view and saved view, the flip asked (view, forced) with the
    /// camera's figures at the call, then the move's end azimuth = the azimuth at the call and its time 0.4 s.
    #[test]
    fn free_look_matches_the_original() {
        let h = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let (mut n, mut flips, mut zoom, mut turned, mut clamped) = (0, 0, 0, 0, 0);
        for line in include_str!("testdata/garage_look_ref.txt").lines() {
            let w: Vec<u32> = line.split_whitespace().filter(|s| *s != "GL" && *s != "->").map(h).collect();
            assert_eq!(w.len(), 23, "{line}");
            let (mut view, mut saved) = (w[0], w[1]);
            let f = |i: usize| f32::from_bits(w[i]);
            let mut l = Look { fov: f(6), azimuth: f(7), incline: f(8) };
            let flip = free_look(&mut l, &mut view, &mut saved, w[2], [w[3] as u8, w[4] as u8], w[5] != 0);
            assert_eq!([l.fov.to_bits(), l.azimuth.to_bits(), l.incline.to_bits(), view, saved], [w[9], w[10], w[11], w[12], w[13]], "{line}");
            let turn = w[14] == 0;
            // (+39232 = 0 exactly when the stick is out of its dead zone: the port's `None` before the turn)
            assert!(turn || w[14] == 0x7f7f_7f7f, "{line}");
            let ax = |b: u32| ((b as f32 - 127.5) * f32::from_bits(0x3C01_0204)).clamp(-1.0, 1.0).abs() > 0.25;
            assert_eq!(turn, ax(w[3]) || ax(w[4]), "{line}");
            match flip {
                Some(fl) => {
                    flips += 1;
                    assert_eq!([w[17], w[18], w[19]], [1, fl.view, 1], "{line}");
                    assert_eq!([fl.at.azimuth.to_bits(), fl.at.incline.to_bits(), fl.at.fov.to_bits()], [w[20], w[21], w[22]], "{line}");
                    assert_eq!([w[15], w[16]], [fl.at.azimuth.to_bits(), FLIP_TIME.to_bits()], "{line}");
                }
                None => assert_eq!([w[15], w[16], w[17]], [0x7f7f_7f7f, 0x7f7f_7f7f, 0], "{line}"),
            }
            zoom += (l.fov.to_bits() != w[6]) as usize;
            turned += turn as usize;
            clamped += (turn && l.incline.to_bits() != w[8] && (l.incline == f32::from_bits(0x3C75_C28F) || l.incline == f32::from_bits(0x3E8C_CCCD) || l.incline == 0.05 || l.incline == 0.9)) as usize;
            n += 1;
        }
        eprintln!("free look: {n} frames, {flips} flips, {zoom} zooms, {turned} turned, {clamped} clamped");
        assert!(n == 20_000 && flips >= 1_000 && zoom >= 2_000 && turned >= 8_000 && clamped >= 500);
    }

    /// Frame 5 keeps the car's heading and drops its pitch and roll (F-32331's frame 6: up (0, 1, 0), z the car's
    /// z made flat). A car standing 1.5 degrees nose down and 0.8 degrees rolled, heading 35 degrees.
    #[test]
    fn panel_frame_is_the_heading_only() {
        let rot = Quat::from_rotation_y(35f32.to_radians()) * Quat::from_rotation_x(1.5f32.to_radians()) * Quat::from_rotation_z(0.8f32.to_radians());
        let f = yaw_frame(rot);
        assert!((f * Vec3::Y - Vec3::Y).length() < 1e-6);
        let z = rot * Vec3::Z;
        let flat = Vec3::new(z.x, 0.0, z.z).normalize();
        assert!((f * Vec3::Z - flat).length() < 1e-6);
        assert!((f * Vec3::X - Vec3::Y.cross(flat)).length() < 1e-6);
        assert!((yaw_frame(Quat::from_rotation_y(-2.0)) * Vec3::Z - Quat::from_rotation_y(-2.0) * Vec3::Z).length() < 1e-6);
    }
}
