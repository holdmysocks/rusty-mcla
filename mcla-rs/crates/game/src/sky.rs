//! Time-of-day sky from the game's own tuning: `tune/sky/clear.xml` (rmwSkydome colour
//! keyframes) and `tune/sky/SunDirection.xml` (sun vector keyframes). Each `KFData`
//! block is `Keys` rows of `time, c0..c{Channels-1}` with time in hours (0..24).

use glam::Vec3;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct Track {
    pub keys: Vec<(f32, [f32; 4])>,
}

impl Track {
    pub fn sample(&self, t: f32) -> [f32; 4] {
        let k = &self.keys;
        if k.is_empty() {
            return [0.0; 4];
        }
        if t <= k[0].0 {
            return k[0].1;
        }
        for w in k.windows(2) {
            if t <= w[1].0 {
                let span = (w[1].0 - w[0].0).max(1e-5);
                let f = ((t - w[0].0) / span).clamp(0.0, 1.0);
                let mut o = [0.0; 4];
                for i in 0..4 {
                    o[i] = w[0].1[i] + (w[1].1[i] - w[0].1[i]) * f;
                }
                return o;
            }
        }
        k[k.len() - 1].1
    }
    pub fn rgb(&self, t: f32) -> Vec3 {
        let v = self.sample(t);
        Vec3::new(v[0], v[1], v[2])
    }
}

/// All `<name>` keyframe tracks of a document, in order of appearance.
fn tracks(xml: &str, name: &str) -> Vec<Track> {
    let open = format!("<{}>", name);
    let close = format!("</{}>", name);
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(a) = rest.find(&open) {
        let body = &rest[a + open.len()..];
        let Some(b) = body.find(&close) else { break };
        let blk = &body[..b];
        let attr = |tag: &str| -> usize {
            blk.find(tag)
                .and_then(|i| blk[i..].find("value=\"").map(|j| i + j + 7))
                .and_then(|i| blk[i..].find('"').map(|j| blk[i..i + j].trim().parse::<usize>().unwrap_or(0)))
                .unwrap_or(0)
        };
        let ch = attr("<Channels").clamp(1, 4);
        let nk = attr("<Keys");
        let mut tr = Track::default();
        if let Some(i) = blk.find("<KeyData") {
            if let Some(j) = blk[i..].find('>') {
                let data = &blk[i + j + 1..];
                let data = &data[..data.find("</KeyData>").unwrap_or(data.len())];
                let vals: Vec<f32> = data.split_whitespace().filter_map(|s| s.parse().ok()).collect();
                for k in 0..nk {
                    let base = k * (ch + 1);
                    if base + ch >= vals.len() {
                        break;
                    }
                    let mut c = [0.0; 4];
                    for i in 0..ch {
                        c[i] = vals[base + 1 + i];
                    }
                    tr.keys.push((vals[base], c));
                }
            }
        }
        out.push(tr);
        rest = &body[b + close.len()..];
    }
    out
}

/// The weather sets on the disc: a sky dome file in `tune/sky` plus a keyed sun light
/// (`TODSun*` in `tune/lighting`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weather {
    Clear,
    Nice,
    Cloudy,
    Foggy,
    Stormy,
}

impl Weather {
    pub const ALL: [Weather; 5] = [Weather::Clear, Weather::Nice, Weather::Cloudy, Weather::Foggy, Weather::Stormy];

