//! Career: the game's own mission graph (`tune/career/missions/*.xml`, 166 missions with
//! prerequisites, objectives and rewards), the reputation thresholds that hand out
//! missions (`tune/career/0x327a2cd4`, system "Progression") and the original texts
//! (`ui/0xa4c1e8fa`).
//!
//! The original fires most objective events from compiled scripts; here the gameplay
//! reports what happened (`Ev`) and each condition type is matched against that:
//! hookman races are won against the roaming racers, series / tournaments are tallies of
//! race wins in the mission's district, and so on.

use hashbrown::{HashMap, HashSet};
use mcla_formats::strtbl::StringTable;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone)]
enum Cond {
    Mission(String),
    Event { event: String, ident: String },
    TallyRace { tally: u32, district: String, min_place: u32 },
    TallyMission { less_than: bool, tally: u32, district: String },
    Stat { stat: String, value: i64 },
    Experience { amount: i64 },
}

#[derive(Debug, Clone)]
struct Requirement {
    desc: String,
    hidden: bool,
    conds: Vec<Cond>,
    /// per condition: a Stat condition's szStatType (Tally, Money, Purse), else empty
    stat_types: Vec<String>,
    /// bAllowUpdate (+120, the constructor's 1 when the file has none): a condition's
    /// Condition_Updated marks the requirement updated and is passed on (F-64780)
    allow_update: bool,
    /// nTargetId (+104): the uid of what the objective is about (a hangout, a racer)
    target_id: i32,
    /// bInitValidation (+119): the arming (F-19696) asks each condition to check at once
    /// (F-97796 -> its update with no mission)
    init_validation: bool,
}

#[derive(Debug, Clone)]
enum Reward {
    Mission { id: String, state: String },
    Car(String),
    /// a call from the mechanic (`game/Career/<Name>Tip` scripts): the string key
    Tip(String),
    /// a mission's mcRewardScript that makes a call (`Career::reward_call`): the script's name
    Call(String),
    /// a Progression threshold's script without a call (`game\Career\unlock_unordered_races`):
    /// nothing to give here, its effect is read from the reached thresholds (`script_reached`)
    Script(String),
}

#[derive(Debug, Clone)]
struct Mission {
    id: String,
    hookman: String,
    district: String,
    hidden: bool,
    prereq: Vec<Requirement>,
    objectives: Vec<Requirement>,
    rewards: Vec<Reward>,
    /// each reward's bHidden (+8) and bGiveImmediately (+9)
    reward_flags: Vec<(bool, bool)>,
    /// place in the mission lists (`0x68d5d432`, then `0x3b47e155`): the save's index
    index: i32,
    /// a South Central mission (the save's add-on block)
    addon: bool,
    /// bAutoStart (+686): READY_TO_START passes on by itself (step 20b)
    auto_start: bool,
    /// a READY_TO_START_ON_ENTER script (the call); its name when it is a career script
    /// (bCareerScript)
    on_enter: bool,
    on_enter_career: Option<String>,
    /// bRepeatable (+693)
    repeatable: bool,
    /// the first mcMissionReminder's szReminderId (aReminders +744; F-88990 only ever takes
    /// reminder[0]: the cursor +740 is never advanced)
    reminder: Option<String>,
    /// bShouldUpdate (+691, the constructor's 1 when the file has none): MISSION RESULTS may
    /// list the mission once it is updated (F-78851 kind 4)
    should_update: bool,
    /// (step 21b) the mission's other event scripts (aEvents, mcMissionEventScript): the
    /// event's name (HW_GARAGE_ENTER, DT_LOCAL_HANGOUT1_ENTER ...) and the state it is
    /// listened to in (2 READY_TO_START, 3 CHECKING_OBJECTIVES)
    events_at: Vec<(String, u8)>,
}

/// The races of a career event and the hookman who runs it, as named by the event's
/// compiled script (`script_obj/**`: strings such as `Game/Race/Hills/Ordered/hl_ordered_mb_03`
/// and `GH_20_2_vp_mit_3000gt_99` next to the mission id or objective identifier).
#[derive(Debug, Clone, Default)]
pub struct EventDef {
    /// lower case, e.g. "hills/ordered/hl_ordered_mb_03"
    pub races: Vec<String>,
    /// car set-up of the rival, e.g. "GH_20_2_vp_mit_3000gt_99"
    pub rival: String,
}

/// Scan the script objects for the races of every key (mission id / objective identifier).
fn scan_events(root: &Path, keys: &HashSet<String>) -> HashMap<String, EventDef> {
    fn walk(p: &Path, out: &mut Vec<std::path::PathBuf>) {
        if let Ok(rd) = std::fs::read_dir(p) {
            let mut v: Vec<_> = rd.flatten().map(|e| e.path()).collect();
            v.sort();
            for c in v {
                if c.is_dir() {
                    walk(&c, out);
                } else {
                    out.push(c);
                }
            }
        }
    }
    let mut files = Vec::new();
    walk(&root.join("script_obj"), &mut files);
    let mut out: HashMap<String, EventDef> = HashMap::new();
    for f in files {
        let Some(sco) = std::fs::read(&f).ok().and_then(|d| mcla_formats::sco::decode(&d).ok()) else { continue };
        let strings = sco.strings(5);
        let mut races: Vec<String> = Vec::new();
        let mut rival = String::new();
        let mut found: Vec<String> = Vec::new();
        for s in &strings {
            let l = s.to_ascii_lowercase();
            if let Some(i) = l.find("ame/race/") {
                let r: String = l[i + 9..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '/').collect();
                if r.matches('/').count() == 2 && !races.contains(&r) {
                    races.push(r);
                }
                continue;
            }
            let t = s.trim();
            let up = t.to_ascii_uppercase();
            if keys.contains(&up) && !found.contains(&up) {
                found.push(up);
            } else if rival.is_empty() && (up.starts_with("GH_") || up.starts_with("SH_") || up.starts_with("SC_GH_") || up.starts_with("SC_SH_")) && (l.contains("_vp_") || l.contains("_vpd_")) {
                rival = t.to_string();
            }
        }
        // generic scripts list every race of the game: not an event
        if races.is_empty() || races.len() > 6 || found.is_empty() {
            continue;
        }
        for k in found {
            out.entry(k).or_insert_with(|| EventDef { races: races.clone(), rival: rival.clone() });
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Stage {
    #[default]
    Locked,
    /// handed out, waiting for its prerequisites
    Armed,
    Active,
    Complete,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MState {
    pub stage: Stage,
    /// tally per objective
    #[serde(default)]
    pub progress: Vec<u32>,
    /// races of the mission's event already won
    #[serde(default)]
    pub won: Vec<String>,
    /// a tournament's points so far, by racer ("YOU" is the player)
    #[serde(default)]
    pub pts: std::collections::HashMap<String, i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SaveState {
    pub missions: std::collections::HashMap<String, MState>,
    pub stats: std::collections::HashMap<String, i64>,
    #[serde(default)]
    pub order: Vec<String>,
    /// the statistics of an original save, "<id>/<sub id or vehicle>" (tools/import_mc4_save.py)
    #[serde(default)]
    pub ostats: std::collections::HashMap<String, f64>,
}

/// What happened in the game.
#[derive(Debug, Clone, Copy)]
pub enum Ev<'a> {
    /// a race ended; `hookman`: it was started by challenging a roaming racer
    /// `race`: the definition's id; `beat_goal`: finished inside the event's target time
    Race { district: &'a str, place: usize, hookman: bool, class: &'a str, race: &'a str, beat_goal: bool },
    Delivery,
    /// mcEvent_PostEventStr(event, identifier): every open mcConditionString that waits for
    /// this event with this identifier is met - Updated and Complete (F-77134: a byte for byte
    /// compare of the identifier, case counts)
    Str(&'a str, &'a str),
    Rep(i64),
    Money(i64),
}

#[derive(Debug, Clone)]
pub enum News {
    Started { id: String, title: String, who: String, text: String },
    Completed { id: String, title: String },
    Car(String),
    /// prize money of an event (series, tournament, wager)
    Money(i64),
    /// a tip over the phone: speech scenario, title and text
    Tip { key: String, title: String, text: String },
    /// a mission's reward script that makes a call (`Career::reward_call`): the script's name
    Call(String),
    /// (step 21b) a story scene a script plays (a CORE at a completion or a meeting, an event
    /// script at a garage / hangout): the mission whose scene it is (`Cutscenes::for_mission`)
    Scene(String),
}

/// `game\Career\CarsUnlockTip_02` -> `SIDEKICK_CARS_UNLOCK_TIP_02` - the string each tip script
/// shows; only VisualPartUnlockTip_01..04 show `SIDEKICK_VISUAL_PART_TIP_0n` (no UNLOCK) - test
/// tip_keys_are_the_scripts_strings
fn tip_key(script: &str) -> String {
    let name = script.rsplit(|c| c == '\\' || c == '/').next().unwrap_or(script);
    let mut out = String::from("SIDEKICK");
    for ch in name.chars() {
        if ch.is_ascii_uppercase() {
            out.push('_');
        }
        out.push(ch.to_ascii_uppercase());
    }
    out.replace("__", "_").replace("SIDEKICK_VISUAL_PART_UNLOCK_TIP", "SIDEKICK_VISUAL_PART_TIP")
}

pub struct ObjectiveView {
    pub text: String,
    pub cur: u32,
    pub target: u32,
    /// what the original's mission log puts after the line: "(n/target)" (step 20)
    pub count: Option<String>,
}

pub struct MissionView {
    /// the mission's id (`BEAT_FREEWAY`): names its picture in the mission log
    pub id: String,
    pub title: String,
    pub who: String,
    pub desc: String,
    pub objectives: Vec<ObjectiveView>,
    /// the event race to drive next, if the mission has its own races
    pub next_race: Option<String>,
}

#[derive(Default)]
pub struct Career {
    missions: Vec<Mission>,
    index: HashMap<String, usize>,
    state: Vec<MState>,
    /// activation order of the active missions (oldest first)
    order: Vec<usize>,
    /// the mission manager's reminder countdown in seconds (+84; None = the constructor's
    /// 500, F-33894) and its round robin counter (+80)
    reminder_t: Option<f32>,
    reminder_rr: u32,
    pub strings: StringTable,
    thresholds: Vec<(f32, Reward)>,
    /// the id of each threshold's entry above in the progression tables (`Progression_7`,
    /// `Unlocking_3`; empty for the port's own), and the ids whose rewards were handed out:
    /// the original gives a threshold's rewards once and keeps a "reached" flag (`mc4`)
    threshold_ids: Vec<String>,
    /// each entry's GiveImmediately (+9): given while its threshold is walked, else appended to the pending list
    threshold_gi: Vec<bool>,
    pub reached_ids: HashSet<String>,
    /// the ability systems' amounts (uses counted by the system itself: `mc4`,
    /// experience.abilities), by system name; a system not here counts by its statistic
    pub ability_amounts: HashMap<String, i64>,
    /// the `Unlocking` system's amount (`mc4`, experience.main) once it is counted on its
    /// own; None: it follows the reputation (a save of the port from before it was counted)
    pub unlocking: Option<i64>,
    /// payback legs won in a row (the original's `nPaybackCurrentWins` of the payback's
    /// results state; not saved), and "the payback is complete" for the caller to take
    pub payback_wins: u32,
    pub payback_done: bool,
    /// tournaments just won: (the event's name, the mission asks for a Hard completion)
    pub completed_events: Vec<(String, usize)>,
    /// the last race decided a tournament: won (bComplete) or lost (bFailure) - the caller
    /// takes it for form 56's banner
    pub tour_decided: Option<bool>,
    /// the standings after the last race of a tournament (form 49); None after any other race
    pub tour_table: Option<TourTable>,
    /// the body part types the last race's class point unlocked: the rewards screen lists them
    pub unlocked_now: Vec<String>,
    /// the performance levels the last race's experience unlocked and the ability levels it
    /// gave (their texts) - the rewards screen's UNLOCKED and REWARDED groups -, and the
    /// page of that screen that is shown
    pub perf_now: Vec<String>,
    pub rewarded_now: Vec<String>,
    pub reward_page: usize,
    /// the cars given since the last reward was worked out (their set-ups' names): the
    /// rewards screen's car page
    pub cars_now: Vec<String>,
    /// time trials: the row chosen at the hookman's panel; the second and third missions
    /// seen started this session; whether the last time trial's time was beaten
    pub tt_pick: usize,
    pub tt_seen: Option<Vec<String>>,
    /// the missions a reward has just put at READY_TO_START (their ON_ENTER scripts run then)
    pub ready_now: Vec<String>,
    pub tt_last: Option<bool>,
    /// an event panel was accepted: the race it held back may start
    pub panel_ok: bool,
    /// a payback is running from its panel; a won leg wants its next client; the payback
    /// races this run has had (lower case)
    pub payback_run: bool,
    pub payback_next: bool,
    pub payback_used: Vec<String>,
    /// the experience systems of the career's tables, with their thresholds (`mc4`)
    pub exp_tables: crate::mc4::ExpTables,
    /// ability systems of the progression table: name -> (uses needed, level)
    pub ability_levels: HashMap<String, Vec<(i64, u8)>>,
    /// "Unlocking" system: performance id (D1, C2, A0 ...) -> reputation needed
    pub perf_unlocks: HashMap<String, i64>,
    /// class systems (Tuner, Muscle ...): part name -> wins in that class needed
    pub part_unlocks: HashMap<String, HashMap<String, i64>>,
    rewarded: HashSet<usize>,
    /// step 20: per mission the requirements' hidden bytes and the conditions' baselines
    pub ext: Vec<MissionExt>,
    /// the completed missions in the order they were completed (the mission manager's list
    /// 2: the mission log's COMPLETED tab; `mc4` completed_slot)
    pub done: Vec<usize>,
    /// what happened to the missions this session, in order: (true, i) entered the log,
    /// (false, i) completed - what the mission and story scripts start follows it (`mc4`)
    pub journal: Vec<(bool, usize)>,
    /// the journal holds the whole career (it began this session)
    pub journal_complete: bool,
    done_now: Vec<usize>,
    pub stats: HashMap<String, i64>,
    /// the statistics carried over from an original save, "<id>/<sub id or vehicle>": what
    /// the statistics pages show where the port keeps no count of its own
    pub ostats: HashMap<String, f64>,
    pub news: Vec<News>,
    /// A career brought over from the game's own save has seen its tips long ago: the tips
    /// that the first tally of its reputation would raise again are counted as seen
    /// (while this is above zero; it falls with each reputation event).
    pub mute_tips: u8,
    rep: i64,
    /// event definitions by mission id / objective identifier (upper case)
    events: HashMap<String, EventDef>,
    /// prize cars of events (`tune/career` event reward tables), by event name (upper case)
    event_cars: HashMap<String, Vec<String>>,
    /// the set-up file a prize car was named by (`SC_tournament_vpd_lr_sport_08` ...), by
    /// the car's folder name in lower case: what its garage record is made from (`mc4`)
    pub car_configs: HashMap<String, String>,
    /// (step 22b) Downtown's pink slip: its event script has played its scene, its race follows
    pub dt_pink_due: bool,
    /// HANGOUT_SERIES_DOWNTOWN's event script ran at the hangout: its series is named
    /// HANGOUT_SERIES_DT_COMPLETE (`series_won`)
    pub dt_series_story: bool,
    /// the location events posted this visit (cleared when the place is left or a race starts)
    pub visit_events: HashSet<String>,
    /// each mission's "updated" byte (+690: F-30196 sets it, F-92621 clears it; not saved)
    /// and what it is found from (`scan_updates`)
    updated: std::cell::RefCell<Vec<bool>>,
    /// the reward manager's pending list (F-64223): the rewards not given yet
    pub pending: Vec<Pend>,
    /// the amounts of the experience manager's own race rewards (F-48818): money (+428, amount +60), Progression
    /// (+236, amount +92) and Unlocking (+332): one object each, so a race queued again before the page overwrites the amount
    pub pend_money: i64,
    pub pend_exp: [i64; 2],
    /// a give-all is running (`Game::give_pending`): a walk's deferred rewards join the list
    pub in_give: bool,
    /// completions held back in the journal behind their pending rewards
    done_wait: Vec<usize>,
    upd: Vec<UpdSnap>,
}

/// What `Career::scan_updates` compares from one call to the next for a mission: its state,
/// each objective's progress and met, each prerequisite's conditions' measures and met, and
/// whether its prerequisites (CHECKING_PREREQUISITES' enter, F-89508) and its objectives
/// (CHECKING_OBJECTIVES' enter, F-73549) have been armed - F-39071 subscribes them, and a
/// requirement never unsubscribes.
#[derive(Clone, Default)]
struct UpdSnap {
    state: u32,
    obj: Vec<(i64, bool)>,
    pre: Vec<(Vec<i64>, bool)>,
    pre_armed: bool,
    obj_armed: bool,
}

fn tag<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name} content=\"ascii\">");
    let i = text.find(&open)? + open.len();
    let e = text[i..].find('<')?;
    Some(text[i..i + e].trim())
}

fn val<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name} value=\"");
    let i = text.find(&open)? + open.len();
    let e = text[i..].find('"')?;
    Some(&text[i..i + e])
}

/// Text between `<name>` and `</name>` (empty for `<name/>`), searched from `from`.
fn section<'a>(text: &'a str, name: &str) -> &'a str {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    match (text.find(&open), text.rfind(&close)) {
        (Some(a), Some(b)) if b > a => &text[a + open.len()..b],
        _ => "",
    }
}

fn requirements(text: &str) -> Vec<Requirement> {
    let mut out = Vec::new();
    for chunk in text.split("<Item type=\"mcRequirement\">").skip(1) {
        let mut conds = Vec::new();
        let mut stat_types = Vec::new();
        for c in chunk.split("<Item type=\"mcCondition").skip(1) {
            stat_types.push(tag(c, "szStatType").unwrap_or("").to_string());
            let kind = c.split('"').next().unwrap_or("");
            let up = |s: Option<&str>| s.unwrap_or("").to_ascii_uppercase();
            let num = |n: &str| val(c, n).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
            conds.push(match kind {
                "Mission" => Cond::Mission(up(tag(c, "szMissionId"))),
                "TallyRace" => Cond::TallyRace { tally: num("nTally").max(1) as u32, district: tag(c, "szDistrict").unwrap_or("").to_string(), min_place: num("nMinimumPlacement") as u32 },
                "TallyMission" => Cond::TallyMission { less_than: tag(c, "szHeuristic").map_or(false, |h| h.eq_ignore_ascii_case("LessThan")), tally: num("nTally") as u32, district: tag(c, "szDistrict").unwrap_or("").to_string() },
                "Stat" => Cond::Stat { stat: tag(c, "szStatId").unwrap_or("").to_string(), value: num("nStatValue") },
                "Experience" => Cond::Experience { amount: num("nExperienceAmount") },
                _ => Cond::Event { event: up(tag(c, "szEventId")), ident: tag(c, "szIdentifier").unwrap_or("").to_string() },
            });
        }
        out.push(Requirement { desc: tag(chunk, "szDescriptionId").unwrap_or("").to_string(), hidden: val(chunk, "bAlwaysHidden") == Some("true"), conds, stat_types, allow_update: val(chunk, "bAllowUpdate") != Some("false"), target_id: val(chunk.split("<aConditions").next().unwrap_or(""), "nTargetId").and_then(|v| v.parse().ok()).unwrap_or(0), init_validation: val(chunk.split("<aConditions").next().unwrap_or(""), "bInitValidation") == Some("true") });
    }
    out
}

/// A district's number (F-67263): "SC" and "South Central" are SouthCentral; Hollywood 0,
/// Beaches 1, Hills 2, Downtown 3, SouthCentral 4, Unknown 5, All 6; anything else -1.
fn district_no(s: &str) -> i32 {
    // (every compare F-40755: ASCII case folded, the CRT's _stricmp)
    let s = if s.eq_ignore_ascii_case("SC") || s.eq_ignore_ascii_case("South Central") { "SouthCentral" } else { s };
    ["Hollywood", "Beaches", "Hills", "Downtown", "SouthCentral", "Unknown", "All"].iter().position(|d| d.eq_ignore_ascii_case(s)).map_or(-1, |p| p as i32)
}

fn same_district(want: &str, got: &str) -> bool {
    want.is_empty() || want.eq_ignore_ascii_case("Unknown") || want.eq_ignore_ascii_case("None") || want.eq_ignore_ascii_case(got)
}

/// Stat id counting the wins of a vehicle class.
fn class_stat(class: &str) -> Option<&'static str> {
    Some(match class.to_ascii_uppercase().as_str() {
        "TUNER" => "kTUW",
        "MUSCLE" => "kMUW",
        "EXOTIC" => "kEXW",
        "LUXURY" => "kLUW",
        "SPORTBIKE" => "kMOW",
        "SUV" => "kSUW",
        "LOWRIDER" => "kLOW",
        _ => return None,
    })
}

