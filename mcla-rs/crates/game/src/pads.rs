//! The game pads, read through SDL3's gamepad layer (static SDL, built from source by `sdl3-sys`).
//! SDL's own Windows backends (XInput, RawInput, the HIDAPI drivers for PlayStation / Switch /
//! 8BitDo / GameSir pads) give the axes in the order its mapping database expects, so every pad SDL
//! knows reads with the right layout (CONTROLS_OPTIONS.md "Pad input through SDL3").
//!
//! Values as the rest of the game reads them: sticks -1..1 with Y up positive, triggers 0..1.
use sdl3_sys::everything::*;
use std::collections::HashMap;
use std::ffi::CStr;

pub use sdl3_sys::everything::SDL_JoystickID as PadId;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    South,
    East,
    West,
    North,
    /// LB / RB
    LeftShoulder,
    RightShoulder,
    /// LT / RT past the press point (their analog value is `Axis::LeftTrigger` / `RightTrigger`)
    LeftTrigger,
    RightTrigger,
    Back,
    Start,
    LeftStick,
    RightStick,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    LeftTrigger,
    RightTrigger,
}

#[derive(Clone, Debug)]
pub enum PadEvent {
    Connected,
    Disconnected,
    Pressed(Button),
    /// a button released
    Released(Button),
    /// a stick or trigger moved to this value
    Moved(Axis, f32),
    /// a raw joystick axis / button / hat, as the driver numbers it (only with `Pads::raw`)
    #[allow(dead_code)]
    Raw(String),
}

/// The Windows lines of the community SDL_GameControllerDB (the one gilrs used).
const COMMUNITY_DB: &str = include_str!("../data/gamecontrollerdb_windows.txt");

/// (a trigger counts as pressed past this, as gilrs had it)
const TRIGGER_PRESS: f32 = 0.75;

/// The extra stick dead zone in percent (the port's own, DECISIONS.md D39): every stick read goes through it,
/// ahead of the original's own zones; 0 = the stick as SDL gives it, the original's.
static STICK_DEAD_PERCENT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn set_stick_dead_zone(percent: u32) {
    STICK_DEAD_PERCENT.store(percent, std::sync::atomic::Ordering::Relaxed);
}

/// A stick past a radial dead zone `z` (0 .. 1): nothing inside it, the rest stretched back to full tilt.
fn stick_past_zone(x: f32, y: f32, z: f32) -> (f32, f32) {
    let m = (x * x + y * y).sqrt();
    if z <= 0.0 {
        (x, y)
    } else if m <= z {
        (0.0, 0.0)
    } else {
        let k = (m.min(1.0) - z) / (1.0 - z) / m;
        (x * k, y * k)
    }
}

fn sdl_button(b: Button) -> Option<SDL_GamepadButton> {
    Some(match b {
        Button::South => SDL_GAMEPAD_BUTTON_SOUTH,
        Button::East => SDL_GAMEPAD_BUTTON_EAST,
        Button::West => SDL_GAMEPAD_BUTTON_WEST,
        Button::North => SDL_GAMEPAD_BUTTON_NORTH,
        Button::LeftShoulder => SDL_GAMEPAD_BUTTON_LEFT_SHOULDER,
        Button::RightShoulder => SDL_GAMEPAD_BUTTON_RIGHT_SHOULDER,
        Button::Back => SDL_GAMEPAD_BUTTON_BACK,
        Button::Start => SDL_GAMEPAD_BUTTON_START,
        Button::LeftStick => SDL_GAMEPAD_BUTTON_LEFT_STICK,
        Button::RightStick => SDL_GAMEPAD_BUTTON_RIGHT_STICK,
        Button::DPadUp => SDL_GAMEPAD_BUTTON_DPAD_UP,
        Button::DPadDown => SDL_GAMEPAD_BUTTON_DPAD_DOWN,
        Button::DPadLeft => SDL_GAMEPAD_BUTTON_DPAD_LEFT,
        Button::DPadRight => SDL_GAMEPAD_BUTTON_DPAD_RIGHT,
        Button::LeftTrigger | Button::RightTrigger => return None,
    })
}