    pub fn parse(s: &str) -> Option<Weather> {
        Self::ALL.iter().copied().find(|w| w.name().eq_ignore_ascii_case(s))
    }
    pub fn name(self) -> &'static str {
        match self {
            Weather::Clear => "clear",
            Weather::Nice => "nice",
            Weather::Cloudy => "cloudy",
            Weather::Foggy => "foggy",
            Weather::Stormy => "stormy",
        }
    }
    fn sky_file(self) -> &'static str {
        match self {
            Weather::Clear => "clear.xml",
            Weather::Nice => "Nice.xml",
            Weather::Cloudy => "Cloudy.xml",
            Weather::Foggy => "Foggy.xml",
            Weather::Stormy => "Stormy.xml",
        }
    }
    fn light_name(self) -> &'static str {
        match self {
            Weather::Clear => "TODSun",
            Weather::Nice => "TODSunNice",
            Weather::Cloudy => "TODSunCloudy",
            Weather::Foggy => "TODSunFoggy",
            Weather::Stormy => "TODSunStormy",
        }
    }
    /// Fog start / end distance (m) and maximum density.
    pub fn fog(self) -> (f32, f32, f32) {
        match self {
            // the game's post-processing preset (`tune/lighting`, technique MC4): fog from 50 m
            // to 1000 m, at most 67 % opaque (`fogparameters`, alpha of `fogmaxcolor`);
            // the bad-weather values are ours
            Weather::Clear => (50.0, 1000.0, 0.675),
            Weather::Nice => (50.0, 1000.0, 0.675),
            Weather::Cloudy => (40.0, 850.0, 0.80),
            Weather::Foggy => (15.0, 420.0, 0.97),
            Weather::Stormy => (30.0, 650.0, 0.90),
        }
    }
    /// Share of the sky covered by cloud (ours: the dome's own cloud parameters are not understood).
    pub fn cloud_cover(self) -> f32 {
        match self {
            Weather::Clear => 0.30,
            Weather::Nice => 0.22,
            Weather::Cloudy => 0.72,
            Weather::Foggy => 0.90,
            Weather::Stormy => 0.96,
        }
    }
    /// How wet the roads are (0..1).
    pub fn wet(self) -> f32 {
        match self {
            Weather::Stormy => 1.0,
            Weather::Foggy => 0.25,
            _ => 0.0,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Sky {
    /// keyed sun light: rgb + intensity (`m_KeyColor` of the weather's TODSun light)
    pub light: Track,
    /// keyed ambient light (`m_KeyAmbLight`)
    pub ambient: Track,
    /// the tone mapper's key (`m_Luminance`: middle grey 0.25 by day, 0.2 at night)
    pub luminance: Track,
    /// how strong the shadows are (`m_ShadowFade`: gone at the 06:00 and 18:00 handover of
    /// sun and moon, and all day in a storm)
    pub shadow_fade: Track,
    /// the night lighting - lamps, lit windows' glow, headlights (`m_NFLFade`: in from 18:00
    /// to 19:00, out from 05:00 to 06:00)
    pub nfl_fade: Track,
    /// the sun's orbit (`m_KeyOrbit`: degrees round the sky, degrees of tilt)
    pub orbit: Track,
    /// `m_OcclusionFades`, `m_BrightPass`, `m_LightAdaption`, `m_Saturation`: as the game feeds them to
    /// its shaders (see `Tod`)
    pub occlusion_fades: Track,
    /// `m_RoadReflection`: how wet the roads are (0.095 in dry weather, 1.17 / 1.19 in a storm, 0 in fog)
    pub road_reflection: Track,
    pub bright_pass: Track,
    pub adaption: Track,
    pub saturation: Track,
    /// `m_DoFParams`: the depth of field (see `Tod::dof`)
    pub dof: Track,
    /// the weather manager's own keys for this weather (`tune/lighting/0xa838c380`,
    /// `sm_Key...Default<Weather>`): colour correction, fog colour and reach
    pub color_correct: Track,
    pub fog_max: Track,
    pub fog_near: Track,
    pub fog_far: Track,
    /// `sm_KeyMinFogColor...`, `sm_KeyFogHeight...`: the particles' fog only (RMPTFX_FogMinColor,
    /// RMPTFX_FogParams.x - mc4_rmptfx_mainlist's vertex programs)
    pub fog_min: Track,
    pub fog_height: Track,
    pub sky_color: Track,
    pub azimuth: Track,
    pub azimuth2: Track,
    pub sun_color: Track,
    pub sunset: Track,
    pub cloud: Track,
    pub cloud_top: Track,
    pub sun_dir: Track,
    /// 4th channel: how strongly the azimuth colours tint the horizon
    pub az_strength: Track,
    /// `UnStrengthThruCSpeedThruStarBrightnessKF`: 3rd channel = AzimuthHeight (how fast the
    /// horizon colours give way to the sky colour with height)
    pub star: Track,
    /// the rest of the dome's keys, as its programs take them (`hat`):
    /// `CloudThresholdThruCloudShadowStrengthKF`, `DetailScaleThruDetailStrengthWithEdgeSmoothThicknessKF`,
    /// `TBiasThruTDetailThruThresholdThruHeightKF`, `SunCentreKF`, `m_MoonColorKFAndBrightness`,
    /// `m_MoonBrightnessGlowStarBrightnessKF`; HDRExposure and HDRSunExposure
    pub cloud_threshold: Track,
    pub detail: Track,
    pub top_bias: Track,
    pub sun_centre: Track,
    pub moon_color: Track,
    pub moon_glow: Track,
    pub hdr: [f32; 2],
    /// the weather's HDRExposureClamp (the mini sky's c100 ceiling)
    pub hdr_clamp: [f32; 3],
    /// SkyhatMiniNoise StarFieldRepeat (the dome's StarFieldUVRepeat), the one tune file of
    /// the sky control object
    pub star_repeat: f32,
    /// how far over the dome shows its keyframes (the older "clear" set is authored
    /// close to display values, the HDR weather sets much darker)
    pub gain: f32,
    pub loaded: bool,
}

/// The original's light state at one hour, as its shaders receive it (measured on the running
/// game against these files - LIGHTING_WORK.md): the weather's `TODSun*` keys are used as they are.
#[derive(Debug, Clone, Copy)]
pub struct Tod {
    /// gLightColor: `m_KeyColor` rgb (its fourth channel is not applied)
    pub light: Vec3,
    /// gLightAmbient: `m_KeyAmbLight`
    pub ambient: Vec3,
    /// `m_KeyColor`'s fourth channel: how much of the fog's keyed colour stands as it is - the
    /// rest is taken times (light + ambient). Nice 0.447 at every hour, Cloudy 1, Stormy / Foggy
    /// 0.502 by day; Stormy 0 at night.
    pub light_w: f32,
    /// unit vector towards the light (-gLightPosDir): by the orbit keys; below the horizon at night
    pub to_light: Vec3,
    /// `m_ShadowFade`: the sun factor runs from 1 - this in shade to 1 in light
    pub shadow_fade: f32,
    /// `m_NFLFade`: above 0 the game draws with its night programs
    pub nfl: f32,
    /// `m_OcclusionFades`
    pub occlusion: [f32; 2],
    /// `m_RoadReflection` (gReflectionParams.xy of the road's programs)
    pub road_reflection: [f32; 2],
    /// `m_BrightPass`: threshold, offset, (unused), white
    pub bright: [f32; 4],
    /// `m_Luminance`: middle grey, white
    pub luminance: [f32; 2],
    /// `m_LightAdaption`: rate (x 20 = Lambda), lower and upper limit of the adapted luminance
    pub adaption: [f32; 3],
    /// `m_Saturation`
    pub saturation: f32,
    /// `m_DoFParams` (DofParams of PSStreakMotionBlur): the distance the far blur begins at (m), its
    /// ramp (m), scale. Nice 517.2 from 18:00 to 06:00, 350 from 07:00 to 17:00, ramps between;
    /// Cloudy 500 / 250, Foggy 400 / 250, Stormy 250; all 3000 and 50. Zero: no such key.
    pub dof: [f32; 4],
    /// the weather's colour correction (the tone map multiplies by twice this)
    pub color_correct: Vec3,
    /// fog: colour and greatest opacity, where it begins and where it has built up (m)
    pub fog_color: [f32; 4],
    pub fog_near: f32,
    pub fog_far: f32,
    /// the particles' fog: the colour at the fog's height (rgb as keyed; the max colour is
    /// `fog_color`) and that height (m)
    pub fog_min: [f32; 4],
    pub fog_height: f32,
}

impl Tod {
    pub fn lerp(&self, o: &Tod, f: f32) -> Tod {
        let m = |a: f32, b: f32| a + (b - a) * f;
        let arr = |a: &[f32], b: &[f32], out: &mut [f32]| {
            for i in 0..out.len() {
                out[i] = m(a[i], b[i]);
            }
        };
        let mut t = *self;
        t.light = self.light.lerp(o.light, f);
        t.ambient = self.ambient.lerp(o.ambient, f);
        t.light_w = m(self.light_w, o.light_w);
        t.to_light = self.to_light.lerp(o.to_light, f).normalize_or(self.to_light);
        t.shadow_fade = m(self.shadow_fade, o.shadow_fade);
        t.nfl = m(self.nfl, o.nfl);
        arr(&self.occlusion, &o.occlusion, &mut t.occlusion);
        arr(&self.road_reflection, &o.road_reflection, &mut t.road_reflection);
        arr(&self.bright, &o.bright, &mut t.bright);
        arr(&self.luminance, &o.luminance, &mut t.luminance);
        arr(&self.adaption, &o.adaption, &mut t.adaption);
        t.saturation = m(self.saturation, o.saturation);
        arr(&self.dof, &o.dof, &mut t.dof);
        t.color_correct = self.color_correct.lerp(o.color_correct, f);
        arr(&self.fog_color, &o.fog_color, &mut t.fog_color);
        t.fog_near = m(self.fog_near, o.fog_near);
        t.fog_far = m(self.fog_far, o.fog_far);
        arr(&self.fog_min, &o.fog_min, &mut t.fog_min);
        t.fog_height = m(self.fog_height, o.fog_height);
        t
    }
}

/// Evaluated sky at one time of day.
#[derive(Debug, Clone, Copy)]
pub struct SkyState {
    pub zenith: Vec3,
    pub horizon: Vec3,
    pub horizon2: Vec3,
    pub sun_color: Vec3,
    pub sunset: Vec3,
    /// cloud colours (lit side, top layer)
    pub cloud: Vec3,
    pub cloud_top: Vec3,
    /// unit vector towards the sun (or the moon at night)
    pub sun_dir: Vec3,
    /// true sun direction (may be below the horizon)
    pub true_sun: Vec3,
    /// 0 = night, 1 = full day
    pub day: f32,
    /// azimuth tint strength at the horizon, display gain of the dome
    pub az_strength: f32,
    pub gain: f32,
    /// the game's keyed sun light (colour x intensity) and ambient light, when present
    pub light: Option<(Vec3, Vec3)>,
    /// the weather's shadow strength, when it keys one
    pub shadow: Option<f32>,
    /// the original's light state, when the weather's files are there
    pub tod: Option<Tod>,
    /// the dome's keys as they are (linear, HDR): sky, western and eastern azimuth colour,
    /// cloud and top cloud colour; and AzimuthStrength, AzimuthHeight
    pub dome: Option<([Vec3; 5], [f32; 2])>,
    /// the constants of the game's own sky dome programs (without the running cloud phase)
    pub hat: Option<crate::render::skyhat::HatConsts>,
}

impl SkyState {
    /// Tune-driven shadows include moonlight; daylight is only the fallback for
    /// the older weather set without m_ShadowFade.
    pub fn shadow_strength(&self) -> f32 {
        match self.shadow {
            Some(fade) => 0.86 * fade.clamp(0.0, 1.0),
            None => 0.82 * self.day.clamp(0.0, 1.0),
        }
    }

    /// Blend towards another state (weather transitions).
    pub fn lerp(&self, o: &SkyState, f: f32) -> SkyState {
        let l = |a: Vec3, b: Vec3| a.lerp(b, f);
        SkyState {
            zenith: l(self.zenith, o.zenith),
            horizon: l(self.horizon, o.horizon),
            horizon2: l(self.horizon2, o.horizon2),
            sun_color: l(self.sun_color, o.sun_color),
            sunset: l(self.sunset, o.sunset),
            cloud: l(self.cloud, o.cloud),
            cloud_top: l(self.cloud_top, o.cloud_top),
            sun_dir: self.sun_dir,
            true_sun: self.true_sun,
            day: self.day,
            az_strength: self.az_strength + (o.az_strength - self.az_strength) * f,
            gain: self.gain + (o.gain - self.gain) * f,
            light: match (self.light, o.light) {
                (Some(a), Some(b)) => Some((l(a.0, b.0), l(a.1, b.1))),
                (a, b) => a.or(b),
            },
            shadow: match (self.shadow, o.shadow) {
                (Some(a), Some(b)) => Some(a + (b - a) * f),
                (a, b) => a.or(b),
            },
            tod: match (self.tod, o.tod) {
                (Some(a), Some(b)) => Some(a.lerp(&b, f)),
                (a, b) => a.or(b),
            },
            hat: match (self.hat, o.hat) {
                (Some(a), Some(b)) => Some(a.lerp(&b, f)),
                (a, b) => a.or(b),
            },
            dome: match (self.dome, o.dome) {
                (Some(a), Some(b)) => Some(([l(a.0[0], b.0[0]), l(a.0[1], b.0[1]), l(a.0[2], b.0[2]), l(a.0[3], b.0[3]), l(a.0[4], b.0[4])], [a.1[0] + (b.1[0] - a.1[0]) * f, a.1[1] + (b.1[1] - a.1[1]) * f])),
                (a, b) => a.or(b),
            },
        }
    }
}

#[cfg(test)]
mod shadow_fade_tests {
    use super::*;

    #[test]
    fn tuned_moon_shadows_survive_zero_daylight_and_storms_disable_day_shadows() {
        let mut night = Sky::default().at(0.0);
        assert_eq!(night.day, 0.0);
        assert_eq!(night.shadow_strength(), 0.0);
        night.shadow = Some(0.8);
        assert!(night.shadow_strength() > 0.68);
        let mut storm = Sky::default().at(12.0);
        assert!(storm.day > 0.0);
        storm.shadow = Some(0.0);
        assert_eq!(storm.shadow_strength(), 0.0);
    }

    #[test]
    #[ignore = "requires user's extracted lighting; set MCLA_DATA_DIR"]
    fn complete_edition_shadow_schedule_controls_day_and_moonlight() {
        let root = std::env::var("MCLA_DATA_DIR").expect("set MCLA_DATA_DIR to extracted/cache");
        let nice = Sky::load_weather(&root, Weather::Nice);
        let night = nice.at(23.5);
        assert_eq!(night.day, 0.0);
        assert_eq!(night.shadow, Some(0.8));
        assert!(night.shadow_strength() > 0.0);
        assert_eq!(nice.at(6.0).shadow_strength(), 0.0);
        let storm = Sky::load_weather(&root, Weather::Stormy);
        for hour in [0.0, 6.0, 12.0, 18.0, 23.5] {
            assert_eq!(storm.at(hour).shadow_strength(), 0.0);
        }
    }
}

/// The hour the light manager (F-70200) evaluates every weather's keys at a second time for the
/// shadows: 9 + (h - 5) / 12 x 6 between 5:00 and 18:00 (9:00 .. 15:30), none outside.
pub fn shadow_hour(h: f32) -> Option<f32> {
    (h > 5.0 && h < 18.0).then(|| ((h - 5.0) * (1.0 / 12.0)).mul_add(6.0, 9.0))
}

/// The shadow pass's light direction (F-16780 for the cascades of F-55997): x of the light
/// direction at `shadow_hour`, y and z of the one at the hour itself, normalised - the shadows'
/// east-west swing runs at half speed round 11:00, so dawn and dusk shadows are short.
/// `to_light` and `x_at_shadow_hour` are the weathers' weighted sums of their (unit) light
/// directions, not normalised: the manager sums x, y and z apart and normalises once.
pub fn shadow_ray(to_light: Vec3, x_at_shadow_hour: f32) -> Vec3 {
    Vec3::new(x_at_shadow_hour, to_light.y, to_light.z).normalize_or(to_light)
}

/// F-16780's other branch: with a cine script running (or the scripts' force-cutscene-lighting
/// byte) while the garage's lighting is on, the light's way (-ray) gets (-0.2, 0, 0.1) added and
/// is normalised again.
pub fn shadow_ray_cutscene(ray: Vec3) -> Vec3 {
    (-ray + Vec3::new(-0.2, 0.0, 0.1)).normalize_or(-ray) * -1.0
}

/// `<tag x="" y="" z=""/>` of a tune file.
fn xyz(x: &str, tag: &str) -> Option<[f32; 3]> {
    let r = &x[x.find(&format!("<{} ", tag))?..];
    let r = &r[..r.find('>')?];
    let get = |a: &str| -> Option<f32> {
        let i = r.find(&format!("{}=\"", a))? + 3;
        r[i..i + r[i..].find('"')?].trim().parse().ok()
    };
    Some([get("x")?, get("y")?, get("z")?])
}

impl Sky {
    pub fn load(data_dir: &str) -> Sky {
        Self::load_weather(data_dir, Weather::Clear)
    }

    pub fn load_weather(data_dir: &str, weather: Weather) -> Sky {
        let dir = Path::new(data_dir).join("tune/sky");
        let mut s = Sky::default();
        let read = |n: &str| std::fs::read_to_string(dir.join(n)).ok();
        // the weather's keyed sun light lives in a hash-named file of tune/lighting
        let want = format!("<szName>{}</szName>", weather.light_name());
        if let Ok(rd) = std::fs::read_dir(Path::new(data_dir).join("tune/lighting")) {
            for e in rd.flatten() {
                let p = e.path();
                if !p.is_file() || p.metadata().map_or(true, |m| m.len() < 2000) {
                    continue;
                }
                let Ok(x) = std::fs::read_to_string(&p) else { continue };
                if x.contains(&want) {
                    s.light = tracks(&x, "m_KeyColor").into_iter().next().unwrap_or_default();
                    s.ambient = tracks(&x, "m_KeyAmbLight").into_iter().next().unwrap_or_default();
                    s.luminance = tracks(&x, "m_Luminance").into_iter().next().unwrap_or_default();
                    s.shadow_fade = tracks(&x, "m_ShadowFade").into_iter().next().unwrap_or_default();
                    s.nfl_fade = tracks(&x, "m_NFLFade").into_iter().next().unwrap_or_default();
                    s.orbit = tracks(&x, "m_KeyOrbit").into_iter().next().unwrap_or_default();
                    s.occlusion_fades = tracks(&x, "m_OcclusionFades").into_iter().next().unwrap_or_default();
                    s.road_reflection = tracks(&x, "m_RoadReflection").into_iter().next().unwrap_or_default();
                    s.bright_pass = tracks(&x, "m_BrightPass").into_iter().next().unwrap_or_default();
                    s.adaption = tracks(&x, "m_LightAdaption").into_iter().next().unwrap_or_default();
                    s.saturation = tracks(&x, "m_Saturation").into_iter().next().unwrap_or_default();
                    s.dof = tracks(&x, "m_DoFParams").into_iter().next().unwrap_or_default();
                    break;
                }
            }
        }
        // the weather manager's file: this weather's colour correction and fog (the older
        // suffix-less set has none of its own: Nice's)
        if let Ok(x) = std::fs::read_to_string(Path::new(data_dir).join("tune/lighting/0xa838c380")) {
            let w = match weather {
                Weather::Clear | Weather::Nice => "Nice",
                Weather::Cloudy => "Cloudy",
                Weather::Foggy => "Foggy",
                Weather::Stormy => "Stormy",
            };
            let one = |n: &str| tracks(&x, &format!("sm_Key{}Default{}", n, w)).into_iter().next().unwrap_or_default();
            s.color_correct = one("ColorCorrect");
            s.fog_max = one("MaxFogColor");
            s.fog_near = one("FogNearZ");
            s.fog_far = one("FogFarZ");
            s.fog_min = one("MinFogColor");
            s.fog_height = one("FogHeight");
        }
        if let Some(x) = read(weather.sky_file()) {
            let first = |n: &str| tracks(&x, n).into_iter().next().unwrap_or_default();
            s.sky_color = first("SkyColorKF");
            let az = tracks(&x, "AzimuthColorKF");
            s.azimuth = az.first().cloned().unwrap_or_default();
            s.azimuth2 = az.get(1).cloned().unwrap_or_else(|| s.azimuth.clone());
            s.sun_color = first("SunColorKF");
            s.sunset = first("SunsetColorKF");
            s.cloud = first("CloudColorKF");
            s.cloud_top = first("TopCloudColorKF");
            s.az_strength = first("CloudShadowOffsetThruAzymuthStrengthKF");
            s.star = first("UnStrengthThruCSpeedThruStarBrightnessKF");
            s.cloud_threshold = first("CloudThresholdThruCloudShadowStrengthKF");
            s.detail = first("DetailScaleThruDetailStrengthWithEdgeSmoothThicknessKF");
            s.top_bias = first("TBiasThruTDetailThruThresholdThruHeightKF");
            s.sun_centre = first("SunCentreKF");
            s.moon_color = first("m_MoonColorKFAndBrightness");
            s.moon_glow = first("m_MoonBrightnessGlowStarBrightnessKF");
            let value = |tag: &str, d: f32| -> f32 {
                x.find(&format!("<{} value=\"", tag)).and_then(|i| {
                    let r = &x[i + tag.len() + 9..];
                    r[..r.find('"')?].trim().parse().ok()
                }).unwrap_or(d)
            };
            s.hdr = [value("HDRExposure", 0.8), value("HDRSunExposure", 0.1)];
            s.hdr_clamp = xyz(&x, "HDRExposureClamp").unwrap_or([2.38, 1.52, 1.48]);
            s.gain = if weather == Weather::Clear { 1.5 } else { 2.4 };
        }
        // rage__SkyhatMiniNoise (the sky control object's own tune)
        s.star_repeat = read("0xbe56a98a").and_then(|x| {
            let i = x.find("<StarFieldRepeat value=\"")? + 24;
            x[i..i + x[i..].find('"')?].trim().parse().ok()
        }).unwrap_or(26.1875);
        if let Some(x) = read("SunDirection.xml") {
            s.sun_dir = tracks(&x, "SunDirectionKF").into_iter().next().unwrap_or_default();
        }
        s.loaded = !s.sky_color.keys.is_empty() && !s.sun_dir.keys.is_empty();
        log::info!(
            "sky ({}): {} light keys, {} ambient keys, {} sky keys, {} azimuth keys, {} sun colour keys, {} sun direction keys",
            weather.name(),
            s.light.keys.len(),
            s.ambient.keys.len(),
            s.sky_color.keys.len(),
            s.azimuth.keys.len(),
            s.sun_color.keys.len(),
            s.sun_dir.keys.len()
        );
        s
    }

    pub fn at(&self, t: f32) -> SkyState {
        let t = t.rem_euclid(24.0);
        if !self.loaded {
            let ang = (t - 6.0) / 12.0 * std::f32::consts::PI;
            let sun = Vec3::new(ang.cos(), ang.sin(), 0.3).normalize();
            let day = sun.y.clamp(0.0, 1.0);
            return SkyState {
                zenith: Vec3::new(0.3, 0.45, 0.7) * day + Vec3::new(0.02, 0.03, 0.05),
                horizon: Vec3::new(0.6, 0.68, 0.8) * day + Vec3::new(0.03, 0.03, 0.05),
                horizon2: Vec3::new(0.6, 0.68, 0.8) * day + Vec3::new(0.03, 0.03, 0.05),
                sun_color: Vec3::new(1.0, 0.92, 0.8),
                sunset: Vec3::new(0.9, 0.5, 0.38),
                cloud: Vec3::splat(0.3) * day,
                cloud_top: Vec3::splat(0.2) * day,
                sun_dir: if sun.y > 0.0 { sun } else { Vec3::new(-sun.x, -sun.y, sun.z).normalize() },
                true_sun: sun,
                day,
                az_strength: 0.3,
                gain: 1.0,
                light: None,
                shadow: None,
                tod: None,
                dome: None,
                hat: None,
            };
        }
        let sd = self.sun_dir.rgb(t).normalize_or(Vec3::Y);
        // the game stores +y up; x = east at sunrise
        let mut day = ((sd.y + 0.08) / 0.30).clamp(0.0, 1.0);
        // (the weather's own schedule of night: its night lighting fades in as the day goes)
        if !self.nfl_fade.keys.is_empty() {
            day = 1.0 - self.nfl_fade.sample(t)[0].clamp(0.0, 1.0);
        }
        let lit = if sd.y > -0.02 { sd } else { Vec3::new(-0.072, 0.215, 0.974).normalize() };
        let lit = Vec3::new(lit.x, lit.y.max(0.12), lit.z).normalize();
        // keyframes are authored in display (gamma) space; the renderer works in linear
        let lin = |c: Vec3| Vec3::new(c.x.max(0.0).powf(2.2), c.y.max(0.0).powf(2.2), c.z.max(0.0).powf(2.2));
        SkyState {
            zenith: lin(self.sky_color.rgb(t)),
            horizon: lin(self.azimuth.rgb(t)),
            horizon2: lin(self.azimuth2.rgb(t)),
            sun_color: lin(self.sun_color.rgb(t)),
            sunset: lin(self.sunset.rgb(t)),
            cloud: lin(self.cloud.rgb(t)),
            cloud_top: lin(self.cloud_top.rgb(t)),
            sun_dir: lit,
            true_sun: sd,
            day,
            az_strength: if self.az_strength.keys.is_empty() { 0.3 } else { self.az_strength.sample(t)[3].clamp(0.0, 1.0) },
            gain: self.gain.max(1.0),
            light: if self.light.keys.is_empty() || self.ambient.keys.is_empty() {
                None
            } else {
                let c = self.light.sample(t);
                let i = c[3].max(0.0);
                let (sun, amb) = (Vec3::new(c[0].max(0.0), c[1].max(0.0), c[2].max(0.0)) * i, self.ambient.rgb(t).max(Vec3::ZERO));
                // The HDR weathers (those that key the tone mapper) are authored dark and
                // exposed by the game: the scene's average luminance - here estimated from
                // the two lights on a mid-grey world - is brought to the keyed middle grey.
                // (How far the game lets the exposure climb is not in the data: 3 by day; at night,
                // where its meter also sees the lamps and lit windows, 1.3.)
                let e = if self.luminance.keys.is_empty() || self.nfl_fade.keys.is_empty() {
                    1.0
                } else {
                    let lum = |v: Vec3| v.dot(Vec3::new(0.2126, 0.7152, 0.0722));
                    (self.luminance.sample(t)[0] / (0.35 * (0.5 * lum(sun) + lum(amb))).max(1e-4)).clamp(0.5, 3.0 - 1.7 * self.nfl_fade.sample(t)[0].clamp(0.0, 1.0))
                };
                Some((sun * e, amb * e))
            },
            shadow: if self.shadow_fade.keys.is_empty() { None } else { Some(self.shadow_fade.sample(t)[0].clamp(0.0, 1.0)) },
            tod: self.tod(t),
            hat: self.hat(t, sd),
            dome: if self.star.keys.is_empty() || self.az_strength.keys.is_empty() {
                None
            } else {
                Some(([self.sky_color.rgb(t), self.azimuth.rgb(t), self.azimuth2.rgb(t), self.cloud.rgb(t), self.cloud_top.rgb(t)], [self.az_strength.sample(t)[3], self.star.sample(t)[2]]))
            },
        }
    }

    /// The constants of the game's sky dome programs at an hour, from the weather's sky file
    /// (which key feeds which constant: found by comparing the file with the constants recorded
    /// from the running game at nine hours - LIGHTING_WORK.md). `sun` is the dome's own sun
    /// direction (`SunDirection.xml`). The cloud phase (Perlin c72, DetailOffset) is left 0:
    /// it runs with time, at the cloud speed kept in `perlin[8][1]`.
    pub fn hat(&self, t: f32, sun: Vec3) -> Option<crate::render::skyhat::HatConsts> {
        if self.cloud_threshold.keys.is_empty() || self.detail.keys.is_empty() || self.top_bias.keys.is_empty() || self.star.keys.is_empty() || self.az_strength.keys.is_empty() || self.sun_centre.keys.is_empty() {
            return None;
        }
        let (ct, ds, tb, st, cso) = (self.cloud_threshold.sample(t), self.detail.sample(t), self.top_bias.sample(t), self.star.sample(t), self.az_strength.sample(t));
        let (sc, mc, mg) = (self.sun_centre.sample(t), self.moon_color.sample(t), self.moon_glow.sample(t));
        let v3 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let one = |x: f32| [x, 0.0, 0.0, 0.0];
        let (sky, az_w, az_e) = (self.sky_color.rgb(t), self.azimuth.rgb(t), self.azimuth2.rgb(t));
        let (sun_c, sunset, cloud, top) = (self.sun_color.sample(t), self.sunset.rgb(t), self.cloud.rgb(t), self.cloud_top.rgb(t));
        // thickness, 1 / edge smoothing, detail scale, detail strength
        let thick = [ct[2], 1.0 / cso[2].max(1e-4), ds[0], ds[1]];
        // (w is the height's MAGNITUDE: at 23:30 the recording has (-0.047, -0.999, 0, +0.999) where
        // the plain height stood here - the one constant of 36 that differed at night, 2026-10-14)
        let sun4 = [sun.x, sun.y, sun.z, sun.y.abs()];
        let disc = 10.0 / (sc[0] * sc[0]).max(1e-9);
        Some(crate::render::skyhat::HatConsts {
            vs: [
                one(cso[0]),
                one(cso[3]),
                thick,
                // StarFieldUVRepeat: SkyhatMiniNoise StarFieldRepeat (26.1875; the recordings'
                // 26.19), pushed as it is by the sky control object (F-52332)
                one(self.star_repeat),
                [0.0; 4],
                [0.0; 4],
                v3(sky, 0.0),
                sun4,
                v3(az_w, 0.0),
                v3(az_e, 0.0),
                one(st[2]),
                tb,
            ],
            ps: [
                // SunCentre: recorded (-8317, 8318, 113.9, 1 / 3) with SunCentreKF (0.0347, 0, 3.559, -1):
                // the disc's threshold 10 / x squared, and 9 z squared
                [1.0 - disc, disc, 9.0 * sc[2] * sc[2], 1.0 / 3.0],
                sun4,
                [sun_c[0], sun_c[1], sun_c[2], 0.0],
                one(mg[1]),
                v3(cloud, 1.0),
                v3(sunset * st[0], st[0]),
                one(ct[0]),
                one(ct[1]),
                one(ds[3]),
                tb,
                v3(top + sunset, 0.0),
                one(ct[3]),
                one(cso[1]),
                thick,
                // StarFieldBrightness (F-52332): with s the star brightness and t the threshold
                // (channels 2 and 3 of m_MoonBrightnessGlowStarBrightnessKF), (3 s s, 0.24 s,
                // 3 s s t, 4); (0, 0, 0, 4) in Nice weather, whose s is 0
                [mg[2] * mg[2] * 3.0, mg[2] * 0.24, mg[3] * (mg[2] * mg[2] * 3.0), 4.0],
                // MoonLight, MoonVisiblity, SunSize, MoonColorConstant, MoonPosition, MoonXVector,
                // MoonYVector: the same in every recording (the moon below the horizon)
                [0.25, 1.0 / 6.0, 0.75, 1.0 / 3.0],
                one(st[3]),
                [0.0, sun_c[3], 0.0, 0.0],
                [mc[0] * mg[0], mc[1] * mg[0], mc[2] * mg[0], mg[0]],
                [15.63, -15.41, 5.595, 0.0],
                [-15.96, 0.0, -1.183, 0.0],
                [0.2549, 15.62, -3.438, 0.0],
                one(self.hdr[0]),
                one(self.hdr[1]),
            ],
            // The Perlin pass: velocities, frequencies, contrast, brightness, persistance and
            // clump as recorded (their source is not the sky file); NoiseWarp is the third
            // channel of the detail key; [8] = (phase, cloud speed)
            perlin: [[-0.01, 0.01, 0.0, 0.0], [0.005, 0.02, 0.0, 0.0], one(1.9), one(1.0), one(1.1), one(0.76), one(ds[2]), one(1.0 / 1.3), [0.0, st[1], 0.0, 0.0], one(1.0)],
            // HDRExposureClamp of the weather (w as recorded)
            clamp: [self.hdr_clamp[0], self.hdr_clamp[1], self.hdr_clamp[2], 2.0],
        })
    }

    /// The original's light state at an hour (None for a weather without the full key set).
    pub fn tod(&self, t: f32) -> Option<Tod> {
        if self.light.keys.is_empty() || self.ambient.keys.is_empty() || self.orbit.keys.is_empty() || self.bright_pass.keys.is_empty() || self.luminance.keys.is_empty() {
            return None;
        }
        let c = self.light.sample(t);
        let a = self.ambient.sample(t);
        // the orbit: degrees round the sky (0 at noon) and the tilt of its plane; the light
        // travels along (sin a, -cos a cos t, -cos a sin t)
        let o = self.orbit.sample(t);
        let (az, tilt) = (o[0].to_radians(), o[1].to_radians());
        let to_light = Vec3::new(-az.sin(), az.cos() * tilt.cos(), az.cos() * tilt.sin());
        let bp = self.bright_pass.sample(t);
        let lum = self.luminance.sample(t);
        let ad = if self.adaption.keys.is_empty() { [0.186, 0.15, 0.2, 0.0] } else { self.adaption.sample(t) };
        let of = if self.occlusion_fades.keys.is_empty() { [1.0, 1.0, 0.0, 0.0] } else { self.occlusion_fades.sample(t) };
        let cc = if self.color_correct.keys.is_empty() { [0.5, 0.5, 0.5, 0.0] } else { self.color_correct.sample(t) };
        let fog = if self.fog_max.keys.is_empty() { [0.1, 0.125, 0.15, 0.665] } else { self.fog_max.sample(t) };
        Some(Tod {
            light: Vec3::new(c[0], c[1], c[2]),
            ambient: Vec3::new(a[0], a[1], a[2]),
            light_w: c[3],
            to_light,
            shadow_fade: if self.shadow_fade.keys.is_empty() { 0.9 } else { self.shadow_fade.sample(t)[0] },
            nfl: if self.nfl_fade.keys.is_empty() { 0.0 } else { self.nfl_fade.sample(t)[0] },
            occlusion: [of[0], of[1]],
            road_reflection: if self.road_reflection.keys.is_empty() { [0.0948, 0.0948] } else { let r = self.road_reflection.sample(t); [r[0], r[1]] },
            bright: bp,
            luminance: [lum[0], lum[1]],
            adaption: [ad[0], ad[1], ad[2]],
            saturation: if self.saturation.keys.is_empty() { 0.9 } else { self.saturation.sample(t)[0] },
            dof: if self.dof.keys.is_empty() { [0.0; 4] } else { self.dof.sample(t) },
            color_correct: Vec3::new(cc[0], cc[1], cc[2]),
            fog_color: fog,
            fog_near: if self.fog_near.keys.is_empty() { 50.0 } else { self.fog_near.sample(t)[0] },
            fog_far: if self.fog_far.keys.is_empty() { 500.0 } else { self.fog_far.sample(t)[0] },
            fog_min: if self.fog_min.keys.is_empty() { [0.0; 4] } else { self.fog_min.sample(t) },
            fog_height: if self.fog_height.keys.is_empty() { 1000.0 } else { self.fog_height.sample(t)[0] },
        })
    }
}

#[cfg(test)]
mod tod_tests {
    use super::*;

    /// The values the running original feeds its shaders at these hours (work/lighting/sweep_nice.tsv).
    #[test]
    #[ignore = "requires user's extracted lighting; set MCLA_DATA_DIR"]
    fn nice_weather_light_state_matches_the_recorded_original() {
        let root = std::env::var("MCLA_DATA_DIR").expect("set MCLA_DATA_DIR to extracted/cache");
        let nice = Sky::load_weather(&root, Weather::Nice);
        let near = |a: Vec3, b: [f32; 3], eps: f32| (a - Vec3::from(b)).abs().max_element() < eps;
        let noon = nice.tod(12.0).unwrap();
        assert!(near(noon.light, [1.0, 0.90588, 0.58823], 1e-4), "{:?}", noon.light);
        assert!(near(noon.ambient, [0.047422, 0.12551, 0.19971], 1e-4), "{:?}", noon.ambient);
        assert!(near(noon.to_light, [0.0, 0.86745, 0.49753], 2e-3), "{:?}", noon.to_light);
        assert!(near(noon.color_correct, [0.5862, 0.53442, 0.43101], 1e-4), "{:?}", noon.color_correct);
        assert!((noon.bright[0] - 1.6203).abs() < 1e-3 && (noon.bright[3] * noon.bright[3] - 0.032782).abs() < 1e-4);
        assert!((noon.luminance[0] - 0.25).abs() < 1e-5 && (noon.luminance[1] * noon.luminance[1] - 0.21646).abs() < 1e-4);
        let nine = nice.tod(9.0).unwrap();
        assert!(near(nine.light, [1.0, 0.82815, 0.42227], 1e-4));
        assert!(near(nine.to_light, [0.60876, 0.7648, 0.21092], 2e-3), "{:?}", nine.to_light);
        assert!((nine.shadow_fade - (1.0 - 2.0 * 0.030604)).abs() < 1e-4);
        let dusk = nice.tod(17.4).unwrap();
        assert!(near(dusk.light, [1.0, 0.44076, 0.058824], 1e-4));
        assert!(near(dusk.to_light, [-0.92388, 0.38179, 0.026139], 2e-3), "{:?}", dusk.to_light);
        assert!(near(dusk.color_correct, [0.58205, 0.49732, 0.41441], 1e-4));
        let night = nice.tod(0.0).unwrap();
        assert!(near(night.light, [0.047059, 0.047059, 0.047059], 1e-5));
        assert!(near(night.to_light, [0.5, -0.86589, -0.015114], 2e-3), "{:?}", night.to_light);
        assert!(night.nfl > 0.99);
    }

    /// The cascades' shadow ray (`shadow_ray`) against the eleven hours read off the original's
    /// shadow matrices (LIGHTING.md, "Shadows are not cast along the light"): (az, el) in degrees.
    #[test]
    #[ignore = "requires user's extracted lighting; set MCLA_DATA_DIR"]
    fn shadow_ray_matches_the_recorded_cascades() {
        let root = std::env::var("MCLA_DATA_DIR").expect("set MCLA_DATA_DIR to extracted/cache");
        let nice = Sky::load_weather(&root, Weather::Nice);
        let rec = [(6.25, -57.8, 32.1), (7.0, -42.5, 47.2), (9.0, -15.3, 68.4), (10.0, -6.8, 68.7), (11.0, 0.0, 65.0), (12.0, 6.2, 59.5), (13.0, 12.5, 62.0), (15.0, 28.0, 58.2), (17.0, 52.8, 36.9), (17.5, 60.9, 29.1), (17.9, 67.7, 22.3)];
        for (h, az, el) in rec {
            let real = nice.tod(h).unwrap().to_light;
            let x = shadow_hour(h).map_or(real.x, |h2| nice.tod(h2).unwrap().to_light.x);
            let r = shadow_ray(real, x);
            let (a, e) = ((-r.x).asin().to_degrees(), r.y.asin().to_degrees());
            println!("{h:5.2}: az {a:6.2} (rec {az}) el {e:5.2} (rec {el})");
            assert!((a - az).abs() < 0.15 && (e - el).abs() < 0.15, "{h}: {a} {e}");
        }
    }

    /// F-16780's cutscene branch on the light's way: normalize(way + (-0.2, 0, 0.1)).
    #[test]
    fn shadow_ray_cutscene_offset() {
        let ray = Vec3::new(0.0, 0.6, 0.8);
        let way = (Vec3::new(0.0, -0.6, -0.8) + Vec3::new(-0.2, 0.0, 0.1)).normalize();
        assert!((shadow_ray_cutscene(ray) + way).length() < 1e-6);
    }

    /// The sky control object's own constants: StarFieldUVRepeat from its tune file, the
    /// weather's HDRExposureClamp (the mini sky's c100, recorded clear and stormy).
    #[test]
    #[ignore = "requires user's extracted lighting; set MCLA_DATA_DIR"]
    fn sky_control_constants() {
        let root = std::env::var("MCLA_DATA_DIR").expect("set MCLA_DATA_DIR to extracted/cache");
        let nice = Sky::load_weather(&root, Weather::Nice);
        let storm = Sky::load_weather(&root, Weather::Stormy);
        let h = nice.hat(0.0, Vec3::Y).unwrap();
        assert_eq!(h.vs[3][0], 26.1875);
        assert_eq!(h.clamp, [2.38, 1.52, 1.48, 2.0]);
        assert_eq!(storm.hat(0.0, Vec3::Y).unwrap().clamp, [2.0, 2.0, 2.6625, 2.0]);
        // StarFieldBrightness at night where the weather has stars
        for w in Weather::ALL {
            let s = Sky::load_weather(&root, w);
            if let Some(h) = s.hat(0.0, Vec3::Y) {
                let mg = s.moon_glow.sample(0.0);
                assert_eq!(h.ps[14], [mg[2] * mg[2] * 3.0, mg[2] * 0.24, mg[3] * (mg[2] * mg[2] * 3.0), 4.0]);
                println!("{:?}: star brightness {} threshold {} -> c78 {:?}", w, mg[2], mg[3], h.ps[14]);
            }
        }
    }
}
/// The garage's own light state (garage round, 2026-10-13; LIGHTING_WORK.md "The garage"), as
/// the running original feeds it to its programs in every garage frame recorded
/// (work/lighting/cap/gar_*.bin - the same at a clock of 12:00 and of 23:30):
///   - the scene's keys are the light file `TODGarage<Weather>` (one key each: gLightColor a
///     grey of 0.0196 straight down, gLightAmbient (0.0353, 0.0510, 0.0275), m_NFLFade 1,
///     m_BrightPass 2.31 / 8 / 1.24, m_Luminance 0.4 / 1, m_Saturation 1, m_RoadReflection
///     1.112 / 0.379);
///   - the sky's and the fog's own keys are read at `sm_fGarageTOD` (19:00);
///   - the cars' two lights and their ambient are the light manager's `sm_vGarageLight0Color`
///     / `sm_fGarageLight0Intensity` (a point 4 m above the car; intensity 0 on the disc),
///     `sm_vGarageLight1Color` / `sm_fGarageLight1Intensity` (a point 1 m above the CAMERA),
///     `sm_vGarageAmbientLighting` - all in `tune/lighting/GlowTuning.xml`;
///   - the colour correction and the fog are those of the area locators of the weather
///     manager's file that hold the car (`mcLocatorColorFilter`, `mcLocatorFogGround` in
///     `tune/lighting/0xa838c380`; four keys each: Nice, Cloudy, Stormy, Foggy).
#[derive(Debug, Clone, Copy)]
pub struct GarageRig {
    pub light0: Vec3,
    pub int0: f32,
    pub light1: Vec3,
    pub int1: f32,
    pub ambient: Vec3,
    /// `sm_fGarageTOD`
    pub hour: f32,
}

/// `LightManagerTune`'s garage figures (None when the file or a figure is missing).
pub fn garage_rig(data_dir: &str) -> Option<GarageRig> {
    static RIG: std::sync::OnceLock<Option<GarageRig>> = std::sync::OnceLock::new();
    *RIG.get_or_init(|| {
        let x = std::fs::read_to_string(Path::new(data_dir).join("tune/lighting/GlowTuning.xml")).ok()?;
        let attr = |tag: &str, a: &str| -> Option<f32> {
            let i = x.find(&format!("<{} ", tag))?;
            let r = &x[i..i + x[i..].find("/>")?];
            let j = r.find(&format!("{}=\"", a))? + a.len() + 2;
            r[j..j + r[j..].find('"')?].trim().parse().ok()
        };
        let v3 = |tag: &str| -> Option<Vec3> { Some(Vec3::new(attr(tag, "x")?, attr(tag, "y")?, attr(tag, "z")?)) };
        Some(GarageRig {
            light0: v3("sm_vGarageLight0Color")?,
            int0: attr("sm_fGarageLight0Intensity", "value")?,
            light1: v3("sm_vGarageLight1Color")?,
            int1: attr("sm_fGarageLight1Intensity", "value")?,
            ambient: v3("sm_vGarageAmbientLighting")?,
            hour: attr("sm_fGarageTOD", "value")?,
        })
    })
}

/// One area locator of the weather manager's file: its outline (world x, z) and its keyed
/// tracks by tag, four of each (Nice, Cloudy, Stormy, Foggy).
struct AreaLocator {
    outline: Vec<[f32; 2]>,
    keys: Vec<(String, Vec<Track>)>,
}

impl AreaLocator {
    fn holds(&self, x: f32, z: f32) -> bool {
        let p = &self.outline;
        let mut inside = false;
        let mut j = p.len().wrapping_sub(1);
        for i in 0..p.len() {
            if (p[i][1] > z) != (p[j][1] > z) && x < (p[j][0] - p[i][0]) * (z - p[i][1]) / (p[j][1] - p[i][1]) + p[i][0] {
                inside = !inside;
            }
            j = i;
        }
        inside
    }
    fn key(&self, tag: &str, weather: usize, hour: f32) -> Option<[f32; 4]> {
        let t = &self.keys.iter().find(|k| k.0 == tag)?.1;
        t.get(weather).or(t.first()).filter(|t| !t.keys.is_empty()).map(|t| t.sample(hour))
    }
}

/// The locators of one kind (`mcLocatorColorFilter`, `mcLocatorFogGround`) drawn as an outline
/// (Shape 2: the points are offsets from the locator's position).
fn area_locators(xml: &str, kind: &str, tags: &[&str]) -> Vec<AreaLocator> {
    let mut out = Vec::new();
    let Some(a) = xml.find(&format!("<{}>", kind)) else { return out };
    let Some(b) = xml[a..].find(&format!("</{}>", kind)) else { return out };
    let open = format!("<Item type=\"{}\">", kind);
    for it in xml[a..a + b].split(open.as_str()).skip(1) {
        let attr = |tag: &str, n: &str| -> Option<f32> {
            let i = it.find(&format!("<{} ", tag))?;
            let r = &it[i..i + it[i..].find("/>")?];
            let j = r.find(&format!("{}=\"", n))? + n.len() + 2;
            r[j..j + r[j..].find('"')?].trim().parse().ok()
        };
        if attr("Shape", "value") != Some(2.0) {
            continue;
        }
        let (Some(px), Some(pz)) = (attr("Position", "x"), attr("Position", "z")) else { continue };
        let Some(i) = it.find("<Points") else { continue };
        let Some(j) = it[i..].find('>') else { continue };
        let Some(e) = it[i..].find("</Points>") else { continue };
        let v: Vec<f32> = it[i + j + 1..i + e].split_whitespace().filter_map(|s| s.parse().ok()).collect();
        let outline: Vec<[f32; 2]> = v.chunks_exact(3).map(|p| [px + p[0], pz + p[2]]).collect();
        if outline.len() < 3 {
            continue;
        }
        let mut keys = Vec::new();
        for tag in tags {
            let (o, c) = (format!("<{}>", tag), format!("</{}>", tag));
            if let Some(s) = it.find(&o) {
                if let Some(n) = it[s..].find(&c) {
                    keys.push((tag.to_string(), tracks(&it[s..s + n], "Item")));
                }
            }
        }
        out.push(AreaLocator { outline, keys });
    }
    out
}

/// The scene's light state in the garage the car at `at` stands in: `TODGarage<Weather>`'s keys,
/// with the colour correction and the fog of the area locators that hold the place (the
/// weather's own at the garage's hour where none does).
pub fn garage_tod(data_dir: &str, weather: Weather, at: Vec3) -> Option<Tod> {
    struct Set {
        skies: Vec<(Weather, Sky)>,
        filters: Vec<AreaLocator>,
        fogs: Vec<AreaLocator>,
    }
    static SET: std::sync::Mutex<Option<Set>> = std::sync::Mutex::new(None);
    let rig = garage_rig(data_dir)?;
    let mut guard = SET.lock().ok()?;
    let set = guard.get_or_insert_with(|| {
        let x = std::fs::read_to_string(Path::new(data_dir).join("tune/lighting/0xa838c380")).unwrap_or_default();
        Set {
            skies: Vec::new(),
            filters: area_locators(&x, "mcLocatorColorFilter", &["KeyColorCorrect"]),
            fogs: area_locators(&x, "mcLocatorFogGround", &["KeyMaxFogColor", "KeyFogNearZ", "KeyFogFarZ"]),
        }
    });
    if !set.skies.iter().any(|s| s.0 == weather) {
        let mut s = Sky::load_weather(data_dir, weather);
        let name = match weather {
            Weather::Clear | Weather::Nice => "TODGarageNice",
            Weather::Cloudy => "TODGarageCloudy",
            Weather::Foggy => "TODGarageFoggy",
            Weather::Stormy => "TODGarageStormy",
        };
        let ok = s.read_light(data_dir, name);
        if !ok {
            s.light = Track::default();
        }
        set.skies.push((weather, s));
    }
    let sky = &set.skies.iter().find(|s| s.0 == weather)?.1;
    let mut t = sky.tod(rig.hour)?;
    // (the locators' four keys: Nice, Cloudy, Stormy, Foggy)
    let w = match weather {
        Weather::Clear | Weather::Nice => 0,
        Weather::Cloudy => 1,
        Weather::Stormy => 2,
        Weather::Foggy => 3,
    };
    if let Some(c) = set.filters.iter().find(|l| l.holds(at.x, at.z)).and_then(|l| l.key("KeyColorCorrect", w, rig.hour)) {
        t.color_correct = Vec3::new(c[0], c[1], c[2]);
    }
    if let Some(l) = set.fogs.iter().find(|l| l.holds(at.x, at.z)) {
        if let Some(c) = l.key("KeyMaxFogColor", w, rig.hour) {
            t.fog_color = c;
        }
        if let Some(c) = l.key("KeyFogNearZ", w, rig.hour) {
            t.fog_near = c[0];
        }
        if let Some(c) = l.key("KeyFogFarZ", w, rig.hour) {
            t.fog_far = c[0];
        }
    }
    Some(t)
}

impl Sky {
    /// Replaces the keyed light tracks by those of the light file named `name` (`TODGarage*`).
    fn read_light(&mut self, data_dir: &str, name: &str) -> bool {
        let want = format!("<szName>{}</szName>", name);
        let Ok(rd) = std::fs::read_dir(Path::new(data_dir).join("tune/lighting")) else { return false };
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_file() || p.metadata().map_or(true, |m| m.len() < 2000) {
                continue;
            }
            let Ok(x) = std::fs::read_to_string(&p) else { continue };
            if x.contains(&want) {
                let one = |n: &str| tracks(&x, n).into_iter().next().unwrap_or_default();
                self.light = one("m_KeyColor");
                self.ambient = one("m_KeyAmbLight");
                self.luminance = one("m_Luminance");
                self.shadow_fade = one("m_ShadowFade");
                self.nfl_fade = one("m_NFLFade");
                self.orbit = one("m_KeyOrbit");
                self.occlusion_fades = one("m_OcclusionFades");
                self.road_reflection = one("m_RoadReflection");
                self.bright_pass = one("m_BrightPass");
                self.adaption = one("m_LightAdaption");
                self.saturation = one("m_Saturation");
                self.dof = one("m_DoFParams");
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod garage_tests {
    use super::*;

    /// The values recorded in the Hollywood garage (work/lighting/cap/gar_top.bin).
    #[test]
    #[ignore = "requires user's extracted lighting; set MCLA_DATA_DIR"]
    fn garage_light_state_matches_the_recorded_original() {
        let root = std::env::var("MCLA_DATA_DIR").expect("set MCLA_DATA_DIR to extracted/cache");
        let near = |a: Vec3, b: [f32; 3], eps: f32| (a - Vec3::from(b)).abs().max_element() < eps;
        let rig = garage_rig(&root).unwrap();
        assert!(near(rig.light1, [1.0, 0.98823, 0.85098], 1e-4) && rig.int1 == 1.0 && rig.int0 == 0.0 && rig.hour == 19.0);
        let t = garage_tod(&root, Weather::Nice, Vec3::new(861.77, 17.0, -809.72)).unwrap();
        assert!(near(t.light, [0.019608, 0.019608, 0.019608], 1e-5), "{:?}", t.light);
        assert!(near(t.ambient, [0.035294, 0.05098, 0.027451], 1e-5), "{:?}", t.ambient);
        assert!(near(t.to_light, [0.0, 1.0, 0.0], 1e-5));
        assert!(near(t.color_correct, [0.37934, 0.39676, 0.483], 1e-4), "{:?}", t.color_correct);
        assert!((t.fog_color[3] - 0.68628).abs() < 1e-4 && (t.fog_far - 124.88).abs() < 0.01 && t.fog_near == 0.0, "{:?} {} {}", t.fog_color, t.fog_near, t.fog_far);
        assert!((t.bright[0] - 2.3103).abs() < 1e-3 && (t.bright[3] * t.bright[3] - 1.541).abs() < 1e-3 && t.luminance == [0.4, 1.0] && t.nfl == 1.0);
        // (outside every outline: the weather's own colour correction at 19:00)
        let o = garage_tod(&root, Weather::Nice, Vec3::new(0.0, 0.0, 0.0)).unwrap();
        assert!(near(o.color_correct, [0.49569, 0.5, 0.5], 1e-4));
    }
}
