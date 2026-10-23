//! Which buttons the prompts show: the glyphs of the pad played with last (the Xbox set,
//! or the PlayStation set when SDL calls the pad a PlayStation one), or the keyboard's keys
//! when a key was pressed last.
//!
//! The original picks its glyph set by the movies' `_global.platform` (`FSCommand:getvar
//! platform`, 0 Xbox 360, 1 PS3): the prompt bars show frame `iconID + platform * 49` of
//! their icon clip, the custom controls' BUTTON column the same, the tab bars' LB / RB
//! clips (`btn_lb`, `btn_rb`) and the phone's back button go to their frame 2. The port
//! sets that variable in the movies when the pad changes (`platform`).
//!
//! The keyboard prompts are the port's own (DECISIONS.md D26): the original has no
//! keyboard. A prompt then shows no icon (`iconID` 0: the movie hides the clip and moves
//! the text into its place) and its text is followed by the key's name in brackets in
//! the bar's own typeface - "ACCEPT [ENTER]".

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Device {
    Xbox,
    PlayStation,
    Keyboard,
}

/// The screen a prompt is on: some buttons are other keys there.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Screen {
    Menu = 0,
    /// the GPS map and its mission log
    Map = 1,
    /// photo mode and the photo album
    Photo = 2,
    /// a replay's legend and its own pause panel
    Replay = 3,
}

static KEYBOARD: AtomicBool = AtomicBool::new(false);
static PAD_PS: AtomicBool = AtomicBool::new(false);
static SCREEN: AtomicU8 = AtomicU8::new(0);
static GPS_KEY: std::sync::Mutex<&'static str> = std::sync::Mutex::new("M");

/// The device the prompts are for.
pub fn device() -> Device {
    if KEYBOARD.load(Ordering::Relaxed) {
        Device::Keyboard
    } else if PAD_PS.load(Ordering::Relaxed) {
        Device::PlayStation
    } else {
        Device::Xbox
    }
}

/// The movies' `platform`: 1 (the PS3's glyphs) when the pad played with last is a
/// PlayStation pad - with the keyboard too, for what has no key name (the controller
/// layout, the custom controls' BUTTON column).
pub fn platform() -> i32 {
    PAD_PS.load(Ordering::Relaxed) as i32
}

/// A key was pressed. True when that changed the device.
pub fn use_keyboard() -> bool {
    !KEYBOARD.swap(true, Ordering::Relaxed)
}

/// A pad was used (a button, or a stick or trigger past half way); `playstation`: SDL's
/// gamepad type of that pad is a PlayStation one. True when that changed the device.
pub fn use_pad(playstation: bool) -> bool {
    let k = KEYBOARD.swap(false, Ordering::Relaxed);
    let p = PAD_PS.swap(playstation, Ordering::Relaxed);
    k || p != playstation
}

/// Set once a frame. (Test hook: `MCLA_GLYPHS=xbox|ps|keyboard` is the device until a pad
/// or a key is used.)
pub fn set_screen(s: Screen) {
    static ENV: std::sync::Once = std::sync::Once::new();
    ENV.call_once(|| match std::env::var("MCLA_GLYPHS").as_deref() {
        Ok("ps") => {
            use_pad(true);
        }
        Ok("keyboard") => {
            use_keyboard();
        }
        _ => {}
    });
    SCREEN.store(s as u8, Ordering::Relaxed);
}

fn screen() -> Screen {
    match SCREEN.load(Ordering::Relaxed) {
        1 => Screen::Map,
        2 => Screen::Photo,
        3 => Screen::Replay,
        _ => Screen::Menu,
    }
}

/// The keyboard's GPS MAP key (the pad's Back in the game: the phone's MISSION LOG).
pub fn set_gps_key(name: &'static str) {
    if let Ok(mut k) = GPS_KEY.lock() {
        *k = name;
    }
}

pub fn gps_key() -> &'static str {
    GPS_KEY.lock().map(|k| *k).unwrap_or("M")
}

/// Marks a prompt text that names its own key (`with_key`): `MARK key MARK text`.
const MARK: char = '\u{6}';

/// A prompt whose keyboard key is not the one its button usually is (`key_name`).
pub fn with_key(key: &str, (text, icon): (String, i32)) -> (String, i32) {
    (format!("{MARK}{key}{MARK}{text}"), icon)
}