fn from_sdl_button(b: u8) -> Option<Button> {
    let b = SDL_GamepadButton(b as i32);
    [
        Button::South, Button::East, Button::West, Button::North, Button::LeftShoulder, Button::RightShoulder, Button::Back, Button::Start,
        Button::LeftStick, Button::RightStick, Button::DPadUp, Button::DPadDown, Button::DPadLeft, Button::DPadRight,
    ]
    .into_iter()
    .find(|x| sdl_button(*x) == Some(b))
}

fn sdl_axis(a: Axis) -> SDL_GamepadAxis {
    match a {
        Axis::LeftX => SDL_GAMEPAD_AXIS_LEFTX,
        Axis::LeftY => SDL_GAMEPAD_AXIS_LEFTY,
        Axis::RightX => SDL_GAMEPAD_AXIS_RIGHTX,
        Axis::RightY => SDL_GAMEPAD_AXIS_RIGHTY,
        Axis::LeftTrigger => SDL_GAMEPAD_AXIS_LEFT_TRIGGER,
        Axis::RightTrigger => SDL_GAMEPAD_AXIS_RIGHT_TRIGGER,
    }
}

fn from_sdl_axis(a: u8) -> Option<Axis> {
    let a = SDL_GamepadAxis(a as i32);
    [Axis::LeftX, Axis::LeftY, Axis::RightX, Axis::RightY, Axis::LeftTrigger, Axis::RightTrigger].into_iter().find(|x| sdl_axis(*x) == a)
}

/// SDL's raw axis value as the game reads it: sticks -1..1 with Y up positive, triggers 0..1.
fn axis_value(a: Axis, raw: i16) -> f32 {
    let v = (raw as f32 / 32767.0).clamp(-1.0, 1.0);
    match a {
        Axis::LeftY | Axis::RightY => -v,
        Axis::LeftTrigger | Axis::RightTrigger => v.max(0.0),
        _ => v,
    }
}

unsafe fn c_str(p: *const std::ffi::c_char) -> String {
    if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
}

/// One open pad.
pub struct Pad {
    raw: *mut SDL_Gamepad,
}

impl Pad {
    pub fn name(&self) -> String {
        unsafe { c_str(SDL_GetGamepadName(self.raw)) }
    }

    /// A stick or trigger (-1..1 / 0..1, Y up positive).
    pub fn axis(&self, a: Axis) -> f32 {
        let raw = |a: Axis| axis_value(a, unsafe { SDL_GetGamepadAxis(self.raw, sdl_axis(a)) });
        let z = STICK_DEAD_PERCENT.load(std::sync::atomic::Ordering::Relaxed) as f32 / 100.0;
        match a {
            Axis::LeftX => stick_past_zone(raw(a), raw(Axis::LeftY), z).0,
            Axis::LeftY => stick_past_zone(raw(Axis::LeftX), raw(a), z).1,
            Axis::RightX => stick_past_zone(raw(a), raw(Axis::RightY), z).0,
            Axis::RightY => stick_past_zone(raw(Axis::RightX), raw(a), z).1,
            _ => raw(a),
        }
    }

    pub fn pressed(&self, b: Button) -> bool {
        match b {
            Button::LeftTrigger => self.axis(Axis::LeftTrigger) >= TRIGGER_PRESS,
            Button::RightTrigger => self.axis(Axis::RightTrigger) >= TRIGGER_PRESS,
            _ => unsafe { SDL_GetGamepadButton(self.raw, sdl_button(b).unwrap()) },
        }
    }