impl Career {
    pub fn load(data_dir: &str) -> Career {
        let root = Path::new(data_dir);
        let mut c = Career::default();
        // the main table first, then the smaller ones beside it (South Central, tips, police)
        if let Ok(d) = std::fs::read(root.join("ui/0xa4c1e8fa")) {
            match StringTable::parse(&d) {
                Ok(t) => c.strings = t,
                Err(e) => log::warn!("career: string table: {}", e),
            }
        }
        // (the cut GT-R's names, which the table lacks: catalog::CUT_STRINGS)
        if let Some(m) = c.strings.langs.first_mut() {
            for (k, v) in crate::catalog::CUT_STRINGS {
                m.entry(mcla_formats::hash::joaat(k)).or_insert_with(|| v.to_string());
            }
            // (the mod cars' long / short names from their manifests: modcars.rs)
            for (k, v) in crate::modcars::strings() {
                m.entry(mcla_formats::hash::joaat(k)).or_insert_with(|| v.clone());
            }
        }
        if let Ok(rd) = std::fs::read_dir(root.join("ui")) {
            let mut files: Vec<std::path::PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.file_name().map_or(false, |n| n != "0xa4c1e8fa" && n != "0xac5297d5")).collect();
            files.sort();
            for f in files {
                if let Some(t) = std::fs::read(&f).ok().and_then(|d| StringTable::parse(&d).ok()) {
                    c.strings.merge(t);
                }
            }
        }
        let dir = root.join("tune/career/missions");
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        for d in [dir.clone(), dir.join("sc")] {
            if let Ok(rd) = std::fs::read_dir(&d) {
                files.extend(rd.flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |e| e.eq_ignore_ascii_case("xml"))));
            }
        }
        files.sort();
        // The career's missions are those of the game's own lists, which name the files:
        // `tune/career/0x68d5d432` ("Mission { Name <file> }", 119 names - the list of 108
        // beside it is the first edition's) and `0x3b47e155` for South Central (24 names,
        // "sc\<file>"). Some twenty mission files are on neither (FIND_SH_02B, RACE_FREEWAY,
        // ESCAPE_POLICE, FIND_GH_27, BESTOF_SC ...): the 100 % save knows none of them,
        // and the game never offers them.
        let read_list = |name: &str| -> Vec<String> {
            std::fs::read(root.join("tune/career").join(name))
                .map(|d| String::from_utf8_lossy(&d).lines().filter_map(|l| l.trim().strip_prefix("Name").map(|n| n.trim().to_ascii_uppercase())).collect())
                .unwrap_or_default()
        };
        let listed: std::collections::HashSet<String> = read_list("0x68d5d432").into_iter().collect();
        // (the lists in their order: a mission's place is the save's index - step 20)
        let list_main: Vec<String> = read_list("0x68d5d432");
        let list_sc: Vec<String> = read_list("0x3b47e155").into_iter().map(|n| n.rsplit(['\\', '/']).next().unwrap_or("").to_string()).collect();
        let listed_sc: std::collections::HashSet<String> = read_list("0x3b47e155").into_iter().map(|n| n.rsplit(['\\', '/']).next().unwrap_or("").to_string()).collect();
        for f in files {
            let Ok(t) = std::fs::read_to_string(&f) else { continue };
            let Some(id) = tag(&t, "szId") else { continue };
            let id = id.to_ascii_uppercase();
            let south = f.parent().and_then(|p| p.file_name()).map_or(false, |n| n.eq_ignore_ascii_case("sc"));
            let stem = f.file_stem().map(|n| n.to_string_lossy().to_ascii_uppercase()).unwrap_or_default();
            let list = if south { &listed_sc } else { &listed };
            if !list.is_empty() && !list.contains(&stem) {
                log::debug!("career: mission {} is on no list, left out", id);
                continue;
            }
            if id == "GLOBAL" || id.starts_with("TEST_") {
                continue;
            }
            // the mission's own rewards follow its objectives
            let after = t.find("</aObjectives>").or_else(|| t.find("<aObjectives/>")).unwrap_or(0);
            let tail = &t[after..];
            let rew = tail.find("<aRewards>").map(|a| &tail[a..tail.find("</aRewards>").unwrap_or(tail.len())]).unwrap_or("");
            let mut rewards = Vec::new();
            let mut reward_flags = Vec::new();
            for r in rew.split("<Item type=\"mcReward").skip(1) {
                let flags = (val(r, "bHidden") == Some("true"), val(r, "bGiveImmediately") == Some("true"));
                match r.split('"').next().unwrap_or("") {
                    "Mission" => rewards.push(Reward::Mission { id: tag(r, "szMissionId").unwrap_or("").to_ascii_uppercase(), state: tag(r, "szTransitionState").unwrap_or("").to_ascii_uppercase().replace('_', "") }),
                    "Car" => rewards.push(Reward::Car(tag(r, "szCarName").unwrap_or("").to_ascii_lowercase())),
                    // (the reward scripts that make a call: `reward_call`)
                    "Script" => match tag(r, "szScriptName").and_then(|n| n.rsplit('/').next()).filter(|n| Self::reward_call(n, Some(0)).is_some()) {
                        Some(n) => rewards.push(Reward::Call(n.to_ascii_uppercase())),
                        None => continue,
                    },
                    _ => continue,
                }
                reward_flags.push(flags);
            }
            c.index.insert(id.clone(), c.missions.len());
            c.missions.push(Mission {
                id,
                hookman: tag(&t, "szHookmanNameId").unwrap_or("").to_string(),
                district: tag(&t, "szDistrictName").unwrap_or("").to_string(),
                hidden: val(&t, "bAlwaysHidden") == Some("true"),
                prereq: requirements(section(&t, "aPrerequisites")),
                objectives: requirements(section(&t, "aObjectives")),
                rewards,
                reward_flags,
                index: if south { list_sc.iter().position(|n| *n == stem).map_or(-1, |k| (list_main.len() + k) as i32) } else { list_main.iter().position(|n| *n == stem).map_or(-1, |k| k as i32) },
                addon: south,
                auto_start: val(&t, "bAutoStart") == Some("true"),
                on_enter: t.contains("_READY_TO_START_ON_ENTER</szEventDescription>"),
                on_enter_career: t.split("<Item type=\"mcMissionEventScript\">").skip(1).find(|e| e.contains("_READY_TO_START_ON_ENTER</szEventDescription>")).filter(|e| e.contains("<bCareerScript value=\"true\"/>")).and_then(|e| e.split("<Item content=\"ascii\">").nth(1)).and_then(|s| s.split('<').next()).map(|s| s.trim().to_string()),
                repeatable: val(&t, "bRepeatable") == Some("true"),
                reminder: tag(&t, "szReminderId").map(|v| v.trim().to_string()).filter(|v| !v.is_empty()),
                should_update: val(&t, "bShouldUpdate") != Some("false"),
                events_at: t
                    .split("<Item type=\"mcMissionEventScript\">")
                    .skip(1)
                    .filter_map(|e| {
                        let d = tag(e, "szEventDescription")?.trim().to_string();
                        let s = match tag(e, "szActiveState")?.trim() {
                            "READY_TO_START" => 2,
                            "CHECKING_OBJECTIVES" => 3,
                            _ => return None,
                        };
                        (!d.ends_with("_ON_ENTER")).then_some((d, s))
                    })
                    .collect(),
            });
        }
        c.state = vec![MState::default(); c.missions.len()];
        c.ext_init();
        for m in &c.missions {
            for r in &m.rewards {
                if let Reward::Mission { id, .. } = r {
                    if let Some(&i) = c.index.get(id) {
                        c.rewarded.insert(i);
                    }
                }
            }
        }
        // reputation thresholds of the "Progression" system: the Complete Edition tables
        // (main game + South Central), else the original one
        let cdir = root.join("tune/career");
        let tables: Vec<&str> = if cdir.join("0x3b8ff776").exists() { vec!["0x3b8ff776", "0x30974b1d"] } else { vec!["0x327a2cd4"] };
        for name in tables {
            let Ok(t) = std::fs::read_to_string(cdir.join(name)) else { continue };
            c.exp_tables.parse(&t);
            // the special abilities level up with use ("System { Type Zone MaxAmount 40 ... AbilityLevel }")
            for sys in t.split("\nSystem").skip(1) {
                let word = |n: &str| sys.split_whitespace().skip_while(|w| *w != n).nth(1);
                let (Some(kind), Some(max)) = (word("Type"), word("MaxAmount").and_then(|v| v.parse::<f32>().ok())) else { continue };
                let mut steps: Vec<(i64, u8)> = Vec::new();
                for th in sys.split("Threshold").skip(1) {
                    let w = |n: &str| th.split_whitespace().skip_while(|x| *x != n).nth(1);
                    if th.contains("AbilityLevel") {
                        if let (Some(p), Some(l)) = (w("Percent").and_then(|v| v.parse::<f32>().ok()), w("Level").and_then(|v| v.parse::<u8>().ok())) {
                            steps.push(((p * max).round() as i64, l));
                        }
                    }
                }
                if !steps.is_empty() {
                    c.ability_levels.insert(kind.to_string(), steps);
                }
                // what reputation unlocks (performance stages, the mechanic's calls about them)
                // and what wins in a class of car unlock (the visual parts)
                for th in sys.split("Threshold").skip(1) {
                    let pct = th.split_whitespace().skip_while(|x| *x != "Percent").nth(1).and_then(|v| v.parse::<f32>().ok()).unwrap_or(2.0);
                    for r in th.split("Reward").skip(1) {
                        let field = |n: &str| r.split_whitespace().skip_while(|w| *w != n).nth(1).unwrap_or("").to_string();
                        match field("Type").as_str() {
                            "PerformanceLevel" => {
                                // (a threshold is reached by the system's share, not by pct x MaxAmount: F-61830)
                                c.perf_unlocks.entry(field("PerformanceId").to_ascii_uppercase()).or_insert(crate::mc4::main_amount_at(pct));
                            }
                            "PartUnlock" => {
                                c.part_unlocks.entry(kind.to_ascii_uppercase()).or_default().entry(field("Name").to_ascii_uppercase()).or_insert(crate::mc4::class_amount_at(pct, max as i64));
                            }
                            "Script" if kind == "Unlocking" && field("ScriptName").to_ascii_lowercase().contains("tip") => {
                                c.thresholds.push((crate::mc4::main_amount_at(pct) as f32, Reward::Tip(tip_key(&field("ScriptName")))));
                                c.threshold_ids.push(th.split_whitespace().skip_while(|w| *w != "Id").nth(1).unwrap_or("").to_string());
                                c.threshold_gi.push(field("GiveImmediately") == "1");
                            }
                            _ => {}
                        }
                    }
                }
            }
            let prog = t.split("\nSystem").next().unwrap_or("");
            let max = prog.split_whitespace().skip_while(|w| *w != "MaxAmount").nth(1).and_then(|v| v.parse::<f32>().ok()).unwrap_or(crate::catalog::MAX_REP as f32);
            for th in prog.split("Threshold").skip(1) {
                let pct = th.split_whitespace().skip_while(|w| *w != "Percent").nth(1).and_then(|v| v.parse::<f32>().ok()).unwrap_or(2.0);
                for r in th.split("Reward").skip(1) {
                    let field = |n: &str| r.split_whitespace().skip_while(|w| *w != n).nth(1).unwrap_or("").to_string();
                    if field("Type") == "Mission" {
                        let id = field("MissionId").to_ascii_uppercase();
                        if let Some(&i) = c.index.get(&id) {
                            c.rewarded.insert(i);
                        }
                        c.thresholds.push((crate::mc4::main_amount_at(pct) as f32, Reward::Mission { id, state: field("TransitionState").to_ascii_uppercase().replace('_', "") }));
                        c.threshold_ids.push(th.split_whitespace().skip_while(|w| *w != "Id").nth(1).unwrap_or("").to_string());
                        c.threshold_gi.push(field("GiveImmediately") == "1");
                    } else if field("Type") == "Script" && field("ScriptName").to_ascii_lowercase().contains("tip") {
                        c.thresholds.push((crate::mc4::main_amount_at(pct) as f32, Reward::Tip(tip_key(&field("ScriptName")))));
                        c.threshold_ids.push(th.split_whitespace().skip_while(|w| *w != "Id").nth(1).unwrap_or("").to_string());
                        c.threshold_gi.push(field("GiveImmediately") == "1");
                    } else if field("Type") == "Script" && field("ScriptName").eq_ignore_ascii_case(r"game\Career\unlock_unordered_races") {
                        c.thresholds.push((crate::mc4::main_amount_at(pct) as f32, Reward::Script("unlock_unordered_races".into())));
                        c.threshold_ids.push(th.split_whitespace().skip_while(|w| *w != "Id").nth(1).unwrap_or("").to_string());
                        c.threshold_gi.push(field("GiveImmediately") == "1");
                    }
                }
            }
        }
        // (every titled mission is started by a decoded rule - a reward of the tables or of another mission, or a
        // Mission prerequisite; the port used to give those it thought "script only" thresholds of its own, but
        // with the Complete Edition's tables there are none: `unstarted`, test `every_mission_has_a_start`)
        let unstarted = c.unstarted();
        if !unstarted.is_empty() {
            log::warn!("career: {} missions have no start rule and stay locked: {:?}", unstarted.len(), unstarted);
        }
        // which races belong to which event: read from the scripts once, then cached
        let mut keys: HashSet<String> = c.missions.iter().map(|m| m.id.clone()).collect();
        for m in &c.missions {
            for r in &m.objectives {
                for x in &r.conds {
                    if let Cond::Event { ident, .. } = x {
                        if !ident.is_empty() {
                            keys.insert(ident.to_ascii_uppercase());
                        }
                    }
                }
            }
        }
        let cache = root.join("career_events.tsv");
        match std::fs::read_to_string(&cache) {
            Ok(t) => {
                for line in t.lines() {
                    let p: Vec<&str> = line.split('\t').collect();
                    if p.len() == 3 {
                        c.events.insert(p[0].to_string(), EventDef { rival: p[1].to_string(), races: p[2].split(';').filter(|x| !x.is_empty()).map(|x| x.to_string()).collect() });
                    }
                }
            }
            Err(_) => {
                c.events = scan_events(root, &keys);
                let mut lines: Vec<String> = c.events.iter().map(|(k, e)| format!("{}\t{}\t{}", k, e.rival, e.races.join(";"))).collect();
                lines.sort();
                let _ = std::fs::write(&cache, lines.join("\n"));
            }
        }
        // (the scan took the blitz races named in the red light man's script for this mission's; it has
        // none: hw_redlight_hk1 posts RED_LIGHT_GH_01_COMPLETE on the win of his own race, 0x69ea.
        // Here too, for a cache already written.)
        for k in ["REDLIGHT_GH_01", "RED_LIGHT_GH_01_COMPLETE"] {
            c.events.remove(k);
        }
        // prize cars: "Event { Name hw_tournament1 Difficulty { .. Reward { Type Car Name vp_.._DUB } } }"
        if let Ok(rd) = std::fs::read_dir(&cdir) {
            for e in rd.flatten().filter(|e| e.path().is_file()) {
                let Ok(t) = std::fs::read_to_string(e.path()) else { continue };
                if !t.trim_start().starts_with("Event") {
                    continue;
                }
                c.exp_tables.parse_events(&t);
                for ev in t.split("\nEvent").map(|x| x.trim_start_matches("Event")) {
                    let w: Vec<&str> = ev.split_whitespace().collect();
                    let Some(name) = w.iter().position(|x| *x == "Name").and_then(|i| w.get(i + 1)) else { continue };
                    let mut cars = Vec::new();
                    for i in 0..w.len().saturating_sub(8) {
                        if w[i] == "Type" && w[i + 1] == "Car" {
                            if let Some(k) = w[i..(i + 10).min(w.len())].iter().position(|x| *x == "Name") {
                                let car = w[i + k + 1].to_ascii_lowercase();
                                if !cars.contains(&car) {
                                    cars.push(car);
                                }
                            }
                        }
                    }
                    if !cars.is_empty() {
                        c.event_cars.entry(name.to_ascii_uppercase()).or_insert(cars);
                    }
                }
            }
        }
        log::info!("career: {} events with their races, {} with prize cars", c.events.len(), c.event_cars.len());
        log::info!("career: {} missions, {} reputation thresholds, {} strings", c.missions.len(), c.thresholds.len(), c.strings.keys.len());
        log::info!("career: ability levels by uses {:?}", c.ability_levels);
        log::info!("career: {} performance unlocks, part unlocks for {} classes", c.perf_unlocks.len(), c.part_unlocks.len());
        c
    }

    /// Wins in a class of car still needed before a visual part can be fitted (0 = free).
    /// `part` as the career table names it (FRONTBUMPER, SPOILER, WIDEBODY ...).
    pub fn part_wins_needed(&self, class: &str, part: &str) -> (i64, i64) {
        let need = self.part_unlocks.get(&class.to_ascii_uppercase()).and_then(|m| m.get(part)).copied().unwrap_or(0);
        let have = class_stat(class).and_then(|s| self.stats.get(s)).copied().unwrap_or(0);
        (need, have)
    }

    /// A statistic by the game's id and sub id: the port's own count where it keeps one,
    /// else the figure carried over from an original save.
    pub fn stat(&self, code: &str, sub: &str) -> Option<f64> {
        if sub == "0" {
            if let Some(v) = self.stats.get(code) {
                return Some(*v as f64);
            }
        }
        self.ostats.get(&format!("{code}/{sub}")).copied()
    }

    /// Level (0..2) of a special ability from how often it has been used.
    pub fn ability_level(&self, system: &str, stat: &str) -> u8 {
        let uses = self.ability_amounts.get(system).copied().unwrap_or_else(|| self.stats.get(stat).copied().unwrap_or(0));
        self.ability_levels.get(system).map_or(0, |s| s.iter().filter(|(need, _)| uses >= *need).map(|(_, l)| *l).max().unwrap_or(0))
    }

    /// Count a use of an ability; returns the new level if this use reached one.
    pub fn ability_used(&mut self, system: &str, stat: &str) -> Option<u8> {
        let before = self.ability_level(system, stat);
        *self.stats.entry(stat.to_string()).or_insert(0) += 1;
        // (the ability's system does not count uses: it gets one point for every race
        // finished with the ability fitted - F-48818 / F-51495; `mc4_race_experience`)
        let after = self.ability_level(system, stat);
        if after > before { Some(after) } else { None }
    }

    /// The event (races, rival) behind a mission, if its script names one.
    /// The races of an event by its key (`hw_local_hangout1_series` ...), as its script
    /// names them (lower case, "district/type/name").
    pub fn event_races(&self, key: &str) -> Vec<String> {
        self.events.get(&key.to_ascii_uppercase()).map(|e| e.races.clone()).unwrap_or_default()
    }

    fn event_of(&self, m: &Mission) -> Option<&EventDef> {
        if let Some(e) = self.events.get(&m.id) {
            return Some(e);
        }
        m.objectives.iter().flat_map(|r| r.conds.iter()).find_map(|c| match c {
            Cond::Event { ident, .. } => self.events.get(&ident.to_ascii_uppercase()),
            _ => None,
        })
    }

    pub fn loaded(&self) -> bool {
        !self.missions.is_empty()
    }

    fn text(&self, key: &str) -> String {
        self.strings.get(key).map(|s| s.to_string()).unwrap_or_else(|| key.replace('_', " "))
    }

    fn title(&self, i: usize) -> String {
        self.text(&format!("{}_TITLE", self.missions[i].id))
    }

    /// The mission's szHookmanNameId (mcMission_GetHookmanNameId), "" for none.
    pub fn hookman_name_id(&self, id: &str) -> &str {
        self.mission_no(id).map_or("", |i| self.missions[i].hookman.as_str())
    }

    fn who(&self, i: usize) -> String {
        let h = &self.missions[i].hookman;
        if h.is_empty() || h.eq_ignore_ascii_case("None") {
            String::new()
        } else {
            self.text(h)
        }
    }

    /// A fresh career: the first mission is the race against Henry on the strip.
    pub fn new_game(&mut self) {
        self.state = vec![MState::default(); self.missions.len()];
        self.order.clear();
        self.stats.clear();
        self.ostats.clear();
        self.news.clear();
        self.unlocking = None;
        self.done.clear();
        self.journal.clear();
        self.journal_complete = true;
        self.ext_init();
        self.upd.clear();
        self.pending.clear();
        self.done_wait.clear();
        if let Some(&i) = self.index.get("RACE_GH_06") {
            self.activate(i);
        }
    }

    pub fn save(&self) -> SaveState {
        SaveState {
            missions: self.missions.iter().zip(self.state.iter()).filter(|(_, s)| s.stage != Stage::Locked).map(|(m, s)| (m.id.clone(), s.clone())).collect(),
            stats: self.stats.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            order: self.order.iter().map(|&i| self.missions[i].id.clone()).collect(),
            ostats: self.ostats.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        }
    }

    pub fn restore(&mut self, s: &SaveState) {
        self.state = vec![MState::default(); self.missions.len()];
        for (id, st) in &s.missions {
            if let Some(&i) = self.index.get(id) {
                self.state[i] = st.clone();
                self.state[i].progress.resize(self.missions[i].objectives.len(), 0);
            }
        }
        self.stats = s.stats.iter().map(|(k, v)| (k.clone(), *v)).collect();
        self.ostats = s.ostats.iter().map(|(k, v)| (k.clone(), *v)).collect();
        self.order = s.order.iter().filter_map(|id| self.index.get(id).copied()).filter(|&i| self.state[i].stage == Stage::Active).collect();
        for i in 0..self.missions.len() {
            if self.state[i].stage == Stage::Active && !self.order.contains(&i) {
                self.order.push(i);
            }
        }
        self.journal.clear();
        self.journal_complete = false;
        self.ext_init();
        self.upd.clear();
        self.pending.clear();
        self.done_wait.clear();
        self.set_done_order(&[]);
        if !self.state.iter().any(|s| s.stage != Stage::Locked) {
            self.new_game();
        }
        self.news.clear();
        self.mute_tips = if !s.ostats.is_empty() && !s.stats.keys().any(|k| k.starts_with("tip:")) { 2 } else { 0 };
    }

    fn activate(&mut self, i: usize) {
        if matches!(self.state[i].stage, Stage::Active | Stage::Complete) {
            return;
        }
        self.state[i] = MState { stage: Stage::Active, progress: vec![0; self.missions[i].objectives.len()], won: Vec::new(), pts: Default::default() };
        // (READY_TO_START waits for the mission's call unless bAutoStart passes it on; the
        // log takes the mission at CHECKING_OBJECTIVES: F-99322, F-58560 - step 20b)
        self.freeze_prereqs(i);
        if self.waits_for_call(i) {
            self.on_ready(i);
            // (the READY_TO_START_ON_ENTER script's call - Sidekick_AppendData: step 21b. A
            // mission that passes READY_TO_START by - bAutoStart, or a reward with
            // CHECKING_OBJECTIVES - has no call: its script never runs)
            if !self.missions[i].hidden {
                self.news.push(News::Started { id: self.missions[i].id.clone(), title: self.title(i), who: self.who(i), text: self.text(&format!("{}_DESC", self.missions[i].id)) });
            }
        } else if self.waits_for_event(i) {
            // (REDLIGHT_GH_01: READY_TO_START until its event - the Hollywood garage entered)
            self.on_ready(i);
        } else {
            self.enter_checking(i);
        }
        log::info!("career: mission {} started", self.missions[i].id);
    }

    /// AddReward F-64223: a reward already pending is ignored; bGiveImmediately gives it now,
    /// else it waits in the list for the REWARDS page (`give_pending`).
    fn add_reward(&mut self, i: usize, k: usize, m: &Mission) {
        if reward_add(&mut self.pending, Pend::Mission(i, k), m.reward_flags.get(k).map_or(true, |f| f.1)) {
            self.give(&m.rewards[k]);
        }
    }

    /// The REWARDS getter F-22948: a pending reward with bHidden clear makes the page show.
    pub fn pending_shown(&self) -> bool {
        reward_shown(&self.pending, |p| self.pend_hidden(p))
    }

    /// An entry's bHidden (+8).
    pub fn pend_hidden(&self, p: &Pend) -> bool {
        match *p {
            Pend::Mission(i, k) => self.missions[i].reward_flags[k].0,
            // (the race's money and Progression experience - built clear by F-69550 / F-78705 - and the PartUnlock rewards
            // are listed; the Unlocking experience is Hidden (F-48818 sets +340); the thresholds' Mission / Script rewards
            // are all Hidden 1)
            Pend::Money | Pend::Exp(false) | Pend::Part(..) => false,
            Pend::Exp(true) | Pend::Threshold(_) => true,
        }
    }

    /// The race listener F-48818: the money (above 1) and the experience (above 1) to the list - money, then Progression,
    /// then Unlocking (`exp`: (Progression, Unlocking), 0 = not given). The objects are the manager's own: queued again
    /// before the page, only the amount changes.
    pub fn queue_race(&mut self, money: i64, exp: (i64, i64)) {
        if money > 1 {
            self.pend_money = money;
            reward_add(&mut self.pending, Pend::Money, false);
        }
        for (k, e) in [exp.0, exp.1].into_iter().enumerate() {
            if e > 1 {
                self.pend_exp[k] = e;
                reward_add(&mut self.pending, Pend::Exp(k == 1), false);
            }
        }
    }

    /// AddReward of a PartUnlock reward of a class threshold (F-61974 from the class point of F-48818): listed, deferred.
    pub fn queue_part(&mut self, class: &str, part: &str) {
        reward_add(&mut self.pending, Pend::Part(class.to_string(), part.to_string()), false);
    }

    /// The give of a mission's reward or of a threshold's (the others are the game's: `Game::give_pending`).
    pub fn give_entry(&mut self, p: &Pend) {
        let r = match *p {
            Pend::Mission(i, k) => self.missions[i].rewards[k].clone(),
            Pend::Threshold(k) => self.thresholds[k].1.clone(),
            _ => return,
        };
        self.give(&r);
    }

    /// After a give-all: what the rewards started settles (`give_pending`).
    pub fn after_give(&mut self) {
        // (a mission handed to CHECKING_PREREQUISITES steps on at once when they are met, as in `event`)
        self.activate_met();
        self.flush_done_wait();
        self.scan_updates();
    }

    /// The Unlocking experience given (F-55689 on the system named at the reward's +60): its thresholds walked.
    pub fn unlocking_given(&mut self) {
        self.walk(true);
        self.activate_met();
        self.scan_updates();
    }

    /// One system's walk F-61830 -> F-61974: its thresholds due, in file order, each marked reached and each of its
    /// rewards handed to AddReward F-64223 - GiveImmediately given at once, the others appended to the pending list
    /// (given after the entries already there by the same give-all F-73125).
    fn walk(&mut self, unlocking: bool) {
        let have = if unlocking { self.unlocking.unwrap_or(self.rep) } else { self.rep };
        let unl = |c: &Career, k: usize| c.threshold_ids.get(k).map_or(false, |id| id.starts_with("Unlocking_"));
        let due: Vec<usize> = (0..self.thresholds.len()).filter(|&k| unl(self, k) == unlocking && self.thresholds[k].0 as i64 <= have && !self.threshold_ids.get(k).map_or(false, |id| !id.is_empty() && self.reached_ids.contains(id))).collect();
        for &k in &due {
            if let Some(id) = self.threshold_ids.get(k).filter(|id| !id.is_empty()).cloned() {
                self.reached_ids.insert(id);
            }
        }
        let mut later: Vec<usize> = Vec::new();
        for k in due {
            let gi = self.threshold_gi.get(k).copied().unwrap_or(true);
            let added = if self.in_give { reward_add(&mut self.pending, Pend::Threshold(k), gi) } else { reward_add(&mut later, k, gi) };
            if added {
                let r = self.thresholds[k].1.clone();
                self.give(&r);
            }
        }
        // (outside a give-all - the port's Unlocking following the reputation, `unlocking` unset - the deferred ones are
        // given after the walk: the original walks Unlocking only from its experience reward, inside a give-all)
        for k in later {
            let r = self.thresholds[k].1.clone();
            self.give(&r);
        }
    }

    /// The completions held behind their pending rewards go to the journal (the rewards given,
    /// or no REWARDS page this time).
    pub fn flush_done_wait(&mut self) {
        for i in std::mem::take(&mut self.done_wait) {
            self.journal.push((false, i));
        }
    }

    /// The REWARDS page's hide empties the list (F-72397 -> F-77851): what was added after its
    /// init is dropped ungiven.
    pub fn pending_hidden(&mut self) {
        self.pending.clear();
    }

    /// A Progression threshold with the script reward `script` has been reached: its id is
    /// among the reached ones (a table without ids: the reputation is at its amount).
    pub fn script_reached(&self, script: &str) -> bool {
        (0..self.thresholds.len()).any(|k| {
            matches!(&self.thresholds[k].1, Reward::Script(n) if n.eq_ignore_ascii_case(script))
                && match self.threshold_ids.get(k).filter(|id| !id.is_empty()) {
                    Some(id) => self.reached_ids.contains(id),
                    None => self.thresholds[k].0 as i64 <= self.rep,
                }
        })
    }

    fn give(&mut self, r: &Reward) {
        match r {
            Reward::Script(_) => {}
            Reward::Mission { id, state } => {
                let Some(&i) = self.index.get(id) else { return };
                // (F-97457: a mission in states 1 .. 4 is left as it is; a COMPLETE one only
                // when bRepeatable (+693) - the REPEAT_* missions hand each other on)
                match self.state[i].stage {
                    Stage::Locked => {}
                    Stage::Complete if self.missions[i].repeatable => self.leave_complete(i),
                    _ => return,
                }
                if state == "CHECKINGPREREQUISITES" {
                    self.state[i].stage = Stage::Armed;
                    self.on_armed(i);
                } else {
                    if state == "READYTOSTART" {
                        self.ready_now.push(id.clone());
                    }
                    self.activate(i);
                    if state == "CHECKINGOBJECTIVES" {
                        self.reward_checking(i);
                    }
                }
            }
            Reward::Tip(key) => {
                let seen = format!("tip:{}", key);
                if self.mute_tips > 0 {
                    self.stats.insert(seen, 1);
                    return;
                }
                if self.stats.get(&seen).copied().unwrap_or(0) == 0 {
                    self.stats.insert(seen, 1);
                    if let Some(text) = self.strings.get(key).map(|s| s.to_string()) {
                        let title = self.strings.get("SIDEKICK_TITLE_TIP").map(|s| s.to_string()).unwrap_or_else(|| "TIP".into());
                        self.news.push(News::Tip { key: key.clone(), title, text });
                    } else {
                        log::debug!("career: tip {} has no text", key);
                    }
                }
            }
            Reward::Call(script) => self.news.push(News::Call(script.clone())),
            Reward::Car(name) => {
                let car = name.find("vpd_").or_else(|| name.find("vp_")).map(|i| name[i..].to_string()).unwrap_or_else(|| name.clone());
                if car != *name {
                    self.car_configs.insert(car.to_ascii_lowercase(), name.clone());
                }
                self.cars_now.push(name.clone());
                self.news.push(News::Car(car));
            }
        }
    }

    fn prereqs_met(&self, i: usize) -> bool {
        // (a Stat / Experience prerequisite counts from its baseline: step 20)
        (0..self.missions[i].prereq.len()).all(|k| self.prereq_req_met(i, k))
    }

    /// How many of something a condition needs (1 for plain events).
    fn target(&self, m: &Mission, c: &Cond) -> u32 {
        // an event with its own race list: a series is decided by the majority of its races
        if let (Cond::Event { event, .. }, Some(e)) = (c, self.event_of(m)) {
            // (a tournament's objective is its one TOURNAMENT_COMPLETE event - 0 / 1, no count
            // per race: the points live in TournamentCore's state - see `tournament_race`)
            if event.starts_with("TOURNAMENT_COMPLETE") && !e.races.is_empty() {
                return 1;
            }
            if event == "SERIES_COMPLETE" && !e.races.is_empty() {
                return (e.races.len() as u32 + 1) / 2;
            }
        }
        match c {
            Cond::TallyRace { tally, .. } => *tally,
            Cond::Stat { value, .. } => (*value).clamp(1, 1_000_000_000) as u32,
            Cond::Experience { amount } => (*amount).max(1) as u32,
            Cond::Event { event, .. } => match event.as_str() {
                // a series is best of three, a tournament four events
                "SERIES_COMPLETE" => 3,
                "TOURNAMENT_COMPLETE" | "TOURNAMENT_COMPLETE_HARD" => 4,
                _ => 1,
            },
            _ => {
                let _ = m;
                1
            }
        }
    }

    /// Feed a gameplay event through every active mission.
    pub fn event(&mut self, ev: Ev) {
        if self.missions.is_empty() {
            return;
        }
        if matches!(ev, Ev::Rep(_)) && self.mute_tips > 0 {
            self.mute_tips -= 1;
        }
        match ev {
            Ev::Race { place, class, district, .. } => {
                if place == 1 {
                    *self.stats.entry("wins".into()).or_default() += 1;
                    if let Some(s) = class_stat(class) {
                        *self.stats.entry(s.into()).or_default() += 1;
                    }
                    if district.eq_ignore_ascii_case("Freeway") {
                        *self.stats.entry("kFWN".into()).or_default() += 1;
                        // (the freeway hookmen's scripts count him beaten after kFWN: kHOK)
                        *self.stats.entry("kHOK".into()).or_default() += 1;
                    }
                }
            }
            Ev::Delivery => *self.stats.entry("xCDL".into()).or_default() += 1,
            Ev::Money(m) => *self.stats.entry("k$WA".into()).or_default() += m.max(0),
            Ev::Rep(r) => {
                self.rep = r;
                self.resolve_pending();
            }
            _ => {}
        }
        // (the conditions' updates - vt#9 of mcConditionString F-77134, Mission F-88673, Stat F-16469, Experience
        // F-79261, TallyMission F-12302 - do nothing while the cheat gate F-68910 is set: no mission moves on)
        let active: Vec<usize> = if crate::rewards::cheat_gate_on() { Vec::new() } else { self.order.clone() };
        for i in active {
            if self.state[i].stage != Stage::Active {
                continue;
            }
            let m = self.missions[i].clone();
            // (a story mission's series runs once its hookman is met: step 20b)
            if matches!(ev, Ev::Race { .. }) && self.story_waits(i) {
                continue;
            }
            // missions with an event of their own only count that event's races, each once
            let ev_races: Vec<String> = self.event_of(&m).map(|e| e.races.clone()).unwrap_or_default();
            let mut ev = ev;
            let mut event_win: Option<String> = None;
            if let Ev::Race { race, place, beat_goal, district, hookman: _, class } = ev {
                if !ev_races.is_empty() {
                    let hit = ev_races.iter().find(|r| race.to_ascii_lowercase().ends_with(r.as_str())).cloned();
                    // (a time trial counts when its target time is beaten, not for being
                    // first of one: the hookman posts the mission's event for "complete" only -
                    // generic_timetrial_hookman.sco 0x20a7)
                    let ok = if m.id.starts_with("TIMETRIAL") { beat_goal } else { place == 1 || m.id.starts_with("RACE_") };
                    match hit {
                        Some(r) if ok && !self.state[i].won.contains(&r) => {
                            event_win = Some(r);
                            // it is this mission's race however it was started
                            ev = Ev::Race { race, place: 1, beat_goal, district, hookman: true, class };
                        }
                        _ => {
                            // not one of ours (or lost, or won before): no progress here
                            if !matches!(m.objectives.first().and_then(|r| r.conds.first()), Some(Cond::TallyRace { .. }) | Some(Cond::Stat { .. }) | Some(Cond::Experience { .. })) {
                                continue;
                            }
                        }
                    }
                }
            }
            if let Some(r) = event_win {
                self.state[i].won.push(r);
            }
            for (oi, req) in m.objectives.iter().enumerate() {
                let Some(c) = req.conds.first() else { continue };
                let target = self.target(&m, c);
                let cur = self.state[i].progress[oi];
                if cur >= target {
                    continue;
                }
                let new = match (c, ev) {
                    // (mcConditionString, F-77134: the posted event with its identifier, nothing else - the
                    // story cores, hangouts, tournaments, the police and the garage post the original's
                    // strings: `CORE_POSTS`, `MEET_POSTS`, `SERIES_NAMES`, `tournament_race`)
                    (Cond::Event { event, ident }, Ev::Str(e, id)) if event == e && ident == id => target,
                    (Cond::TallyRace { district, min_place, .. }, Ev::Race { place, district: d, .. }) if (*min_place == 0 || place as u32 <= *min_place) && same_district(district, d) => cur + 1,
                    // (from the baseline armed with the mission: step 20)
                    (Cond::Stat { stat, .. }, _) => self.ext_stat_progress(i, oi, stat, target),
                    (Cond::Experience { .. }, _) => self.ext_exp_progress(i, oi, target),
                    (Cond::Mission(id), _) => self.index.get(id).map_or(0, |&j| (self.state[j].stage == Stage::Complete) as u32),
                    _ => cur,
                };
                self.state[i].progress[oi] = new.min(target);
            }
            self.freeze_met(i);
            let done = m.objectives.iter().enumerate().all(|(oi, req)| req.conds.first().map_or(true, |c| self.state[i].progress[oi] >= self.target(&m, c)));
            if done && !m.objectives.is_empty() {
                self.state[i].stage = Stage::Complete;
                self.order.retain(|&x| x != i);
                self.mark_done(i);
                self.post_percent();
                if !m.hidden {
                    self.news.push(News::Completed { id: m.id.clone(), title: self.title(i) });
                }
                // (the mission's CORE script plays the next mission's story scene: step 21b)
                for (after, scene) in SCENE_AFTER {
                    if m.id == after {
                        self.news.push(News::Scene(scene.to_string()));
                    }
                }
                log::info!("career: mission {} complete", m.id);
                for k in 0..m.rewards.len() {
                    self.add_reward(i, k, &m);
                }
                self.mission_complete_event();
                // what the event itself pays: prize cars from the reward tables, the rival's
                // car for pink slips, prize money (amounts are ours: the original computes
                // them in script)
                let mut kinds: Vec<&str> = Vec::new();
                for req in &m.objectives {
                    for c in &req.conds {
                        if let Cond::Event { event, ident } = c {
                            kinds.push(event.as_str());
                            // (the event's prize car and money are the event's own rewards:
                            // given when the event completes - F-67065, `mc4_event_complete`)
                            let _ = ident;
                        }
                    }
                }
                // (a pink slip mission's car is the race's reward - wager_hangout_core
                // BuildRewardCarB, `mc4_pink_end` - not the mission's: step 21)
                let _ = kinds;
            }
        }
        // reputation hands out missions
        if let Ev::Rep(_) = ev {
            // (a threshold of the tables whose rewards were handed out is not handed out again)
            // (the Unlocking system's thresholds go by its own amount)
            // (the race's Progression experience is given before its Unlocking one - F-48818 adds them to the list in that
            // order: `Game::give_pending`; here, outside a give-all, the two walks follow each other)
            self.walk(false);
            self.walk(true);
        }
        self.activate_met();
        // (step 20: completions after what they handed out; Experience baselines take the
        // reputation in hand - not after a race or its money, whose reputation follows)
        for i in std::mem::take(&mut self.done_now) {
            // (a completion whose rewards wait in the pending list follows them: `give_pending`)
            if self.pending.iter().any(|p| matches!(*p, Pend::Mission(j, _) if j == i)) {
                self.done_wait.push(i);
            } else {
                self.journal.push((false, i));
            }
        }
        if !matches!(ev, Ev::Race { .. } | Ev::Money(_)) {
            self.resolve_pending();
        }
        self.scan_updates();
    }

    /// The races of the missions already done (race path lower case): they stay on the map
    /// to be driven again (the 100 % save's map is full of them, reference `z00`).
    pub fn done_races(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (i, m) in self.missions.iter().enumerate() {
            if self.state[i].stage == Stage::Complete {
                if let Some(e) = self.event_of(m) {
                    out.extend(e.races.iter().cloned());
                }
            }
        }
        out
    }

    /// Every race of a mission that is open or done, with the mission's id: what the GPS
    /// map marks, and by which kind of event.
    pub fn map_events(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (i, m) in self.missions.iter().enumerate() {
            if matches!(self.state[i].stage, Stage::Active | Stage::Complete) {
                if let Some(e) = self.event_of(m) {
                    out.extend(e.races.iter().map(|r| (r.clone(), m.id.clone())));
                }
            }
        }
        out
    }

    /// Races the active missions are waiting for: (race path lower case, mission title).
    pub fn mission_races(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for &i in &self.order {
            let m = &self.missions[i];
            if let Some(e) = self.event_of(m) {
                for r in &e.races {
                    if !self.state[i].won.contains(r) {
                        out.push((r.clone(), self.title(i)));
                    }
                }
            }
        }
        out
    }

    /// Is this race (definition id) the time trial of an active mission? Those are driven
    /// alone against the event's target time.
    pub fn is_time_trial(&self, race_id: &str) -> bool {
        let l = race_id.to_ascii_lowercase();
        self.order.iter().any(|&i| self.missions[i].id.starts_with("TIMETRIAL") && self.event_of(&self.missions[i]).map_or(false, |e| e.races.iter().any(|r| l.ends_with(r.as_str()))))
    }

    /// The scripted race of the oldest hookman mission (the one a challenge leads to).
    pub fn hookman_race(&self) -> Option<String> {
        let i = *self.order.iter().find(|&&i| {
            let id = &self.missions[i].id;
            id.starts_with("RACE_") || id.starts_with("BEAT_")
        })?;
        let e = self.event_of(&self.missions[i])?;
        e.races.iter().find(|r| !self.state[i].won.contains(*r)).cloned()
    }

    /// Titled, visible missions that nothing starts: not a reward of the tables or of another mission, no Mission
    /// prerequisite (RACE_GH_06 is the career start-up script's own: `game/career/0x400221ca` 0xba5..0xbc2).
    pub fn unstarted(&self) -> Vec<String> {
        (0..self.missions.len())
            .filter(|&i| {
                let m = &self.missions[i];
                !self.rewarded.contains(&i) && m.id != "RACE_GH_06" && !m.hidden && !m.prereq.iter().any(|r| r.conds.iter().any(|x| matches!(x, Cond::Mission(_)))) && self.strings.get(&format!("{}_TITLE", m.id)).is_some()
            })
            .map(|i| self.missions[i].id.clone())
            .collect()
    }

    /// South Central's set-up script (`sc/Career/CareerSetup_SC`, started at every career start
    /// with the add-on: F-64159 A-25462) six seconds after the player's car is there
    /// (0x6d): a mission still at CHECKING_PREREQUISITES whose missions are complete goes to
    /// READY_TO_START (0xf4 .. 0x303). The missions started.
    pub fn sc_setup(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        for (id, needs) in SC_SETUP_MISSIONS {
            let Some(&i) = self.index.get(id) else { continue };
            let done = needs.iter().all(|n| self.index.get(*n).map_or(false, |&j| self.state[j].stage == Stage::Complete));
            if done && self.state[i].stage == Stage::Armed {
                self.activate(i);
                out.push(id.to_string());
            }
        }
        out
    }

    /// Id of the mission that is waiting for a race against a roaming racer.
    /// A race of an active tournament was run, `order` its finishing order ("YOU" the player).
    /// Every race of a tournament is run once and pays points by place - 7, 4, 2, 1, the
    /// table the game's career manager is built with (A-96970) and its
    /// calculateResultsTournament adds up; after the last race the racer with the most points
    /// has won. A lost tournament starts over. Returns a line for the results screen.
    pub fn tournament_race(&mut self, race_id: &str, order: &[String]) -> Option<String> {
        const POINTS: [i32; 4] = [7, 4, 2, 1];
        let l = race_id.to_ascii_lowercase();
        for k in 0..self.order.len() {
            let i = self.order[k];
            let Some(oi) = self.missions[i].objectives.iter().position(|r| matches!(r.conds.first(), Some(Cond::Event { event, .. }) if event.starts_with("TOURNAMENT_COMPLETE"))) else { continue };
            let races = match self.event_of(&self.missions[i]) {
                Some(e) if !e.races.is_empty() => e.races.clone(),
                _ => continue,
            };
            let Some(r) = races.iter().find(|r| l.ends_with(r.as_str())).cloned() else { continue };
            let st = &mut self.state[i];
            if st.won.contains(&r) {
                continue;
            }
            st.won.push(r);
            // (F-31368 on the finishing order; a damaged-out player is already last in it, main.rs)
            let mut table = [0i32; 17];
            table[..POINTS.len()].copy_from_slice(&POINTS);
            let mut rec: Vec<(i32, i32)> = order.iter().enumerate().map(|(p, who)| (if who == "YOU" { 0 } else { 1 }, p as i32 + 1)).collect();
            let mut add: Vec<i32> = vec![0; order.len()];
            tournament_points(&mut rec, &vec![false; order.len()], false, &table, &mut add);
            for (who, a) in order.iter().zip(&add) {
                *st.pts.entry(who.clone()).or_default() += *a;
            }
            let rows = order.iter().enumerate().map(|(p, who)| (who.clone(), POINTS.get(p).copied().unwrap_or(0), st.pts.get(who).copied().unwrap_or(0))).collect();
            let last = st.won.len() >= races.len();
            self.tour_table = Some(TourTable { rows, decided: None, last });
            let st = &mut self.state[i];
            let mine = st.pts.get("YOU").copied().unwrap_or(0);
            let rival = st.pts.iter().filter(|(k, _)| k.as_str() != "YOU").map(|(_, v)| *v).max().unwrap_or(0);
            // (the decision F-66541 on the points per racer slot, the player's first)
            let pts: Vec<i32> = std::iter::once(mine).chain(st.pts.iter().filter(|(k, _)| k.as_str() != "YOU").map(|(_, v)| *v)).collect();
            let decision = tournament_decision(st.won.len() as i32 - 1, races.len() as i32, &pts, 0);
            let set = |st: &mut MState, v: u32| {
                if let Some(p) = st.progress.get_mut(oi) {
                    *p = v;
                }
            };
            if decision == TourDecision::Open {
                return Some(format!("{} POINTS   BEST RIVAL {}", mine, rival));
            }
            if matches!(decision, TourDecision::Complete { .. }) {
                st.pts.clear();
                // (TOURNAMENT_COMPLETE with the event's name, and TOURNAMENT_COMPLETE_HARD as well
                // when the tournament was entered on Hard: F-66541 A-25857..A-17203)
                let (ident, hard) = match self.missions[i].objectives[oi].conds.first() {
                    Some(Cond::Event { event, ident }) => (ident.clone(), event.ends_with("_HARD")),
                    _ => (String::new(), false),
                };
                let diff = self.stats.get(&format!("tour_diff:{}", ident.to_ascii_lowercase())).copied().map_or(if hard { 2 } else { 1 }, |d| d.clamp(0, 3) as usize);
                self.completed_events.push((ident.clone(), diff));
                self.tour_decided = Some(true);
                if let Some(t) = self.tour_table.as_mut() {
                    t.decided = Some(true);
                }
                self.event(Ev::Str("TOURNAMENT_COMPLETE", &ident));
                if diff == 2 {
                    self.event(Ev::Str("TOURNAMENT_COMPLETE_HARD", &ident));
                }
                // (won, but not the tournament this mission waits for: it is run again)
                if self.state[i].stage != Stage::Complete {
                    self.state[i].won.clear();
                }
                return Some(format!("TOURNAMENT WON   {} POINTS TO {}", mine, rival));
            }
            st.won.clear();
            st.pts.clear();
            set(st, 0);
            self.tour_decided = Some(false);
            if let Some(t) = self.tour_table.as_mut() {
                t.decided = Some(false);
            }
            return Some(format!("TOURNAMENT LOST   {} POINTS TO {}", mine, rival));
        }
        None
    }

    /// RESTART TOURNAMENT (TournamentCore `restart`, 0x571): the tournament race `race_id` is
    /// part of starts again - no race run, every point at nought; its first race.
    pub fn tournament_restart(&mut self, race_id: &str) -> Option<String> {
        let l = race_id.to_ascii_lowercase();
        for k in 0..self.order.len() {
            let i = self.order[k];
            let Some(oi) = self.missions[i].objectives.iter().position(|r| matches!(r.conds.first(), Some(Cond::Event { event, .. }) if event.starts_with("TOURNAMENT_COMPLETE"))) else { continue };
            let Some(races) = self.event_of(&self.missions[i]).map(|e| e.races.clone()) else { continue };
            if !races.iter().any(|r| l.ends_with(r.as_str())) {
                continue;
            }
            let st = &mut self.state[i];
            st.won.clear();
            st.pts.clear();
            if let Some(p) = st.progress.get_mut(oi) {
                *p = 0;
            }
            return races.first().cloned();
        }
        None
    }

    /// A race of an active series was won by `winner` ("YOU" the player). The game counts
    /// every racer's wins (`nRacerWins_<n>`, calculateResultsSeries) and the first to three
    /// takes the series - so a rival can: then it starts over. Returns a line for the
    /// results screen.
    pub fn series_race(&mut self, race_id: &str, winner: &str) -> Option<String> {
        let l = race_id.to_ascii_lowercase();
        for k in 0..self.order.len() {
            let i = self.order[k];
            let Some(oi) = self.missions[i].objectives.iter().position(|r| matches!(r.conds.first(), Some(Cond::Event { event, .. }) if event == "SERIES_COMPLETE")) else { continue };
            if self.story_waits(i) {
                continue;
            }
            let races = match self.event_of(&self.missions[i]) {
                Some(e) if !e.races.is_empty() => e.races.clone(),
                _ => continue,
            };
            if !races.iter().any(|r| l.ends_with(r.as_str())) {
                continue;
            }
            // (nRacesToWin: the event's +3252, set by each story script - the majority of its races)
            let m = self.missions[i].clone();
            let need = m.objectives[oi].conds.first().map_or(1, |c| self.target(&m, c)) as i32;
            let st = &mut self.state[i];
            let n = {
                let e = st.pts.entry(winner.to_string()).or_default();
                *e += 1;
                *e
            };
            if winner == "YOU" {
                return Some(format!("{} OF {} WINS", n.min(need), need));
            }
            if n >= need {
                st.won.clear();
                st.pts.clear();
                if let Some(p) = st.progress.get_mut(oi) {
                    *p = 0;
                }
                self.story_reset(i);
                return Some("SERIES LOST".to_string());
            }
            return Some(format!("RIVAL HAS {} OF {} WINS", n, need));
        }
        None
    }

    /// The series a story man runs when he is flashed (`<ID>_CORE` starts OrderedSeriesCore):
    /// its races in the script's order and nRacesToWin. None while the mission is not open, for
    /// another kind of mission, or before its hookman is met.
    pub fn series_mission(&self, id: &str) -> Option<(Vec<String>, u32)> {
        let &i = self.index.get(id)?;
        if self.state[i].stage != Stage::Active || self.story_waits(i) {
            return None;
        }
        let m = &self.missions[i];
        let c = m.objectives.iter().filter_map(|r| r.conds.first()).find(|c| matches!(c, Cond::Event { event, .. } if event == "SERIES_COMPLETE"))?;
        let races = self.event_of(m).map(|e| e.races.clone()).filter(|r| !r.is_empty())?;
        Some((races, self.target(m, c)))
    }

    /// A story series given up (QUIT SERIES, RESTART SERIES, RACE SERIES AGAIN): its wins are the
    /// script's (`nRacerWins_<i>`) and go with it. Nothing for a mission it completed.
    pub fn series_drop(&mut self, id: &str) {
        let Some(&i) = self.index.get(id) else { return };
        if self.state[i].stage != Stage::Active {
            return;
        }
        let Some(oi) = self.missions[i].objectives.iter().position(|r| matches!(r.conds.first(), Some(Cond::Event { event, .. }) if event == "SERIES_COMPLETE")) else { return };
        let st = &mut self.state[i];
        st.won.clear();
        st.pts.clear();
        if let Some(p) = st.progress.get_mut(oi) {
            *p = 0;
        }
    }

    /// The active missions whose hookman only races a certain kind of car: (mission id, the
    /// game's sentence for it - "You must be driving a MUSCLE vehicle to challenge MARCEL.").
    pub fn restrictions(&self) -> Vec<(String, String)> {
        self.order
            .iter()
            .map(|&i| self.missions[i].id.clone())
            .filter_map(|id| self.strings.get(&format!("{}_RESTRICTION", id)).map(|t| (id.to_ascii_uppercase(), t.to_string())))
            .collect()
    }

    /// The races of one mission's event not yet won (`mission_races` for one mission)
    pub fn races_of(&self, id: &str) -> Vec<String> {
        let Some(&i) = self.index.get(id) else { return Vec::new() };
        self.event_of(&self.missions[i]).map_or(Vec::new(), |e| e.races.iter().filter(|r| !self.state[i].won.contains(*r)).cloned().collect())
    }

    pub fn hookman_mission(&self) -> Option<&str> {
        self.order.iter().map(|&i| self.missions[i].id.as_str()).find(|id| id.starts_with("RACE_") || id.starts_with("BEAT_") || id.starts_with("REDLIGHT_"))
    }

    /// Character id ("GH_06") of the rival the oldest hookman mission is about.
    pub fn rival_id(&self) -> Option<String> {
        let id = self.rival_mission()?;
        let parts: Vec<&str> = id.split('_').collect();
        let k = parts.iter().position(|p| *p == "GH" || *p == "SH")?;
        let num: String = parts.get(k + 1)?.chars().take_while(|c| c.is_ascii_digit()).collect();
        if num.is_empty() {
            return None;
        }
        Some(format!("{}_{}", parts[k], num))
    }

    /// The mission whose man `rival_id` names: the oldest RACE_ / BEAT_ / REDLIGHT_ / FIND_
    /// mission of the log.
    pub fn rival_mission(&self) -> Option<String> {
        let i = *self.order.iter().find(|&&i| {
            let id = &self.missions[i].id;
            id.starts_with("RACE_") || id.starts_with("BEAT_") || id.starts_with("REDLIGHT_") || id.starts_with("FIND_")
        })?;
        Some(self.missions[i].id.clone())
    }

    /// The objective of mission `id` whose target racer's icon flashes (racer +466, `icon_subtype`
    /// 1 of the 3D icon): the refresh F-92169 takes each mission that is not bAlwaysHidden
    /// (+685) and, through F-96100, in state 2 READY_TO_START or 3 CHECKING_OBJECTIVES, the
    /// first objective that is not satisfied (+116), not always hidden (+117) and not hidden (+118);
    /// its target's vehicle element and racer are flagged (F-39702), every other one cleared
    /// (F-44139).
    pub fn pulse_objective(&self, id: &str) -> Option<usize> {
        let &i = self.index.get(id)?;
        if self.missions[i].hidden {
            return None;
        }
        self.open_objective(i)
    }

    /// F-96100 on mission `i`: in state 2 READY_TO_START or 3 CHECKING_OBJECTIVES, the first
    /// objective that is not satisfied (+116), not always hidden (+117) and not hidden (+118).
    fn open_objective(&self, i: usize) -> Option<usize> {
        let m = &self.missions[i];
        if self.state[i].stage != Stage::Active || !matches!(self.state_id_of(i), 2 | 3) {
            return None;
        }
        let ext = self.ext.get(i).filter(|_| self.ext_ok(i));
        m.objectives.iter().enumerate().position(|(k, r)| {
            let hidden = ext.map_or(r.hidden, |e| e.obj_always[k] || e.obj_hidden[k]);
            let target = r.conds.first().map_or(1, |c| self.target(m, c));
            let met = self.state[i].progress.get(k).copied().unwrap_or(0) >= target;
            !hidden && !met
        })
    }

    /// mcMissionManager_DoesUIDHaveActiveMission (F-75396 -> F-93550): a mission of the
    /// manager's list whose open objective (F-96100; none: 0) has the target id `uid`.
    pub fn uid_has_active_mission(&self, uid: i32) -> bool {
        (0..self.missions.len()).any(|i| self.open_objective(i).map_or(0, |k| self.missions[i].objectives[k].target_id) == uid)
    }

    /// Display name of a character id.
    pub fn name_of(&self, id: &str) -> String {
        self.strings.get(id).map(|s| s.to_string()).unwrap_or_else(|| id.replace('_', " "))
    }

    /// Is a mission waiting for a race against a roaming racer? Its rival's name.
    pub fn hookman_wanted(&self) -> Option<String> {
        self.order.iter().find(|&&i| {
            let id = &self.missions[i].id;
            id.starts_with("RACE_") || id.starts_with("BEAT_") || id.starts_with("REDLIGHT_") || id.starts_with("FIND_")
        }).map(|&i| {
            // "RACE_GH_06" -> GH_06 is the rival; story hookmen are named by the mission
            let id = &self.missions[i].id;
            let rival = id.split('_').collect::<Vec<_>>();
            let key = rival.iter().position(|p| *p == "GH" || *p == "SH").and_then(|k| rival.get(k + 1).map(|n| format!("{}_{}", rival[k], n.trim_end_matches(char::is_alphabetic))));
            key.and_then(|k| self.strings.get(&k).map(|s| s.to_string())).unwrap_or_else(|| self.who(i))
        })
    }

    /// The missions already completed, in the order they were started, for the
    /// mission log's second tab.
    pub fn completed_views(&self) -> Vec<MissionView> {
        self.views(|s| *s == Stage::Complete)
    }

    /// The car of the racer who runs mission `id`'s event, from the event's set-up name
    /// (`GH_20_2_vp_mit_3000gt_99` -> `vp_mit_3000gt_99`).
    pub fn event_rival_car(&self, id: &str) -> Option<String> {
        let m = &self.missions[*self.index.get(id)?];
        let r = &self.event_of(m)?.rival;
        let at = r.to_ascii_lowercase().find("vp")?;
        Some(r[at..].to_ascii_lowercase())
    }

    /// How hard mission `id`'s racer is: the event's set-up names one of the hookman's four
    /// cars, `GH_06_0_...` the easiest to `GH_06_3_...` the hardest (the disc has the four
    /// set-ups of each; a new career's first man comes as `_0_` and is EASIEST green in
    /// the original).
    pub fn event_rival_diff(&self, id: &str) -> Option<usize> {
        let m = &self.missions[*self.index.get(id)?];
        let r = self.event_of(m)?.rival.to_ascii_lowercase();
        let at = r.find("_vp")?;
        r[..at].rsplit('_').next()?.parse::<usize>().ok().filter(|d| *d < 4)
    }

    /// The same for the hookman out on the road now.
    pub fn hookman_diff(&self) -> Option<usize> {
        self.event_rival_diff(self.hookman_mission()?)
    }

    /// The character who runs event `key` ("GH_01" of `GH_01_1_vp_chv_impala_96`).
    pub fn event_rival_char(&self, key: &str) -> Option<String> {
        let r = &self.events.get(&key.to_ascii_uppercase())?.rival;
        let at = r.to_ascii_lowercase().find("_vp")?;
        let head = &r[..at];
        Some(head.rsplit_once('_').map_or(head, |x| x.0).to_string())
    }

    /// Mission `id`'s title.
    pub fn title_of(&self, id: &str) -> Option<String> {
        self.index.get(id).map(|&i| self.title(i))
    }

    /// The lines of mission `id`'s objectives (those it shows), whatever its stage.
    ///
    /// A satisfied objective (requirement +116: its progress at the target, as `pulse_objective`
    /// reads it) starts with `crate::game::OBJ_DONE`: MISSION RESULTS ticks it (F-66221).
    pub fn objective_texts(&self, id: &str) -> Vec<String> {
        let Some(&i) = self.index.get(id) else { return Vec::new() };
        let m = &self.missions[i];
        m.objectives
            .iter()
            .enumerate()
            .filter(|(_, r)| !r.hidden)
            .map(|(k, r)| {
                let met = self.state[i].progress.get(k).copied().unwrap_or(0) >= r.conds.first().map_or(1, |c| self.target(m, c));
                (met, self.text(&r.desc))
            })
            .filter(|(_, t)| !t.is_empty())
            .map(|(met, t)| if met { format!("{}{}", crate::game::OBJ_DONE, t) } else { t })
            .collect()
    }

    /// The active missions, oldest first, for the mission log.
    pub fn active(&self) -> Vec<MissionView> {
        self.views(|s| *s == Stage::Active)
    }

    fn views(&self, want: impl Fn(&Stage) -> bool) -> Vec<MissionView> {
        // (the log's two lists: the open missions in the order they entered it, the completed
        // ones in the order of completion - lists 1 and 2 of the mission manager, F-26006 /
        // F-18577; mcMissionLog F-19994 reads list 1 for "active", list 2 for
        // "completed": F-64028)
        (if want(&Stage::Complete) { &self.done } else { &self.order })
            .iter()
            .filter(|&&i| !self.missions[i].hidden && want(&self.state[i].stage))
            .map(|&i| {
                let m = &self.missions[i];
                MissionView {
                    id: m.id.clone(),
                    title: self.title(i),
                    who: self.who(i),
                    desc: self.text(&format!("{}_DESC", m.id)),
                    next_race: self.event_of(m).and_then(|e| e.races.iter().find(|r| !self.state[i].won.contains(*r))).map(|r| r.rsplit('/').next().unwrap_or(r).replace('_', " ").to_uppercase()),
                    objectives: m
                        .objectives
                        .iter()
                        .enumerate()
                        .filter(|(oi, r)| !self.ext.get(i).filter(|_| self.ext_ok(i)).map_or(r.hidden, |e| e.obj_always[*oi] || e.obj_hidden[*oi]))
                        .map(|(oi, r)| {
                            let (cur, target) = (self.state[i].progress.get(oi).copied().unwrap_or(0), r.conds.first().map_or(1, |c| self.target(m, c)));
                            ObjectiveView { text: self.text(&r.desc), cur, target, count: self.objective_count(i, oi, cur.min(target), target) }
                        })
                        .collect(),
                }
            })
            .collect()
    }

    /// Whether the career has reached mission `id` (it is under way or done).
    /// The open tournament a race belongs to: (the event's name, races of it done, races
    /// in all, the mission asks for TOURNAMENT_COMPLETE_HARD).
    pub fn tournament_of(&self, race_id: &str) -> Option<(String, usize, usize, bool)> {
        let l = race_id.to_ascii_lowercase();
        if l.is_empty() {
            return None;
        }
        for &i in &self.order {
            let Some((ident, hard)) = self.missions[i].objectives.iter().find_map(|r| match r.conds.first() {
                Some(Cond::Event { event, ident }) if event.starts_with("TOURNAMENT_COMPLETE") => Some((ident.clone(), event.ends_with("_HARD"))),
                _ => None,
            }) else { continue };
            let Some(e) = self.event_of(&self.missions[i]) else { continue };
            if e.races.iter().any(|r| l.ends_with(r.as_str())) {
                return Some((ident, self.state[i].won.len(), e.races.len(), hard));
            }
        }
        None
    }

    /// The mission is being done (its objectives are open).
    /// The mission has been started: it is active or complete (its script has run).
    pub fn mission_started(&self, id: &str) -> bool {
        self.index.get(id).or_else(|| self.index.get(&id.to_ascii_lowercase())).map_or(false, |&i| matches!(self.state[i].stage, Stage::Active | Stage::Complete))
    }

    /// The mission is complete.
    pub fn mission_complete(&self, id: &str) -> bool {
        self.index.get(id).map_or(false, |&i| self.state[i].stage == Stage::Complete)
    }

    pub fn mission_active(&self, id: &str) -> bool {
        self.index.get(id).map_or(false, |&i| self.state[i].stage == Stage::Active)
    }

    /// The mission is in the log: active and past READY_TO_START (state 3 or 4). Its own script
    /// `Game/Career/Missions/<ID>` starts then (F-58560 -> F-68362), and with it the
    /// slot request of its man (`missions/race_gh_06.sco` 0x4e).
    pub fn mission_in_log(&self, id: &str) -> bool {
        self.index.get(id).map_or(false, |&i| self.state[i].stage == Stage::Active && self.ext.get(i).map_or(true, |e| e.state != 2))
    }

    /// A car handed over (a reward of type Car).
    pub fn give_car(&mut self, name: &str) {
        self.give(&Reward::Car(name.to_string()));
    }

    /// The race belongs to a pink slip mission that is open.
    pub fn is_pink_slip_race(&self, race_id: &str) -> bool {
        let l = race_id.to_ascii_lowercase();
        !l.is_empty() && self.order.iter().any(|&i| self.missions[i].id.contains("PINKSLIP") && self.event_of(&self.missions[i]).map_or(false, |e| e.races.iter().any(|r| l.ends_with(r.as_str()))))
    }

    pub fn reached(&self, id: &str) -> bool {
        self.index.get(id).map_or(false, |&i| self.state[i].stage != Stage::Locked)
    }

    pub fn completed(&self) -> usize {
        self.state.iter().filter(|s| s.stage == Stage::Complete).count()
    }

    pub fn total(&self) -> usize {
        self.missions.iter().filter(|m| !m.hidden).count()
    }

    /// Diagnostics: play through the graph with synthetic events and log what opens up.
    pub fn selftest(&mut self) {
        self.new_game();
        let districts = ["Hollywood", "Beaches", "Downtown", "Hills", "SouthCentral"];
        let classes = ["TUNER", "MUSCLE", "EXOTIC", "LUXURY", "SPORTBIKE", "SUV", "LowRider"];
        let mut rep = 0i64;
        for step in 0..400 {
            let before = self.completed();
            let d = districts[step % districts.len()];
            // win whatever the missions ask for, plus an ordinary race
            for (r, _) in self.mission_races() {
                let full = format!("game/race/{}", r);
                self.event(Ev::Race { district: d, place: 1, hookman: true, class: classes[step % classes.len()], race: &full, beat_goal: true });
            }
            self.event(Ev::Race { district: d, place: 1, hookman: true, class: classes[step % classes.len()], race: "", beat_goal: false });
            self.event(Ev::Race { district: "Freeway", place: 1, hookman: false, class: "TUNER", race: "", beat_goal: false });
            // (and post every string an open mission waits for)
            let want: Vec<(String, String)> = self.order.iter().flat_map(|&i| self.missions[i].objectives.iter().filter_map(|r| match r.conds.first() {
                Some(Cond::Event { event, ident }) => Some((event.clone(), ident.clone())),
                _ => None,
            })).collect();
            for (e, id) in &want {
                self.event(Ev::Str(e, id));
            }
            self.event(Ev::Delivery);
            self.event(Ev::Money(20_000));
            rep = (rep + 300).min(crate::catalog::MAX_REP);
            self.event(Ev::Rep(rep));
            let news: Vec<String> = std::mem::take(&mut self.news)
                .into_iter()
                .map(|n| match n {
                    News::Started { title, who, .. } => format!("+{} ({})", title, who),
                    News::Completed { title, .. } => format!("-{}", title),
                    News::Car(c) => format!("car {}", c),
                    News::Money(m) => format!("${}", m),
                    News::Tip { key, text, .. } => format!("tip {} [{}]", key, text.chars().take(50).collect::<String>()),
                    News::Scene(id) => format!("scene {}", id),
                    News::Call(s) => format!("call {}", s),
                })
                .collect();
            if !news.is_empty() {
                log::info!("career test step {} rep {}: {}", step, rep, news.join(" | "));
            }
            if step == 30 {
                // a save / load round trip must not change anything
                let (a, done) = (self.order.clone(), self.completed());
                let snap = serde_json::to_string(&self.save()).unwrap_or_default();
                let back: SaveState = serde_json::from_str(&snap).unwrap_or_default();
                self.restore(&back);
                log::info!("career test: save round trip {} ({} bytes)", if a == self.order && done == self.completed() { "ok" } else { "DIFFERS" }, snap.len());
            }
            if self.completed() == before && step > 150 {
                break;
            }
        }
        let stuck: Vec<String> = self.order.iter().map(|&i| self.missions[i].id.clone()).collect();
        let locked: Vec<String> = self.missions.iter().zip(self.state.iter()).filter(|(m, s)| !m.hidden && matches!(s.stage, Stage::Locked | Stage::Armed)).map(|(m, _)| m.id.clone()).collect();
        log::info!("career test: {} of {} complete; still active {:?}; never reached {:?}", self.completed(), self.missions.len(), stuck, locked);
        self.new_game();
    }
}

