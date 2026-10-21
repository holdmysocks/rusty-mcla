//! Photo mode and the photo album, as the original runs them: the UI states of
//! `tune/ui/PhotoAlbum.sc.xml` and `tune/ui/photo/*.sc.xml` (SnapShot, SS_SeeFullScreenPhoto,
//! PhotoAlbum with its sixteen `photo/Slot.sc` slots, PA_ChooseSlot, LocalMode,
//! FullScreenPicture, PA_LoadFile / PA_SaveFile and the two message boxes), the album object
//! `mcPhotoAlbum` (script methods sub_8263FA10: PhotoSlotChosen 0, SetPreviewPicture 1
//! (sub_8263DFC0), ShowNextPhoto 6 (sub_8263CDC0), ShowPreviousPhoto 7 (sub_8263CE88),
//! LoadPhotoAlbum 8 (sub_8263E428), SaveSnapshot 10, PostLoad 20, PostSave 21; the write
//! sub_8263DCF8) and its movie `photoalbum` (`resources/ui/photoalbum/0xdfc28929.r27`), which
//! lays itself out from the variables the states write (`cur_visibility*`, `overwritemode`,
//! `helptext`, `cur_Selection`, `empty_slot`, `preview_letterbox`, `takephoto_trig`, ...).
//! Notes: `work/career2/gasphoto/notes.md` section 2, `work/career2/photo/`; log in
//! `docs/CAREER_RACES.md` ("Photo album").
//!
//! The port keeps the pictures as PNG files, `<data>/../photos/album/slot_NN.png`
//! (`mc4b::album_dir`); the snapshot waiting for a slot is `snapshot.png` there.

use crate::flash_ui::FlashMovie;
use crate::game::{Game, MenuAction};
use mcla_flash::value::Value;
use std::path::PathBuf;

pub const PHOTO_MOVIE: &str = "resources/ui/photoalbum/0xdfc28929.r27";
pub const SLOTS: usize = crate::mc4b::ALBUM_SLOTS;

/// Where the flow stands (the focused UI state).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
    Off,
    /// `SnapShot` (photo/SnapshotScreen.sc): the framing, prompt 7, the camera free
    Snap,
    /// SnapshotScreen's Uninterruptible sub-state: `game.startTimer(1.0)`, input swallowed
    Shutter(f32),
    /// `photoAlbum.TakePicture()`: the frame asked of the renderer; frames waited
    Taking(u32),
    /// `SS_SeeFullScreenPhoto` (photo/SnapshotConfirm.sc): the shot full screen, prompt 11
    SeeShot,
    /// `PA_LoadFile` (photo/Dialog.sc, Xenon: `file.startTimer('3')`), input locked
    Loading(f32),
    /// `PA_ChooseSlot`: prompt 9, `overwritemode` 1
    Choose,
    /// a message box over the chooser (PA_ConfirmPhotoOverwrite / PA_Confirm_SnapshotLostAndExit)
    Dialog(Dialog),
    /// `PA_SaveFile` (Xenon: 3 s timer, then LocalMode and PostSave), input locked
    Saving(f32),
    /// `LocalMode`: viewing
    Local,
    /// `FullScreenPicture`: one picture full screen, left / right through the used slots
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dialog {
    /// PA_ConfirmPhotoOverwrite (Save_Dialog_Title_Warning / Save_PA_Overwrite, YES / NO)
    Overwrite,
    /// PA_Confirm_SnapshotLostAndExit (Save_Dialog_DiscardSnapshotConfirmAndExit, YES / NO)
    LostAndExit,
}

/// A pad direction of the grid (photo/Slot.sc transitions).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// The slot focused after a move (PhotoAlbum.sc.xml's sixteen `photo/Slot.sc` includes:
/// arg2 down, arg3 left, arg4 right, arg5 up): a 4 x 4 grid, left / right wrap inside the row,
/// up / down wrap over the sixteen.
pub fn grid_move(k: usize, d: Dir) -> usize {
    let k = k % SLOTS;
    match d {
        Dir::Down => (k + 4) % SLOTS,
        Dir::Up => (k + SLOTS - 4) % SLOTS,
        Dir::Right => k / 4 * 4 + (k % 4 + 1) % 4,
        Dir::Left => k / 4 * 4 + (k % 4 + 3) % 4,
    }
}

/// sub_8263DC38: the slot the chooser opens on - the first free one, slot 0 when all are used.
pub fn first_free(used: &[bool; SLOTS]) -> usize {
    used.iter().position(|u| !u).unwrap_or(0)
}

/// `used(k)` as the album object reads it (+860 bit k); slot 16 (ShowPreviousPhoto's wrap
/// writes 16, not 15) is never used.
fn is_used(used: &[bool; SLOTS], k: usize) -> bool {
    used.get(k).copied().unwrap_or(false)
}

/// ShowNextPhoto (sub_8263CDC0): from the cursor on, the next used slot (after 15 comes 0),
/// sixteen tries; None when there is none.
pub fn next_used(used: &[bool; SLOTS], k: usize) -> Option<usize> {
    let mut k = k;
    for _ in 0..SLOTS {
        k += 1;
        if k >= 16 {
            k = 0;
        }
        if is_used(used, k) {
            return Some(k);
        }
    }
    None
}