    /// SDL's kind of pad (Xbox 360, PS5, ...; "unknown" for a pad only the mapping database knows).
    pub fn kind(&self) -> &'static str {
        match unsafe { SDL_GetGamepadType(self.raw) } {
            SDL_GAMEPAD_TYPE_STANDARD => "standard",
            SDL_GAMEPAD_TYPE_XBOX360 => "Xbox 360",
            SDL_GAMEPAD_TYPE_XBOXONE => "Xbox One",
            SDL_GAMEPAD_TYPE_PS3 => "PS3",
            SDL_GAMEPAD_TYPE_PS4 => "PS4",
            SDL_GAMEPAD_TYPE_PS5 => "PS5",
            SDL_GAMEPAD_TYPE_NINTENDO_SWITCH_PRO => "Switch Pro",
            SDL_GAMEPAD_TYPE_NINTENDO_SWITCH_JOYCON_LEFT | SDL_GAMEPAD_TYPE_NINTENDO_SWITCH_JOYCON_RIGHT | SDL_GAMEPAD_TYPE_NINTENDO_SWITCH_JOYCON_PAIR => "Joy-Con",
            SDL_GAMEPAD_TYPE_GAMECUBE => "GameCube",
            SDL_GAMEPAD_TYPE_STEAM => "Steam",
            _ => "unknown",
        }
    }

    /// A PlayStation pad (SDL's PS3, PS4 and PS5 kinds): the prompts show the PS3's glyphs.
    pub fn playstation(&self) -> bool {
        matches!(unsafe { SDL_GetGamepadType(self.raw) }, SDL_GAMEPAD_TYPE_PS3 | SDL_GAMEPAD_TYPE_PS4 | SDL_GAMEPAD_TYPE_PS5)
    }

    pub fn has_rumble(&self) -> bool {
        unsafe { SDL_GetBooleanProperty(SDL_GetGamepadProperties(self.raw), SDL_PROP_GAMEPAD_CAP_RUMBLE_BOOLEAN, false) }
    }

    /// Runs the pad's two motors (left / low frequency, right / high frequency, 0..65535) for `ms`
    /// milliseconds (`crate::rumble`: the original's XInputSetState values).
    pub fn rumble(&self, low: u16, high: u16, ms: u32) -> bool {
        unsafe { SDL_RumbleGamepad(self.raw, low, high, ms) }
    }

    /// The layout line SDL uses for this pad (for the log).
    pub fn mapping(&self) -> String {
        unsafe {
            let p = SDL_GetGamepadMapping(self.raw);
            let s = c_str(p);
            if !p.is_null() {
                SDL_free(p as *mut _);
            }
            s
        }
    }

    /// Vendor and product id.
    pub fn usb_id(&self) -> (u16, u16) {
        unsafe { (SDL_GetGamepadVendor(self.raw), SDL_GetGamepadProduct(self.raw)) }
    }

    /// Which of two pads used in the same frame is taken: one with rumble (XInput and the
    /// HIDAPI drivers), then one SDL knows the kind of, then any other.
    pub fn rank(&self) -> u8 {
        (self.has_rumble() as u8) * 2 + (self.kind() != "unknown") as u8
    }
}

impl Drop for Pad {
    fn drop(&mut self) {
        unsafe { SDL_CloseGamepad(self.raw) }
    }
}

/// SDL's gamepad layer and the pads that are plugged in.
pub struct Pads {
    pads: HashMap<PadId, Pad>,
    /// each trigger's pressed state, for its press / release events (with gilrs's hysteresis)
    triggers: HashMap<(PadId, u8), bool>,
    /// also report the joysticks' raw axes, buttons and hats (the pad recorder)
    pub raw: bool,
}

impl Pads {
    /// Starts SDL's gamepad layer (no window, audio or video: the game has its own).
    pub fn new() -> Result<Pads, String> {
        unsafe {
            // (the window is winit's, not SDL's: without this SDL would drop pad input as if the
            // game were in the background)
            SDL_SetHint(SDL_HINT_JOYSTICK_ALLOW_BACKGROUND_EVENTS, c"1".as_ptr());
            if !SDL_Init(SDL_INIT_GAMEPAD) {
                return Err(c_str(SDL_GetError()));
            }
            // (the community database's lines for the pads SDL's own one does not know; SDL's own
            // lines and the user's SDL_GAMECONTROLLERCONFIG are kept)
            for line in COMMUNITY_DB.lines() {
                // (a device line: 32 hex digits of GUID first - not a comment or the "xinput" line)
                let g = line.split(',').next().unwrap_or("");
                if g.len() != 32 || !g.bytes().all(|c| c.is_ascii_hexdigit()) {
                    continue;
                }
                let Ok(guid) = std::ffi::CString::new(g) else { continue };
                let known = SDL_GetGamepadMappingForGUID(SDL_StringToGUID(guid.as_ptr()));
                if known.is_null() {
                    if let Ok(l) = std::ffi::CString::new(line) {
                        SDL_AddGamepadMapping(l.as_ptr());
                    }
                } else {
                    SDL_free(known as *mut _);
                }
            }
        }
        // (the pads already plugged in come as ADDED events on the first poll)
        Ok(Pads { pads: HashMap::new(), triggers: HashMap::new(), raw: false })
    }