// ---------------------------------------------------------------------------------------
// step 20b (save work): the mission states READY_TO_START / CHECKING_OBJECTIVES, the story
// hookmen's meeting places, the completion statistic, the frozen statistic values
// ---------------------------------------------------------------------------------------

/// Where the story CORE scripts wait for the player (mcTargetManager_RegisterLocation's point
/// and the Racer_DistanceToPoint radius of their meeting loop): BEAT_SH_02_EXOTIC_CORE 0x1c /
/// 0x823 (30 m), BESTOF_SH_05_CORE 0x42 / 0x85d (30 m), CITY_CHAMP_CORE 0x327 / 0x762 (20 m),
/// RACE_SH_01_CORE 0x149 / 0x45c (30 m). With the police on the player's tail the script shows
/// LOSE_COPS and waits.
pub const STORY_POINTS: [(&str, [f32; 3], f32); 4] = [
    ("BEAT_SH_02_EXOTIC", [-2649.97, -3.81, 33.44], 30.0),
    ("BESTOF_SH_05", [1390.364, -6.993, 284.309], 30.0),
    ("CITY_CHAMP", [1658.652, -1.997, 224.921], 20.0),
    ("RACE_SH_01", [-125.6, 22.0, -735.4], 30.0),
];

impl Career {
    /// The mission waits in READY_TO_START for its READY_TO_START_ON_ENTER script (the
    /// sidekick's call, then mcMission_ChangeState(3)): bAutoStart not set (F-99322 moves
    /// it on by itself otherwise) and the script there. A mission the log never lists (always
    /// hidden) has no call in the port and goes on at once; so does one without the script.
    fn waits_for_call(&self, i: usize) -> bool {
        let m = &self.missions[i];
        !m.auto_start && m.on_enter && !m.hidden
    }