/// ShowPreviousPhoto (sub_8263CE88): the previous used slot; below 0 the original goes to 16
/// (a slot that is never used, so the next try is 15), sixteen tries.
pub fn prev_used(used: &[bool; SLOTS], k: usize) -> Option<usize> {
    let mut k = k as i32;
    for _ in 0..SLOTS {
        k -= 1;
        if k < 0 {
            k = 16;
        }
        if is_used(used, k as usize) {
            return Some(k as usize);
        }
    }
    None
}

/// SetPreviewPicture's prompt (sub_8263DFC0): 9 with a snapshot waiting for a slot; else 8 on
/// a used slot, 56 on a free one while the album holds pictures (+856 > 0), 58 when it is empty.
pub fn preview_prompt(pending: bool, used_k: bool, count: usize) -> u8 {
    if pending {
        9
    } else if used_k {
        8
    } else if count > 0 {
        56
    } else {
        58
    }
}

/// PhotoSlotChosen's prompt as it goes full screen (0x8263FC2C): 10 with more than one picture,
/// else 57.
pub fn full_prompt(count: usize) -> u8 {
    if count > 1 {
        10
    } else {
        57
    }
}

/// The prompt sets (sub_8220F040: static lists built by sub_827B51B0 (7), sub_827B5258 (8),
/// sub_827B52D8 (9), sub_827B5330 (10), sub_827B5398 (11); 56 / 57 / 58 written in place):
/// (string key, button icon) in the game's order.
pub fn prompt_rows(n: u8) -> &'static [(&'static str, i32)] {
    match n {
        7 => &[("PA_snap_photo", 1), ("prompt_cancel", 2), ("PA_photo_album", 4), ("prompt_Rotate_Camera", 15), ("Zoom In/Out", 23), ("Pan", 11)],
        8 => &[("prompt_exit", 2), ("PA_publish", 3), ("PA_view_photo", 1), ("PA_select_photo", 19)],
        9 => &[("prompt_cancel", 2), ("PA_Choose_Slot", 1)],
        10 => &[("PA_photo_album", 2), ("PA_publish", 3), ("PA_nextprev_photo", 21)],
        11 => &[("prompt_cancel", 2), ("PA_save_photo", 1)],
        56 => &[("prompt_exit", 2), ("PA_publish", 3), ("PA_select_photo", 19)],
        57 => &[("PA_photo_album", 2), ("PA_publish", 3)],
        58 => &[("prompt_exit", 2), ("PA_select_photo", 19)],
        _ => &[],
    }
}

/// The album's state and the movie.
pub struct PhotoAlbum {
    pub stage: Stage,
    /// +860 bit k: slot k holds a picture
    pub used: [bool; SLOTS],
    /// +1180: the slot under the cursor (and the one shown full screen)
    pub sel: usize,
    /// +1170: a snapshot waits for a slot
    pub pending: bool,
    /// the album was entered from SnapShot (its B / YES pop back there), not from the pause menu
    pub from_snap: bool,
    /// 0x8290621C: the album read since the city came back (EnableCity clears it - sub_8263D1E0)
    pub loaded: bool,
    /// the prompt set on the bar (`game.setPrompt`)
    pub prompt: u8,
    /// `cur_Selection` as the movie has it (a Slot's focus writes it; ShowNext / ShowPrevious do not)
    shown_sel: usize,
    pub movie: Option<FlashMovie>,
    thumbs: [Option<egui::TextureHandle>; SLOTS],
    /// PreviewPicture: the snapshot, or the picture of a used slot under the cursor
    preview: Option<egui::TextureHandle>,
    shot: Option<egui::TextureHandle>,
    /// pictures still to read while PA_LoadFile stands
    to_load: Vec<usize>,
    /// for the host (main.rs): the frame to write, a debug shot, sounds, leave photo mode
    pub capture: Option<String>,
    pub debug_shot: Option<String>,
    pub sounds: Vec<&'static str>,
    pub leave: bool,
    /// test hook frames since photo mode began
    test_frame: usize,
    /// the flow's first frame is still to come (its states' `onfocused` writes)
    fresh: bool,
}

impl Default for PhotoAlbum {
    fn default() -> Self {
        PhotoAlbum {
            stage: Stage::Off,
            used: [false; SLOTS],
            sel: 0,
            pending: false,
            from_snap: false,
            loaded: false,
            prompt: 0,
            shown_sel: 0,
            movie: None,
            thumbs: Default::default(),
            preview: None,
            shot: None,
            to_load: Vec::new(),
            capture: None,
            debug_shot: None,
            sounds: Vec::new(),
            leave: false,
            test_frame: 0,
            fresh: false,
        }
    }
}

impl PhotoAlbum {
    /// Photo mode begins: TAKE PHOTO (`SnapShot`, false) or the MODES tab's PHOTO ALBUM
    /// (`PM_PhotoAlbum`, true: straight into the album).
    pub fn begin(&mut self, album: bool) {
        self.stage = if album { Stage::Local } else { Stage::Snap };
        self.from_snap = false;
        self.pending = false;
        self.leave = false;
        self.test_frame = 0;
        self.fresh = true;
        // (asked of the movie on the first frame - `ui`)
        self.prompt = if album { 0 } else { 7 };
        self.capture = None;
    }

