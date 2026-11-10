//! The save manager's dialogs a PC build can reach (docs/reference/save/save_flow_messages.md).
//!
//! The original's mcSaveGameDataManager posts a message number to the UI (F-96657); the number is an
//! index into the table of `file.*` event names the manager object keeps (`EVENT_NAMES`), and
//! SaveGame.sc.xml maps the events to its dialogs. A PC build has a file where the console has a
//! profile, a storage device and a dongle, so of the 46 messages only the write result is its own:
//! a save that cannot be written (message 5 `saveError` / 41 `unexpectedSaveError`, SG_SaveGameError)
//! and one that cannot be written for want of room (28 `noRoomToSave`, SG_NoRoomToSave).

/// The `file.*` events by message number (the table at the manager's +52; read from the executable's image).
pub const EVENT_NAMES: [&str; 47] = [
    "loadStart", "loadSuccess", "loadError", "saveStart", "saveSuccess", "saveError", "enumerateStart", "enumerateSuccess",
    "enumerateError", "startupChecksStart", "startupChecksSuccess", "startupChecksError", "startupChecksComplete", "allFilesLoaded",
    "userDoesNotWantToSave", "createNewSaveGame", "noGamesFound", "multipleGamesFound", "gameFound", "userCanceledDeviceSelection",
    "noSignedInPlayer", "playerHasSignedOut", "playerHasSignedIn", "storageDeviceChanged", "saveGameRemoved", "saveGameRemovedOnLoad",
    "saveGameVerified", "verifyAndSaveStart", "noRoomToSave", "dongleError", "disableSaving", "createNewSaveGameError", "fileCorrupt",
    "deviceSelectionStart", "deviceChangeStart", "deviceSelectionSuccess", "errorOccured", "autosaveDisabled", "newSaveCreated",
    "overwriteConfirmation", "fullDeviceSelected", "unexpectedSaveError", "controllerUnplugged", "endFreeSpaceCheck", "timerExpired",
    "downloadableReboot", "downloadableContentChanged",
];

/// The dialogs of SaveGameDialogs that a failed write can reach (SaveGame.sc.xml, the XENON build).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SgBox {
    /// SG_SaveGameError: `file.saveError` / `file.unexpectedSaveError`
    SaveGameError,
    /// SG_NoRoomToSave: `file.noRoomToSave`
    NoRoomToSave,
    /// SG_AutoSaveDisabled: from CONTINUE WITHOUT SAVING
    AutoSaveDisabled,
}

/// What the player's answer does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// `UI.ActivateAndFocus(SG_VerifyAndSaveGame)`: the save is written again, behind the SAVING tag
    Retry,
    /// `UI.ActivateAndFocus(SG_AutoSaveDisabled)`
    Box(SgBox),
    /// `file.disableAutosave()` and the NO_SAVE_EVENT: nothing is written from now on
    DisableAutosave,
}

/// The message a failed write posts: no room is 28 (NO_FREE_SPACE, F-74254's room test), anything else 5.
pub fn message_for(e: &std::io::Error) -> u8 {
    // (ERROR_DISK_FULL 112, ERROR_HANDLE_DISK_FULL 39, ENOSPC 28; the quota error 1295 / EDQUOT 122 are no room too)
    if e.kind() == std::io::ErrorKind::StorageFull || matches!(e.raw_os_error(), Some(112 | 39 | 28 | 1295 | 122)) {
        28
    } else {
        5
    }
}

/// The dialog SaveGame.sc.xml opens for a message number, when it is one a PC build posts.
pub fn dialog_for(msg: u8) -> Option<SgBox> {
    match msg {
        5 | 41 => Some(SgBox::SaveGameError),
        28 => Some(SgBox::NoRoomToSave),
        _ => None,
    }
}

impl SgBox {
    /// (title key, description key, buttons as (id, key) in the XML's order). The buttons of the XML that need
    /// a storage device (`Common_ChangeStorageDevice`, `file.changeDevice()`) are not here: a PC has none.
    pub fn spec(self) -> (&'static str, &'static str, &'static [(&'static str, &'static str)]) {
        match self {
            SgBox::SaveGameError => ("Save_Dialog_Title_Error", "Save_Dialog_SaveError", &[("retry", "Common_Yes"), ("nosave", "Common_ContinueWithoutSaving")]),
            SgBox::NoRoomToSave => ("Save_Dialog_Title_Error", "Save_NotEnoughFreeBlocks", &[("nosave", "Common_ContinueWithoutSaving")]),
            SgBox::AutoSaveDisabled => ("Save_Dialog_Title_Warning", "Save_Dialog_SaveDisabled", &[]),
        }
    }