    /// READY_TO_START entered (F-97429): the objectives shown unless always hidden and
    /// armed; the READY_TO_START_ON_ENTER script runs (the port's: `ready_now`, the call).
    fn on_ready(&mut self, i: usize) {
        if let Some(e) = self.ext.get_mut(i) {
            e.state = 2;
        }
        if self.ext_ok(i) {
            for k in 0..self.missions[i].objectives.len() {
                if !self.ext[i].obj_always[k] {
                    self.ext[i].obj_hidden[k] = false;
                }
            }
        }
        let id = self.missions[i].id.clone();
        if !self.ready_now.iter().any(|x| x.eq_ignore_ascii_case(&id)) {
            self.ready_now.push(id);
        }
    }

    /// CHECKING_OBJECTIVES entered: into the log (list 1), objectives armed (F-73549).
    fn enter_checking(&mut self, i: usize) {
        if let Some(e) = self.ext.get_mut(i) {
            e.state = 3;
        }
        if !self.order.contains(&i) {
            self.order.push(i);
        }
        self.on_log(i);
    }

    /// The mission's call has been made: its READY_TO_START_ON_ENTER script changes it to
    /// CHECKING_OBJECTIVES (mcMission_ChangeState(m, 3) after Sidekick_HasDataBeenAnnounced).
    /// TUNER_REP's script changes MUSCLE_REP the same way (0x1ad / 0x1ba).
    pub fn call_delivered(&mut self, id: &str) {
        let Some(i) = self.mission_no(id) else { return };
        if self.state[i].stage != Stage::Active || self.ext.get(i).map_or(true, |e| e.state != 2) {
            return;
        }
        self.enter_checking(i);
        if self.missions[i].id == "TUNER_REP" {
            if let Some(j) = self.mission_no("MUSCLE_REP") {
                self.force_checking(j);
            }
        }
    }

    /// mcMission_ChangeState(m, 3) from a script, whatever state the mission is in.
    fn force_checking(&mut self, j: usize) {
        match self.state[j].stage {
            Stage::Complete => {}
            Stage::Active => {
                if self.ext.get(j).map_or(false, |e| e.state == 2) {
                    self.enter_checking(j);
                }
            }
            _ => {
                self.activate(j);
                if self.ext.get(j).map_or(false, |e| e.state == 2) {
                    self.enter_checking(j);
                }
            }
        }
    }

    /// A Mission reward with TransitionState CHECKING_OBJECTIVES passes READY_TO_START by.
    fn reward_checking(&mut self, i: usize) {
        if self.ext.get(i).map_or(false, |e| e.state == 2) {
            self.enter_checking(i);
        }
    }

    /// A save's record in READY_TO_START: the mission re-enters it with the loading flag, which
    /// posts its ON_ENTER event again (F-83568): out of the log, the call again.
    pub fn resume_ready(&mut self, i: usize) {
        if self.state[i].stage != Stage::Active {
            return;
        }
        self.order.retain(|&x| x != i);
        self.on_ready(i);
        if !self.missions[i].hidden {
            self.news.push(News::Started { id: self.missions[i].id.clone(), title: self.title(i), who: self.who(i), text: self.text(&format!("{}_DESC", self.missions[i].id)) });
        }
    }

    /// The original's state number of mission `i`.
    pub fn state_id_of(&self, i: usize) -> u32 {
        match self.state[i].stage {
            Stage::Active => self.ext.get(i).map_or(3, |e| e.state as u32),
            s => Self::stage_state_id(s),
        }
    }

    /// The bCareerScript READY_TO_START_ON_ENTER scripts of the missions waiting for their call
    /// (children of the career's start-up script while they run: F-68175 -> F-22458).
    pub fn waiting_career_scripts(&self) -> Vec<String> {
        (0..self.missions.len()).filter(|&i| self.state[i].stage == Stage::Active && self.ext.get(i).map_or(false, |e| e.state == 2)).filter_map(|i| self.missions[i].on_enter_career.clone()).collect()
    }

    /// Every bCareerScript ON_ENTER script of the career's missions.
    pub fn career_on_enter_scripts(&self) -> Vec<String> {
        self.missions.iter().filter_map(|m| m.on_enter_career.clone()).collect()
    }

    /// The story mission waits for its meeting this session: the second objective is still
    /// always hidden (the CORE script clears that byte at the meeting; a load puts the XML's back).
    fn story_waits(&self, i: usize) -> bool {
        STORY_SECOND_OBJECTIVE.contains(&self.missions[i].id.as_str()) && self.ext_ok(i) && self.missions[i].objectives.len() >= 2 && self.ext[i].obj_always[1]
    }

    /// The player at `p` (police on the tail: `pursued`): a story hookman met. Its CORE
    /// script satisfies the first objective and shows the second (0x7f4..0x822 in
    /// CITY_CHAMP_CORE). Returns the mission met.
    pub fn story_meet_at(&mut self, p: [f32; 3], pursued: bool) -> Option<String> {
        if pursued {
            return None;
        }
        for k in 0..self.order.len() {
            let i = self.order[k];
            let Some((_, at, r)) = STORY_POINTS.iter().find(|s| s.0 == self.missions[i].id) else { continue };
            if !self.story_waits(i) {
                continue;
            }
            let d = crate::brain::racer_distance_to_point(Some(glam::Vec3::from(p)), glam::Vec3::from(*at));
            if d >= *r {
                continue;
            }
            let m = self.missions[i].clone();
            let t0 = m.objectives[0].conds.first().map_or(1, |c| self.target(&m, c));
            if let Some(p0) = self.state[i].progress.first_mut() {
                *p0 = (*p0).max(t0);
            }
            self.ext[i].obj_always[1] = false;
            self.ext[i].obj_hidden[1] = false;
            // (the CORE plays the story scene there: CITY_CHAMP_CORE 0x1049 / night 0x149d,
            // BESTOF_SH_05_CORE 0xd16, BEAT_SH_02_EXOTIC_CORE 0xd79 - step 21b)
            self.news.push(News::Scene(m.id.clone()));
            log::info!("career: story hookman of {} met", m.id);
            return Some(m.id);
        }
        None
    }

    /// The completion statistic `k%AG` (F-20532): the completed list's missions over all
    /// missions, both without the always hidden (+685) and the repeatable (+693) ones - 100 in
    /// the 100 % save (107 of 107), 1 in the new game (2 of 107). The fraction is an f32 quotient
    /// (kept at the manager's +76), times 100.0 cut towards zero: 53 of 100 is 52. With nothing
    /// counted the kept fraction is posted, which stays 0 (the mission set never changes).
    pub fn percent_complete(&self) -> i64 {
        let counts = |i: &usize| !self.missions[*i].hidden && !self.missions[*i].repeatable;
        let total = (0..self.missions.len()).filter(counts).count();
        let done = self.done.iter().filter(|i| counts(i)).count();
        percent_of(done, total)
    }

    /// The statistic is posted when a mission joins the completed list (F-97326) and when
    /// a save has been read (F-23748).
    pub fn post_percent(&mut self) {
        let v = self.percent_complete();
        self.stats.insert("k%AG".to_string(), v);
    }

    /// A Stat condition's last value stops with the update that meets it (F-16469): the
    /// statistic at that moment is kept.
    fn freeze_met(&mut self, i: usize) {
        if !self.ext_ok(i) {
            return;
        }
        let m = self.missions[i].clone();
        for (k, r) in m.objectives.iter().enumerate() {
            if let Some(c @ Cond::Stat { stat, .. }) = r.conds.first() {
                let t = self.target(&m, c);
                if self.state[i].progress.get(k).copied().unwrap_or(0) >= t && self.ext[i].obj_last[k].is_none() {
                    self.ext[i].obj_last[k] = Some(self.stats.get(stat).copied().unwrap_or(0));
                }
            }
        }
    }

    /// The prerequisites are met as the mission opens: their statistics stop there.
    fn freeze_prereqs(&mut self, i: usize) {
        if !self.ext_ok(i) {
            return;
        }
        let m = self.missions[i].clone();
        for (k, r) in m.prereq.iter().enumerate() {
            if let Some(Cond::Stat { stat, .. }) = r.conds.first() {
                if self.ext[i].pre_last[k].is_none() {
                    self.ext[i].pre_last[k] = Some(self.stats.get(stat).copied().unwrap_or(0));
                }
            }
        }
    }

    /// A pink slip won at hangout `ident` (`hw_wager_hangout1` ...): the event
    /// PINK_SLIP_COMPLETE with that identifier (wager_hangout_core 0x5909) meets the open
    /// missions' objectives that wait for it (step 21).
    /// mcEvent_PostEventStr("MISSION_EVENT", ident) from a script (step 23: FIND_SH_02C_CORE
    /// 0x1361): the objectives of the open missions that wait for it are met.
    pub fn mission_event(&mut self, ident: &str) {
        self.event(Ev::Str("MISSION_EVENT", ident));
        let rep = self.rep;
        self.event(Ev::Rep(rep));
    }

    pub fn pink_slip_won(&mut self, ident: &str) {
        self.event(Ev::Str("PINK_SLIP_COMPLETE", ident));
        let rep = self.rep;
        self.event(Ev::Rep(rep));
    }

    /// The story CORE's MISSION_EVENT after the race against its man (`CORE_POSTS`): `place`
    /// the player's, `clean` the race finished neither damaged out nor arrested, `rep` the
    /// reputation with the race's paid: the race is paid (GENERIC_RACE_COMPLETE) before the core
    /// posts, so a mission it starts takes its Experience baseline from it.
    pub fn core_race_end(&mut self, mission: &str, place: usize, clean: bool, rep: i64) {
        let Some(&(_, ident, win)) = CORE_POSTS.iter().find(|c| c.0.eq_ignore_ascii_case(mission)) else { return };
        if clean && (!win || place == 1) {
            log::info!("career: {} posts MISSION_EVENT {}", mission, ident);
            // (the Rep event itself follows with the results)
            self.rep = rep;
            self.event(Ev::Str("MISSION_EVENT", ident));
        }
    }

    /// The story CORE's MISSION_EVENT at the meeting with its man (`MEET_POSTS`).
    pub fn core_meet(&mut self, mission: &str) {
        if let Some(&(_, ident)) = MEET_POSTS.iter().find(|c| c.0.eq_ignore_ascii_case(mission)) {
            self.mission_event(ident);
        }
    }

    /// A series won by the player: SERIES_COMPLETE with its name (OrderedSeriesCore's +3208) -
    /// a story mission's from its script (`SERIES_NAMES`), a hangout's `<hangout>_series`
    /// (local_hangout_core), Downtown's under HANGOUT_SERIES_DOWNTOWN's event script
    /// HANGOUT_SERIES_DT_COMPLETE (HANGOUT_SERIES_DOWNTOWN_HANGOUT_ON_ENTER sets +3208 so).
    pub fn series_won(&mut self, story: Option<&str>, key: &str) {
        let name = match story {
            Some(id) => match SERIES_NAMES.iter().find(|c| c.0.eq_ignore_ascii_case(id)) {
                Some(&(_, n)) => n.to_string(),
                None => return,
            },
            None if key == "dt_local_hangout1_series" && std::mem::take(&mut self.dt_series_story) => "HANGOUT_SERIES_DT_COMPLETE".to_string(),
            None => key.to_string(),
        };
        log::info!("career: SERIES_COMPLETE {}", name);
        self.event(Ev::Str("SERIES_COMPLETE", &name));
    }

    /// A Mission reward (`state` as the tables write it without '_': READYTOSTART ...).
    pub fn give_mission(&mut self, id: &str, state: &str) {
        self.give(&Reward::Mission { id: id.to_ascii_uppercase(), state: state.to_string() });
    }