    /// The camera takes the player's input (SnapShot: `game.enableCameraInput()`; the shot
    /// disables it - `game.disableCameraInput()`).
    pub fn camera_input(&self) -> bool {
        self.stage == Stage::Snap
    }

    pub fn count(&self) -> usize {
        self.used.iter().filter(|u| **u).count()
    }
}

fn snapshot_path(data_dir: &str) -> PathBuf {
    crate::mc4b::album_dir(data_dir).join("snapshot.png")
}

fn slot_path(data_dir: &str, k: usize) -> PathBuf {
    crate::mc4b::album_dir(data_dir).join(format!("slot_{k:02}.png"))
}

/// A PNG picture for the movie, cut down by whole steps to at most 1024 wide.
fn load_png(path: &std::path::Path) -> Option<egui::ColorImage> {
    let mut dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0u8; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let ch = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        _ => 1,
    };
    let px = |x: usize, y: usize| -> [u32; 3] {
        let o = (y * w + x) * ch;
        match ch {
            3 | 4 => [buf[o] as u32, buf[o + 1] as u32, buf[o + 2] as u32],
            _ => [buf[o] as u32; 3],
        }
    };
    let f = w.div_ceil(1024).max(1);
    let (ow, oh) = (w / f, h / f);
    let mut out = Vec::with_capacity(ow * oh * 4);
    for y in 0..oh {
        for x in 0..ow {
            let mut s = [0u32; 3];
            for dy in 0..f {
                for dx in 0..f {
                    let p = px(x * f + dx, y * f + dy);
                    s[0] += p[0];
                    s[1] += p[1];
                    s[2] += p[2];
                }
            }
            let n = (f * f) as u32;
            out.extend_from_slice(&[(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8, 255]);
        }
    }
    Some(egui::ColorImage::from_rgba_unmultiplied([ow, oh], &out))
}

// The movie's clips that take the game's textures (`__texturename` Picture1..16 and
// PreviewPicture; the 16:9 panel - `aspect` is 1 in the port).
const PREVIEW_CLIPS: [&str; 2] = ["screen.preview_panel.panel_wide.renderTarget", "screen.panel_wide.crossfader"];
fn thumb_clip(k: usize) -> String {
    format!("screen.panel_wide.small_picture{:02}", k + 1)
}

impl Game {
    fn pa_set_int(&mut self, name: &str, v: i32) {
        if let Some(m) = self.album.movie.as_mut() {
            if name.contains('.') {
                m.player.set_path(name, Value::Int(v));
            } else {
                m.player.set_global(name, Value::Int(v));
            }
        }
    }

    /// `game.setString(movie, var, key)`: the string table's text.
    fn pa_set_str(&mut self, name: &str, key: &str) {
        let text = if key.is_empty() { String::new() } else { self.career.strings.get(key).map(|s| s.to_string()).unwrap_or_else(|| key.to_string()) };
        if let Some(m) = self.album.movie.as_mut() {
            if name.contains('.') {
                m.player.set_path(name, Value::str(&text));
            } else {
                m.player.set_global(name, Value::str(&text));
            }
        }
    }

    fn pa_sound(&mut self, name: &'static str) {
        self.album.sounds.push(name);
    }

    fn pa_used_from_disk(&mut self) {
        for k in 0..SLOTS {
            self.album.used[k] = slot_path(&self.data_dir, k).exists();
        }
    }

    /// Point a clip at a texture of ours, or back at its own bitmap (`None`).
    fn pa_bind(&mut self, clip: &str, key: u32, tex: Option<egui::TextureHandle>) {
        let Some(m) = self.album.movie.as_mut() else { return };
        match tex {
            Some(h) => {
                m.swap_handle(clip, key, h);
            }
            None => {
                if let Some(id) = m.player.find(clip) {
                    if let Some(i) = m.player.inst_mut(id) {
                        i.tex_swap = 0;
                    }
                }
            }
        }
    }

    fn pa_bind_preview(&mut self) {
        let t = self.album.preview.clone();
        for (i, c) in PREVIEW_CLIPS.iter().enumerate() {
            self.pa_bind(c, 100 + i as u32, t.clone());
        }
    }

    /// The slot's picture read again (after a load or a save).
    fn pa_load_thumb(&mut self, ctx: &egui::Context, k: usize) {
        let t = if self.album.used[k] { load_png(&slot_path(&self.data_dir, k)).map(|img| ctx.load_texture(format!("photoalbum:slot{k}"), img, egui::TextureOptions::LINEAR)) } else { None };
        self.album.thumbs[k] = t.clone();
        self.pa_bind(&thumb_clip(k), 1 + k as u32, t);
    }

    /// A Slot's `onfocused` (photo/Slot.sc): `cur_Selection` = k, SetPreviewPicture(k).
    fn pa_focus_slot(&mut self, k: usize) {
        self.album.sel = k;
        self.album.shown_sel = k;
        self.pa_set_int("cur_Selection", k as i32);
        self.pa_preview(k);
    }