    /// The answer `choice` (the button chosen; 0 for a box without buttons) gives.
    pub fn answer(self, choice: usize) -> Next {
        match (self, choice) {
            (SgBox::SaveGameError, 0) => Next::Retry,
            (SgBox::SaveGameError, _) | (SgBox::NoRoomToSave, _) => Next::Box(SgBox::AutoSaveDisabled),
            (SgBox::AutoSaveDisabled, _) => Next::DisableAutosave,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_numbers_are_the_tables_indexes() {
        // (the numbers read from the handlers: F-96657's callers F-31703 / F-51830 / F-62781)
        assert_eq!(EVENT_NAMES[3], "saveStart");
        assert_eq!(EVENT_NAMES[4], "saveSuccess");
        assert_eq!(EVENT_NAMES[5], "saveError");
        assert_eq!(EVENT_NAMES[28], "noRoomToSave");
        assert_eq!(EVENT_NAMES[41], "unexpectedSaveError");
        assert_eq!(EVENT_NAMES[44], "timerExpired");
        assert_eq!(EVENT_NAMES[46], "downloadableContentChanged");
    }

    #[test]
    fn a_failed_write_opens_the_xml_dialogs() {
        let full = std::io::Error::from(std::io::ErrorKind::StorageFull);
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(message_for(&full), 28);
        assert_eq!(message_for(&std::io::Error::from_raw_os_error(112)), 28);
        assert_eq!(message_for(&denied), 5);
        assert_eq!(dialog_for(5), Some(SgBox::SaveGameError));
        assert_eq!(dialog_for(41), Some(SgBox::SaveGameError));
        assert_eq!(dialog_for(28), Some(SgBox::NoRoomToSave));
        assert_eq!(dialog_for(23), None);
        let (t, d, o) = SgBox::SaveGameError.spec();
        assert_eq!((t, d), ("Save_Dialog_Title_Error", "Save_Dialog_SaveError"));
        assert_eq!(o.iter().map(|b| b.1).collect::<Vec<_>>(), ["Common_Yes", "Common_ContinueWithoutSaving"]);
    }

    #[test]
    fn answers_follow_the_xml() {
        // SG_SaveGameError: YES -> setSelectedDeviceToAny + SG_VerifyAndSaveGame (the save again),
        // CONTINUE WITHOUT SAVING -> SG_AutoSaveDisabled, whose accept is disableAutosave
        assert_eq!(SgBox::SaveGameError.answer(0), Next::Retry);
        assert_eq!(SgBox::SaveGameError.answer(1), Next::Box(SgBox::AutoSaveDisabled));
        assert_eq!(SgBox::NoRoomToSave.answer(0), Next::Box(SgBox::AutoSaveDisabled));
        assert_eq!(SgBox::AutoSaveDisabled.answer(0), Next::DisableAutosave);
    }

    /// A save that cannot be written posts the manager's message and the step puts its dialog up.
    #[test]
    fn a_save_that_cannot_be_written_posts_the_message() {
        use std::sync::atomic::Ordering;
        std::env::remove_var("MCLA_SAVE_FILE");
        let dir = std::env::temp_dir().join(format!("mcla_saveflow_{}", std::process::id()));
        // (a folder where the file should be: the write fails)
        std::fs::create_dir_all(dir.join("mcla_save.json")).unwrap();
        let mut g = crate::game::Game::new(Vec::new(), 0);
        g.write_save(&dir.to_string_lossy());
        assert_eq!(g.save_fail.load(Ordering::Relaxed), 5);
        // (no popup movie in a test: the step takes the message and shows nothing)
        g.save_dialog_step();
        assert_eq!(g.save_fail.load(Ordering::Relaxed), 0);
        assert!(g.save_dlg.is_none() && !g.no_save);
        // AUTO SAVE off writes nothing and posts nothing
        g.no_save = true;
        g.write_save(&dir.to_string_lossy());
        assert_eq!(g.save_fail.load(Ordering::Relaxed), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A damaged save (a cut-off copy in a folder of its own, never the player's): the start-up checks raise
    /// SG_LoadError (RETRY / CREATE NEW SAVE), RETRY reads the file again, CREATE NEW SAVE asks
    /// SG_CreateNewSaveConfirmation (YES / NO), NO goes back, YES starts the new career; nothing is written over the
    /// file before YES. The boxes' strings and buttons are SaveGame.sc.xml's.
    #[test]
    fn a_damaged_save_raises_sg_load_error() {
        std::env::remove_var("MCLA_SAVE_FILE");
        std::env::remove_var("MCLA_NEW_GAME");
        let dir = std::env::temp_dir().join(format!("mcla_loaderr_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("mcla_save.json");
        let damaged = b"{\"car\": \"vp_dge_challenger_70\", \"color\": 3, \"cfg";
        std::fs::write(&file, damaged).unwrap();
        let d = dir.to_string_lossy().to_string();
        let mut g = crate::game::Game::new(Vec::new(), 0);
        g.data_dir = d.clone();
        g.load_save(&d);
        assert!(g.save_damaged && g.fresh_game, "a new career until the box is answered");
        g.write_save(&d);
        assert_eq!(std::fs::read(&file).unwrap(), damaged, "nothing written over the damaged file");
        let shown = |g: &mut crate::game::Game| -> (String, String, Vec<String>) {
            let (m, _, _) = g.message_box.take().expect("a box");
            (m.title, m.description, m.options.into_iter().map(|o| o.1).collect())
        };
        assert!(g.load_error_boxes());
        assert_eq!(shown(&mut g), ("Save_Dialog_Title_Error".into(), "Save_Dialog_LoadError".into(), vec!["Common_Retry".into(), "Save_CreateNewSave".into()]));
        // RETRY, still damaged: the same box again
        g.box_sel = 0;
        assert!(g.load_error_boxes() && g.save_box == 0 && !g.relaunch);
        assert!(g.load_error_boxes() && g.save_box == 6);
        g.message_box = None;
        // CREATE NEW SAVE: the confirmation; NO: SG_LoadError again
        g.box_sel = 1;
        assert!(g.load_error_boxes());
        assert_eq!(shown(&mut g), ("Save_Dialog_Title_Message".into(), "Save_Dialog_OverwriteDamageDataConf".into(), vec!["Common_Yes".into(), "Common_No".into()]));
        g.box_sel = 1;
        assert!(g.load_error_boxes() && g.save_box == 0 && g.save_damaged);
        assert!(g.load_error_boxes() && g.save_box == 6);
        g.message_box = None;
        g.box_sel = 1;
        assert!(g.load_error_boxes() && g.save_box == 7);
        g.message_box = None;
        // YES: CREATE_NEW_SAVE (the new career's SAVING / LOADING), the next save writes over the file
        g.box_sel = 0;
        assert!(g.load_error_boxes() && g.save_box == 3 && !g.save_damaged && !g.relaunch);
        assert!(!g.load_error_boxes(), "the new career's own steps from here");
        g.write_save(&d);
        assert!(!crate::game::Game::save_damaged_at(&d), "a readable save now");
        // RETRY on a file that reads again: the game starts again to load it, and writes nothing meanwhile
        let good = std::fs::read(&file).unwrap();
        g.save_damaged = true;
        g.save_box = 6;
        g.box_sel = 0;
        assert!(g.load_error_boxes() && g.relaunch && g.save_box == 0);
        g.write_save(&d);
        assert_eq!(std::fs::read(&file).unwrap(), good);
        let _ = std::fs::remove_dir_all(dir);
        // the boxes as the XML has them
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/cache/tune/ui/SaveGame.sc.xml");
        let Ok(x) = std::fs::read_to_string(p) else { return };
        for (id, title, desc, buttons) in [
            ("SG_LoadError", "Save_Dialog_Title_Error", "Save_Dialog_LoadError", ["Common_Retry", "Save_CreateNewSave"]),
            ("SG_CreateNewSaveConfirmation", "Save_Dialog_Title_Message", "Save_Dialog_OverwriteDamageDataConf", ["Common_Yes", "Common_No"]),
        ] {
            let at = x.find(&format!("\"{id}\" class")).unwrap();
            let next = x[at + 10..].find("<state id").map_or(x.len(), |n| at + 10 + n);
            let s = &x[at..next];
            assert!(s.contains(&format!("expr=\"{title}\"")) && s.contains(&format!("expr=\"{desc}\"")), "{id}");
            let (a, b) = (s.find(&format!("desc=\"{}\"", buttons[0])).unwrap(), s.find(&format!("desc=\"{}\"", buttons[1])).unwrap());
            assert!(a < b, "{id}: button order");
        }
        assert!(x.contains("event=\"file.loadError\">
               <action expr=\"UI.ActivateAndFocus(SG_LoadError)\"") || x.contains("UI.ActivateAndFocus(SG_LoadError)"));
    }

    /// The strings and buttons come out of the XML itself.
    #[test]
    fn spec_matches_save_game_xml() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../extracted/cache/tune/ui/SaveGame.sc.xml");
        let Ok(x) = std::fs::read_to_string(p) else { return };
        let state = |id: &str| -> String {
            let at = x.find(&format!("\"{id}\" class")).unwrap();
            // (the buttons are states with a `desc` and no id: take up to the next box)
            let next = x[at + 10..].find("<state id").map_or(x.len(), |n| at + 10 + n);
            x[at..next].to_string()
        };
        for (b, id) in [(SgBox::SaveGameError, "SG_SaveGameError"), (SgBox::NoRoomToSave, "SG_NoRoomToSave"), (SgBox::AutoSaveDisabled, "SG_AutoSaveDisabled")] {
            let s = state(id);
            let (t, d, _) = b.spec();
            assert!(s.contains(&format!("expr=\"{t}\"")), "{id} title");
            assert!(s.contains(&format!("expr=\"{d}\"")), "{id} description");
        }
        // (the XENON build's buttons of SG_SaveGameError, in order, then the one a PC has no use for)
        let s = &state("SG_SaveGameError");
        let keys: Vec<&str> = s.split("desc=\"").skip(1).filter_map(|p| p.split('"').next()).collect();
        assert_eq!(keys, ["Common_Yes", "Common_ContinueWithoutSaving", "Common_ChangeStorageDevice"]);
        assert!(s.contains("file.setSelectedDeviceToAny()") && s.contains("SG_VerifyAndSaveGame") && s.contains("SG_AutoSaveDisabled"));
    }
}