    /// A save's frozen values (a met Stat condition's last value).
    pub fn last_from_save(&mut self, i: usize, obj: &[Option<i64>], pre: &[Option<i64>]) {
        if !self.ext_ok(i) {
            return;
        }
        for (k, v) in obj.iter().enumerate().take(self.ext[i].obj_last.len()) {
            if v.is_some() {
                self.ext[i].obj_last[k] = *v;
            }
        }
        for (k, v) in pre.iter().enumerate().take(self.ext[i].pre_last.len()) {
            if v.is_some() {
                self.ext[i].pre_last[k] = *v;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// step 20 (save work): the mission manager's two lists, the requirements' "hidden" bytes,
// the conditions' baselines (`mc4`: the Career entry's mission records)
// ---------------------------------------------------------------------------------------

/// What the original keeps per mission beside its state (mission records, written by
/// F-56966 / F-57390, read by F-60562 / F-47948):
/// - `obj_hidden`: each objective's byte +118. 1 when the requirement is made
///   (F-61179); set to 0 when the mission enters READY_TO_START, CHECKING_OBJECTIVES or
///   READY_TO_COMPLETE and to 1 when it enters CHECKING_PREREQUISITES (F-97429 /
///   F-73549 / F-49654 / F-89508 -> F-78896, which leaves it alone while
///   +117 is set); mcRequirement_Hide (F-82003) sets it from a script;
/// - `obj_always`: byte +117 - the XML's bAlwaysHidden, changed by mcRequirement_AlwaysHide
///   (F-61276). Not saved: after a load it is the XML's again. The mission log lists an
///   objective when neither is set (F-74020 A-10736);
/// - `obj_base` / `pre_base`: the baseline of an objective's / a prerequisite's Stat condition
///   (the statistic's value when the condition was armed, F-70875) or Experience
///   condition (the system's amount, F-95975). Armed when the mission enters
///   CHECKING_OBJECTIVES (objectives: F-73549 -> F-19696) or CHECKING_PREREQUISITES
///   (prerequisites: F-89508); kept as saved when a save is loaded (the load re-enters the
///   state with the loading flag, which skips the capture). None: not known (a save of the
///   port from before): counted from 0, as the port did;
/// - `obj_pending`: an Experience baseline waiting for the reputation of the race whose end
///   armed it (the original pays the race before its CORE script completes the mission; the
///   port reports the race to the career before the reputation).
#[derive(Debug, Clone, Default)]
pub struct MissionExt {
    pub obj_hidden: Vec<bool>,
    pub obj_always: Vec<bool>,
    pub obj_base: Vec<Option<i64>>,
    pub obj_pending: Vec<bool>,
    pub pre_base: Vec<Option<i64>>,
    /// the original's state number (0 .. 5; the port's Active is 2, 3 or 4) - step 20b
    pub state: u8,
    /// a met Stat condition's last value, frozen at the moment it was met
    pub obj_last: Vec<Option<i64>>,
    pub pre_last: Vec<Option<i64>>,
    /// a prerequisite's TallyMission LessThan condition met (+40, saved: F-63669 / F-78825):
    /// cleared at the arming (F-42425), set when a check finds too few open missions
    pub pre_tally: Vec<bool>,
}

/// The story missions whose CORE script shows the second objective once the hookman is met
/// (mcRequirement_Satisfy(objective 0, 1), mcRequirement_AlwaysHide(objective 1, 0),
/// mcRequirement_Hide(objective 1, 0), then mcMission_ChangeState(3)) and puts the first
/// step back when the series is not won (Satisfy(0, 0), AlwaysHide(1, 1), Hide(1, 1)):
/// `Game/Career/Story/<ID>_CORE.sco` (BESTOF_SH_05 0x8bf..0x8ed / 0x123d..0x126b, CITY_CHAMP,
/// BEAT_SH_02_EXOTIC; RACE_SH_01's is on no mission list).
/// The standings after a tournament's race (mcTournamentResultsForm, form 49, filled by
/// F-26722): each racer's name ("YOU" the player), this race's points and the total, in
/// the race's order; `decided` won / lost with this race (only its last: F-66541), `last`
/// the event's last race was run.
#[derive(Clone, Debug, Default)]
pub struct TourTable {
    pub rows: Vec<(String, i32, i32)>,
    pub decided: Option<bool>,
    pub last: bool,
}

/// The story CORE scripts' MISSION_EVENT after the race against their man
/// (`Game/Career/Story/<ID>_CORE.sco`, South Central's `sc/Career/Story/`): (mission, identifier,
/// only on a win). Missions on no list (BEAT_GH_04_MUSCLE, RACE_SH_01, ...) keep their row. Every core posts
/// after a finished race - not damaged out (Race_IsDamagedOut), not arrested
/// (Race_HasBeenArrested), the results' +3144 nought. The generic core (BEAT_SH_03_CORE
/// 0x3d6f..0x3dc7, and the fourteen built on it, which pass their identifier as an argument)
/// and BEAT_GH_04_MUSCLE (0x2f6a..0x2fb2), BEAT_GH_05_TUNER, BEAT_SH_02_DCLASS and
/// BEAT_SH_04_BCLASS ask for the player's place (results +164) 1 as well; RACE_GH_02,
/// RACE_SH_01 (its identifier RACE_SH_02_RACE_COMPLETE), BEAT_SH_04B and RACE_GH_06 post after
/// any finished race.
pub const CORE_POSTS: [(&str, &str, bool); 23] = [
    ("BEAT_GH_04_MUSCLE", "BEAT_GH_04_MUSCLE_RACE_COMPLETE", true),
    ("BEAT_GH_05_TUNER", "BEAT_TUNER_GH_05_RACE_COMPLETE", true),
    ("BEAT_GH_26_SUV", "BEAT_GH_26_SUV_RACE_COMPLETE", true),
    ("BEAT_SH_02_BCLASS", "BEAT_SH_02_BCLASS_RACE_COMPLETE", true),
    ("BEAT_SH_02_CCLASS", "BEAT_SH_02_CCLASS_RACE_COMPLETE", true),
    ("BEAT_SH_02_DCLASS", "BEAT_SH_02_DCLASS_RACE_COMPLETE", true),
    ("BEAT_SH_03", "BEAT_SH_03_RACE_COMPLETE", true),
    ("BEAT_SH_03_ACLASS", "BEAT_SH_03_ACLASS_RACE_COMPLETE", true),
    ("BEAT_SH_03_CCLASS", "BEAT_SH_03_CCLASS_RACE_COMPLETE", true),
    ("BEAT_SH_04", "BEAT_SH_04_RACE_COMPLETE", true),
    ("BEAT_SH_04B", "BEAT_SH_04B_RACE_COMPLETE", false),
    ("BEAT_SH_04_BCLASS", "BEAT_SH_04_BCLASS_RACE_COMPLETE", true),
    ("BEAT_SH_04_MOTORCYCLE", "BEAT_SH_04_MOTORCYCLE_COMPLETE", true),
    ("BEAT_SH_05_ACLASS", "BEAT_SH_05_ACLASS_COMPLETE", true),
    ("EXOTIC_CHAMP", "EXOTIC_CHAMP_OBJ_01_COMPLETE", true),
    ("LUXURY_CHAMP", "LUXURY_CHAMP_OBJ_01_COMPLETE", true),
    ("MOTORCYCLE_CHAMP", "MOTO_CHAMP_OBJ_01_COMPLETE", true),
    ("MUSCLE_CHAMP", "MUSCLE_CHAMP_OBJ_01_COMPLETE", true),
    ("TUNER_CHAMP", "TUNER_CHAMP_OBJ_01_COMPLETE", true),
    ("RACE_GH_02", "RACE_GH_02_RACE_COMPLETE", false),
    ("RACE_GH_06", "RACE_GH_06_COMPLETE", false),
    ("RACE_SH_01", "RACE_SH_02_RACE_COMPLETE", false),
    ("RACE_SH_04_MOTORCYCLE", "RACE_SH_04_MOTORCYCLE_COMPLETE", true),
];

/// The story CORE scripts that post their MISSION_EVENT at the meeting with their man, after
/// its scene (mcHookman_SetState, Racer_Activate, then the event): the FIND_ cores and
/// BEAT_SH_05_CORE. (FIND_SH_02C's is posted at its place: `mc4_find02c_meet`.)
pub const MEET_POSTS: [(&str, &str); 7] = [
    ("FIND_SH_01", "FIND_SH_01_OBJ_01_COMPLETE"),
    ("FIND_SH_01B", "FIND_SH_01B_OBJ_01_COMPLETE"),
    ("FIND_SH_02", "FIND_SH_02_OBJ_01_COMPLETE"),
    ("FIND_SH_02B", "FIND_SH_02B_OBJ_01_COMPLETE"),
    ("FIND_SH_03", "FIND_SH_03_OBJ_01_COMPLETE"),
    ("FIND_SH_03B", "FIND_SH_03B_OBJ_01_COMPLETE"),
    ("BEAT_SH_05", "BEAT_SH_05_COMPLETE"),
];

/// The series name each story script gives OrderedSeriesCore (+3208), posted with
/// SERIES_COMPLETE when the player wins it. DISTRICT_CHAMPS waits for three of them.
pub const SERIES_NAMES: [(&str, &str); 20] = [
    ("BEAT_SH_02_EXOTIC", "BEAT_SH_02_EXOTIC_SERIES"),
    ("BESTOF_GH_04", "BESTOF_GH_04_SERIES"),
    ("BESTOF_GH_05", "BESTOF_GH_05_SERIES"),
    ("BESTOF_GH_22", "BESTOF_GH_22_SERIES"),
    ("BESTOF_GH_26", "BESTOF_GH_26_SERIES"),
    ("BESTOF_GH_27", "BESTOF_GH_27_SERIES"),
    ("BESTOF_GH_31", "BESTOF_GH_31_SERIES"),
    ("BESTOF_SH_02", "BESTOF_SH_02_SERIES"),
    ("BESTOF_SH_03", "BESTOF_SH_03_SERIES"),
    ("BESTOF_SH_03_LUXURY", "BESTOF_SH_03_LUXURY_SERIES"),
    ("BESTOF_SH_05", "BESTOF_SH_05_SERIES"),
    ("CITY_CHAMP", "CITY_CHAMP_SERIES"),
    ("DISTRICT_CHAMPS_BEACHES", "DISTRICT_CHAMPS_BEACHES_SERIES"),
    ("DISTRICT_CHAMPS_DOWNTOWN", "DISTRICT_CHAMPS_DOWNTOWN_SERIES"),
    ("DISTRICT_CHAMPS_HILLS", "DISTRICT_CHAMPS_HILLS_SERIES"),
    ("REPEAT_BESTOF_BEACHES", "REPEAT_BESTOF_BE_SERIES"),
    ("REPEAT_BESTOF_DOWNTOWN", "REPEAT_BESTOF_DT_SERIES"),
    ("REPEAT_BESTOF_HILLS", "REPEAT_BESTOF_HL_SERIES"),
    ("REPEAT_BESTOF_HOLLYWOOD", "REPEAT_BESTOF_HW_SERIES"),
    ("REPEAT_BESTOF_SC", "REPEAT_BESTOF_SC_SERIES"),
];

pub const STORY_SECOND_OBJECTIVE: [&str; 4] = ["BEAT_SH_02_EXOTIC", "BESTOF_SH_05", "CITY_CHAMP", "RACE_SH_01"];

/// `sc/Career/CareerSetup_SC` (`sc/career/0x70e862b0`): the mission and the missions it
/// checks for COMPLETE, in the script's order.
pub const SC_SETUP_MISSIONS: [(&str, &[&str]); 6] = [
    ("GARAGE_SC", &["GARAGE_HOLLYWOOD"]),
    ("BESTOF_GH_31", &["DELIVERY"]),
    ("LOWRIDER_REP", &["GARAGE_HOLLYWOOD"]),
    ("DELIVERY_SC", &["GARAGE_HOLLYWOOD", "DELIVERY"]),
    ("TOURNAMENT_SC", &["HANGOUT_SERIES_SC"]),
    ("RACES_IN_SC", &["HANGOUT_SERIES_SC", "HANGOUT_SERIES_DOWNTOWN"]),
];

/// A requirement as the save holds it (the record's fields the port can give).
#[derive(Debug, Clone, Default)]
pub struct ReqSave {
    /// the condition classes, in the XML's order: String, Mission, TallyRace, TallyMission,
    /// Stat, Experience (the record's condition types 1, 2, 5, 6, 7, 0)
    pub kinds: Vec<&'static str>,
    pub hidden: bool,
    /// the port's verdict: every condition met
    pub met: bool,
    /// a Tally condition's count (up to its target)
    pub count: u32,
    /// a Stat / Experience condition's baseline
    pub base: Option<i64>,
    /// a Stat condition's statistic now
    pub value: Option<i64>,
}

/// A mission as the save holds it.
#[derive(Debug, Clone, Default)]
pub struct MissionSave {
    pub id: String,
    /// place in the mission lists (`tune/career/0x68d5d432`, then `0x3b47e155`): mission+696
    pub index: i32,
    pub addon: bool,
    pub stage: Stage,
    /// the original's state number (step 20b)
    pub state_id: u32,
    pub objectives: Vec<ReqSave>,
    pub prerequisites: Vec<ReqSave>,
}

fn cond_kind(c: &Cond) -> &'static str {
    match c {
        Cond::Mission(_) => "Mission",
        Cond::TallyRace { .. } => "TallyRace",
        Cond::TallyMission { .. } => "TallyMission",
        Cond::Stat { .. } => "Stat",
        Cond::Experience { .. } => "Experience",
        Cond::Event { .. } => "String",
    }
}

/// Money as F-36073 writes it (with the currency sign), by the language (the string
/// table's +28: 0 English, 1 Spanish, 2 French, 3 German, 4 Italian): digits grouped by three
/// (F-33429) with ',' (0 and others), '.' (1, 3, 4) or '|' (2); "$" before (0, 3 and
/// others) or after (1, 2, 4).
pub fn money_text_in(v: i64, lang: usize) -> String {
    let g = money_digits_in(v, lang);
    match lang {
        1 | 2 | 4 => format!("{g}$"),
        _ => format!("${g}"),
    }
}

/// The count an Experience, TallyRace or TallyMission condition puts after its objective in the mission log
/// (F-34957, F-62132, F-95375: "%s (%d/%d)", signed words): the target once the condition is complete, else its
/// count - TallyRace +192, TallyMission +156, Experience the experience record's value less the base armed with
/// the condition (no text when the record is not found), none of them cut to the target.
pub fn count_text(complete: bool, count: i32, target: i32) -> String {
    format!("({}/{})", if complete { target } else { count }, target)
}

/// A Stat condition's count after its objective in the mission log (F-97850): the statistic type compared byte
/// for byte (case kept) - `Money` / `Purse` give "($cur/$target)" in the language's money text (F-13827), `Tally`
/// "(cur/target)" as signed words, any other nothing. The original writes it after the text and a space ("%s (..)");
/// `cur` is the target once the condition is complete, else its progress (F-13232).
pub fn stat_count_text(stat_type: &str, cur: i32, target: i32, lang: usize) -> Option<String> {
    match stat_type {
        "Money" | "Purse" => Some(format!("({}/{})", money_text_in(cur as i64, lang), money_text_in(target as i64, lang))),
        "Tally" => Some(format!("({cur}/{target})")),
        _ => None,
    }
}

/// The digits alone, as F-36073 writes them with its sign flag clear (the GPS panel's
/// `Navmap_Bank_Amount`, F-27560).
pub fn money_digits_in(v: i64, lang: usize) -> String {
    let d = v.abs().to_string();
    let sep = match lang {
        1 | 3 | 4 => '.',
        2 => '|',
        _ => ',',
    };
    let mut g = String::new();
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i) % 3 == 0 {
            g.push(sep);
        }
        g.push(c);
    }
    if v < 0 { format!("-{g}") } else { g }
}

fn money_text(v: i64) -> String {
    money_text_in(v, 0)
}

impl Career {
    /// The original's state number (0 INACTIVE, 1 CHECKING_PREREQUISITES, 2 READY_TO_START,
    /// 3 CHECKING_OBJECTIVES, 4 READY_TO_COMPLETE, 5 COMPLETE) of a stage. Active is
    /// CHECKING_OBJECTIVES: the port has no READY_TO_START / READY_TO_COMPLETE of its own.
    pub fn stage_state_id(s: Stage) -> u32 {
        match s {
            Stage::Locked => 0,
            Stage::Armed => 1,
            Stage::Active => 3,
            Stage::Complete => 5,
        }
    }

    /// The stage of an original's state number.
    pub fn state_stage(id: u32) -> Stage {
        match id {
            0 => Stage::Locked,
            1 => Stage::Armed,
            2 | 3 | 4 => Stage::Active,
            _ => Stage::Complete,
        }
    }

    /// An objective's byte +118 as the states a mission has been through leave it: cleared
    /// in READY_TO_START .. READY_TO_COMPLETE (and so in COMPLETE, which does not touch it)
    /// unless +117 is set; set at construction and in CHECKING_PREREQUISITES.
    fn default_hidden(stage: Stage, always: bool) -> bool {
        match stage {
            Stage::Active | Stage::Complete => always,
            _ => true,
        }
    }

    /// The per-mission state of step 20 for every mission, by its stage (a new career, a
    /// save without the block - the block's records are put over it: `mc4`).
    pub fn ext_init(&mut self) {
        self.ext = (0..self.missions.len())
            .map(|i| {
                let m = &self.missions[i];
                let stage = self.state.get(i).map_or(Stage::Locked, |s| s.stage);
                let always: Vec<bool> = m.objectives.iter().map(|r| r.hidden).collect();
                MissionExt {
                    obj_hidden: always.iter().map(|a| Self::default_hidden(stage, *a)).collect(),
                    obj_always: always,
                    obj_base: vec![None; m.objectives.len()],
                    obj_pending: vec![false; m.objectives.len()],
                    pre_base: vec![None; m.prereq.len()],
                    state: Self::stage_state_id(stage) as u8,
                    obj_last: vec![None; m.objectives.len()],
                    pre_last: vec![None; m.prereq.len()],
                    pre_tally: vec![false; m.prereq.len()],
                }
            })
            .collect();
    }

    fn ext_ok(&self, i: usize) -> bool {
        self.ext.get(i).map_or(false, |e| e.obj_hidden.len() == self.missions[i].objectives.len() && e.pre_base.len() == self.missions[i].prereq.len())
    }

    /// A statistic counted from a baseline (F-13232, `stat_progress`): a Purse statistic as it
    /// stands, any other from its value when the condition was armed, 0 ..= `target`.
    fn stat_since(&self, stat: &str, purse: bool, base: Option<i64>, target: i64) -> i64 {
        let v = self.stats.get(stat).copied().unwrap_or(0);
        stat_progress(purse, v as i32, base.unwrap_or(0) as i32, target as i32) as i64
    }

    /// The condition's type is "Purse", compared byte for byte (F-13232; case counts).
    fn purse(r: &Requirement, n: usize) -> bool {
        r.stat_types.get(n).map_or(false, |t| t == "Purse")
    }

    /// The mission enters the mission log (the port's Active: CHECKING_OBJECTIVES): it leaves
    /// the completed list if it was in it (F-97266); its objectives are shown unless
    /// always hidden; their Stat / Experience conditions are armed (F-73549).
    fn on_log(&mut self, i: usize) {
        self.done.retain(|&x| x != i);
        self.journal.push((true, i));
        if !self.ext_ok(i) {
            return;
        }
        let m = self.missions[i].clone();
        for (k, r) in m.objectives.iter().enumerate() {
            if !self.ext[i].obj_always[k] {
                self.ext[i].obj_hidden[k] = false;
            }
            match r.conds.first() {
                Some(Cond::Stat { stat, .. }) => {
                    let v = self.stats.get(stat).copied().unwrap_or(0);
                    self.ext[i].obj_base[k] = Some(v);
                }
                Some(Cond::Experience { .. }) => {
                    self.ext[i].obj_base[k] = Some(self.rep);
                    self.ext[i].obj_pending[k] = true;
                }
                _ => {}
            }
        }
    }

    /// A repeatable mission leaves COMPLETE (F-62492: the common exit, then F-10825 resets
    /// every prerequisite and objective - F-22437 per requirement: each condition's reset,
    /// met +116 and +140 cleared, hidden +118 set; +117 kept). It stays in the completed
    /// list until it joins the log again (F-58560 -> `on_log`).
    fn leave_complete(&mut self, i: usize) {
        self.state[i] = MState { stage: Stage::Locked, progress: vec![0; self.missions[i].objectives.len()], won: Vec::new(), pts: Default::default() };
        self.order.retain(|&x| x != i);
        if !self.ext_ok(i) {
            return;
        }
        let e = &mut self.ext[i];
        e.obj_hidden.iter_mut().for_each(|h| *h = true);
        e.obj_base.iter_mut().for_each(|b| *b = None);
        e.obj_pending.iter_mut().for_each(|p| *p = false);
        e.obj_last.iter_mut().for_each(|b| *b = None);
        e.pre_base.iter_mut().for_each(|b| *b = None);
        e.pre_last.iter_mut().for_each(|b| *b = None);
        e.pre_tally.iter_mut().for_each(|t| *t = false);
        e.state = 0;
    }

    /// The mission enters CHECKING_PREREQUISITES (F-89508): its objectives are hidden
    /// unless always hidden (that is, they keep +118), its prerequisites armed.
    fn on_armed(&mut self, i: usize) {
        if !self.ext_ok(i) {
            return;
        }
        let m = self.missions[i].clone();
        for k in 0..m.objectives.len() {
            if !self.ext[i].obj_always[k] {
                self.ext[i].obj_hidden[k] = true;
            }
        }
        for (k, r) in m.prereq.iter().enumerate() {
            match r.conds.first() {
                Some(Cond::Stat { stat, .. }) => self.ext[i].pre_base[k] = Some(self.stats.get(stat).copied().unwrap_or(0)),
                Some(Cond::Experience { .. }) => self.ext[i].pre_base[k] = Some(self.rep),
                _ => {}
            }
            self.ext[i].pre_tally[k] = false;
            if r.init_validation {
                self.tally_check(i, k);
            }
        }
    }

    /// The open missions of a district for a TallyMission LessThan condition (F-12302 ->
    /// F-65871 with mask 28: states 2 READY_TO_START, 3 CHECKING_OBJECTIVES and 4
    /// READY_TO_COMPLETE, a state's mask being 1 << its number - F-33894; the filter F-78851
    /// of kind 0 passes all; the mission's district equal to the condition's, or the
    /// condition's "All").
    fn tally_open(&self, district: &str) -> u32 {
        let want = district_no(district);
        (0..self.missions.len()).filter(|&j| (2..=4).contains(&self.state_id_of(j)) && (want == 6 || district_no(&self.missions[j].district) == want)).count() as u32
    }

    /// A prerequisite's TallyMission LessThan check (F-12302, at the arming with
    /// bInitValidation and on every MISSION_COMPLETE, which F-97326 posts once the mission is
    /// in state 5 - F-58560, then F-63586): met, and kept, when fewer than nTally missions of
    /// its district are open.
    fn tally_check(&mut self, i: usize, k: usize) {
        let Some(Cond::TallyMission { less_than: true, tally, district }) = self.missions[i].prereq[k].conds.first() else { return };
        if !self.ext_ok(i) || self.ext[i].pre_tally[k] {
            return;
        }
        if self.tally_open(district) < *tally {
            self.ext[i].pre_tally[k] = true;
        }
    }

    /// The mission is complete: it joins the completed list at its end (F-97326 ->
    /// F-26006 list 2, unless it is in it).
    fn mark_done(&mut self, i: usize) {
        if !self.done.contains(&i) {
            self.done.push(i);
        }
        self.done_now.push(i);
    }

    /// MISSION_COMPLETE (F-63586, after the mission's state 5 and its rewards): the armed
    /// missions' TallyMission conditions check.
    fn mission_complete_event(&mut self) {
        for j in 0..self.missions.len() {
            if self.state[j].stage == Stage::Armed {
                for k in 0..self.missions[j].prereq.len() {
                    self.tally_check(j, k);
                }
            }
        }
    }