    /// SetPreviewPicture (sub_8263DFC0): +1180 = k, the prompt, `empty_slot`, `preview_letterbox`
    /// (the picture's 16:9 flag against the screen's: always both 16:9 in the port), and the
    /// slot's picture into PreviewPicture (sub_8263C9C8; a free slot only sets `empty_slot`).
    fn pa_preview(&mut self, k: usize) {
        self.album.sel = k;
        let used_k = self.album.used[k];
        self.album.prompt = preview_prompt(self.album.pending, used_k, self.album.count());
        self.pa_set_int("empty_slot", (!used_k) as i32);
        if !self.album.pending {
            self.pa_set_int("preview_letterbox", 0);
        }
        self.pa_show_slot(k);
    }

    /// sub_8263C9C8: a used slot's picture becomes PreviewPicture; a free one sets `empty_slot`.
    fn pa_show_slot(&mut self, k: usize) {
        if self.album.used[k] {
            self.album.preview = self.album.thumbs[k].clone();
            self.pa_bind_preview();
        } else {
            self.pa_set_int("empty_slot", 1);
        }
    }

    /// `SnapShot` focused: SnapshotScreen's and SnapShot's `onfocused` (and EnableCity).
    fn pa_enter_snap(&mut self) {
        self.album.stage = Stage::Snap;
        self.album.loaded = false;
        self.pa_set_int("cur_visibility", 1);
        self.pa_set_int("cur_visibility_framing", 1);
        self.pa_set_str("t_overlaytitle", "PA_takephoto_desc");
        self.pa_set_str("t_overlaydescription", "Common_Null");
        self.album.prompt = 7;
    }

    /// The `PhotoAlbum` state focused (DisableCity, the album shown, `menu.name` PA_Title), then
    /// LoadPhotoAlbum (sub_8263E428).
    fn pa_enter_album(&mut self) {
        self.pa_set_int("cur_visibility", 1);
        self.pa_set_int("cur_visibility_album", 1);
        self.pa_set_str("menu.name", "PA_Title");
        if !self.album.loaded {
            // (not loaded: the album read - file.loadStart -> PA_LoadFile; the slots' pictures
            // are read while it stands)
            self.album.stage = Stage::Loading(3.0);
            self.pa_set_str("helptext", "Save_Loading_Photoalbum");
            self.pa_used_from_disk();
            self.album.to_load = (0..SLOTS).collect();
        } else if self.album.pending {
            // (event 35 chooseSlot)
            self.pa_enter_choose();
        } else {
            self.pa_enter_local();
            let k = self.album.sel;
            self.pa_focus_slot(k);
        }
    }

    /// `PA_ChooseSlot` focused.
    fn pa_enter_choose(&mut self) {
        self.album.stage = Stage::Choose;
        self.pa_set_str("helptext", "Photo_ChooseSlot");
        self.album.prompt = 9;
        self.pa_set_int("overwritemode", 1);
    }

    /// `PA_ChooseSlot` unfocused (WaitingForSaveMode or a box takes over).
    fn pa_leave_choose(&mut self) {
        self.pa_set_int("overwritemode", 0);
        self.pa_set_str("helptext", "");
    }

    /// `LocalMode` focused.
    fn pa_enter_local(&mut self) {
        self.album.stage = Stage::Local;
        self.pa_set_str("helptext", "Photo_ViewSlot");
    }

    /// The `PhotoAlbum` state left with `stackPop()`: back to SnapShot when it was entered from
    /// there, else out of photo mode (PhotoAlbumScene unfocused: SnapshotCancelled, EnableCity,
    /// every visibility 0) - the pause menu again.
    fn pa_pop_album(&mut self) {
        self.pa_set_int("cur_visibility_album", 0);
        self.pa_set_int("overwritemode", 0);
        if self.album.from_snap {
            self.album.from_snap = false;
            self.pa_enter_snap();
        } else {
            self.pa_close();
        }
    }

    fn pa_close(&mut self) {
        self.album.pending = false;
        self.album.loaded = false;
        self.album.stage = Stage::Off;
        self.album.prompt = 0;
        self.album.leave = true;
        for v in ["cur_visibility", "cur_visibility_album", "cur_visibility_FullScreenPicture", "cur_visibility_framing", "overwritemode"] {
            self.pa_set_int(v, 0);
        }
        self.pa_set_str("helptext", "");
        let _ = std::fs::remove_file(snapshot_path(&self.data_dir));
    }

    /// sub_8263DCF8 for the chosen slot (+1160): the snapshot written into it, the counts
    /// (`mc4b_photo_save`), the slot's texture; +1170 / +1176 cleared, PA_ChooseSlot
    /// deactivated; file.saveStart -> PA_SaveFile.
    fn pa_save(&mut self) {
        let k = self.album.sel;
        let to = std::path::PathBuf::from(self.mc4b_photo_save(k));
        let from = snapshot_path(&self.data_dir);
        if std::fs::copy(&from, &to).is_ok() {
            let _ = std::fs::remove_file(&from);
        }
        self.album.used[k] = to.exists();
        self.album.thumbs[k] = self.album.shot.clone();
        let t = self.album.shot.clone();
        self.pa_bind(&thumb_clip(k), 1 + k as u32, t);
        self.album.pending = false;
        self.pa_leave_choose();
        // PA_SaveFile: overwritemode 0, helptext Save_Dialog_Saving, WaitingForSaveMode, 3 s
        self.pa_set_int("overwritemode", 0);
        self.pa_set_str("helptext", "Save_Dialog_Saving");
        self.album.stage = Stage::Saving(3.0);
        log::info!("photo album: saved into slot {} ({} used)", k, self.album.count());
    }