    pub fn get(&self, id: PadId) -> Option<&Pad> {
        self.pads.get(&id)
    }

    /// Everything the pads did since the last call (once a frame); the pads' state is current after it.
    pub fn poll(&mut self) -> Vec<(PadId, PadEvent)> {
        let mut out = Vec::new();
        unsafe {
            let mut ev = std::mem::MaybeUninit::<SDL_Event>::uninit();
            while SDL_PollEvent(ev.as_mut_ptr()) {
                let ev = ev.assume_init_ref();
                let ty = SDL_EventType(ev.r#type);
                if ty == SDL_EVENT_GAMEPAD_ADDED {
                    let id = ev.gdevice.which;
                    if !self.pads.contains_key(&id) {
                        let raw = SDL_OpenGamepad(id);
                        if !raw.is_null() {
                            self.pads.insert(id, Pad { raw });
                            out.push((id, PadEvent::Connected));
                        }
                    }
                } else if ty == SDL_EVENT_GAMEPAD_REMOVED {
                    let id = ev.gdevice.which;
                    self.triggers.retain(|k, _| k.0 != id);
                    if self.pads.remove(&id).is_some() {
                        out.push((id, PadEvent::Disconnected));
                    }
                } else if ty == SDL_EVENT_GAMEPAD_BUTTON_DOWN || ty == SDL_EVENT_GAMEPAD_BUTTON_UP {
                    let b = ev.gbutton;
                    if let Some(button) = from_sdl_button(b.button) {
                        out.push((b.which, if b.down { PadEvent::Pressed(button) } else { PadEvent::Released(button) }));
                    }
                } else if self.raw && ty == SDL_EVENT_JOYSTICK_AXIS_MOTION {
                    out.push((ev.jaxis.which, PadEvent::Raw(format!("a{} = {}", ev.jaxis.axis, ev.jaxis.value))));
                } else if self.raw && (ty == SDL_EVENT_JOYSTICK_BUTTON_DOWN || ty == SDL_EVENT_JOYSTICK_BUTTON_UP) {
                    out.push((ev.jbutton.which, PadEvent::Raw(format!("b{} {}", ev.jbutton.button, if ev.jbutton.down { "down" } else { "up" }))));
                } else if self.raw && ty == SDL_EVENT_JOYSTICK_HAT_MOTION {
                    out.push((ev.jhat.which, PadEvent::Raw(format!("h{}.{}", ev.jhat.hat, ev.jhat.value))));
                } else if ty == SDL_EVENT_GAMEPAD_AXIS_MOTION {
                    let a = ev.gaxis;
                    if let Some(axis) = from_sdl_axis(a.axis) {
                        let v = axis_value(axis, a.value);
                        out.push((a.which, PadEvent::Moved(axis, v)));
                        // (a trigger crossing the press point is a button too, as gilrs had it)
                        let tb = match axis { Axis::LeftTrigger => Some(Button::LeftTrigger), Axis::RightTrigger => Some(Button::RightTrigger), _ => None };
                        if let Some(tb) = tb {
                            let held = self.triggers.entry((a.which, tb as u8)).or_insert(false);
                            let now = if *held { v >= TRIGGER_PRESS - 0.1 } else { v >= TRIGGER_PRESS };
                            if now != *held {
                                *held = now;
                                out.push((a.which, if now { PadEvent::Pressed(tb) } else { PadEvent::Released(tb) }));
                            }
                        }
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::stick_past_zone;

    #[test]
    fn stick_dead_zone() {
        // (0: untouched, the original's stick)
        assert_eq!(stick_past_zone(0.03, -0.7, 0.0), (0.03, -0.7));
        // (drift inside the circle: nothing; on the edge the stick starts from 0, at full tilt it is full)
        assert_eq!(stick_past_zone(0.15, 0.1, 0.2), (0.0, 0.0));
        let (x, y) = stick_past_zone(0.6, 0.0, 0.2);
        assert!((x - 0.5).abs() < 1e-6 && y == 0.0);
        let (x, y) = stick_past_zone(0.0, -1.0, 0.2);
        assert!(x == 0.0 && (y + 1.0).abs() < 1e-6);
    }
}