    /// Experience baselines armed by a race take the reputation that race paid.
    /// Missions whose prerequisites have just been met step on.
    fn activate_met(&mut self) {
        loop {
            let mut changed = false;
            for i in 0..self.missions.len() {
                let waiting = match self.state[i].stage {
                    Stage::Armed => true,
                    // nobody hands this one out: it follows from its prerequisites alone
                    Stage::Locked => !self.rewarded.contains(&i) && self.missions[i].prereq.iter().any(|r| r.conds.iter().any(|c| matches!(c, Cond::Mission(_)))),
                    _ => false,
                };
                if waiting && self.prereqs_met(i) {
                    self.activate(i);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn resolve_pending(&mut self) {
        let rep = self.rep;
        for e in self.ext.iter_mut() {
            for (k, p) in e.obj_pending.iter_mut().enumerate() {
                if *p {
                    *p = false;
                    e.obj_base[k] = Some(rep);
                }
            }
        }
    }

    /// An objective's Stat progress (F-13232, up to the target).
    fn ext_stat_progress(&self, i: usize, oi: usize, stat: &str, target: u32) -> u32 {
        let base = self.ext.get(i).and_then(|e| e.obj_base.get(oi).copied()).flatten();
        let purse = Self::purse(&self.missions[i].objectives[oi], 0);
        // (a Purse value is not clamped; below 0 it shows 0 here - approx.: the original's
        // word would be negative)
        self.stat_since(stat, purse, base, target as i64).max(0) as u32
    }

    /// An objective's Experience progress: the amount since its baseline (F-79261); 0
    /// while the baseline waits for the race's reputation.
    fn ext_exp_progress(&self, i: usize, oi: usize, target: u32) -> u32 {
        let e = self.ext.get(i);
        if e.map_or(false, |e| e.obj_pending.get(oi).copied().unwrap_or(false)) {
            return 0;
        }
        let base = e.and_then(|e| e.obj_base.get(oi).copied()).flatten().unwrap_or(0);
        (self.rep - base).clamp(0, target as i64) as u32
    }

    /// One prerequisite's verdict (the port's rules; a Stat / Experience condition from its
    /// baseline).
    fn prereq_req_met(&self, i: usize, k: usize) -> bool {
        let r = &self.missions[i].prereq[k];
        let base = self.ext.get(i).and_then(|e| e.pre_base.get(k).copied()).flatten();
        r.conds.iter().enumerate().all(|(ci, c)| match c {
            Cond::Mission(id) => self.index.get(id).map_or(false, |&j| self.state[j].stage == Stage::Complete),
            // (LessThan: the kept verdict of the checks, F-12302; without it the original counts
            // the completions since the arming - no mission file has one)
            Cond::TallyMission { less_than: true, .. } => self.ext.get(i).and_then(|e| e.pre_tally.get(k).copied()).unwrap_or(false),
            Cond::TallyMission { less_than: false, tally, district } => {
                let n = self.missions.iter().zip(self.state.iter()).filter(|(m, s)| s.stage == Stage::Complete && same_district(district, &m.district)).count() as u32;
                n >= *tally
            }
            Cond::Experience { amount } => self.rep - base.unwrap_or(0) >= *amount,
            Cond::Stat { stat, value } => self.stat_since(stat, Self::purse(r, ci), base, *value as i64) >= *value as i64,
            _ => true,
        })
    }

    /// The story missions' second objective comes out once the first is done.
    fn story_meet(&mut self, i: usize) {
        if !STORY_SECOND_OBJECTIVE.contains(&self.missions[i].id.as_str()) || !self.ext_ok(i) || self.missions[i].objectives.len() < 2 {
            return;
        }
        let m = &self.missions[i];
        let t0 = m.objectives[0].conds.first().map_or(1, |c| self.target(m, c));
        if self.state[i].progress.first().copied().unwrap_or(0) >= t0 && self.ext[i].obj_always[1] {
            self.ext[i].obj_always[1] = false;
            self.ext[i].obj_hidden[1] = false;
        }
    }

    /// ... and goes back when the series is lost.
    fn story_reset(&mut self, i: usize) {
        if !STORY_SECOND_OBJECTIVE.contains(&self.missions[i].id.as_str()) || !self.ext_ok(i) || self.missions[i].objectives.len() < 2 {
            return;
        }
        if let Some(p) = self.state[i].progress.first_mut() {
            *p = 0;
        }
        self.ext[i].obj_always[1] = true;
        self.ext[i].obj_hidden[1] = true;
    }

    /// The text the mission log puts after an objective (the condition's +40 routine, for the
    /// classes whose byte +41 is set - Experience, Tally, TallyRace, TallyMission, Stat:
    /// F-34957, F-62132, F-95375, F-97850): "(n/target)", for a Money or
    /// Purse statistic "($n/$target)", nothing for a statistic of another type, a String or a
    /// Mission condition.
    fn objective_count(&self, i: usize, oi: usize, cur: u32, target: u32) -> Option<String> {
        let r = &self.missions[i].objectives[oi];
        match r.conds.first()? {
            Cond::Stat { .. } => stat_count_text(r.stat_types.first().map_or("", |s| s.as_str()), cur as i32, target as i32, self.strings.lang),
            Cond::Experience { .. } | Cond::TallyRace { .. } | Cond::TallyMission { .. } => Some(count_text(false, cur as i32, target as i32)),
            _ => None,
        }
    }

    // ---- MISSION RESULTS: the missions' "updated" bytes (docs/reference/ui/mission_updates_and_pending_rewards.md) ----

    /// A Condition_Updated reaching requirement R (F-64780) and, passed on as
    /// Requirement_Updated, R's mission (F-30196): (R's +140 set, the mission's +690 set).
    /// `updated_msg`: the message is Condition_Updated (R's handle +108); `cond_in_req`: its
    /// condition is one of R's (+124); `satisfied`: R's +116; `mission_msg`: the mission hears
    /// it (its handle +660 - listeners on in states 1 to 4); `member`: R is one of the mission's
    /// prerequisites or objectives (F-88090).
    pub fn mu_update(updated_msg: bool, cond_in_req: bool, satisfied: bool, allow_update: bool, mission_msg: bool, member: bool) -> (bool, bool) {
        let req = updated_msg && cond_in_req && !satisfied && allow_update;
        (req, req && mission_msg && member)
    }

    /// MISSION RESULTS takes the mission: the collector F-65871 with mask 60 (states 2 to 5, a
    /// state's mask being 1 << its number - F-33894) and the filter F-78851 of kind 4.
    pub fn mu_listed(state: u32, always_hidden: bool, should_update: bool, updated: bool) -> bool {
        (2..=5).contains(&state) && !always_hidden && should_update && updated
    }

    /// What makes a prerequisite's condition post Condition_Updated when it changes: a Mission
    /// condition its mission completing (F-88673), a TallyMission one without LessThan each
    /// completion in its district (F-12302; with LessThan it posts no update), an Experience
    /// one the experience (F-79261), a Stat one its statistic (F-16469). (The one TallyRace
    /// prerequisite of the files the port does not count: 0.)
    fn upd_measure(&self, c: &Cond) -> i64 {
        match c {
            Cond::Mission(id) => self.index.get(id).map_or(0, |&j| (self.state[j].stage == Stage::Complete) as i64),
            Cond::TallyMission { less_than: false, district, .. } => self.missions.iter().zip(self.state.iter()).filter(|(m, s)| s.stage == Stage::Complete && same_district(district, &m.district)).count() as i64,
            Cond::Experience { .. } => self.rep,
            Cond::Stat { stat, .. } => self.stats.get(stat).copied().unwrap_or(0),
            _ => 0,
        }
    }

    fn upd_snap(&self, i: usize, prev: Option<&UpdSnap>) -> UpdSnap {
        let m = &self.missions[i];
        let state = self.state_id_of(i);
        let obj = m
            .objectives
            .iter()
            .enumerate()
            .map(|(k, r)| {
                let cur = self.state[i].progress.get(k).copied().unwrap_or(0);
                let met = self.state[i].stage == Stage::Complete || r.conds.first().map_or(true, |c| cur >= self.target(m, c));
                (cur as i64, met)
            })
            .collect();
        let pre = m.prereq.iter().enumerate().map(|(k, r)| (r.conds.iter().map(|c| self.upd_measure(c)).collect(), self.state[i].stage != Stage::Locked && self.prereq_req_met(i, k))).collect();
        // (armed by the enter of state 1 - the prerequisites - and of state 3 - the objectives; a
        // load enters the saved state)
        let (pa, oa) = prev.map_or((false, false), |p| (p.pre_armed, p.obj_armed));
        UpdSnap { state, obj, pre, pre_armed: pa || state == 1, obj_armed: oa || matches!(state, 3 | 4) }
    }

    /// The missions' "updated" bytes: since the last call, a condition of a requirement that is
    /// armed, not met (+116), with bAllowUpdate, posted Condition_Updated - in the port: an
    /// objective's progress or a prerequisite's measure changed - so the requirement passed
    /// Requirement_Updated on (F-64780) to its mission, which, with its listeners on (states 1 to
    /// 4: F-39892, off in 0 and 5: F-28093), is updated (+690 = 1, F-30196). Called after every
    /// career event and before MISSION RESULTS reads the bytes; the first call after a load or a
    /// new game only takes the snapshot.
    pub fn scan_updates(&mut self) {
        let n = self.missions.len();
        if self.state.len() != n {
            return;
        }
        if self.upd.len() != n {
            self.upd = (0..n).map(|i| self.upd_snap(i, None)).collect();
            *self.updated.borrow_mut() = vec![false; n];
            return;
        }
        for i in 0..n {
            let now = self.upd_snap(i, Some(&self.upd[i]));
            let prev = &self.upd[i];
            let hears = (1..=4).contains(&prev.state);
            let m = &self.missions[i];
            // (an armed requirement's condition posted Condition_Updated: its measure moved)
            let posted = |p: Option<&(i64, bool)>, q: Option<&(i64, bool)>| p.zip(q).map_or(false, |(p, q)| p.0 != q.0);
            let obj = prev.obj_armed && m.objectives.iter().enumerate().any(|(k, r)| posted(prev.obj.get(k), now.obj.get(k)) && Self::mu_update(true, true, prev.obj[k].1, r.allow_update, hears, true).1);
            let pre = prev.pre_armed && m.prereq.iter().enumerate().any(|(k, r)| prev.pre.get(k).zip(now.pre.get(k)).map_or(false, |(p, q)| p.0 != q.0) && Self::mu_update(true, true, prev.pre[k].1, r.allow_update, hears, true).1);
            if obj || pre {
                self.updated.borrow_mut()[i] = true;
            }
            self.upd[i] = now;
        }
    }

    /// The missions MISSION RESULTS lists, in the manager's order (its list of every mission:
    /// the lists' order, `index`): the collector F-65871 with the states 2 to 5 (mask 60; each
    /// state's mask is 1 << its number, F-33894) and the filter F-78851 of kind 4 - not
    /// bAlwaysHidden, bShouldUpdate and updated. The form comes up when there is one (F-44151;
    /// in a network game never).
    pub fn mission_updates(&mut self) -> Vec<usize> {
        self.scan_updates();
        let upd = self.updated.borrow();
        let mut v: Vec<usize> = (0..self.missions.len().min(upd.len()))
            .filter(|&i| Self::mu_listed(self.state_id_of(i), self.missions[i].hidden, self.missions[i].should_update, upd[i]))
            .collect();
        v.sort_by_key(|&i| self.missions[i].index);
        v
    }

    /// The form filled (F-66221): per mission its title, every objective's line - the
    /// description and, for the conditions that show one, the count (F-26170) - ticked by
    /// its +116, and whether it is complete (state 5: the header's stamp); a mission that is not
    /// complete loses its "updated" byte now (F-92621), a complete one when the form goes
    /// (`clear_mission_updates`).
    pub fn take_mission_updates(&mut self) -> Vec<(String, Vec<String>, bool)> {
        let list = self.mission_updates();
        let mut out = Vec::new();
        for i in list {
            let complete = self.state_id_of(i) == 5;
            if !complete {
                self.updated.borrow_mut()[i] = false;
            }
            let m = &self.missions[i];
            let lines = m
                .objectives
                .iter()
                .enumerate()
                .map(|(k, r)| {
                    let target = r.conds.first().map_or(1, |c| self.target(m, c));
                    let cur = self.state[i].progress.get(k).copied().unwrap_or(0);
                    let met = complete || cur >= target;
                    let text = match self.objective_count(i, k, cur.min(target), target) {
                        Some(c) => format!("{} {}", self.text(&r.desc), c),
                        None => self.text(&r.desc),
                    };
                    if met { format!("{}{}", crate::game::OBJ_DONE, text) } else { text }
                })
                .collect();
            out.push((self.title(i), lines, complete));
        }
        out
    }

    /// The MISSION RESULTS form hidden (F-40042 -> F-75010): every mission with bShouldUpdate
    /// and the byte loses it.
    pub fn clear_mission_updates(&self) {
        for (i, u) in self.updated.borrow_mut().iter_mut().enumerate() {
            if self.missions.get(i).map_or(false, |m| m.should_update) {
                *u = false;
            }
        }
    }

    // ---- what `mc4` reads and writes ----

    pub fn mission_count(&self) -> usize {
        self.missions.len()
    }

    pub fn mission_no(&self, id: &str) -> Option<usize> {
        self.index.get(&id.to_ascii_uppercase()).copied()
    }

    pub fn mission_id(&self, i: usize) -> &str {
        &self.missions[i].id
    }

    pub fn stage_of(&self, i: usize) -> Stage {
        self.state[i].stage
    }

    /// The open missions in the order they entered the mission log (list 1).
    pub fn log_order(&self) -> &[usize] {
        &self.order
    }

    /// The unhidden missions of the mission log (list 1: CHECKING_OBJECTIVES or
    /// READY_TO_TURN_IN), in its order.
    fn reminder_list(&self) -> Vec<usize> {
        self.order.iter().copied().filter(|&i| self.state[i].stage == Stage::Active && matches!(self.state_id_of(i), 3 | 4) && !self.missions[i].hidden).collect()
    }

    /// F-12937's countdown, run on the frames its gates let through (the caller's: offline,
    /// police activity <= 1, no warp, no cine script, Cruising_Offline, no reminder waiting):
    /// with an unhidden mission in the log the timer (+84) goes down by `dt`; below 0 it is
    /// drawn again from the career generator, min (+88, 400) + (max (+92, 500) - min) x
    /// (lo & 0x7FFFFF) x 2^-23, and the (+80 mod n)-th unhidden logged mission is picked, +80
    /// then counting on. The mission (index) and its reminder id.
    pub fn reminder_tick(&mut self, dt: f32) -> Option<(usize, String)> {
        let list = self.reminder_list();
        if list.is_empty() {
            return None;
        }
        let t = self.reminder_t.unwrap_or(500.0) - dt;
        self.reminder_t = Some(t);
        if !(t < 0.0) {
            return None;
        }
        let r = crate::oppick::career_rng().next();
        self.reminder_t = Some((100.0f32 * (r & 0x7F_FFFF) as f32).mul_add(1.0 / 8_388_608.0, 400.0));
        let k = (self.reminder_rr as i32).wrapping_rem(list.len() as i32);
        let i = list[k.max(0) as usize];
        self.reminder_rr = self.reminder_rr.wrapping_add(1);
        // (F-88990: no reminder -> "Invalid_Reminder")
        Some((i, self.missions[i].reminder.clone().unwrap_or_else(|| "Invalid_Reminder".into())))
    }

    /// The career's load script (`game/career/0x62e9486d` 0x74c, after loadSuccess): with a
    /// mission in the log, mcMissionManager_SetReminderCountdown(5.0) (F-41212: +84).
    pub fn reminder_after_load(&mut self) {
        if self.order.iter().any(|&i| self.state[i].stage == Stage::Active && matches!(self.state_id_of(i), 3 | 4)) {
            self.reminder_t = Some(5.0);
        }
    }

    /// The mission's caller (its hookman's name) for the phone card.
    pub fn caller_of(&self, i: usize) -> String {
        self.who(i)
    }

    /// The mission log's order from a save: the missions named that are open, in that
    /// order, then the other open ones as they were.
    pub fn set_log_order(&mut self, ids: &[String]) {
        let mut out: Vec<usize> = ids.iter().filter_map(|id| self.mission_no(id)).filter(|&i| self.state[i].stage == Stage::Active).collect();
        out.dedup();
        for &i in &self.order {
            if !out.contains(&i) {
                out.push(i);
            }
        }
        self.order = out;
    }

    /// The completed list from a save: the missions named that are complete, in that order,
    /// then the other complete ones in the order of the mission lists (the port's own rule:
    /// nothing tells when they were done).
    pub fn set_done_order(&mut self, ids: &[String]) {
        let mut out: Vec<usize> = Vec::new();
        for id in ids {
            if let Some(i) = self.mission_no(id).filter(|&i| self.state[i].stage == Stage::Complete) {
                if !out.contains(&i) {
                    out.push(i);
                }
            }
        }
        let mut rest: Vec<usize> = (0..self.missions.len()).filter(|&i| self.state[i].stage == Stage::Complete && !out.contains(&i)).collect();
        rest.sort_by_key(|&i| (self.missions[i].index < 0, self.missions[i].index, self.missions[i].id.clone()));
        out.extend(rest);
        self.done = out;
    }

    /// A mission's requirements from its save record: the objectives' +118, the baselines.
    /// A load: a prerequisite's TallyMission LessThan verdict is the record's (+40, F-78825).
    pub fn tally_from_save(&mut self, i: usize, met: &[bool]) {
        if !self.ext_ok(i) {
            return;
        }
        for (k, m) in met.iter().enumerate().take(self.ext[i].pre_tally.len()) {
            if matches!(self.missions[i].prereq[k].conds.first(), Some(Cond::TallyMission { less_than: true, .. })) {
                self.ext[i].pre_tally[k] = *m;
            }
        }
    }

    pub fn ext_from_save(&mut self, i: usize, obj_hidden: &[bool], obj_base: &[Option<i64>], pre_base: &[Option<i64>]) {
        if !self.ext_ok(i) {
            return;
        }
        let e = &mut self.ext[i];
        for (k, h) in obj_hidden.iter().enumerate().take(e.obj_hidden.len()) {
            e.obj_hidden[k] = *h;
        }
        for (k, b) in obj_base.iter().enumerate().take(e.obj_base.len()) {
            if b.is_some() {
                e.obj_base[k] = *b;
                e.obj_pending[k] = false;
            }
        }
        for (k, b) in pre_base.iter().enumerate().take(e.pre_base.len()) {
            if b.is_some() {
                e.pre_base[k] = *b;
            }
        }
    }

    /// A load: an open mission's String objectives take the record's verdict (`satisfied`,
    /// +116: a met one is done - e.g. a story hookman already met); those that are not met
    /// carry nothing over - the event's count (series wins, tournament points) lives in its
    /// script, which the save does not hold (the event starts again).
    pub fn drop_event_progress(&mut self, i: usize, satisfied: &[bool]) {
        let m = self.missions[i].clone();
        let mut dropped = false;
        for (k, r) in m.objectives.iter().enumerate() {
            let Some(c @ Cond::Event { .. }) = r.conds.first() else { continue };
            let t = self.target(&m, c);
            if let Some(p) = self.state[i].progress.get_mut(k) {
                if satisfied.get(k).copied().unwrap_or(false) {
                    *p = (*p).max(t);
                } else if *p > 0 && *p < t {
                    *p = 0;
                    dropped = true;
                }
            }
        }
        // (a series' or tournament's standing: `pts`, with the races it has had)
        if dropped || !self.state[i].pts.is_empty() {
            self.state[i].won.clear();
            self.state[i].pts.clear();
        }
    }

    /// Mission `i` as the save holds it.
    pub fn save_view(&self, i: usize) -> MissionSave {
        let m = &self.missions[i];
        let st = &self.state[i];
        let ext = self.ext.get(i).filter(|_| self.ext_ok(i));
        let stage = st.stage;
        let objectives = m
            .objectives
            .iter()
            .enumerate()
            .map(|(k, r)| {
                let c = r.conds.first();
                let target = c.map_or(1, |c| self.target(m, c));
                let cur = st.progress.get(k).copied().unwrap_or(0);
                let met = match stage {
                    Stage::Complete => true,
                    Stage::Active => cur >= target,
                    _ => false,
                };
                // (a met condition's value as it was when it was met: step 20b)
                let value = match c {
                    Some(Cond::Stat { stat, .. }) => Some(ext.and_then(|e| e.obj_last[k]).unwrap_or_else(|| self.stats.get(stat).copied().unwrap_or(0))),
                    _ => None,
                };
                ReqSave {
                    kinds: r.conds.iter().map(cond_kind).collect(),
                    hidden: ext.map_or_else(|| Self::default_hidden(stage, r.hidden), |e| e.obj_hidden[k]),
                    met,
                    count: if met { target } else { cur.min(target) },
                    base: ext.and_then(|e| e.obj_base[k]),
                    value,
                }
            })
            .collect();
        let prerequisites = m
            .prereq
            .iter()
            .enumerate()
            .map(|(k, r)| {
                let met = stage != Stage::Locked && self.prereq_req_met(i, k);
                // (a TallyMission condition counts completions only without the LessThan
                // heuristic - F-12302; not before it is armed)
                let count = match r.conds.first() {
                    Some(Cond::TallyMission { less_than: false, district, .. }) if stage != Stage::Locked => self.missions.iter().zip(self.state.iter()).filter(|(m, s)| s.stage == Stage::Complete && same_district(district, &m.district)).count() as u32,
                    _ => 0,
                };
                ReqSave {
                    kinds: r.conds.iter().map(cond_kind).collect(),
                    hidden: true,
                    met,
                    count,
                    base: ext.and_then(|e| e.pre_base[k]),
                    value: match r.conds.first() {
                        Some(Cond::Stat { stat, .. }) => Some(ext.and_then(|e| e.pre_last[k]).unwrap_or_else(|| self.stats.get(stat).copied().unwrap_or(0))),
                        _ => None,
                    },
                }
            })
            .collect();
        MissionSave { id: m.id.clone(), index: m.index, addon: m.addon, stage, state_id: self.state_id_of(i), objectives, prerequisites }
    }
}

// ---------------------------------------------------------------------------------------
// step 21b: what the missions' other scripts do - scenes, the garage's mission, the GPS
// ---------------------------------------------------------------------------------------

/// The story scenes a mission's CORE script plays when that mission is complete: (the
/// mission, the mission whose scene it is - `Cutscenes::for_mission`). Read from the CORE
/// scripts' START_NEW_SCRIPT_WITH_ARGS of `Game/CineScripts/generated/story/cut_story_*`:
/// RACE_GH_02_CORE 0x4781 (after the race, the mission complete: 0x42ca), FIND_SH_01_CORE
/// 0x8de, FIND_SH_02_CORE 0x5e2, FIND_SH_03_CORE 0x5fb, FIND_SH_03B_CORE 0xa42,
/// FIND_SH_01B_CORE 0xc75 (night 0x10f1), BEAT_SH_04_BCLASS_CORE 0x3d6b. Each opens the
/// mission it introduces, which passes READY_TO_START by (bAutoStart or a reward with
/// CHECKING_OBJECTIVES) and so has no call of its own.
pub const SCENE_AFTER: [(&str, &str); 7] = [
    ("RACE_GH_02", "REP_ON_HW_STRIP"),
    ("FIND_SH_01", "FIND_SH_04"),
    ("FIND_SH_02", "HANGOUT_PINKSLIP_HILLS"),
    ("FIND_SH_03", "HANGOUT_SERIES_HOLLYWOODC"),
    ("FIND_SH_03B", "RACES_IN_BEACHES"),
    ("FIND_SH_01B", "DISTRICT_CHAMPS"),
    ("BEAT_SH_04_BCLASS", "MOTORCYCLE_REP"),
];

/// The missions whose story scene is not played by their READY_TO_START_ON_ENTER script
/// (the call): a CORE at a completion (`SCENE_AFTER`), a CORE at the meeting
/// (`STORY_POINTS`), or an event script at a garage / hangout (`location_enter`). Their call,
/// where they have one, is a phone call. (Step 23: FIND_SH_02C's CORE plays its scene at its
/// place, after the money box - `mc4_find02c_meet`.)
pub const SCENE_ELSEWHERE: [&str; 14] = ["FIND_SH_02C", "REP_ON_HW_STRIP", "FIND_SH_04", "HANGOUT_PINKSLIP_HILLS", "HANGOUT_SERIES_HOLLYWOODC", "RACES_IN_BEACHES", "DISTRICT_CHAMPS", "MOTORCYCLE_REP", "CITY_CHAMP", "BESTOF_SH_05", "BEAT_SH_02_EXOTIC", "GARAGE_HOLLYWOOD", "HANGOUT_SERIES_DOWNTOWN", "HANGOUT_PINKSLIP_DOWNTOWN"];

/// An event script of a mission (aEvents, bConsume): (the mission, its event, the state it
/// listens in, the mission whose scene it plays, the state it moves the mission to).
/// REDLIGHT_GH_01_GARAGE_ON_ENTER: the garage's intro (CIN_01's Camaro), the entry scene,
/// hw_garage_state 5, mcMission_ChangeState(REDLIGHT_GH_01, 3) (0x6b5);
/// HANGOUT_SERIES_DOWNTOWN_HANGOUT_ON_ENTER (0x5ca) and HANGOUT_PINKSLIP_DOWNTOWN_HANGOUT_ON_ENTER
/// (0x602, then the story race) play their scene in place of the hangout's entry scene;
/// GARAGE_BEACHES_PAYBACK_ON_ENTER (a career script) the payback's intro (0x6a0).
pub const EVENT_SCENES: [(&str, &str, u8, &str, u8); 4] = [
    ("REDLIGHT_GH_01", "HW_GARAGE_ENTER", 2, "GARAGE_HOLLYWOOD", 3),
    ("HANGOUT_SERIES_DOWNTOWN", "DT_LOCAL_HANGOUT1_ENTER", 3, "HANGOUT_SERIES_DOWNTOWN", 3),
    ("HANGOUT_PINKSLIP_DOWNTOWN", "DT_WAGER_HANGOUT1_ENTER", 3, "HANGOUT_PINKSLIP_DOWNTOWN", 3),
    ("GARAGE_BEACHES", "BE_PAYBACK_ENTER", 3, "PAYBACK_INTRO", 3),
];

impl Career {
    /// A mission without bAutoStart and without a READY_TO_START_ON_ENTER script that
    /// listens for an event in READY_TO_START (REDLIGHT_GH_01: HW_GARAGE_ENTER) waits there.
    fn waits_for_event(&self, i: usize) -> bool {
        let m = &self.missions[i];
        !m.auto_start && !m.on_enter && !m.hidden && m.events_at.iter().any(|e| e.1 == 2)
    }

    /// A location's event (`<LOCATION>_ENTER`) is posted: the event script of a mission that
    /// listens for it in the state it is in runs (`EVENT_SCENES`) - every time (step 22b: the
    /// event scripts have no guard of their own - HANGOUT_PINKSLIP_DOWNTOWN_HANGOUT_ON_ENTER and
    /// HANGOUT_SERIES_DOWNTOWN_HANGOUT_ON_ENTER set the hangout's state 1 and go straight to
    /// their scene, 0x2e5 / 0x300; REDLIGHT_GH_01's leaves READY_TO_START with it). Returns the
    /// mission whose scene plays. Downtown's pink slip then runs its race (`dt_pink_due`).
    pub fn location_enter(&mut self, event: &str) -> Option<String> {
        for (id, ev, st, scene, to) in EVENT_SCENES {
            if ev != event {
                continue;
            }
            let Some(i) = self.mission_no(id) else { continue };
            if self.state[i].stage != Stage::Active || self.ext.get(i).map_or(3, |e| e.state) != st || !self.missions[i].events_at.iter().any(|e| e.0 == ev && e.1 == st) {
                continue;
            }
            // (the event is posted once a visit: at the drive-in - the port's pull-in test runs
            // on while the car stands there)
            if !self.visit_events.insert(ev.to_string()) {
                continue;
            }
            if id == "HANGOUT_PINKSLIP_DOWNTOWN" {
                self.dt_pink_due = true;
            }
            if id == "HANGOUT_SERIES_DOWNTOWN" {
                self.dt_series_story = true;
            }
            if to == 3 && st == 2 {
                self.enter_checking(i);
            }
            log::info!("career: {} at {} plays {}", id, ev, scene);
            // (GARAGE_BEACHES_PAYBACK_ON_ENTER posts its objective once the payback's intro is
            // over - 0x6a0; here as the scene is started)
            if id == "GARAGE_BEACHES" {
                self.mission_event("GARAGE_BEACHES_OBJ_01_COMPLETE");
            }
            return Some(scene.to_string());
        }
        None
    }

    /// The mission's state number is at least `n` (complete counts as 5).
    pub fn state_at_least(&self, id: &str, n: u32) -> bool {
        self.mission_no(id).map_or(false, |i| self.state_id_of(i) >= n)
    }

    /// The mission's story scene is played by another script than its call (`SCENE_ELSEWHERE`).
    pub fn scene_elsewhere(id: &str) -> bool {
        SCENE_ELSEWHERE.contains(&id)
    }

    /// The call a mission's reward script makes (mcRewardScript, bGiveImmediately): the line it
    /// appends and the title key, each a type-1 call of the mechanic's (`mec_mc_01_set`) or GH_27's,
    /// with no prompt, after the script's wait for Cruising_Offline and the police (activity > 1).
    /// DELIVERY_CONFIRM: SIDEKICK_DELIVERY_POST (0x8c..0x11a). BESTOF_GH_27_POST (sc):
    /// SIDEKICK_BESTOF_GH_27_POST, then it waits for the announcement. INSTALL_ABILITY_CONFIRM:
    /// the player's active ability (Racer_GetActiveAbility, 0x0d0) picks SIDEKICK_ZONE / _AGRO /
    /// _ROAR / _EMP (0 .. 3); with none it splashes INSTALL_ABILITY_CONFIRM_SPLASH for 2 s instead
    /// (0x1f8..0x222): `None` here. `ability` is the index in that order.
    pub fn reward_call(script: &str, ability: Option<usize>) -> Option<(&'static str, &'static str)> {
        match script.to_ascii_uppercase().as_str() {
            "DELIVERY_CONFIRM" => Some(("SIDEKICK_DELIVERY_POST", "DELIVERY_POST_TITLE")),
            "BESTOF_GH_27_POST" => Some(("SIDEKICK_BESTOF_GH_27_POST", "SIDEKICK_BESTOF_GH_27_POST_TITLE")),
            "INSTALL_ABILITY_CONFIRM" => ["SIDEKICK_ZONE", "SIDEKICK_AGRO", "SIDEKICK_ROAR", "SIDEKICK_EMP"].get(ability?).map(|l| (*l, "INSTALL_ABILITY_CONFIRM_TITLE")),
            // (`<d>_unlock_time_trial.sco` 0xa4..0x143: the district's time trial man calls before the
            // unlock; the unlock and the hookman's script are COMPLETE_SCRIPTS / mc4_tt_apply_block)
            "HW_UNLOCK_TIME_TRIAL" => Some(("SIDEKICK_TIMETRIAL_HOLLYWOOD_POST", "SIDEKICK_TIMETRIAL_HOLLYWOOD_POST_TITLE")),
            "BE_UNLOCK_TIME_TRIAL" => Some(("SIDEKICK_TIMETRIAL_BEACHES_POST", "SIDEKICK_TIMETRIAL_BEACHES_POST_TITLE")),
            "HL_UNLOCK_TIME_TRIAL" => Some(("SIDEKICK_TIMETRIAL_HILLS_POST", "SIDEKICK_TIMETRIAL_HILLS_POST_TITLE")),
            "DT_UNLOCK_TIME_TRIAL" => Some(("SIDEKICK_TIMETRIAL_DOWNTOWN_POST", "SIDEKICK_TIMETRIAL_DOWNTOWN_POST_TITLE")),
            "SC_UNLOCK_TIME_TRIAL" => Some(("SIDEKICK_TIMETRIAL_SC_POST", "SIDEKICK_TIMETRIAL_SC_POST_TITLE")),
            _ => None,
        }
    }

    /// The line a mission's call speaks and shows: its READY_TO_START_ON_ENTER script appends
    /// `SIDEKICK_<mission>` - except BEAT_SH_02_EXOTIC's, which appends FIND_SH_02B's
    /// (`SIDEKICK_FIND_SH_02B` at 0x23), and RACE_SH_01's, which appends RACE_GH_02's: the lines
    /// with a voice (CareerScript.xml). Test call_lines_are_the_scripts.
    pub fn call_line(key: &str) -> &str {
        match key {
            "SIDEKICK_BEAT_SH_02_EXOTIC" => "SIDEKICK_FIND_SH_02B",
            "SIDEKICK_RACE_SH_01" => "SIDEKICK_RACE_GH_02",
            k => k,
        }
    }
}

/// AddReward F-64223 on the pending list: a reward already in it is ignored; with
/// bGiveImmediately it is to be given now (true), else it is appended.
/// An entry of the reward manager's pending list.
#[derive(Clone, Debug, PartialEq)]
pub enum Pend {
    /// a mission's reward (mission, reward)
    Mission(usize, usize),
    /// the race's money (`Career::pend_money`)
    Money,
    /// the race's experience: Progression (false) / Unlocking (true) (`Career::pend_exp`)
    Exp(bool),
    /// a class threshold's PartUnlock: (class, body part type)
    Part(String, String),
    /// a Progression / Unlocking threshold's reward (entry of `Career::thresholds`)
    Threshold(usize),
}

pub fn reward_add<T: PartialEq>(list: &mut Vec<T>, r: T, give_now: bool) -> bool {
    if list.contains(&r) {
        return false;
    }
    if give_now {
        return true;
    }
    list.push(r);
    false
}

/// The REWARDS getter F-22948: any pending reward whose bHidden is clear.
pub fn reward_shown<T>(list: &[T], hidden: impl Fn(&T) -> bool) -> bool {
    list.iter().any(|r| !hidden(r))
}

/// One racer record of the results' estimate F-23843 (the results block's 192-byte record and the racer it names).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EstRacer {
    /// +16: 0 the player, 2 an AI racer
    pub kind: i32,
    /// +32 the next checkpoint (count in the lap), +36 the lap
    pub next: i32,
    pub lap: i32,
    /// +40 the finish time in ms (0x7FFFFFFF not finished, -1 out)
    pub time: i32,
    /// +172
    pub place: i32,
    /// the car's chassis state word (+592; 1 = deactivated)
    pub state: i32,
    /// the racer's performance float (the no-finisher branch)
    pub perf: f32,
    /// the racer's +192 vector and its car's position
    pub p192: [f32; 3],
    pub pos: [f32; 3],
}

/// The not-finished time (0x7FFFFFFF) of the results.
pub const UNFINISHED: i32 = 0x7FFF_FFFF;

/// sqrt(dx dx + (dz dz + dy dy)) as the original fuses it (float steps).
fn est_dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (dx, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    // (the guest's fmadds: the fused result rounded to a double, then to a float)
    let fma = |a: f32, b: f32, c: f32| (a as f64).mul_add(b as f64, c as f64) as f32;
    fma(dx, dx, fma(dz, dz, dy * dy)).sqrt()
}

/// fctiwz as the guest does it: NaN -> i32::MIN, else saturated towards zero.
fn est_int(x: f32) -> i32 {
    if x.is_nan() { i32::MIN } else { x as i32 }
}

/// The results' estimate F-23843 for the racers who did not finish. `cps` the checkpoints (centres), `per_lap` their
/// count in a lap, `laps` the race's raw lap count, `grid` the race's grid object vector, `base` the race table's
/// base time when the results name a race (+0 != -1), `rng` the career manager's generator words. Online: nothing.
/// The reference is the finished player, else the finished racer placed first. With one: every AI racer not
/// finished and placed behind it gets ref + (1 - covered / total) x ref ms (covered from the grid along the
/// checkpoints to the next one times the lap + 1, plus the way to it; total = laps x (ref's way to the start +
/// the lap's path)); a non-AI or deactivated one -1. Without: the non-player placed first gets base -
/// (1 - perf) x -3000, every other non-player that + (generator % 1000) + place x 6000.
pub fn estimate_times(rec: &mut [EstRacer], cps: &[[f32; 3]], per_lap: i32, laps: i32, grid: [f32; 3], base: Option<i32>, rng: &mut (u32, u32), online: bool) {
    if online {
        return;
    }
    let n = rec.len();
    let mut best = 1000;
    let mut r: Option<usize> = None;
    for (i, e) in rec.iter().enumerate() {
        if e.time == UNFINISHED || e.time == -1 {
            continue;
        }
        if e.kind == 0 {
            r = Some(i);
            break;
        }
        if best > e.place {
            r = Some(i);
            best = e.place;
        }
    }
    let Some(r) = r else {
        let mut r: Option<usize> = None;
        for (i, e) in rec.iter().enumerate() {
            if e.kind != 0 && best > e.place {
                best = e.place;
                r = Some(i);
            }
        }
        let Some(r) = r else { return };
        if let Some(b) = base {
            let k = (1.0f32 - rec[r].perf) * -3000.0f32;
            rec[r].time = b.wrapping_sub(est_int(k));
        }
        for i in 0..n {
            if i == r || rec[i].kind == 0 {
                continue;
            }
            let t = (rng.0 as u64) * 0x5CDC_FAA7 + rng.1 as u64;
            *rng = (t as u32, (t >> 32) as u32);
            let x = (t as u32 & 0x7FFF_FFFF) as i32;
            rec[i].time = rec[r].time.wrapping_add(x % 1000).wrapping_add(rec[i].place.wrapping_mul(6000));
        }
        return;
    };
    let cp0 = cps[0];
    let mut way = est_dist(rec[r].p192, cp0);
    for j in 1..per_lap.max(0) as usize {
        way = est_dist(cps[j - 1], cps[j]) + way;
    }
    let total = laps.clamp(1, 100) as f32 * way;
    let ref_t = rec[r].time;
    let mut f31 = 0.0f32;
    for i in 0..n {
        if i == r || rec[i].time != UNFINISHED {
            continue;
        }
        let e = rec[i];
        if e.kind != 2 || e.state == 1 {
            rec[i].time = -1;
            continue;
        }
        if e.place <= rec[r].place {
            continue;
        }
        let mut f0;
        if e.next > 0 {
            let k = e.next as usize;
            f0 = est_dist(grid, cp0);
            for j in 1..k {
                f0 = est_dist(cps[j - 1], cps[j]) + f0;
            }
            f31 = e.lap.wrapping_add(1) as f32 * f0;
            f0 = est_dist(cps[k - 1], e.pos);
        } else {
            f0 = est_dist(e.p192, e.pos);
        }
        if f0 + f31 > total {
            f31 = 0.0;
            f0 = 0.0;
        }
        let c = f0 + f31;
        let t = (1.0f32 - c / total) * ref_t as f32;
        rec[i].time = est_int(t).wrapping_add(ref_t);
    }
}

/// The series' decision F-63112: bComplete and bFailure are first set to "0"; then the first racer slot (in slot
/// order) whose nRacerWins_<i> has reached nRacesToWin decides it - Some(true) (bComplete = "1") when that slot is
/// the player's (type 0), Some(false) (bFailure = "1") otherwise; None while nobody has.
pub fn series_decision(types: &[i32], wins: &[i32], need: i32) -> Option<bool> {
    types.iter().zip(wins).find(|(_, w)| **w >= need).map(|(t, _)| *t == 0)
}

/// A row entry of the series' results (F-23726's 24-byte entry): the record's type (+16), finish time (+40) and place
/// (+172), and the racer's nRacerWins_<i>.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeriesRow {
    pub kind: i32,
    pub time: i32,
    pub place: i32,
    pub wins: i32,
}

/// The rows' comparator F-33081 on the entry's score F-66331 (1 - place): the higher score first; equal scores: two AI
/// racers (type 2) -> -1 when the first's finish time is the larger, else 0; any other pair -1.
pub fn series_row_cmp(a: &SeriesRow, b: &SeriesRow) -> i32 {
    results_row_cmp(1i32.wrapping_sub(a.place), 1i32.wrapping_sub(b.place), (a.kind, a.time), (b.kind, b.time))
}

/// F-33081 on two entries' scores and their records' (type, finish time).
pub fn results_row_cmp(sa: i32, sb: i32, a: (i32, i32), b: (i32, i32)) -> i32 {
    if sa == sb {
        if a.0 == 2 && b.0 == 2 {
            if a.1 > b.1 { -1 } else { 0 }
        } else {
            -1
        }
    } else if sa > sb {
        -1
    } else {
        1
    }
}

/// A row entry of the tournament's results (F-26722's 24-byte entry): the record's type, finish time and place, and
/// the racer's nRacerPoints_<i> total.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TourRow {
    pub kind: i32,
    pub time: i32,
    pub place: i32,
    pub total: i32,
}

/// This race's points of a place: the career manager's table +4 * (93 + clamp(place - 1, 0, 16)).
pub fn place_points(table: &[i32; 17], place: i32) -> i32 {
    table[place.wrapping_sub(1).clamp(0, 16) as usize]
}

/// The Tournament_RESULTS rows' order (F-26722): the entries in record order, sorted by the CRT qsort with F-33081
/// on the tournament entry's score F-91345 = this race's points (not the total). Row i shows fs_text3 = this race's
/// points, fs_text4 = the total, fs_property1 22 for the player (else 1).
pub fn tournament_rows(rows: &[TourRow], table: &[i32; 17]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..rows.len()).collect();
    crate::racemark::crt_qsort(&mut idx, |a, b| {
        let (ra, rb) = (&rows[*a], &rows[*b]);
        results_row_cmp(place_points(table, ra.place), place_points(table, rb.place), (ra.kind, ra.time), (rb.kind, rb.time))
    });
    idx
}

/// The Series_RESULTS rows' order (F-23726): the entries in record order, sorted by the CRT qsort (F-79639) with
/// F-33081. Returns the record index of each row; row i shows position i + 1, "<wins> / <nRacesToWin>" and the
/// player's property 22 (else 1).
pub fn series_rows(rows: &[SeriesRow]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..rows.len()).collect();
    crate::racemark::crt_qsort(&mut idx, |a, b| series_row_cmp(&rows[*a], &rows[*b]));
    idx
}