    /// A message box of photo/Dialog.sc.xml up through the popup movie (YES = the confirm).
    fn pa_open_box(&mut self, d: Dialog) {
        let id = match d {
            Dialog::Overwrite => "PA_ConfirmPhotoOverwrite",
            Dialog::LostAndExit => "PA_Confirm_SnapshotLostAndExit",
        };
        let def = std::fs::read_to_string(std::path::Path::new(&self.data_dir).join("tune/ui/photo/Dialog.sc.xml")).ok().and_then(|t| crate::flash_ui::MessageBoxDef::parse(&t, id));
        self.album.stage = Stage::Dialog(d);
        log::info!("photo album: box {} (found {}, popup movie {})", id, def.is_some(), self.flash_popup.is_some());
        match def {
            Some(mut def) if self.flash_popup.is_some() => {
                // (the box's YES is its Common_Yes option)
                for o in def.options.iter_mut() {
                    if o.1.eq_ignore_ascii_case("Common_Yes") && !o.0.to_ascii_lowercase().ends_with("yes") {
                        o.0.push_str("_yes");
                    }
                }
                self.message_box = Some((def, 0, MenuAction::PhotoMode));
            }
            // (no popup movie: the question is taken as answered YES)
            _ => self.pa_box_answer(d, true),
        }
    }

    fn pa_box_answer(&mut self, d: Dialog, yes: bool) {
        match (d, yes) {
            // OVERWRITE_PHOTO_SLOT: +1176 = 1, verify, then the write into +1160
            (Dialog::Overwrite, true) => self.pa_save(),
            // OVERWRITE_PHOTO_SLOT_DECLINE, PA_ChooseSlot again on the same slot
            (Dialog::Overwrite, false) => self.pa_enter_choose(),
            // SnapshotCancelled, stackPop
            (Dialog::LostAndExit, true) => {
                self.album.pending = false;
                let _ = std::fs::remove_file(snapshot_path(&self.data_dir));
                self.pa_pop_album();
            }
            // (NO has no action: the box closes, the chooser has the input again)
            (Dialog::LostAndExit, false) => self.pa_enter_choose(),
        }
    }

    /// Test hook: MCLA_ALBUM_TEST="w60 A w90 A w200 R D A ..." plays pad presses into photo mode
    /// from its first frame - A B X Y, U D L R, `wN` waits N frames (20 between presses
    /// otherwise), `#name` writes a screenshot with the UI to MCLA_ALBUM_SHOTS/name.png.
    fn pa_test_hook(&mut self) {
        let Ok(script) = std::env::var("MCLA_ALBUM_TEST") else { return };
        let frame = self.album.test_frame;
        self.album.test_frame += 1;
        let mut at = 0usize;
        for tok in script.split_whitespace() {
            if let Some(n) = tok.strip_prefix('w').and_then(|n| n.parse::<usize>().ok()) {
                at += n;
                continue;
            }
            if let Some(name) = tok.strip_prefix('#') {
                if at == frame {
                    let dir = std::env::var("MCLA_ALBUM_SHOTS").unwrap_or_else(|_| ".".into());
                    self.album.debug_shot = Some(format!("{dir}/{name}.png"));
                    log::info!("album test: frame {} shot {} (stage {:?}, sel {}, prompt {})", frame, name, self.album.stage, self.album.sel, self.album.prompt);
                }
                at += 1;
                continue;
            }
            at += 20;
            if at == frame {
                match tok {
                    "A" => self.nav_enter = true,
                    "B" => self.nav_back = true,
                    "X" => self.nav_x = true,
                    "Y" => self.nav_y = true,
                    "U" => self.nav_up = true,
                    "D" => self.nav_down = true,
                    "L" => self.nav_left = true,
                    "R" => self.nav_right = true,
                    _ => {}
                }
                log::info!("album test: frame {} key {} (stage {:?}, sel {}, prompt {})", frame, tok, self.album.stage, self.album.sel, self.album.prompt);
            }
        }
    }

