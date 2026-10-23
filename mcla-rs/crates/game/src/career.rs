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
}

#[derive(Debug, Clone)]
enum Reward {
    Mission { id: String, state: String },
    Car(String),
    /// a call from the mechanic (`game/Career/<Name>Tip` scripts): the string key
    Tip(String),
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
    Payback,
    Delivery,
    Garage,
    Ability,
    EscapedPolice,
    /// pulled up behind a roaming racer
    FoundHookman,
    /// the player is driving around in this district
    District(&'a str),
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
    /// (step 21b) a story scene a script plays (a CORE at a completion or a meeting, an event
    /// script at a garage / hangout): the mission whose scene it is (`Cutscenes::for_mission`)
    Scene(String),
}

/// `game\Career\CarsUnlockTip_02` -> `SIDEKICK_CARS_UNLOCK_TIP_02`
fn tip_key(script: &str) -> String {
    let name = script.rsplit(|c| c == '\\' || c == '/').next().unwrap_or(script);
    let mut out = String::from("SIDEKICK");
    for ch in name.chars() {
        if ch.is_ascii_uppercase() {
            out.push('_');
        }
        out.push(ch.to_ascii_uppercase());
    }
    out.replace("__", "_")
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
    pub strings: StringTable,
    thresholds: Vec<(f32, Reward)>,
    /// the id of each threshold's entry above in the progression tables (`Progression_7`,
    /// `Unlocking_3`; empty for the port's own), and the ids whose rewards were handed out:
    /// the original gives a threshold's rewards once and keeps a "reached" flag (`mc4`)
    threshold_ids: Vec<String>,
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
    /// the location events posted this visit (cleared when the place is left or a race starts)
    pub visit_events: HashSet<String>,
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
        out.push(Requirement { desc: tag(chunk, "szDescriptionId").unwrap_or("").to_string(), hidden: val(chunk, "bAlwaysHidden") == Some("true"), conds, stat_types });
    }
    out
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
            for r in rew.split("<Item type=\"mcReward").skip(1) {
                match r.split('"').next().unwrap_or("") {
                    "Mission" => rewards.push(Reward::Mission { id: tag(r, "szMissionId").unwrap_or("").to_ascii_uppercase(), state: tag(r, "szTransitionState").unwrap_or("").to_ascii_uppercase().replace('_', "") }),
                    "Car" => rewards.push(Reward::Car(tag(r, "szCarName").unwrap_or("").to_ascii_lowercase())),
                    _ => {}
                }
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
                index: if south { list_sc.iter().position(|n| *n == stem).map_or(-1, |k| (list_main.len() + k) as i32) } else { list_main.iter().position(|n| *n == stem).map_or(-1, |k| k as i32) },
                addon: south,
                auto_start: val(&t, "bAutoStart") == Some("true"),
                on_enter: t.contains("_READY_TO_START_ON_ENTER</szEventDescription>"),
                on_enter_career: t.split("<Item type=\"mcMissionEventScript\">").skip(1).find(|e| e.contains("_READY_TO_START_ON_ENTER</szEventDescription>")).filter(|e| e.contains("<bCareerScript value=\"true\"/>")).and_then(|e| e.split("<Item content=\"ascii\">").nth(1)).and_then(|s| s.split('<').next()).map(|s| s.trim().to_string()),
                repeatable: val(&t, "bRepeatable") == Some("true"),
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
                                // (a threshold is reached by the system's share, not by pct x MaxAmount: sub_826BC090)
                                c.perf_unlocks.entry(field("PerformanceId").to_ascii_uppercase()).or_insert(crate::mc4::main_amount_at(pct));
                            }
                            "PartUnlock" => {
                                c.part_unlocks.entry(kind.to_ascii_uppercase()).or_default().entry(field("Name").to_ascii_uppercase()).or_insert(crate::mc4::class_amount_at(pct, max as i64));
                            }
                            "Script" if kind == "Unlocking" && field("ScriptName").to_ascii_lowercase().contains("tip") => {
                                c.thresholds.push((crate::mc4::main_amount_at(pct) as f32, Reward::Tip(tip_key(&field("ScriptName")))));
                                c.threshold_ids.push(th.split_whitespace().skip_while(|w| *w != "Id").nth(1).unwrap_or("").to_string());
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
                    } else if field("Type") == "Script" && field("ScriptName").to_ascii_lowercase().contains("tip") {
                        c.thresholds.push((crate::mc4::main_amount_at(pct) as f32, Reward::Tip(tip_key(&field("ScriptName")))));
                        c.threshold_ids.push(th.split_whitespace().skip_while(|w| *w != "Id").nth(1).unwrap_or("").to_string());
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
        // finished with the ability fitted - sub_826B9AE8 / sub_826BB750; `mc4_race_experience`)
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
        // log takes the mission at CHECKING_OBJECTIVES: sub_826CD880, sub_826C7710 - step 20b)
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

    fn give(&mut self, r: &Reward) {
        match r {
            Reward::Mission { id, state } => {
                let Some(&i) = self.index.get(id) else { return };
                if self.state[i].stage != Stage::Locked {
                    return;
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
                    }
                }
            }
            Ev::Delivery => *self.stats.entry("xCDL".into()).or_default() += 1,
            Ev::EscapedPolice => *self.stats.entry("tPUR".into()).or_default() += 1,
            Ev::Money(m) => *self.stats.entry("k$WA".into()).or_default() += m.max(0),
            Ev::Rep(r) => {
                self.rep = r;
                self.resolve_pending();
            }
            _ => {}
        }
        // a hookman win counts for one mission only: the oldest one waiting for it
        let mut hookman_used = false;
        let mut found_used = false;
        let active: Vec<usize> = self.order.clone();
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
            if let Ev::Race { race, place, beat_goal, district, hookman, class } = ev {
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
                let racing_id = m.id.starts_with("RACE_") || m.id.starts_with("BEAT_") || m.id.starts_with("REDLIGHT_");
                let new = match (c, ev) {
                    // (a RACE_ mission asks for the race, not the win: FIRST IMPRESSIONS / RACE HENRY was
                    // stamped COMPLETE on second place in the recomp - `ng_mission_results`; BEAT_ and
                    // REDLIGHT_ ask for the win)
                    (Cond::Event { event, .. }, Ev::Race { place, hookman: true, .. }) if event == "MISSION_EVENT" && racing_id && !hookman_used && (place == 1 || m.id.starts_with("RACE_")) => {
                        // both steps of "meet him, then race him" are done by the same race
                        if oi + 1 == m.objectives.len() {
                            hookman_used = true;
                        }
                        cur + 1
                    }
                    (Cond::Event { event, .. }, Ev::FoundHookman) if event == "MISSION_EVENT" && m.id.starts_with("FIND_") && m.id != "FIND_SH_02C" && !found_used => {
                        found_used = true;
                        cur + 1
                    }
                    (Cond::Event { event, .. }, Ev::EscapedPolice) if event == "MISSION_EVENT" && m.id.contains("POLICE") => cur + 1,
                    (Cond::Event { event, .. }, Ev::Race { place: 1, district, .. }) if event == "MISSION_EVENT" && !racing_id && !m.id.starts_with("FIND_") && !m.id.contains("POLICE") && (!ev_races.is_empty() || same_district(&m.district, district)) => cur + 1,
                    // (a tournament with races is completed by tournament_race on the points, not per race)
                    (Cond::Event { event, .. }, Ev::Race { place: 1, district, .. }) if matches!(event.as_str(), "SERIES_COMPLETE" | "TOURNAMENT_COMPLETE" | "TOURNAMENT_COMPLETE_HARD" | "WAGER_RACE_COMPLETE") && !(event.starts_with("TOURNAMENT_COMPLETE") && !ev_races.is_empty()) && (!ev_races.is_empty() || same_district(&m.district, district)) => cur + 1,
                    (Cond::Event { event, .. }, Ev::Payback) if event == "PAYBACK_COMPLETE" => cur + 1,
                    (Cond::Event { event, .. }, Ev::Garage) if event == "LOCATION_EVENT" && m.id.starts_with("GARAGE") => cur + 1,
                    (Cond::Event { event, .. }, Ev::District(d)) if event == "LOCATION_EVENT" && !m.id.starts_with("GARAGE") && same_district(&m.district, d) => cur + 1,
                    (Cond::Event { event, .. }, Ev::Ability) if event == "ABILITY_EVENT" => cur + 1,
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
                for r in &m.rewards {
                    self.give(r);
                }
                // what the event itself pays: prize cars from the reward tables, the rival's
                // car for pink slips, prize money (amounts are ours: the original computes
                // them in script)
                let mut kinds: Vec<&str> = Vec::new();
                for req in &m.objectives {
                    for c in &req.conds {
                        if let Cond::Event { event, ident } = c {
                            kinds.push(event.as_str());
                            // (the event's prize car and money are the event's own rewards:
                            // given when the event completes - sub_826CD1E8, `mc4_event_complete`)
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
        if let Ev::Rep(rep) = ev {
            // (a threshold of the tables whose rewards were handed out is not handed out again)
            // (the Unlocking system's thresholds go by its own amount)
            let unl = self.unlocking.unwrap_or(rep);
            let due: Vec<(usize, Reward)> = self.thresholds.iter().enumerate().filter(|(k, (p, _))| *p as i64 <= if self.threshold_ids.get(*k).map_or(false, |id| id.starts_with("Unlocking_")) { unl } else { rep } && !self.threshold_ids.get(*k).map_or(false, |id| !id.is_empty() && self.reached_ids.contains(id))).map(|(k, (_, r))| (k, r.clone())).collect();
            for (_, r) in &due {
                self.give(r);
            }
            for (k, _) in due {
                if let Some(id) = self.threshold_ids.get(k).filter(|id| !id.is_empty()).cloned() {
                    self.reached_ids.insert(id);
                }
            }
        }
        // missions whose prerequisites have just been met
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
        // (step 20: completions after what they handed out; Experience baselines take the
        // reputation in hand - not after a race or its money, whose reputation follows)
        for i in std::mem::take(&mut self.done_now) {
            self.journal.push((false, i));
        }
        if !matches!(ev, Ev::Race { .. } | Ev::Money(_)) {
            self.resolve_pending();
        }
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
            id.starts_with("RACE_") || id.starts_with("BEAT_") || id.starts_with("REDLIGHT_")
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
    /// with the add-on: sub_826C0C50 0x826C0CAC) six seconds after the player's car is there
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
    /// table the game's career manager is built with (0x826c1a64) and its
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
            for (p, who) in order.iter().enumerate() {
                *st.pts.entry(who.clone()).or_default() += POINTS.get(p).copied().unwrap_or(0);
            }
            let rows = order.iter().enumerate().map(|(p, who)| (who.clone(), POINTS.get(p).copied().unwrap_or(0), st.pts.get(who).copied().unwrap_or(0))).collect();
            let last = st.won.len() >= races.len();
            self.tour_table = Some(TourTable { rows, decided: None, last });
            let st = &mut self.state[i];
            let mine = st.pts.get("YOU").copied().unwrap_or(0);
            let rival = st.pts.iter().filter(|(k, _)| k.as_str() != "YOU").map(|(_, v)| *v).max().unwrap_or(0);
            let set = |st: &mut MState, v: u32| {
                if let Some(p) = st.progress.get_mut(oi) {
                    *p = v;
                }
            };
            if st.won.len() < races.len() {
                return Some(format!("{} POINTS   BEST RIVAL {}", mine, rival));
            }
            if mine >= rival {
                set(st, 1);
                st.pts.clear();
                // (TOURNAMENT_COMPLETE with the event's name, and TOURNAMENT_COMPLETE_HARD only
                // when the tournament was entered on Hard: sub_826CA6D8 0x826CA88C..0x826CA8B4)
                let done = self.missions[i].objectives[oi].conds.first().and_then(|c| match c {
                    Cond::Event { event, ident } => Some((ident.clone(), event.ends_with("_HARD"))),
                    _ => None,
                });
                if let Some((ident, hard)) = done {
                    let diff = self.stats.get(&format!("tour_diff:{}", ident.to_ascii_lowercase())).copied().map_or(if hard { 2 } else { 1 }, |d| d.clamp(0, 3) as usize);
                    self.completed_events.push((ident, diff));
                    if hard && diff != 2 {
                        // (won, but not the tournament this mission waits for)
                        st.won.clear();
                        set(st, 0);
                    }
                }
                self.tour_decided = Some(true);
                if let Some(t) = self.tour_table.as_mut() {
                    t.decided = Some(true);
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
            let st = &mut self.state[i];
            let n = {
                let e = st.pts.entry(winner.to_string()).or_default();
                *e += 1;
                *e
            };
            if winner == "YOU" {
                return Some(format!("{} OF 3 WINS", n.min(3)));
            }
            if n >= 3 {
                st.won.clear();
                st.pts.clear();
                if let Some(p) = st.progress.get_mut(oi) {
                    *p = 0;
                }
                self.story_reset(i);
                return Some("SERIES LOST".to_string());
            }
            return Some(format!("RIVAL HAS {} OF 3 WINS", n));
        }
        None
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
    /// 1 of the 3D icon): the refresh sub_8220A0B8 takes each mission that is not bAlwaysHidden
    /// (+685) and, through sub_826C72D8, in state 2 READY_TO_START or 3 CHECKING_OBJECTIVES, the
    /// first objective that is not satisfied (+116), not always hidden (+117) and not hidden (+118);
    /// its target's vehicle element and racer are flagged (sub_822378F8), every other one cleared
    /// (sub_82238080).
    pub fn pulse_objective(&self, id: &str) -> Option<usize> {
        let &i = self.index.get(id)?;
        let m = &self.missions[i];
        if m.hidden || self.state[i].stage != Stage::Active || !matches!(self.state_id_of(i), 2 | 3) {
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
    pub fn objective_texts(&self, id: &str) -> Vec<String> {
        self.index.get(id).map(|&i| self.missions[i].objectives.iter().filter(|r| !r.hidden).map(|r| self.text(&r.desc)).filter(|t| !t.is_empty()).collect()).unwrap_or_default()
    }

    /// The active missions, oldest first, for the mission log.
    pub fn active(&self) -> Vec<MissionView> {
        self.views(|s| *s == Stage::Active)
    }

    fn views(&self, want: impl Fn(&Stage) -> bool) -> Vec<MissionView> {
        // (the log's two lists: the open missions in the order they entered it, the completed
        // ones in the order of completion - lists 1 and 2 of the mission manager, sub_826C3F20 /
        // sub_826C4058; mcMissionLog sub_8262F7A8 reads list 1 for "active", list 2 for
        // "completed": sub_8262C5B0)
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
    /// `Game/Career/Missions/<ID>` starts then (sub_826C7710 -> sub_826C6CC8), and with it the
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
            self.event(Ev::FoundHookman);
            // win whatever the missions ask for, plus an ordinary race
            for (r, _) in self.mission_races() {
                let full = format!("game/race/{}", r);
                self.event(Ev::Race { district: d, place: 1, hookman: true, class: classes[step % classes.len()], race: &full, beat_goal: true });
            }
            self.event(Ev::Race { district: d, place: 1, hookman: true, class: classes[step % classes.len()], race: "", beat_goal: false });
            self.event(Ev::Race { district: "Freeway", place: 1, hookman: false, class: "TUNER", race: "", beat_goal: false });
            self.event(Ev::District(d));
            self.event(Ev::Garage);
            self.event(Ev::Ability);
            self.event(Ev::Payback);
            self.event(Ev::Delivery);
            self.event(Ev::EscapedPolice);
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
    /// sidekick's call, then mcMission_ChangeState(3)): bAutoStart not set (sub_826CD880 moves
    /// it on by itself otherwise) and the script there. A mission the log never lists (always
    /// hidden) has no call in the port and goes on at once; so does one without the script.
    fn waits_for_call(&self, i: usize) -> bool {
        let m = &self.missions[i];
        !m.auto_start && m.on_enter && !m.hidden
    }

    /// READY_TO_START entered (sub_826CD8A8): the objectives shown unless always hidden and
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

    /// CHECKING_OBJECTIVES entered: into the log (list 1), objectives armed (sub_826CD9D8).
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
    /// posts its ON_ENTER event again (sub_826CD528): out of the log, the call again.
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
    /// (children of the career's start-up script while they run: sub_826D3910 -> sub_826BD870).
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
            let d = ((p[0] - at[0]).powi(2) + (p[1] - at[1]).powi(2) + (p[2] - at[2]).powi(2)).sqrt();
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

    /// The completion statistic `k%AG` (sub_826C4388): the completed list's missions over all
    /// missions, both without the always hidden (+685) and the repeatable (+693) ones - 100 in
    /// the 100 % save (107 of 107), 1 in the new game (2 of 107) - cut to a whole number.
    pub fn percent_complete(&self) -> i64 {
        let counts = |i: &usize| !self.missions[*i].hidden && !self.missions[*i].repeatable;
        let total = (0..self.missions.len()).filter(counts).count();
        let done = self.done.iter().filter(|i| counts(i)).count();
        if total == 0 { 0 } else { (done * 100 / total) as i64 }
    }

    /// The statistic is posted when a mission joins the completed list (sub_826CDB60) and when
    /// a save has been read (sub_826C5DB8).
    pub fn post_percent(&mut self) {
        let v = self.percent_complete();
        self.stats.insert("k%AG".to_string(), v);
    }

    /// A Stat condition's last value stops with the update that meets it (sub_826D25E8): the
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
        for k in 0..self.order.len() {
            let i = self.order[k];
            let m = self.missions[i].clone();
            for (oi, r) in m.objectives.iter().enumerate() {
                if let Some(c @ Cond::Event { event, ident: id }) = r.conds.first() {
                    if event == "MISSION_EVENT" && id.eq_ignore_ascii_case(ident) {
                        let t = self.target(&m, c);
                        if let Some(p) = self.state[i].progress.get_mut(oi) {
                            *p = (*p).max(t);
                        }
                    }
                }
            }
        }
        let rep = self.rep;
        self.event(Ev::Rep(rep));
    }

    pub fn pink_slip_won(&mut self, ident: &str) {
        for k in 0..self.order.len() {
            let i = self.order[k];
            let m = self.missions[i].clone();
            for (oi, r) in m.objectives.iter().enumerate() {
                if let Some(c @ Cond::Event { event, ident: id }) = r.conds.first() {
                    if event == "PINK_SLIP_COMPLETE" && id.eq_ignore_ascii_case(ident) {
                        let t = self.target(&m, c);
                        if let Some(p) = self.state[i].progress.get_mut(oi) {
                            *p = (*p).max(t);
                        }
                    }
                }
            }
        }
        let rep = self.rep;
        self.event(Ev::Rep(rep));
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
/// sub_826C69C8 / sub_826CC3F8, read by sub_826C7530 / sub_826CCC50):
/// - `obj_hidden`: each objective's byte +118. 1 when the requirement is made
///   (sub_826CCA30); set to 0 when the mission enters READY_TO_START, CHECKING_OBJECTIVES or
///   READY_TO_COMPLETE and to 1 when it enters CHECKING_PREREQUISITES (sub_826CD8A8 /
///   sub_826CD9D8 / sub_826CDAF0 / sub_826CD768 -> sub_826CC168, which leaves it alone while
///   +117 is set); mcRequirement_Hide (sub_826CC8A0) sets it from a script;
/// - `obj_always`: byte +117 - the XML's bAlwaysHidden, changed by mcRequirement_AlwaysHide
///   (sub_826CC810). Not saved: after a load it is the XML's again. The mission log lists an
///   objective when neither is set (sub_8262E8D8 0x8262E9EC);
/// - `obj_base` / `pre_base`: the baseline of an objective's / a prerequisite's Stat condition
///   (the statistic's value when the condition was armed, sub_826D1938) or Experience
///   condition (the system's amount, sub_826D2020). Armed when the mission enters
///   CHECKING_OBJECTIVES (objectives: sub_826CD9D8 -> sub_826CC4B0) or CHECKING_PREREQUISITES
///   (prerequisites: sub_826CD768); kept as saved when a save is loaded (the load re-enters the
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
}

/// The story missions whose CORE script shows the second objective once the hookman is met
/// (mcRequirement_Satisfy(objective 0, 1), mcRequirement_AlwaysHide(objective 1, 0),
/// mcRequirement_Hide(objective 1, 0), then mcMission_ChangeState(3)) and puts the first
/// step back when the series is not won (Satisfy(0, 0), AlwaysHide(1, 1), Hide(1, 1)):
/// `Game/Career/Story/<ID>_CORE.sco` (BESTOF_SH_05 0x8bf..0x8ed / 0x123d..0x126b, CITY_CHAMP,
/// BEAT_SH_02_EXOTIC; RACE_SH_01's is on no mission list).
/// The standings after a tournament's race (mcTournamentResultsForm, form 49, filled by
/// sub_826CA1E8): each racer's name ("YOU" the player), this race's points and the total, in
/// the race's order; `decided` won / lost with this race (only its last: sub_826CA6D8), `last`
/// the event's last race was run.
#[derive(Clone, Debug, Default)]
pub struct TourTable {
    pub rows: Vec<(String, i32, i32)>,
    pub decided: Option<bool>,
    pub last: bool,
}

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

/// Money as sub_82217820 writes it (with the currency sign), by the language (the string
/// table's +28: 0 English, 1 Spanish, 2 French, 3 German, 4 Italian): digits grouped by three
/// (sub_82217680) with ',' (0 and others), '.' (1, 3, 4) or '|' (2); "$" before (0, 3 and
/// others) or after (1, 2, 4).
pub fn money_text_in(v: i64, lang: usize) -> String {
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
    let g = if v < 0 { format!("-{g}") } else { g };
    match lang {
        1 | 2 | 4 => format!("{g}$"),
        _ => format!("${g}"),
    }
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
                }
            })
            .collect();
    }

    fn ext_ok(&self, i: usize) -> bool {
        self.ext.get(i).map_or(false, |e| e.obj_hidden.len() == self.missions[i].objectives.len() && e.pre_base.len() == self.missions[i].prereq.len())
    }

    /// A statistic counted from a baseline (sub_826D2538): a Purse statistic as it stands,
    /// any other from its value when the condition was armed (not below 0).
    fn stat_since(&self, stat: &str, purse: bool, base: Option<i64>) -> i64 {
        let v = self.stats.get(stat).copied().unwrap_or(0);
        if purse { v } else { (v - base.unwrap_or(0)).max(0) }
    }

    fn purse(r: &Requirement, n: usize) -> bool {
        r.stat_types.get(n).map_or(false, |t| t.eq_ignore_ascii_case("Purse"))
    }

    /// The mission enters the mission log (the port's Active: CHECKING_OBJECTIVES): it leaves
    /// the completed list if it was in it (sub_826C6C50); its objectives are shown unless
    /// always hidden; their Stat / Experience conditions are armed (sub_826CD9D8).
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

    /// The mission enters CHECKING_PREREQUISITES (sub_826CD768): its objectives are hidden
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
        }
    }

    /// The mission is complete: it joins the completed list at its end (sub_826CDB60 ->
    /// sub_826C3F20 list 2, unless it is in it).
    fn mark_done(&mut self, i: usize) {
        if !self.done.contains(&i) {
            self.done.push(i);
        }
        self.done_now.push(i);
    }

    /// Experience baselines armed by a race take the reputation that race paid.
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

    /// An objective's Stat progress (sub_826D2538, up to the target).
    fn ext_stat_progress(&self, i: usize, oi: usize, stat: &str, target: u32) -> u32 {
        let base = self.ext.get(i).and_then(|e| e.obj_base.get(oi).copied()).flatten();
        let purse = Self::purse(&self.missions[i].objectives[oi], 0);
        self.stat_since(stat, purse, base).clamp(0, target as i64) as u32
    }

    /// An objective's Experience progress: the amount since its baseline (sub_826D1F38); 0
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
            Cond::TallyMission { less_than, tally, district } => {
                let n = self.missions.iter().zip(self.state.iter()).filter(|(m, s)| s.stage == Stage::Complete && same_district(district, &m.district)).count() as u32;
                if *less_than { n < *tally } else { n >= *tally }
            }
            Cond::Experience { amount } => self.rep - base.unwrap_or(0) >= *amount,
            Cond::Stat { stat, value } => self.stat_since(stat, Self::purse(r, ci), base) >= *value,
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
    /// sub_826D2090, sub_826D24C8, sub_826D2500, sub_826D2718): "(n/target)", for a Money or
    /// Purse statistic "($n/$target)", nothing for a statistic of another type, a String or a
    /// Mission condition.
    fn objective_count(&self, i: usize, oi: usize, cur: u32, target: u32) -> Option<String> {
        let r = &self.missions[i].objectives[oi];
        match r.conds.first()? {
            Cond::Stat { .. } => {
                let t = r.stat_types.first().map(|s| s.to_ascii_lowercase()).unwrap_or_default();
                match t.as_str() {
                    "tally" => Some(format!("({}/{})", cur, target)),
                    "money" | "purse" => Some(format!("({}/{})", money_text_in(cur as i64, self.strings.lang), money_text_in(target as i64, self.strings.lang))),
                    _ => None,
                }
            }
            Cond::Experience { .. } | Cond::TallyRace { .. } | Cond::TallyMission { .. } => Some(format!("({}/{})", cur, target)),
            _ => None,
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
                // heuristic - sub_826D2870; not before it is armed)
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
            if to == 3 && st == 2 {
                self.enter_checking(i);
            }
            log::info!("career: {} at {} plays {}", id, ev, scene);
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
}
