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
    /// from the world's axes (the view's azimuth plus the car's heading)
    pub azimuth: f32,
    pub incline: f32,
    pub roll: f32,
    /// radians
    pub fov: f32,
}

impl CamState {
    pub fn of(view: &View, pos: Vec3, rot: Quat, turn: f32) -> CamState {
        let back = rot * Vec3::Z;
        let off = if std::env::var("MCLA_GARAGE_OFFYAW").map_or(false, |v| v == "car") { rot * view.offset } else { view.offset };
        CamState { target: pos + off, height: view.height, dist: if view.dist < 0.5 { 0.5 } else { view.dist }, azimuth: view.azimuth + turn + garage_yaw() + back.x.atan2(back.z), incline: view.incline, roll: view.roll, fov: view.fov.to_radians() }
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
        CamState { target: self.target.lerp(to.target, k), height: l(self.height, to.height), dist: l(self.dist, to.dist), azimuth: l(from_az, to.azimuth), incline: l(self.incline, to.incline), roll: l(self.roll, to.roll), fov: l(self.fov, to.fov) }
    }

    /// Eye, the point looked at, the up vector and the field of view.
    pub fn place(&self) -> (Vec3, Vec3, Vec3, f32) {
        let dir = Vec3::new(self.azimuth.sin() * self.incline.cos(), self.incline.sin(), self.azimuth.cos() * self.incline.cos());
        let look = self.target + Vec3::Y * self.height;
        let up0 = (Vec3::Y - dir * dir.y).normalize_or(Vec3::Y);
        let right = up0.cross(dir);
        (look + dir * self.dist, look, up0 * self.roll.cos() - right * self.roll.sin(), self.fov)
    }
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