    /// Photo mode's frame (the whole screen is the album's while `photo_on`): input, the flow,
    /// the movie, the prompt bar and the boxes.
    pub fn photo_album_ui(&mut self, ctx: &egui::Context) -> MenuAction {
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        // the movie, first time (after its first frame: its own globals are set then)
        if self.album.movie.is_none() {
            if let Some(shared) = self.ui_shared.clone() {
                if let Some(mut m) = FlashMovie::load(&self.data_dir, PHOTO_MOVIE, &shared) {
                    m.player.tick();
                    self.album.movie = Some(m);
                }
            }
        }
        if self.album.stage == Stage::Off {
            return MenuAction::None;
        }
        // (the first frame of a flow: the states' onfocused writes)
        if std::mem::take(&mut self.album.fresh) {
            self.pa_used_from_disk();
            self.pa_set_int("cur_visibility_FullScreenPicture", 0);
            self.pa_set_str("helptext", "");
            if self.album.stage == Stage::Snap {
                self.pa_enter_snap();
            } else {
                // (PM_PhotoAlbum: WaitingForLoadModeFromPauseMenu, PhotoAlbum)
                self.album.prompt = 0;
                self.pa_enter_album();
            }
        }
        self.pa_test_hook();
        let up = std::mem::take(&mut self.nav_up);
        let down = std::mem::take(&mut self.nav_down);
        let left = std::mem::take(&mut self.nav_left);
        let right = std::mem::take(&mut self.nav_right);
        let enter = std::mem::take(&mut self.nav_enter);
        let back = std::mem::take(&mut self.nav_back);
        let y = std::mem::take(&mut self.nav_y);
        let _x = std::mem::take(&mut self.nav_x);
        let dir = if up {
            Some(Dir::Up)
        } else if down {
            Some(Dir::Down)
        } else if left {
            Some(Dir::Left)
        } else if right {
            Some(Dir::Right)
        } else {
            None
        };
        let mut draw = true;
        // (a box opened this frame takes the input from the next one)
        let stage0 = self.album.stage;
        match self.album.stage {
            Stage::Off => {}
            Stage::Snap => {
                if back {
                    // SnapShot's cancel: UI_MENU_BACK, closeReplay(5), stackPop - the pause menu
                    self.pa_sound("UI_MENU_BACK");
                    self.pa_close();
                } else if y {
                    // Y: PHOTO ALBUM (WaitingForLoadModeFromSnapshot, PhotoAlbum, stackPush)
                    self.pa_sound("UI_MENU_SELECT");
                    self.pa_set_int("cur_visibility_framing", 0);
                    self.album.prompt = 0;
                    self.pa_set_int("cur_visibility_FullScreenPicture", 0);
                    self.album.from_snap = true;
                    self.pa_enter_album();
                } else if enter {
                    // the shutter: UI_PHOTO_CAMERA_SHOT_MASTER, framing off, the flash, car and
                    // camera input off, then the 1.0 s timer with overwritemode 1 and prompt 0
                    self.pa_sound("UI_PHOTO_CAMERA_SHOT_MASTER");
                    self.pa_set_int("cur_visibility", 1);
                    self.pa_set_int("cur_visibility_framing", 0);
                    self.pa_set_int("takephoto_trig", 1);
                    self.pa_set_int("overwritemode", 1);
                    self.album.prompt = 0;
                    self.album.stage = Stage::Shutter(1.0);
                }
            }
            Stage::Shutter(t) => {
                let t = t - dt;
                if t > 0.0 {
                    self.album.stage = Stage::Shutter(t);
                } else {
                    // timerExpired: the movie hidden, TakePicture
                    self.pa_set_int("cur_visibility", 0);
                    self.pa_set_int("cur_visibility_FullScreenPicture", 0);
                    self.pa_set_int("cur_visibility_album", 0);
                    let p = snapshot_path(&self.data_dir);
                    let _ = std::fs::create_dir_all(p.parent().unwrap_or(std::path::Path::new(".")));
                    let _ = std::fs::remove_file(&p);
                    self.album.capture = Some(p.to_string_lossy().to_string());
                    self.album.stage = Stage::Taking(0);
                    draw = false;
                }
            }
            Stage::Taking(n) => {
                draw = false;
                let p = snapshot_path(&self.data_dir);
                if p.exists() || n > 30 {
                    // snapshotTaken: SnapShot unfocused (prompt 0, framing 0), SS_SeeFullScreenPhoto
                    self.album.shot = load_png(&p).map(|img| ctx.load_texture("photoalbum:snapshot", img, egui::TextureOptions::LINEAR));
                    self.album.preview = self.album.shot.clone();
                    self.pa_bind_preview();
                    self.pa_set_int("cur_visibility_framing", 0);
                    self.pa_sound("UI_PHOTO_MENU_MOVEMENT_MASTER");
                    self.album.prompt = 11;
                    self.pa_set_int("cur_visibility", 1);
                    self.pa_set_int("cur_visibility_FullScreenPicture", 1);
                    self.pa_set_int("cur_visibility_album", 1);
                    self.pa_sound("UI_PHOTO_MENU_MOVEMENT_MASTER");
                    self.album.stage = Stage::SeeShot;
                    draw = true;
                } else {
                    self.album.stage = Stage::Taking(n + 1);
                }
            }
            Stage::SeeShot => {
                if back {
                    // SnapshotConfirm's cancel: SnapshotCancelled, EnableCity, back to SnapShot
                    self.pa_sound("UI_MENU_BACK");
                    self.album.pending = false;
                    let _ = std::fs::remove_file(snapshot_path(&self.data_dir));
                    self.pa_sound("UI_PHOTO_MENU_MOVEMENT_MASTER");
                    self.pa_set_int("cur_visibility_album", 0);
                    self.pa_enter_snap();
                } else if enter {
                    // SaveSnapshot (+1170 = 1; sub_8263DC38 focuses the first free slot when the
                    // album is loaded), WaitingForLoadMode, PhotoAlbum
                    self.pa_sound("UI_MENU_SELECT");
                    self.album.pending = true;
                    if self.album.loaded {
                        let k = first_free(&self.album.used);
                        self.pa_focus_slot(k);
                    }
                    self.pa_sound("UI_PHOTO_MENU_MOVEMENT_MASTER");
                    self.pa_set_int("cur_visibility_album", 0);
                    self.pa_set_int("cur_visibility_FullScreenPicture", 0);
                    self.album.from_snap = true;
                    self.pa_enter_album();
                }
            }
            Stage::Loading(t) => {
                // (a picture a frame while the 3 s stand)
                if let Some(k) = self.album.to_load.pop() {
                    self.pa_load_thumb(ctx, k);
                }
                let t = t - dt;
                if t > 0.0 || !self.album.to_load.is_empty() {
                    self.album.stage = Stage::Loading(t);
                } else {
                    // PostLoad (method 20): loaded; a waiting snapshot -> event 35 chooseSlot and
                    // the first free slot; else event 40 (LocalMode) on slot 0
                    self.album.loaded = true;
                    if self.album.pending {
                        self.pa_enter_choose();
                        let k = first_free(&self.album.used);
                        self.pa_focus_slot(k);
                    } else {
                        self.pa_enter_local();
                        self.pa_focus_slot(0);
                    }
                }
            }
            Stage::Choose => {
                if let Some(d) = dir {
                    self.pa_sound("UI_PHOTO_SCROLL_MASTER");
                    let k = grid_move(self.album.sel, d);
                    self.pa_focus_slot(k);
                } else if enter {
                    // PhotoSlotChosen (+1160 = +1180), WaitingForSaveMode (PA_ChooseSlot unfocused)
                    self.pa_sound("UI_MENU_SELECT");
                    self.pa_leave_choose();
                    if self.album.used[self.album.sel] {
                        // PHOTO_SLOT_TAKEN -> photoSlotTaken -> PA_ConfirmPhotoOverwrite
                        self.pa_open_box(Dialog::Overwrite);
                    } else {
                        self.pa_save();
                    }
                } else if back {
                    self.pa_sound("UI_MENU_BACK");
                    self.pa_leave_choose();
                    self.pa_open_box(Dialog::LostAndExit);
                }
            }
            // (the box runs after the movie is drawn - below - so that it stands over it)
            Stage::Dialog(_) => {}
            Stage::Saving(t) => {
                let t = t - dt;
                if t > 0.0 {
                    self.album.stage = Stage::Saving(t);
                } else {
                    // timerExpired: LocalMode, PostSave (nothing waits any more). The prompt is
                    // left as PA_ChooseSlot set it until the cursor moves.
                    self.pa_enter_local();
                }
            }
            Stage::Local => {
                if let Some(d) = dir {
                    self.pa_sound("UI_PHOTO_SCROLL_MASTER");
                    let k = grid_move(self.album.sel, d);
                    self.pa_focus_slot(k);
                } else if enter {
                    // PhotoSlotChosen without a snapshot: a used slot goes full screen
                    let k = self.album.sel;
                    if self.album.used[k] && self.album.loaded {
                        self.album.prompt = full_prompt(self.album.count());
                        self.pa_sound("UI_MENU_SELECT");
                        // event 10 fullscreenModeStart: PhotoAlbum out, FullScreenPicture in
                        self.pa_set_int("cur_visibility_album", 0);
                        self.pa_sound("UI_PHOTO_MENU_MOVEMENT_MASTER");
                        self.pa_set_int("cur_visibility_FullScreenPicture", 1);
                        self.pa_set_int("cur_visibility_album", 1);
                        self.pa_set_int("cur_visibility", 1);
                        self.album.stage = Stage::Full;
                    }
                } else if back {
                    self.pa_sound("UI_MENU_BACK");
                    self.pa_pop_album();
                }
                // (X PUBLISH ALBUM is the online upload - not in the port, DECISIONS.md D7)
            }
            Stage::Full => {
                if back {
                    self.pa_sound("UI_MENU_BACK");
                    self.pa_sound("UI_PHOTO_MENU_MOVEMENT_MASTER");
                    self.pa_set_int("cur_visibility_FullScreenPicture", 0);
                    self.pa_set_int("cur_visibility_album", 0);
                    self.pa_enter_local();
                    // PhotoAlbum again: LoadPhotoAlbum focuses +1180 (its Slot: cur_Selection,
                    // SetPreviewPicture), then setPrompt(8)
                    self.pa_enter_album();
                    self.album.prompt = 8;
                } else if left || right {
                    let k = if right { next_used(&self.album.used, self.album.sel) } else { prev_used(&self.album.used, self.album.sel) };
                    if let Some(k) = k {
                        if self.album.count() > 1 {
                            self.pa_sound("UI_PHOTO_FULLSCREEN_NEXT_MASTER");
                        }
                        self.album.sel = k;
                        self.pa_show_slot(k);
                    }
                }
            }
        }
        if !draw || self.album.stage == Stage::Off {
            if let Some(fp) = self.flash_prompt.as_mut() {
                fp.hide();
            }
            return MenuAction::None;
        }
        // the movie
        if let Some(m) = self.album.movie.as_mut() {
            m.player.update(dt);
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("photoalbum"));
            // (in the layer order, under the prompt bar and the boxes moved up after it)
            ctx.move_to_top(layer);
            m.paint(ctx, &ctx.layer_painter(layer));
        }
        // the box (popup movie) over the album
        if let (Stage::Dialog(d), Stage::Dialog(_)) = (self.album.stage, stage0) {
            let r = self.message_box_ui(ctx, up, down, enter, back);
            if self.message_box.is_none() {
                self.pa_box_answer(d, r.is_some());
            }
        }
        // the prompt bar (the boxes carry their own)
        let rows: Vec<(String, i32)> = if matches!(self.album.stage, Stage::Dialog(_)) {
            Vec::new()
        } else {
            prompt_rows(self.album.prompt).iter().map(|(k, i)| (self.career.strings.get(k).map(|s| s.to_string()).unwrap_or_else(|| k.to_uppercase()), *i)).collect()
        };
        if let Some(fp) = self.flash_prompt.as_mut() {
            if rows.is_empty() {
                fp.hide();
            } else {
                fp.show(&rows, None, None);
            }
            fp.movie.player.update(dt);
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("photoalbum_prompt"));
            ctx.move_to_top(layer);
            fp.movie.paint(ctx, &ctx.layer_painter(layer));
        }
        MenuAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// photo/Slot.sc's arguments in PhotoAlbum.sc.xml (s0: down s4, left s3, right s1, up s12, ...).
    #[test]
    fn grid_wraps_like_the_slot_states() {
        let table: [(usize, usize, usize, usize, usize); 16] = [
            (0, 4, 3, 1, 12),
            (1, 5, 0, 2, 13),
            (2, 6, 1, 3, 14),
            (3, 7, 2, 0, 15),
            (4, 8, 7, 5, 0),
            (5, 9, 4, 6, 1),
            (6, 10, 5, 7, 2),
            (7, 11, 6, 4, 3),
            (8, 12, 11, 9, 4),
            (9, 13, 8, 10, 5),
            (10, 14, 9, 11, 6),
            (11, 15, 10, 8, 7),
            (12, 0, 15, 13, 8),
            (13, 1, 12, 14, 9),
            (14, 2, 13, 15, 10),
            (15, 3, 14, 12, 11),
        ];
        for (k, down, left, right, up) in table {
            assert_eq!(grid_move(k, Dir::Down), down, "down from {k}");
            assert_eq!(grid_move(k, Dir::Left), left, "left from {k}");
            assert_eq!(grid_move(k, Dir::Right), right, "right from {k}");
            assert_eq!(grid_move(k, Dir::Up), up, "up from {k}");
        }
    }

    #[test]
    fn first_free_next_prev_and_prompts() {
        let mut used = [false; SLOTS];
        assert_eq!(first_free(&used), 0);
        used[0] = true;
        used[1] = true;
        used[5] = true;
        assert_eq!(first_free(&used), 2);
        // ShowNextPhoto / ShowPreviousPhoto: the next used slot round the sixteen
        assert_eq!(next_used(&used, 1), Some(5));
        assert_eq!(next_used(&used, 5), Some(0));
        assert_eq!(prev_used(&used, 0), Some(5));
        assert_eq!(prev_used(&used, 5), Some(1));
        assert_eq!(next_used(&[false; SLOTS], 3), None);
        // (the wrap below 0 lands on 16, never used, then 15)
        let mut last = [false; SLOTS];
        last[15] = true;
        assert_eq!(prev_used(&last, 0), Some(15));
        // all sixteen used: the chooser opens on slot 0
        assert_eq!(first_free(&[true; SLOTS]), 0);
        // SetPreviewPicture's prompts
        assert_eq!(preview_prompt(true, true, 3), 9);
        assert_eq!(preview_prompt(false, true, 3), 8);
        assert_eq!(preview_prompt(false, false, 3), 56);
        assert_eq!(preview_prompt(false, false, 0), 58);
        assert_eq!((full_prompt(1), full_prompt(2)), (57, 10));
        assert_eq!(prompt_rows(9), &[("prompt_cancel", 2), ("PA_Choose_Slot", 1)]);
        assert_eq!(prompt_rows(7).len(), 6);
    }

    /// sub_8263DCF8: a new slot raises the count (aFAB as it reaches 16), an overwrite does
    /// not; tPTS + 1 for every picture saved.
    #[test]
    fn save_into_chosen_slot_counts() {
        let mut g = Game::new(Vec::new(), 0);
        let dir = std::env::temp_dir().join(format!("mcla_photoalbum_{}", std::process::id()));
        let data = dir.join("cache");
        std::fs::create_dir_all(&data).unwrap();
        g.data_dir = data.to_string_lossy().to_string();
        // fifteen slots, out of order, then an overwrite of one of them
        for k in (1..16).rev() {
            let p = g.mc4b_photo_save(k);
            assert!(p.ends_with(&format!("slot_{k:02}.png")));
            std::fs::write(&p, b"png").unwrap();
        }
        assert!(!g.mc4_awarded("aFAB"));
        std::fs::write(g.mc4b_photo_save(7), b"png").unwrap();
        assert!(!g.mc4_awarded("aFAB"));
        assert_eq!(g.mc4_stat("tPTS", 0), 16.0);
        // the sixteenth slot: aFAB
        std::fs::write(g.mc4b_photo_save(0), b"png").unwrap();
        assert!(g.mc4_awarded("aFAB"));
        assert_eq!(g.mc4_stat("tPTS", 0), 17.0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