/// What a tournament's results decide (F-66541).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TourDecision {
    /// not the last race: nothing set
    Open,
    /// bComplete = "1" and TOURNAMENT_COMPLETE posted, TOURNAMENT_COMPLETE_HARD as well when `hard`
    Complete { hard: bool },
    /// bFailure = "1"
    Failure,
}

/// The tournament decision F-66541: on the last race (`index + 1 >= total`) the tournament is won when no
/// racer slot 1..n has more points than the player's slot 0 (a tie goes to the player; one racer or none: won),
/// else lost; HARD when the results' difficulty (+3100) is 2. Before the last race nothing is set.
pub fn tournament_decision(index: i32, total: i32, pts: &[i32], difficulty: i32) -> TourDecision {
    if index.wrapping_add(1) < total {
        return TourDecision::Open;
    }
    let ahead = pts.iter().skip(1).filter(|&&p| pts[0] < p).count();
    if ahead == 0 {
        TourDecision::Complete { hard: difficulty == 2 }
    } else {
        TourDecision::Failure
    }
}

/// The points of a tournament race (F-31368) on (type, place) per racer slot: on a replay the player is put
/// last first (F-87120); then per slot, the player's slot put last when it is damaged out, and the slot's
/// points += `table[clamp(place - 1, 0, 16)]` (32-bit words; the career manager's table +4 * 93..).
pub fn tournament_points(rec: &mut [(i32, i32)], damaged: &[bool], replay: bool, table: &[i32; 17], pts: &mut [i32]) {
    if replay {
        put_player_last(rec);
    }
    for i in 0..rec.len() {
        if rec[i].0 == 0 && damaged[i] {
            put_player_last(rec);
        }
        let k = rec[i].1.wrapping_sub(1).clamp(0, 16) as usize;
        pts[i] = pts[i].wrapping_add(table[k]);
    }
}

/// The race winner of a series' results (F-37351) on (type, place) per racer slot: on a replay the player is put
/// last first (F-87120); then per slot the player's slot put last when it is damaged out, and the slot with the
/// lowest place below count + 1 (the first of equal ones) wins - its nRacerWins_<i> gets one more.
pub fn series_winner(rec: &mut [(i32, i32)], damaged: &[bool], replay: bool) -> Option<usize> {
    if replay {
        put_player_last(rec);
    }
    let mut best = (rec.len() as i32).wrapping_add(1);
    let mut win = None;
    for i in 0..rec.len() {
        if rec[i].0 == 0 && damaged[i] {
            put_player_last(rec);
        }
        if rec[i].1 < best {
            win = Some(i);
            best = rec[i].1;
        }
    }
    win
}

/// A series' or tournament's results putting the player last (F-87120), on (type, place) per racer
/// (type 0 = the player): the first player's place becomes the racer count, and every other non-player
/// racer placed behind the player's old place (-1 if there is no player) moves up one.
pub fn put_player_last(rec: &mut [(i32, i32)]) {
    let n = rec.len() as i32;
    let old = match rec.iter_mut().find(|r| r.0 == 0) {
        Some(r) => std::mem::replace(&mut r.1, n),
        None => -1,
    };
    for r in rec.iter_mut() {
        if r.0 != 0 && r.1 > old {
            r.1 -= 1;
        }
    }
}

/// A Stat condition's progress (F-13232) from the statistic's word `v`: a Purse condition's `v`
/// as it is, any other `v - base` (32-bit) cut to 0 ..= `target`.
pub fn stat_progress(purse: bool, v: i32, base: i32, target: i32) -> i32 {
    if purse {
        return v;
    }
    let d = v.wrapping_sub(base);
    if d < 0 {
        0
    } else if d > target {
        target
    } else {
        d
    }
}

/// F-20532's value: f32(done) / f32(total) x 100.0, fctiwz (0 with nothing counted).
pub fn percent_of(done: usize, total: usize) -> i64 {
    if total == 0 {
        return 0;
    }
    let f = done as i32 as f32 / total as i32 as f32;
    (f * 100.0) as i32 as i64
}

#[cfg(test)]
mod tests {
    /// The three reward scripts that make a call append `reward_call`'s line and title (the
    /// disassemblies of `work/career2/re/dis`, MCLA_SCRIPT_DIS to override), and the missions'
    /// files name them as mcRewardScript.
    #[test]
    fn reward_calls_are_the_scripts() {
        assert_eq!(Career::reward_call("INSTALL_ABILITY_CONFIRM", None), None);
        assert_eq!(Career::reward_call("Police_Tutorial", Some(0)), None);
        let dir = std::env::var("MCLA_SCRIPT_DIS").unwrap_or_else(|_| "D:/MCLA/work/career2/re/dis".to_string());
        let strings = |f: &str| std::fs::read_to_string(format!("{dir}/{f}")).ok().map(|t| t.lines().filter_map(|l| l.split("string '").nth(1)).map(|v| v.trim_end_matches('\'').to_string()).collect::<Vec<_>>());
        let Some(dc) = strings("game__career__events__DELIVERY_CONFIRM.sco.txt") else { return };
        let (line, title) = Career::reward_call("DELIVERY_CONFIRM", None).unwrap();
        assert!(dc.iter().any(|v| v == line) && dc.iter().any(|v| v == title));
        let gh = strings("sc__career__events__BESTOF_GH_27_POST.sco.txt").unwrap();
        let (line, title) = Career::reward_call("BESTOF_GH_27_POST", None).unwrap();
        assert!(gh.iter().any(|v| v == line) && gh.iter().any(|v| v == title));
        let ia = strings("game__career__events__INSTALL_ABILITY_CONFIRM.sco.txt").unwrap();
        let lines: Vec<&String> = ia.iter().filter(|v| v.starts_with("SIDEKICK_")).collect();
        for (k, l) in lines.iter().enumerate() {
            assert_eq!(Career::reward_call("INSTALL_ABILITY_CONFIRM", Some(k)).map(|c| c.0), Some(l.as_str()));
        }
        assert_eq!(lines.len(), 4);
        assert!(ia.iter().any(|v| v == "INSTALL_ABILITY_CONFIRM_TITLE"));
        for (f, d) in [("game__career__hw_unlock_time_trial", "HW"), ("game__career__be_unlock_time_trial", "BE"), ("game__career__hl_unlock_time_trial", "HL"), ("game__career__dt_unlock_time_trial", "DT"), ("sc__career__sc_unlock_time_trial", "SC")] {
            let v = strings(&format!("{f}.sco.txt")).unwrap();
            let (line, title) = Career::reward_call(&format!("{d}_UNLOCK_TIME_TRIAL"), None).unwrap();
            assert!(v.iter().any(|x| x == line) && v.iter().any(|x| x == title), "{f}");
        }
        // (with the game's data: the three missions carry the call and each line has its text; the
        // three title keys are in none of the string tables - what the phone shows then is open)
        let data = std::env::var("MCLA_DATA").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        let c = Career::load(&data);
        if c.missions.is_empty() {
            return;
        }
        let calls: Vec<&str> = c.missions.iter().flat_map(|m| m.rewards.iter()).filter_map(|r| if let Reward::Call(s) = r { Some(s.as_str()) } else { None }).collect();
        assert_eq!(calls.len(), 8, "{calls:?}");
        for s in ["DELIVERY_CONFIRM", "BESTOF_GH_27_POST", "INSTALL_ABILITY_CONFIRM", "HW_UNLOCK_TIME_TRIAL", "BE_UNLOCK_TIME_TRIAL", "HL_UNLOCK_TIME_TRIAL", "DT_UNLOCK_TIME_TRIAL", "SC_UNLOCK_TIME_TRIAL"] {
            assert!(calls.contains(&s), "{s}");
            for k in 0..4 {
                let (line, _title) = Career::reward_call(s, Some(k)).unwrap();
                assert!(c.strings.get(line).is_some(), "{line}");
            }
        }
        assert!(c.strings.get("INSTALL_ABILITY_CONFIRM_SPLASH").is_some());
    }

    /// The objectives' nTargetId and mcMissionManager_DoesUIDHaveActiveMission: the hangout's
    /// uid 500 (hangout_series_hollywood.xml) once its mission checks objectives.
    #[test]
    fn uid_active_mission() {
        let data = std::env::var("MCLA_DATA").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        let mut c = Career::load(&data);
        let Some(&i) = c.index.get("HANGOUT_SERIES_HOLLYWOOD") else { return };
        assert_eq!(c.missions[i].objectives[0].target_id, 500);
        assert!(!c.uid_has_active_mission(500));
        c.activate(i);
        assert!(matches!(c.state_id_of(i), 2 | 3), "{}", c.state_id_of(i));
        assert!(c.uid_has_active_mission(500));
        assert!(!c.uid_has_active_mission(501));
    }

    /// The reminders: reminder[0] of the file (the three district championships share
    /// REMINDER_DISTRICT_CHAMPS), the 400..500 s countdown and the round robin.
    #[test]
    fn reminders() {
        let data = std::env::var("MCLA_DATA").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        let mut c = Career::load(&data);
        let Some(&b) = c.index.get("DISTRICT_CHAMPS_BEACHES") else { return };
        assert_eq!(c.missions[b].reminder.as_deref(), Some("REMINDER_DISTRICT_CHAMPS"));
        let h = c.index["HANGOUT_SERIES_HOLLYWOOD"];
        assert_eq!(c.missions[h].reminder.as_deref(), Some("REMINDER_HANGOUT_SERIES_HOLLYWOOD"));
        assert_eq!(c.reminder_tick(1000.0), None);
        let i = c.missions.iter().position(|m| !m.hidden && m.reminder.is_some()).unwrap();
        c.activate(i);
        let id = c.missions[i].id.clone();
        c.call_delivered(&id);
        assert!(matches!(c.state_id_of(i), 3 | 4), "{} {}", id, c.state_id_of(i));
        assert_eq!(c.reminder_tick(499.0), None);
        let (k, r) = c.reminder_tick(2.0).unwrap();
        assert_eq!((k, r.as_str()), (i, c.missions[i].reminder.as_deref().unwrap()));
        let t = c.reminder_t.unwrap();
        assert!((400.0..=500.0).contains(&t), "{t}");
        c.reminder_after_load();
        assert_eq!(c.reminder_t, Some(5.0));
    }

    /// The Progression table's script reward without a call: Progression_24 (50 %) carries
    /// `unlock_unordered_races`, reached with its id.
    #[test]
    fn unordered_races_threshold() {
        let data = std::env::var("MCLA_DATA").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        let mut c = Career::load(&data);
        if c.missions.is_empty() {
            return;
        }
        let k: Vec<usize> = (0..c.thresholds.len()).filter(|&k| matches!(&c.thresholds[k].1, Reward::Script(n) if n == "unlock_unordered_races")).collect();
        assert_eq!(k.len(), 1);
        assert_eq!(c.threshold_ids[k[0]], "Progression_24");
        assert!(!c.script_reached("unlock_unordered_races"));
        c.reached_ids.insert("Progression_24".into());
        assert!(c.script_reached("unlock_unordered_races"));
    }

    /// Each READY_TO_START_ON_ENTER script's call line is `call_line` of its mission's key
    /// (the disassemblies of `work/career2/re/dis`, MCLA_SCRIPT_DIS to override).
    #[test]
    fn call_lines_are_the_scripts() {
        assert_eq!(Career::call_line("SIDEKICK_BEAT_SH_02_EXOTIC"), "SIDEKICK_FIND_SH_02B");
        let dir = std::env::var("MCLA_SCRIPT_DIS").unwrap_or_else(|_| "D:/MCLA/work/career2/re/dis".to_string());
        let Ok(rd) = std::fs::read_dir(&dir) else { return };
        let mut n = 0;
        for e in rd.flatten() {
            let f = e.file_name().to_string_lossy().to_string();
            let Some(id) = f.strip_prefix("game__career__events__").or_else(|| f.strip_prefix("sc__career__events__")).and_then(|m| m.strip_suffix("_READY_TO_START_ON_ENTER.sco.txt")) else { continue };
            let text = std::fs::read_to_string(e.path()).unwrap_or_default();
            let Some(line) = text.lines().filter_map(|l| l.split("string '").nth(1)).map(|v| v.trim_end_matches('\'')).find(|v| v.starts_with("SIDEKICK_") && *v != "SIDEKICK_TITLE_TIP") else { continue };
            assert_eq!(Career::call_line(&format!("SIDEKICK_{}", id.to_ascii_uppercase())), line, "{id}");
            n += 1;
        }
        assert!(n == 0 || n > 100, "{n} call scripts");
    }

    /// Every career tip script's key is the SIDEKICK string the script itself shows (the
    /// disassemblies of `work/career2/re/dis`, MCLA_SCRIPT_DIS to override; the four
    /// VisualPartUnlockTip keys are checked without them).
    #[test]
    fn tip_keys_are_the_scripts_strings() {
        assert_eq!(tip_key(r"game\Career\VisualPartUnlockTip_03"), "SIDEKICK_VISUAL_PART_TIP_03");
        assert_eq!(tip_key(r"game\Career\CarsUnlockTip_02"), "SIDEKICK_CARS_UNLOCK_TIP_02");
        let dir = std::env::var("MCLA_SCRIPT_DIS").unwrap_or_else(|_| "D:/MCLA/work/career2/re/dis".to_string());
        let Ok(rd) = std::fs::read_dir(&dir) else { return };
        let mut n = 0;
        for e in rd.flatten() {
            let f = e.file_name().to_string_lossy().to_string();
            let Some(name) = f.strip_prefix("game__career__").and_then(|m| m.strip_suffix(".sco.txt")).filter(|m| m.contains("Tip") && !m.contains("__")) else { continue };
            let text = std::fs::read_to_string(e.path()).unwrap_or_default();
            let key = tip_key(name);
            assert!(text.contains(&format!("string '{key}'")), "{name}: {key}");
            n += 1;
        }
        assert!(n == 0 || n >= 40, "{n} tip scripts");
    }

    use super::*;