/// The keyboard's key for a prompt icon (`flash_ui::ICON_*`, the prompt clip's frames: 1 A,
/// 2 B, 3 X, 4 Y, 5 LB, 6 RB, 7 LT, 8 RT, 9 Start, 10 Back, 11..14 the left stick, 15..18 the
/// right stick, 19..22 the d-pad, 23 LT/RT, 24 LB/RB) on the current screen. None: no key
/// (25 is the arrows drawn about the text, no button).
pub fn key_name(icon: i32) -> Option<&'static str> {
    let s = screen();
    Some(match icon {
        1 => "ENTER",
        2 => "ESC",
        3 => "X",
        4 => "Y",
        5 => "Q",
        6 => "E",
        24 => "Q/E",
        7 | 8 | 23 => match s {
            Screen::Map => "PGUP/PGDN",
            Screen::Photo => "Q/E",
            _ => "LEFT/RIGHT",
        },
        9 => match s {
            Screen::Map => "N",
            Screen::Replay => "P",
            _ => "ESC",
        },
        10 => match s {
            Screen::Replay => "BKSP",
            _ => "M",
        },
        11..=14 if s == Screen::Photo => "WASD",
        // (the replay's SLOW: the arrows step it as the stick does)
        11..=14 if s == Screen::Replay => "LEFT/RIGHT",
        15..=18 if s == Screen::Photo => "MOUSE",
        11..=22 => "ARROWS",
        _ => return None,
    })
}

/// What the prompt bar shows for a prompt: the text and the icon frame the movie adds its
/// `platform * 49` to; on the keyboard the text with the key's name and no icon.
pub fn prompt(text: &str, icon: i32) -> (String, i32) {
    let (key, text) = match text.strip_prefix(MARK).and_then(|t| t.split_once(MARK)) {
        Some((k, t)) => (Some(k), t),
        None => (None, text),
    };
    if device() != Device::Keyboard {
        return (text.to_string(), icon);
    }
    match key.or_else(|| key_name(icon)) {
        Some(k) => (format!("{text} [{k}]"), 0),
        None => (text.to_string(), icon),
    }
}

/// `Player::key_caps` for a movie's button clips (instance name, the icon it shows): the
/// keys' names on the keyboard, none (the clips' own pictures) with a pad.
pub fn key_caps(clips: &[(&str, i32)]) -> Vec<(String, String, u32)> {
    if device() != Device::Keyboard {
        return Vec::new();
    }
    clips.iter().filter_map(|(name, icon)| key_name(*icon).map(|k| (name.to_string(), format!("[{k}]"), 0xffff_ffff))).collect()
}

/// A button's name for the port's own text (the start screen without its movie, the movie
/// skip line): the key on the keyboard, else the pad's button.
pub fn button_name(icon: i32) -> &'static str {
    match device() {
        Device::Keyboard => key_name(icon).unwrap_or(""),
        Device::Xbox => match icon {
            1 => "A",
            2 => "B",
            3 => "X",
            4 => "Y",
            9 => "START",
            10 => "BACK",
            _ => "",
        },
        Device::PlayStation => match icon {
            1 => "CROSS",
            2 => "CIRCLE",
            3 => "SQUARE",
            4 => "TRIANGLE",
            9 => "START",
            10 => "SELECT",
            _ => "",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_follow_the_device() {
        set_screen(Screen::Menu);
        use_pad(false);
        assert_eq!(prompt("ACCEPT", 1), ("ACCEPT".to_string(), 1));
        assert_eq!(platform(), 0);
        assert!(use_pad(true));
        assert_eq!(platform(), 1);
        assert_eq!(prompt("ACCEPT", 1), ("ACCEPT".to_string(), 1));
        assert!(use_keyboard());
        assert!(!use_keyboard());
        assert_eq!(platform(), 1);
        assert_eq!(prompt("ACCEPT", 1), ("ACCEPT [ENTER]".to_string(), 0));
        let (t, i) = with_key("V", ("VIEW SONGS".to_string(), 1));
        assert_eq!(prompt(&t, i), ("VIEW SONGS [V]".to_string(), 0));
        // (the arrows about a text are no button)
        assert_eq!(prompt("CAR", 25), ("CAR".to_string(), 25));
        set_screen(Screen::Map);
        assert_eq!(prompt("MAP MODE", 9), ("MAP MODE [N]".to_string(), 0));
        set_screen(Screen::Menu);
        assert!(use_pad(false));
        assert_eq!(prompt(&t, i), ("VIEW SONGS".to_string(), 1));
    }
}