    /// MISSION RESULTS' rules against the original's code run offline
    /// (`mcla-rs/tools/ui_mission_update_reference.py`): the collector F-65871 with the filter F-78851 of kind 4 over a
    /// manager of random missions, and the form's hide F-75010 (only the missions with bShouldUpdate and the byte lose it,
    /// with their requirements' +140 and conditions' +84); a requirement's listener F-64780 and the mission's F-30196 for every
    /// combination of message, condition, +116, bAllowUpdate, the mission's handle and membership.
    /// The pending reward list against the original's AddReward F-64223, getter F-22948, give-all
    /// F-73125 and clear F-77851 run offline (`tools/ui_reward_list_reference.py`).
    #[test]
    fn reward_list_matches_the_original() {
        let text = include_str!("testdata/ui_reward_list_ref.txt");
        let (mut cases, mut differ) = (0, 0);
        for line in text.lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let (head, ops) = input.split_once(" ; ").unwrap();
            let h: Vec<u32> = head.split_whitespace().skip(1).map(|v| u32::from_str_radix(v, 16).unwrap()).collect();
            let flags: Vec<(bool, bool)> = (0..h[0] as usize).map(|k| (h[1 + 2 * k] != 0, h[2 + 2 * k] != 0)).collect();
            let mut list: Vec<usize> = Vec::new();
            let mut got = Vec::new();
            for op in ops.split_whitespace() {
                match op.as_bytes()[0] {
                    b'a' => {
                        let k = usize::from_str_radix(&op[1..], 16).unwrap();
                        got.push(if reward_add(&mut list, k, flags[k].1) { format!("{k:x}") } else { ".".into() });
                    }
                    b'g' => got.push(if reward_shown(&list, |&k| flags[k].0) { "3".into() } else { "2".into() }),
                    b'p' => got.push(if list.is_empty() { ".".into() } else { list.iter().map(|k| format!("{k:x}")).collect() }),
                    _ => {
                        list.clear();
                        got.push("-".into());
                    }
                }
            }
            let mut out = got.join(" ");
            out.push_str(" |");
            for k in &list {
                out.push_str(&format!(" {k:x}"));
            }
            cases += 1;
            if out.trim() != want.trim() {
                differ += 1;
                eprintln!("{input}: want {want} got {out}");
            }
        }
        eprintln!("reward list: {cases} cases, {differ} differ");
        assert!(cases >= 600 && differ == 0);
    }

    /// What the race listener F-48818 hands to the pending list (`rewards::race_split` + `Career::queue_race`): the
    /// entries, their amounts and flags, also when a second race is queued before the page.
    #[test]
    fn race_rewards_match_the_original() {
        let text = include_str!("testdata/ui_race_rewards_ref.txt");
        let (mut cases, mut differ) = (0, 0);
        for line in text.lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let v: Vec<u32> = input.split(" ; ").nth(1).unwrap().split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let mut c = Career::default();
            for k in v.chunks(8) {
                let (exp, money, racers, place, arcade, net, kind, editor) = (f32::from_bits(k[0]), f32::from_bits(k[1]), k[2] as usize, k[3] as usize, k[4] != 0, k[5] != 0, k[6], k[7] != 0);
                // (a race of the race editor: nothing; x 0.6 from ARCADE, else x 0.35 in a network game)
                if editor {
                    continue;
                }
                let factor = if arcade { 0.6 } else if net { 0.35 } else { 1.0 };
                let (m, e) = crate::rewards::race_split(exp, money, place, racers, factor, kind == 10);
                // (in a network game the experience goes to Unlocking alone)
                c.queue_race(m, (if net { 0 } else { e }, e));
            }
            // (all three wait: GiveImmediately 0 - `queue_race` appends them)
            let out: Vec<String> = c.pending.iter().map(|p| match p {
                Pend::Money => format!("M {:x} {} 0", c.pend_money as u32, c.pend_hidden(p) as u8),
                Pend::Exp(false) => format!("P {:x} {} 0", c.pend_exp[0] as u32, c.pend_hidden(p) as u8),
                Pend::Exp(true) => format!("U {:x} {} 0", c.pend_exp[1] as u32, c.pend_hidden(p) as u8),
                _ => "?".into(),
            }).collect();
            cases += 1;
            if out.join(" ") != want.trim() {
                differ += 1;
                eprintln!("{input}: want {want} got {}", out.join(" "));
            }
        }
        eprintln!("race rewards: {cases} cases, {differ} differ");
        assert!(cases >= 600 && differ == 0);
    }

    /// The threshold walk inside the pending list (F-61830 -> F-61974 -> F-64223, give-all F-73125), as `event`'s
    /// Ev::Rep gives a race's thresholds: in order, GiveImmediately at once, the others after.
    #[test]
    fn reward_thresholds_match_the_original() {
        let text = include_str!("testdata/ui_reward_threshold_ref.txt");
        let (mut cases, mut differ) = (0, 0);
        for line in text.lines() {
            let (input, want) = line.split_once(" -> ").unwrap();
            let mut part = input.split(" ; ");
            let h: Vec<u32> = part.next().unwrap().split_whitespace().skip(1).map(|v| u32::from_str_radix(v, 16).unwrap()).collect();
            let flags: Vec<(bool, u32)> = (0..h[0] as usize).map(|k| (h[2 + 3 * k] != 0, h[3 + 3 * k])).collect();
            // (each system: its share, then each threshold's Percent and rewards)
            let mut t = part.next().unwrap().split_whitespace();
            let ns = usize::from_str_radix(t.next().unwrap(), 16).unwrap();
            let mut systems: Vec<(f32, Vec<(f32, Vec<usize>)>)> = Vec::new();
            for _ in 0..ns {
                let share: f32 = t.next().unwrap().parse().unwrap();
                let m = usize::from_str_radix(t.next().unwrap(), 16).unwrap();
                let mut ths = Vec::new();
                for _ in 0..m {
                    let pct: f32 = t.next().unwrap().parse().unwrap();
                    let r = usize::from_str_radix(t.next().unwrap(), 16).unwrap();
                    ths.push((pct, (0..r).map(|_| usize::from_str_radix(t.next().unwrap(), 16).unwrap()).collect()));
                }
                systems.push((share, ths));
            }
            let ops = part.next().unwrap();
            let mut reached: Vec<Vec<bool>> = systems.iter().map(|s| vec![false; s.1.len()]).collect();
            let mut list: Vec<usize> = Vec::new();
            let mut given: Vec<usize> = Vec::new();
            fn walk(s: usize, systems: &[(f32, Vec<(f32, Vec<usize>)>)], reached: &mut [Vec<bool>], list: &mut Vec<usize>, given: &mut Vec<usize>, flags: &[(bool, u32)]) {
                for t in 0..systems[s].1.len() {
                    let (pct, ref rs) = systems[s].1[t];
                    if reached[s][t] || systems[s].0 < pct {
                        continue;
                    }
                    reached[s][t] = true;
                    for &k in rs {
                        if reward_add(list, k, flags[k].0) {
                            give(k, systems, reached, list, given, flags);
                        }
                    }
                }
            }
            fn give(k: usize, systems: &[(f32, Vec<(f32, Vec<usize>)>)], reached: &mut [Vec<bool>], list: &mut Vec<usize>, given: &mut Vec<usize>, flags: &[(bool, u32)]) {
                given.push(k);
                if flags[k].1 > 0 {
                    walk(flags[k].1 as usize - 1, systems, reached, list, given, flags);
                }
            }
            for op in ops.split_whitespace() {
                match op.as_bytes()[0] {
                    b'a' => {
                        let k = usize::from_str_radix(&op[1..], 16).unwrap();
                        if reward_add(&mut list, k, flags[k].0) {
                            give(k, &systems, &mut reached, &mut list, &mut given, &flags);
                        }
                    }
                    b'w' => walk(usize::from_str_radix(&op[1..], 16).unwrap(), &systems, &mut reached, &mut list, &mut given, &flags),
                    _ => {
                        let mut n = 0;
                        while n < list.len() {
                            give(list[n], &systems, &mut reached, &mut list, &mut given, &flags);
                            n += 1;
                        }
                    }
                }
            }
            let out = format!("{} | {}", given.iter().map(|k| format!("{k:x}")).collect::<Vec<_>>().join(" "), reached.iter().flatten().map(|&r| (r as u8).to_string()).collect::<Vec<_>>().join(" "));
            cases += 1;
            if out.split_whitespace().collect::<Vec<_>>() != want.split_whitespace().collect::<Vec<_>>() {
                differ += 1;
                eprintln!("{input}: want {want} got {out}");
            }
        }
        eprintln!("reward thresholds: {cases} cases, {differ} differ");
        assert!(cases >= 600 && differ == 0);
    }

    #[test]
    fn mission_updates_match_the_original() {
        let (mut n, mut bad) = ([0usize; 2], 0usize);
        for line in include_str!("testdata/ui_mission_update_ref.txt").lines() {
            let (case, out) = line.split_once(" -> ").unwrap();
            let v: Vec<u32> = case.split_whitespace().skip(1).map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let ok = if case.starts_with('L') {
                n[0] += 1;
                let ms: Vec<&[u32]> = v[1..].chunks(7).collect();
                let listed: Vec<String> = (0..ms.len()).filter(|&k| Career::mu_listed(ms[k][0], ms[k][1] != 0, ms[k][2] != 0, ms[k][3] != 0)).map(|k| format!("{:x}", k)).collect();
                // (F-75010: a mission with +691 and +690 loses its byte and its requirements' and conditions' marks)
                let after: Vec<String> = ms.iter().map(|m| if m[2] != 0 && m[3] != 0 { "0 0 0".to_string() } else { format!("{:x} {:x} {:x}", m[3], m[5], m[6]) }).collect();
                let want = format!("{} ; {}", listed.join(" "), after.join(" "));
                out.split_whitespace().collect::<Vec<_>>() == want.split_whitespace().collect::<Vec<_>>()
            } else {
                n[1] += 1;
                let b = |k: usize| v[k] != 0;
                let (r, m) = Career::mu_update(b(0), b(1), b(2), b(3), b(4), b(5));
                out == format!("{} {}", r as u8, m as u8)
            };
            if !ok {
                bad += 1;
                if bad <= 10 {
                    eprintln!("differs: {line}");
                }
            }
        }
        eprintln!("compared {n:?}, {bad} differ");
        assert!(n[0] > 0 && n[1] > 0);
        assert_eq!(bad, 0);
    }

    /// A won race in the Beaches moves RACES_IN_BEACHES' tally (its TallyRace posts Condition_Updated): the mission is on
    /// MISSION RESULTS once, with its line counted (1/6) and not ticked; the fill clears its byte (not complete), so the next
    /// form has nothing; a race elsewhere updates nothing.
    #[test]
    fn a_tally_moved_lists_its_mission_once() {
        let dir = std::env::var("MCLA_DATA").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        if !std::path::Path::new(&dir).join("tune/career").is_dir() {
            return;
        }
        let mut c = Career::load(&dir);
        c.new_game();
        let mut s = c.save();
        s.missions.insert("RACES_IN_BEACHES".into(), MState { stage: Stage::Active, ..Default::default() });
        c.restore(&s);
        let race = |d| Ev::Race { district: d, place: 1, hookman: false, class: "", race: "x", beat_goal: false };
        c.event(race("Hills"));
        assert!(c.mission_updates().is_empty());
        c.event(race("Beaches"));
        let v = c.take_mission_updates();
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(!v[0].2 && v[0].1.len() == 1 && v[0].1[0].ends_with("(1/6)") && !v[0].1[0].starts_with(crate::game::OBJ_DONE), "{v:?}");
        assert!(c.take_mission_updates().is_empty());
    }

    /// The mission log's Experience / TallyRace / TallyMission count against the original run offline
    /// (tools/count_text_reference.py: F-34957, F-62132, F-95375, 300 cases each): the whole text equal.
    #[test]
    fn count_text_matches_the_original() {
        let text = include_str!("testdata/count_text_ref.txt");
        let mut n = 0;
        for line in text.lines() {
            let (kind, rest) = line.split_once('|').unwrap();
            let (prefix, rest) = rest.split_once('|').unwrap();
            let (args, out) = rest.split_once(" -> ").unwrap();
            let v: Vec<i32> = args.split(' ').map(|x| x.parse().unwrap()).collect();
            let (done, count, target, base, found) = (v[0] != 0, v[1], v[2], v[3], v[4] != 0);
            let want = match kind {
                "0" if !found => prefix.to_string(),
                "0" => format!("{prefix} {}", count_text(done, count.wrapping_sub(base), target)),
                _ => format!("{prefix} {}", count_text(done, count, target)),
            };
            assert_eq!(out, want, "{line}");
            n += 1;
        }
        assert_eq!(n, 900);
    }

    /// The mission log's Stat count against the original run offline (tools/stat_text_reference.py: F-97850 with
    /// the money text F-13827, 10 type texts x 40 values / prefixes / languages, 400 cases): the whole text equal.
    #[test]
    fn stat_count_text_matches_the_original() {
        let text = include_str!("testdata/stat_text_ref.txt");
        let mut n = 0;
        for line in text.lines() {
            let (ty, rest) = line.split_once('|').unwrap();
            let (prefix, rest) = rest.split_once('|').unwrap();
            let (args, out) = rest.split_once(" -> ").unwrap();
            let v: Vec<i64> = args.split(' ').map(|x| x.parse().unwrap()).collect();
            let cur = if v[0] != 0 { v[2] } else { v[1] } as i32;
            let want = match stat_count_text(ty, cur, v[2] as i32, v[3] as usize) {
                Some(c) => format!("{prefix} {c}"),
                None => prefix.to_string(),
            };
            assert_eq!(out, want, "{line}");
            n += 1;
        }
        assert_eq!(n, 400);
    }

    /// The repeat missions' TallyMission LessThan prerequisite (F-12302): met when fewer than
    /// nTally (2) missions of the district are open - states 2 to 4 (mask 28) - checked at the
    /// arming (bInitValidation) and on each MISSION_COMPLETE, kept once met; completed missions
    /// do not count (the port had counted the completions).
    #[test]
    fn tally_mission_counts_the_open_missions() {
        let dir = std::env::var("MCLA_DATA").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        if !std::path::Path::new(&dir).join("tune/career").is_dir() {
            return;
        }
        let mut c = Career::load(&dir);
        let r = c.mission_no("REPEAT_BESTOF_HILLS").unwrap();
        let hills: Vec<usize> = (0..c.missions.len()).filter(|&j| j != r && c.missions[j].district == "Hills" && !c.missions[j].repeatable).take(3).collect();
        assert_eq!(hills.len(), 3);
        assert_eq!(district_no("SC"), 4);
        assert_eq!(district_no("South Central"), 4);
        assert_eq!(district_no("All"), 6);
        assert_eq!(district_no("Nowhere"), -1);
        let set = |c: &mut Career, open: &[usize], done: &[usize]| {
            let mut s = c.save();
            s.missions.clear();
            for &j in open {
                s.missions.insert(c.missions[j].id.clone(), MState { stage: Stage::Active, ..Default::default() });
            }
            for &j in done {
                s.missions.insert(c.missions[j].id.clone(), MState { stage: Stage::Complete, ..Default::default() });
            }
            c.restore(&s);
            c.state[r].stage = Stage::Armed;
            c.on_armed(r);
        };
        // two open: not met at the arming, though nothing of Hills is complete
        set(&mut c, &hills[..2], &[]);
        assert_eq!(c.tally_open("Hills"), 2);
        assert!(!c.prereq_req_met(r, 0));
        // one of them completes: MISSION_COMPLETE finds one open
        c.state[hills[0]].stage = Stage::Complete;
        c.mission_complete_event();
        assert!(c.prereq_req_met(r, 0));
        // kept when the count rises again before the mission steps on
        c.state[hills[0]].stage = Stage::Active;
        assert!(c.prereq_req_met(r, 0));
        // one open, two complete: met at the arming
        set(&mut c, &hills[..1], &hills[1..3]);
        assert!(c.prereq_req_met(r, 0));
        // the count is only checked at the arming and on MISSION_COMPLETE
        set(&mut c, &hills[..2], &[]);
        c.state[hills[0]].stage = Stage::Locked;
        assert_eq!(c.tally_open("Hills"), 1);
        assert!(!c.prereq_req_met(r, 0));
        // a load takes the record's verdict
        c.tally_from_save(r, &[true]);
        assert!(c.prereq_req_met(r, 0));
    }

    /// The district lookup F-67263 run offline on 1,936 strings (tools/district_lookup_reference.py:
    /// every name and short form in mixed case, the mission files' values, near misses, random
    /// strings): `district_no` gives the same number for each.
    /// F-20532 run offline (tools/percent_complete_reference.py, 3,000 mission sets of 0..140,
    /// hidden / repeatable flags, completed lists): the value posted and the kept fraction equal.
    #[test]
    fn percent_complete_matches_the_original() {
        let text = include_str!("testdata/percent_complete_ref.txt");
        let mut n = 0;
        for line in text.lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = a.split(' ').collect();
            let flags: Vec<u8> = if w[2] == "-" { vec![] } else { w[2].bytes().map(|c| c - b'0').collect() };
            let done: Vec<usize> = if w[3] == "-" { vec![] } else { w[3].split(',').map(|x| x.parse().unwrap()).collect() };
            let o: Vec<u32> = b.split(' ').map(|x| x.parse().unwrap()).collect();
            let counts = |i: usize| flags[i] == 0;
            let total = (0..flags.len()).filter(|&i| counts(i)).count();
            let k = done.iter().filter(|&&i| counts(i)).count();
            assert_eq!(o[0], 1, "{line}");
            assert_eq!(percent_of(k, total) as i32 as u32, o[1], "{line}");
            let frac = if total == 0 { 0.0f32 } else { k as i32 as f32 / total as i32 as f32 };
            assert_eq!(frac.to_bits(), o[2], "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    /// F-13232 run offline (tools/stat_progress_reference.py, 2,996 cases: the type spelled
    /// Purse / purse / PURSE / Purses / Money / Tally / empty, 32-bit values, baselines, targets).
    #[test]
    fn stat_progress_matches_the_original() {
        let text = include_str!("testdata/stat_progress_ref.txt");
        let mut n = 0;
        for line in text.lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = a.split(' ').collect();
            let num = |i: usize| w[i].parse::<i64>().unwrap() as i32;
            let purse = w[0] == "Purse";
            assert_eq!(stat_progress(purse, num(1), num(2), num(3)), b.parse::<i32>().unwrap(), "{line}");
            n += 1;
        }
        assert_eq!(n, 2996);
    }

    #[test]
    fn results_estimate_matches_the_original() {
        let text = include_str!("testdata/results_estimate_ref.txt");
        let v3 = |s: &str| -> [f32; 3] {
            let w: Vec<f32> = s.split(',').map(|x| x.parse().unwrap()).collect();
            [w[0], w[1], w[2]]
        };
        let mut n = 0;
        for line in text.lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = a.split(' ').collect();
            let num = |i: usize| w[i].parse::<i32>().unwrap();
            let (online, cnt, m, laps, race) = (num(0) != 0, num(1) as usize, num(2), num(3), num(4));
            let table: Vec<i32> = w[5].split(',').map(|x| x.parse().unwrap()).collect();
            let (g0, g1) = w[6].split_once(',').unwrap();
            let mut rng = (g0.parse::<u32>().unwrap(), g1.parse::<u32>().unwrap());
            let grid = v3(w[7]);
            let cps: Vec<[f32; 3]> = w[8].split(';').map(v3).collect();
            let mut rec: Vec<EstRacer> = w[9..].iter().filter(|s| **s != "-").map(|s| {
                let f: Vec<&str> = s.split(',').collect();
                let i = |k: usize| f[k].parse::<i32>().unwrap();
                let fl = |k: usize| f[k].parse::<f32>().unwrap();
                EstRacer { kind: i(0), next: i(1), lap: i(2), time: i(3), place: i(4), state: i(5), perf: fl(6),
                           p192: [fl(7), fl(8), fl(9)], pos: [fl(10), fl(11), fl(12)] }
            }).collect();
            assert_eq!(rec.len(), cnt);
            let base = if race == -1 { None } else { Some(table[race as usize]) };
            estimate_times(&mut rec, &cps, m, laps, grid, base, &mut rng, online);
            let (times, g) = b.split_once('|').unwrap();
            let want: Vec<i32> = times.split_whitespace().map(|x| x.parse().unwrap()).collect();
            assert_eq!(rec.iter().map(|e| e.time).collect::<Vec<_>>(), want, "{line}");
            let gw: Vec<u32> = g.split_whitespace().map(|x| x.parse().unwrap()).collect();
            assert_eq!((rng.0, rng.1), (gw[0], gw[1]), "{line}");
            n += 1;
        }
        assert_eq!(n, 2593);
    }

    #[test]
    fn tournament_decision_matches_the_original() {
        let text = include_str!("testdata/tournament_decision_ref.txt");
        let mut n = 0;
        for line in text.lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = a.split(' ').collect();
            let num = |i: usize| w[i].parse::<i32>().unwrap();
            let (null, ev, k) = (num(0), num(1), num(2) as usize);
            let pts: Vec<i32> = w[6].split(',').map(|s| s.parse().unwrap()).collect();
            let got = if null != 0 { TourDecision::Open } else { tournament_decision(num(3), num(4), &pts[..k.min(pts.len())], num(5)) };
            let set = "1,12544,2";
            let mut want = Vec::new();
            match got {
                TourDecision::Complete { hard } => {
                    if ev != 0 {
                        want.push("TOURNAMENT_COMPLETE:4956".to_string());
                        if hard {
                            want.push("TOURNAMENT_COMPLETE_HARD:4956".to_string());
                        }
                    }
                    want.push(set.into());
                    want.push("0,0,0".into());
                }
                TourDecision::Failure => {
                    want.push("0,0,0".into());
                    want.push(set.into());
                }
                TourDecision::Open => {
                    want.push("0,0,0".into());
                    want.push("0,0,0".into());
                }
            }
            assert_eq!(b, want.join(" "), "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    #[test]
    fn tournament_points_matches_the_original() {
        let text = include_str!("testdata/tournament_points_ref.txt");
        let list = |s: &str| -> Vec<i32> { s.split(',').map(|x| x.parse().unwrap()).collect() };
        let mut n = 0;
        for line in text.lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = a.split(' ').collect();
            let num = |i: usize| w[i].parse::<i32>().unwrap();
            let (null, trig, replay, k) = (num(0), num(1), num(2), num(3) as usize);
            let (types, places, dmg, mut pts) = (list(w[4]), list(w[5]), list(w[6]), list(w[7]));
            let table: [i32; 17] = list(w[8]).try_into().unwrap();
            let mut rec: Vec<(i32, i32)> = types.iter().zip(&places).take(k).map(|(t, p)| (*t, *p)).collect();
            let mut want = String::new();
            let mut set = vec![0u32; k];
            if null == 0 {
                let damaged: Vec<bool> = dmg.iter().take(k).map(|d| *d != 0).collect();
                tournament_points(&mut rec, &damaged, trig == 1 && replay == 1, &table, &mut pts[..k]);
                set = vec![1; k];
                want.push_str("est dec fill ");
            }
            want.push_str("| ");
            for r in &rec {
                want.push_str(&format!("{} ", r.1));
            }
            want.push_str("| ");
            for i in 0..k {
                want.push_str(&format!("{},{} ", set[i], pts[i]));
            }
            assert_eq!(b.split_whitespace().collect::<Vec<_>>(), want.split_whitespace().collect::<Vec<_>>(), "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    #[test]
    fn series_winner_matches_the_original() {
        let text = include_str!("testdata/series_wins_ref.txt");
        let list = |s: &str| -> Vec<i32> { s.split(',').map(|x| x.parse().unwrap()).collect() };
        let mut n = 0;
        for line in text.lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = a.split(' ').collect();
            let num = |i: usize| w[i].parse::<i32>().unwrap();
            let (null, trig, replay, k) = (num(0), num(1), num(2), num(3) as usize);
            let (types, places, dmg, mut wins) = (list(w[4]), list(w[5]), list(w[6]), list(w[7]));
            let mut rec: Vec<(i32, i32)> = types.iter().zip(&places).take(k).map(|(t, p)| (*t, *p)).collect();
            let mut set = vec![0u32; k];
            let mut want = String::new();
            if null == 0 {
                let damaged: Vec<bool> = dmg.iter().take(k).map(|d| *d != 0).collect();
                if let Some(i) = series_winner(&mut rec, &damaged, trig == 1 && replay == 1) {
                    wins[i] += 1;
                    set[i] = 1;
                }
                want.push_str("est dec fill ");
            }
            want.push_str("| ");
            for r in &rec {
                want.push_str(&format!("{} ", r.1));
            }
            want.push_str("| ");
            for i in 0..k {
                want.push_str(&format!("{},{} ", set[i], wins[i]));
            }
            assert_eq!(b.split_whitespace().collect::<Vec<_>>(), want.split_whitespace().collect::<Vec<_>>(), "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    #[test]
    fn series_decision_matches_the_original() {
        let text = include_str!("testdata/series_decision_ref.txt");
        let mut n = 0;
        for line in text.lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = a.split(' ').collect();
            let num = |i: usize| w[i].parse::<i32>().unwrap();
            let (null, k, need) = (num(0), num(1) as usize, num(2));
            let types: Vec<i32> = if w[3] == "-" { Vec::new() } else { w[3].split(',').map(|x| x.parse().unwrap()).collect() };
            let wins: Vec<i32> = w[4].split(',').map(|x| x.parse().unwrap()).collect();
            let got = if null != 0 { None } else { series_decision(&types[..k], &wins[..k], need) };
            let (zero, one) = ("0,12288,2", "1,12544,2");
            let want = match got {
                Some(true) => format!("{one} {zero}"),
                Some(false) => format!("{zero} {one}"),
                None => format!("{zero} {zero}"),
            };
            assert_eq!(b, want, "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    #[test]
    fn series_rows_match_the_original() {
        let text = include_str!("testdata/series_rows_ref.txt");
        let mut n = 0;
        for line in text.lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = a.split(' ').collect();
            let (k, need) = (w[0].parse::<usize>().unwrap(), w[1].parse::<i32>().unwrap());
            let rows: Vec<SeriesRow> = w[2..].iter().filter(|s| **s != "-").map(|s| {
                let f: Vec<i32> = s.split(',').map(|x| x.parse().unwrap()).collect();
                SeriesRow { kind: f[0], time: f[1], place: f[2], wins: f[3] }
            }).collect();
            assert_eq!(rows.len(), k);
            let order = series_rows(&rows);
            let mut want = String::new();
            for (i, &r) in order.iter().enumerate() {
                want.push_str(&format!("r{},{} RO_RESULTS_{}={}_/_{} ", r, i, i, rows[r].wins, need));
            }
            want.push_str("| ");
            for (i, &r) in order.iter().enumerate() {
                want.push_str(&format!("{},{},1 ", i + 1, if rows[r].kind == 0 { 22 } else { 1 }));
            }
            assert_eq!(b.split_whitespace().collect::<Vec<_>>(), want.split_whitespace().collect::<Vec<_>>(), "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    #[test]
    fn tournament_rows_match_the_original() {
        let text = include_str!("testdata/tournament_rows_ref.txt");
        let mut n = 0;
        for line in text.lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = a.split(' ').collect();
            let k = w[0].parse::<usize>().unwrap();
            let table: [i32; 17] = w[1].split(',').map(|x| x.parse().unwrap()).collect::<Vec<i32>>().try_into().unwrap();
            let rows: Vec<TourRow> = w[2..].iter().filter(|s| **s != "-").map(|s| {
                let f: Vec<i32> = s.split(',').map(|x| x.parse().unwrap()).collect();
                TourRow { kind: f[0], time: f[1], place: f[2], total: f[3] }
            }).collect();
            assert_eq!(rows.len(), k);
            let order = tournament_rows(&rows, &table);
            let mut want = String::new();
            for (i, &r) in order.iter().enumerate() {
                want.push_str(&format!("r{},{} ", r, i));
            }
            want.push_str("| ");
            for &r in &order {
                want.push_str(&format!("{},{},{} ", if rows[r].kind == 0 { 22 } else { 1 }, place_points(&table, rows[r].place), rows[r].total));
            }
            assert_eq!(b.split_whitespace().collect::<Vec<_>>(), want.split_whitespace().collect::<Vec<_>>(), "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    #[test]
    fn put_player_last_matches_the_original() {
        let text = include_str!("testdata/put_last_ref.txt");
        let mut n = 0;
        for line in text.lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let mut w = a.split(' ');
            let k: usize = w.next().unwrap().parse().unwrap();
            let mut rec: Vec<(i32, i32)> = w.filter(|s| *s != "-").map(|s| {
                let (t, p) = s.split_once(',').unwrap();
                (t.parse().unwrap(), p.parse().unwrap())
            }).collect();
            assert_eq!(rec.len(), k);
            put_player_last(&mut rec);
            let want: Vec<i32> = b.split(' ').filter(|s| *s != "-").map(|s| s.parse().unwrap()).collect();
            assert_eq!(rec.iter().map(|r| r.1).collect::<Vec<_>>(), want, "{line}");
            n += 1;
        }
        assert_eq!(n, 3000);
    }

    #[test]
    fn district_lookup_matches_the_original() {
        let text = include_str!("testdata/career_district_ref.txt");
        let mut n = 0;
        for line in text.lines() {
            let (h, want) = line.split_once(' ').unwrap();
            let bytes: Vec<u8> = if h == "-" { Vec::new() } else { (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect() };
            let s = String::from_utf8(bytes).unwrap();
            assert_eq!(district_no(&s), want.parse::<i32>().unwrap(), "{:?}", s);
            n += 1;
        }
        assert_eq!(n, 1936);
    }

    /// A mission reward moves a COMPLETE mission on only when it is repeatable (F-97457): the
    /// REPEAT_BESTOF / REPEAT_HANGOUT_WAGER pair of a district hand each other back to
    /// CHECKING_PREREQUISITES, reset (F-62492 -> F-10825), and with no other mission of the
    /// district open the repeat opens again; a mission in states 1 .. 4 and a plain complete
    /// one are left.
    #[test]
    fn repeat_missions_rearm_on_their_reward() {
        let dir = std::env::var("MCLA_DATA").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        if !std::path::Path::new(&dir).join("tune/career").is_dir() {
            return;
        }
        let mut c = Career::load(&dir);
        let b = c.mission_no("REPEAT_BESTOF_HILLS").unwrap();
        let w = c.mission_no("REPEAT_HANGOUT_WAGER_HILLS").unwrap();
        let plain = (0..c.missions.len()).find(|&j| c.missions[j].district == "Hills" && !c.missions[j].repeatable && !c.missions[j].prereq.is_empty()).unwrap();
        let mut s = c.save();
        s.missions.clear();
        for j in [b, w, plain] {
            s.missions.insert(c.missions[j].id.clone(), MState { stage: Stage::Complete, progress: vec![1; c.missions[j].objectives.len()], ..Default::default() });
        }
        c.restore(&s);
        let rw = c.missions[b].rewards[0].clone();
        assert!(matches!(&rw, Reward::Mission { id, state } if id == "REPEAT_HANGOUT_WAGER_HILLS" && state == "CHECKINGPREREQUISITES"));
        c.give(&rw);
        assert_eq!(c.state[w].stage, Stage::Armed);
        assert!(c.state[w].progress.iter().all(|&p| p == 0));
        assert!(c.ext[w].obj_hidden.iter().all(|&h| h));
        // nothing of Hills open: its TallyMission (bInitValidation) is met at the arming
        assert!(c.prereq_req_met(w, 0));
        c.activate_met();
        assert_eq!(c.state[w].stage, Stage::Active);
        // given again while it is open: left as it is
        let st = c.state_id_of(w);
        c.give(&rw);
        assert_eq!(c.state_id_of(w), st);
        assert_eq!(c.state[w].stage, Stage::Active);
        // a plain complete mission is not moved by a reward
        let id = c.missions[plain].id.clone();
        c.give(&Reward::Mission { id, state: "CHECKINGPREREQUISITES".into() });
        assert_eq!(c.state[plain].stage, Stage::Complete);
    }

    /// Every mcConditionString of the missions is met by a string the port posts as the original
    /// does: the cores' (`CORE_POSTS`, `MEET_POSTS`, `SERIES_NAMES`), the places' LOCATION_EVENT,
    /// the hangouts' series and pink slips, the tournaments' and time trials' keys, and the few
    /// fixed ones; the objectives the CORE scripts satisfy themselves (`STORY_SECOND_OBJECTIVE`)
    /// aside. (The 135 mission files hold 102 such conditions; 101 are on the mission lists and
    /// South Central's. Of the others, BESTOF_SH_01_MUSCLE and BESTOF_SH_01_TUNER wait for series
    /// no script names.)
    /// The story cores' rules: the generic core asks for the win (BEAT_SH_04 on second place:
    /// nothing), BEAT_SH_04B's own core for a finished race only; a damaged-out race posts
    /// nothing; the identifier is compared byte for byte (F-77134).
    #[test]
    fn story_cores_post_by_their_rules() {
        let dir = std::env::var("MCLA_DATA").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        if !std::path::Path::new(&dir).join("tune/career").is_dir() {
            return;
        }
        let mut c = Career::load(&dir);
        c.new_game();
        let mut s = c.save();
        for id in ["BEAT_SH_04", "BEAT_SH_04B", "BEAT_SH_05_ACLASS"] {
            let mut st = MState::default();
            st.stage = Stage::Active;
            s.missions.insert(id.to_string(), st);
            s.order.push(id.to_string());
        }
        c.restore(&s);
        c.core_race_end("BEAT_SH_04", 2, true, 0);
        c.core_race_end("BEAT_SH_04B", 2, true, 0);
        c.core_race_end("BEAT_SH_05_ACLASS", 1, false, 0);
        assert!(c.mission_active("BEAT_SH_04") && c.mission_complete("BEAT_SH_04B") && c.mission_active("BEAT_SH_05_ACLASS"));
        c.mission_event("beat_sh_04_race_complete");
        assert!(c.mission_active("BEAT_SH_04"), "case counts");
        c.core_race_end("BEAT_SH_04", 1, true, 0);
        assert!(c.mission_complete("BEAT_SH_04"));
    }

    #[test]
    fn every_string_condition_has_its_poster() {
        let dir = std::env::var("MCLA_DATA").unwrap_or_else(|_| "D:/MCLA/extracted/cache".to_string());
        if !std::path::Path::new(&dir).join("tune/career").is_dir() {
            return;
        }
        let c = Career::load(&dir);
        let mut posted: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
        let mut add = |e: &str, i: &str| {
            posted.insert((e.to_string(), i.to_string()));
        };
        for (_, i, _) in CORE_POSTS {
            add("MISSION_EVENT", i);
        }
        for (_, i) in MEET_POSTS {
            add("MISSION_EVENT", i);
        }
        for (_, i) in SERIES_NAMES {
            add("SERIES_COMPLETE", i);
        }
        for t in &crate::mc4::TIME_TRIALS {
            add("MISSION_EVENT", t.key);
        }
        for su in &crate::hangoutin::SETUPS {
            add("LOCATION_EVENT", su.key);
            add("SERIES_COMPLETE", &format!("{}_series", su.key));
            add("PINK_SLIP_COMPLETE", su.key);
        }
        for su in &crate::garagein::SETUPS {
            add("LOCATION_EVENT", su.key);
        }
        for i in ["COP_EVADE", "RED_LIGHT_GH_01_COMPLETE", "FIND_SH_02C_OBJ_01_COMPLETE", "HANGOUT_PINK_DT_OBJ_01_COMPLETE", "GARAGE_BEACHES_OBJ_01_COMPLETE"] {
            add("MISSION_EVENT", i);
        }
        add("SERIES_COMPLETE", "HANGOUT_SERIES_DT_COMPLETE");
        add("ABILITY_EVENT", "ABILITY_EQUIPPED");
        add("PAYBACK_COMPLETE", "be_payback");
        let (mut n, mut open) = (0, Vec::new());
        for m in &c.missions {
            for (k, r) in m.objectives.iter().enumerate() {
                for cond in &r.conds {
                    let Cond::Event { event, ident } = cond else { continue };
                    n += 1;
                    // (a tournament posts its own condition's identifier: `tournament_race`)
                    if event.starts_with("TOURNAMENT_COMPLETE") && ident.ends_with("_tournament1") {
                        continue;
                    }
                    if k == 0 && STORY_SECOND_OBJECTIVE.iter().any(|s| s.eq_ignore_ascii_case(&m.id)) && ident.ends_with("_OBJ_01_COMPLETE") {
                        continue;
                    }
                    if !posted.contains(&(event.clone(), ident.clone())) {
                        open.push(format!("{} {} {}", m.id, event, ident));
                    }
                }
            }
        }
        open.sort();
        assert_eq!(n, 101);
        assert_eq!(open, Vec::<String>::new());
    }

    /// Money text as the original writes it, F-36073 (language, currency flag) with its separator
    /// inserter F-33429, run unchanged offline (`mcla-rs/tools/career_reference/money_fx.py`, CSV in
    /// `MCLA_MONEY_REFERENCE`; the C run-time's `sprintf` and the engine's `snprintf` are the host's):
    /// every text equals `money_text_in`, negative values and the languages past the table included.
    #[test]
    #[ignore = "requires CSV from mcla-rs/tools/career_reference/money_fx.py; set MCLA_MONEY_REFERENCE"]
    fn money_text_matches_the_original() {
        let path = std::env::var("MCLA_MONEY_REFERENCE").expect("set MCLA_MONEY_REFERENCE to the reference CSV");
        let text = std::fs::read_to_string(path).unwrap();
        let (mut n, mut bad) = (0, 0);
        for line in text.lines().skip(1) {
            let f: Vec<&str> = line.splitn(6, ',').collect();
            if f[0] != "0" || f[5].is_empty() || f[4] != "1" {
                continue;
            }
            let (lang, v): (usize, i64) = (f[1].parse().unwrap(), f[3].parse().unwrap());
            n += 1;
            if money_text_in(v, lang) != f[5] {
                bad += 1;
                if bad <= 10 {
                    eprintln!("differs: {line} -> {}", money_text_in(v, lang));
                }
            }
        }
        eprintln!("compared {n} texts, {bad} differ");
        assert!(n > 1000);
        assert_eq!(bad, 0);
    }
}
