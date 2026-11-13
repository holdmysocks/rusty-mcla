//! Leaving a race for the cruise: which of the race's opponents stay on the road (career2, 2026-11-12;
//! docs/CAREER_RACES.md "the opponents after a race drive away as traffic").
//!
//! 29 race scripts run one function, identical in all of them (work/career2/raceover), when the race is left for the
//! cruise: RaceWrapper (`game/race/0xc4e26a20` 0x2c51, for every race-over command but NEXT RACE / REPLAY), the series
//! race (`0xb8b948b6`), the tournament race (`0x039dd301`), the wager / pink slip race (`Hangouts/0x16e89a34`), the
//! downtown series / pink slip hangouts, TelephoneChallenge, generic_local_hookman and twenty story cores (`STORY_RELEASE`;
//! the cores at the end of their local race-over dispatch - RETURN TO CRUISE, QUIT and GO TO GARAGE fall into it, NEXT
//! RACE / RESTART / REPLAY leave before). For each AI slot `s` of the race data (k = 0 .. +3084 - 1, s = k + +3080 -
//! +3084): Racer_DistanceBetween(the player, his racer) < 500 and not Race_HasBeenArrested - Racer_SetStreamingUnlocked,
//! Opponent_ToggleDeleteOnEviction(1), BrainFactory_CreateBrainCruising, AppendDest of route `min(s, 7)` (the switch over
//! 0..6 and its default; `roamers::DRIVE_AWAY`, equal to the script's floats bit for bit), BrainCruising_SetLooping(1),
//! DriveToNearestRail; any other: RemoveOpponentFromWorld and DeleteOpponent (Story/0x156c2b36 0x42f9..0x49c2).
//!
//! The post-race taunt of the cores in `STORY_TAUNT` and generic_local_hookman: on RETURN TO CRUISE, before the loop,
//! mcCareerSpeech_PlayTauntpostRace(record 1's racer, the player, the player's record +164 == 1) (0x420c..0x426a), and
//! after the loop, whatever the command, PlayTauntpostRace(record 1's racer, the player, 0) (0x4a9c..0x4ab9). The
//! other scripts' taunts (`taunts`): the tournament, TelephoneChallenge and RACE_GH_06 on QUIT only, NOT_IN_FIRST; the
//! wager / pink slip hangout NOT_IN_FIRST on QUIT, on RETURN TO CRUISE by its end sequence's play flag; the series by
//! its chain after the replay check (OrderedSeriesCore 0x3d3d..0x3e84), none when a won series' reward cut plays, and
//! NOT_IN_FIRST when its beat me there leg is left (0x22b5..0x2396). RaceWrapper, BEAT_SH_04B and the downtown
//! hangouts have none. Every speaker is record 1's racer.

use glam::Vec3;

/// The story cores whose race-over runs the release (story script = joaat of `<mission>_core.sco`).
pub const STORY_RELEASE: [&str; 20] = [
    "MUSCLE_CHAMP",
    "LUXURY_CHAMP",
    "EXOTIC_CHAMP",
    "MOTORCYCLE_CHAMP",
    "TUNER_CHAMP",
    "RACE_SH_04_MOTORCYCLE",
    "RACE_GH_06",
    "BEAT_SH_02_BCLASS",
    "BEAT_SH_02_CCLASS",
    "BEAT_SH_03",
    "BEAT_SH_03_ACLASS",
    "BEAT_SH_03_CCLASS",
    "BEAT_SH_04",
    "BEAT_SH_04B",
    "BEAT_SH_04_MOTORCYCLE",
    "BEAT_SH_05_ACLASS",
    "BEAT_SH_06_BCLASS",
    "BEAT_SH_06_SUV",
    "BEAT_GH_26_SUV",
    "TEST_MISSION",
];

/// The story cores that run a race of their own with no release (their race-over deletes every opponent).
pub const STORY_NO_RELEASE: [&str; 6] = ["RACE_GH_02", "RACE_SH_01", "BEAT_SH_02_DCLASS", "BEAT_SH_04_BCLASS", "BEAT_GH_04_MUSCLE", "BEAT_GH_05_TUNER"];

/// The cores with the two post-race taunts of the include (all of `STORY_RELEASE` but BEAT_SH_04B, which has none, and
/// RACE_GH_06, which has one of its own).
pub fn story_taunts(mission: &str) -> bool {
    STORY_RELEASE.contains(&mission) && !matches!(mission, "BEAT_SH_04B" | "RACE_GH_06")
}

/// The script a race runs under, as far as the leave for the cruise goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    /// RaceWrapper: an ordinary race, career or ARCADE
    Wrapper,
    /// OrderedSeriesCore
    Series,
    /// the tournament core
    Tournament,
    /// the wager / pink slip hangout
    Hangout,
    TelephoneChallenge,
    /// generic_local_hookman: a cruising hookman's race
    LocalHookman,
    /// a story core's own race
    Story(String),
    /// red light, freeway, time trial, delivery, payback, flag games, the race editor: no release
    Other,
}

impl Owner {
    pub fn releases(&self) -> bool {
        match self {
            Owner::Story(m) => STORY_RELEASE.contains(&m.as_str()),
            Owner::Other => false,
            _ => true,
        }
    }
}

/// The distance under which an opponent stays (Racer_DistanceBetween, 0x433e).
pub const KEEP_WITHIN: f32 = 500.0;

/// One opponent of the race at its leave: his slot in the race data and his position.
#[derive(Clone, Copy, Debug)]
pub struct Opp {
    pub slot: usize,
    pub uid: u32,
    pub pos: Vec3,
}

/// What the leave does: the opponents kept (uid, route of `roamers::DRIVE_AWAY`) and the taunt scenarios asked for, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub keep: Vec<(u32, usize)>,
    pub taunts: Vec<&'static str>,
}

/// The story series whose script sets OrderedSeriesCore's +3260 (the cut after a won series): BESTOF_GH_04
/// (cut_reward_nikolai_generated) and BESTOF_GH_05 (cut_reward_marcel_generated).
pub const SERIES_REWARD_CUT: [&str; 2] = ["BESTOF_GH_04", "BESTOF_GH_05"];

/// The race-over command that leaves the race (the registry's raceOverCommand): RETURN TO CRUISE, QUIT, GO TO GARAGE.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Cruise,
    Quit,
    Garage,
}

/// What the scripts read at the leave.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// Race_HasBeenArrested
    pub arrested: bool,
    /// the player's record +164 is 1 (finished first)
    pub first: bool,
    /// the player's record +32 is not INT_MAX (he has a finish time)
    pub finished: bool,
    /// the series: bComplete (the player won it with this race)
    pub series_won: bool,
    /// the series: local 32 - not damaged out, bComplete and the series' +3260 cut script set (BESTOF_GH_04's
    /// cut_reward_nikolai, BESTOF_GH_05's cut_reward_marcel: their story scripts write it before starting
    /// OrderedSeriesCore; every other caller leaves it NULL, and StringCompare(NULL, '') is equal) - the reward cut
    /// plays and no taunt does
    pub series_cut: bool,
    /// the series' beat me there leg is on (its own race-over at 0x22b5)
    pub bmt: bool,
}

const IN: &str = "TAUNT_POST_IN_FIRST";
const NOT: &str = "TAUNT_POST_NOT_IN_FIRST";

/// The post-race taunts of a leave, in order (each PlayTauntpostRace(record 1's racer, the player, b): b set -
/// TAUNT_POST_IN_FIRST, else TAUNT_POST_NOT_IN_FIRST).
pub fn taunts(owner: &Owner, cmd: Command, f: &Facts) -> Vec<&'static str> {
    let by = |b: bool| if b { IN } else { NOT };
    match owner {
        // (the include: RETURN TO CRUISE by the place before the loop, NOT_IN_FIRST after it for every command)
        Owner::Story(m) if story_taunts(m) => match cmd {
            Command::Cruise => vec![by(f.first), NOT],
            _ => vec![NOT],
        },
        Owner::LocalHookman => match cmd {
            Command::Cruise => vec![by(f.first), NOT],
            _ => vec![NOT],
        },
        // (TournamentCore, TelephoneChallenge, RACE_GH_06_core: 'quit' only, 0)
        Owner::Tournament | Owner::TelephoneChallenge if cmd == Command::Quit => vec![NOT],
        Owner::Story(m) if m == "RACE_GH_06" && cmd == Command::Quit => vec![NOT],
        // (Hangouts/0x16e89a34 0x42b4..0x436b: returnToHangout nothing, 'quit' 0, 'returnToCruise' its local 11: 1, cleared
        // by the end sequence's "did not finish" (no finish time; the kinds 16 / 17 and a decided series or tournament,
        // which keep it, are no wager) and "damaged out or arrested" (arrested without nPlayerCaught, which only a
        // payback sets) - 0x2ca8..0x2f04. So a finished wager says IN_FIRST in any place.)
        Owner::Hangout => match cmd {
            Command::Quit => vec![NOT],
            Command::Cruise => vec![by(f.finished && !f.arrested)],
            Command::Garage => vec![],
        },
        // (OrderedSeriesCore: its beat me there leg's race-over - returnToCruise / quit / returnToGarage, 0; then the
        // local race's chain, all of it under !local32: bComplete 1, else returnToCruise or quit 0, else +164 == 1 1,
        // else 0)
        Owner::Series if f.bmt => vec![NOT],
        Owner::Series if f.series_cut => vec![],
        Owner::Series if f.series_won => vec![IN],
        Owner::Series if matches!(cmd, Command::Cruise | Command::Quit) => vec![NOT],
        Owner::Series => vec![by(f.first)],
        _ => vec![],
    }
}

/// The leave of a race by `cmd`.
pub fn plan(owner: &Owner, cmd: Command, f: &Facts, player: Option<Vec3>, opps: &[Opp]) -> Plan {
    let mut p = Plan { keep: Vec::new(), taunts: taunts(owner, cmd, f) };
    if owner.releases() {
        for o in opps {
            if crate::brain::racer_distance(player, Some(o.pos)) < KEEP_WITHIN && !f.arrested {
                p.keep.push((o.uid, o.slot.min(7)));
            }
        }
    }
    p
}

/// A kept opponent becomes a cruising racer: his streaming lock off, deleted on eviction (`evict`), the cruising brain on
/// the route, looping, driving to the nearest rail (`cruisedrive::setup` with the start flag clear).
pub fn release(vehicles: &mut crate::vehicles::Vehicles, col: &crate::physics::collision::WorldCollider, uid: u32, route: usize) -> bool {
    let Some(i) = vehicles.cars.iter().position(|c| c.uid == uid) else { return false };
    let pts: Vec<Vec3> = crate::roamers::DRIVE_AWAY[route.min(7)].iter().map(|p| Vec3::from(*p)).collect();
    let c = &mut vehicles.cars[i];
    c.pinned = false;
    c.deactivated = false;
    c.driver_off = false;
    let tune = c.model.aitunes.as_ref().map(|t| std::sync::Arc::new(mcla_formats::rage::aitune::AiTune::lerp(&t.base, &t.mods, c.perf)));
    if c.ai.is_none() {
        c.ai = Some(crate::ai::AiDriver::new(uid));
    }
    if let (Some(ai), Some(t)) = (c.ai.as_mut(), tune) {
        ai.tune = Some(t);
        ai.brain.0 = None;
        ai.path = None;
    }
    let pos = c.state.pos;
    crate::cruisedrive::setup(vehicles, col, i, &pts, 0, false, pos)
}

/// Opponent_ToggleDeleteOnEviction(1): a kept opponent goes once the streaming walk would evict him - his flat distance
/// less the loaded bonus past the eviction reach (`oppmgr::priority`, as `Roamers` deletes the man driving away).
pub fn evict(vehicles: &mut crate::vehicles::Vehicles, kept: &mut Vec<u32>) {
    if kept.is_empty() {
        return;
    }
    let lim = crate::oppmgr::Limits::default();
    let pp = vehicles.player().map_or(Vec3::ZERO, |c| c.state.pos);
    kept.retain(|&uid| match vehicles.cars.iter().position(|c| c.uid == uid) {
        Some(i) if crate::oppmgr::flat_distance(vehicles.cars[i].state.pos.x, vehicles.cars[i].state.pos.z, pp.x, pp.z) - lim.loaded_bonus > lim.evict => {
            log::info!("race leave: opponent {} evicted, deleted", uid);
            vehicles.cars.remove(i);
            false
        }
        Some(_) => true,
        None => false,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opp(slot: usize, x: f32) -> Opp {
        Opp { slot, uid: 100 + slot as u32, pos: Vec3::new(x, 0.0, 0.0) }
    }

    #[test]
    fn the_leave_is_the_scripts() {
        use Command::*;
        let opps = [opp(1, 10.0), opp(2, 499.0), opp(3, 500.0), opp(9, 20.0)];
        let won = Facts { first: true, finished: true, ..Default::default() };
        let lost = Facts { finished: true, ..Default::default() };
        let o = Some(Vec3::ZERO);
        let p = plan(&Owner::Wrapper, Cruise, &won, o, &opps);
        // (within 500 m, the route by slot - 7 and above take the default, route 7; RaceWrapper has no taunt)
        assert_eq!(p, Plan { keep: vec![(101, 1), (102, 2), (109, 7)], taunts: vec![] });
        // (arrested: nobody stays)
        assert!(plan(&Owner::Wrapper, Cruise, &Facts { arrested: true, ..lost }, o, &opps).keep.is_empty());
        // (a story core with the include's taunts: by the place on RETURN TO CRUISE, then NOT_IN_FIRST after the loop -
        // QUIT and GO TO GARAGE fall into the loop with no first taunt)
        let s = Owner::Story("BEAT_SH_03".into());
        assert_eq!(plan(&s, Cruise, &won, o, &opps).taunts, vec![IN, NOT]);
        assert_eq!(plan(&s, Quit, &won, o, &opps).taunts, vec![NOT]);
        assert_eq!(plan(&s, Garage, &won, o, &opps).taunts, vec![NOT]);
        assert_eq!(plan(&Owner::LocalHookman, Cruise, &lost, o, &opps).taunts, vec![NOT, NOT]);
        // (the cores without the release, and BEAT_SH_04B's release with no taunt)
        assert_eq!(plan(&Owner::Story("RACE_SH_01".into()), Cruise, &won, o, &opps), Plan::default());
        let b = plan(&Owner::Story("BEAT_SH_04B".into()), Cruise, &won, o, &opps);
        assert_eq!((b.keep.len(), b.taunts.len()), (3, 0));
        assert_eq!(plan(&Owner::Other, Cruise, &won, o, &opps), Plan::default());
        // (no player racer: the distance is 100000)
        assert!(plan(&Owner::Wrapper, Cruise, &won, None, &opps).keep.is_empty());
        // (the tournament, TelephoneChallenge, RACE_GH_06: QUIT only, 0)
        for w in [Owner::Tournament, Owner::TelephoneChallenge, Owner::Story("RACE_GH_06".into())] {
            assert_eq!(taunts(&w, Quit, &won), vec![NOT], "{:?}", w);
            assert!(taunts(&w, Cruise, &won).is_empty() && taunts(&w, Garage, &won).is_empty(), "{:?}", w);
        }
        // (the wager: QUIT 0, RETURN TO CRUISE the end sequence's flag - set in any place -, GO TO GARAGE nothing)
        assert_eq!(taunts(&Owner::Hangout, Quit, &won), vec![NOT]);
        assert_eq!(taunts(&Owner::Hangout, Cruise, &lost), vec![IN]);
        assert_eq!(taunts(&Owner::Hangout, Cruise, &Facts::default()), vec![NOT]);
        assert_eq!(taunts(&Owner::Hangout, Cruise, &Facts { arrested: true, ..won }), vec![NOT]);
        assert!(taunts(&Owner::Hangout, Garage, &won).is_empty());
        // (the series' chain, its reward cut and its beat me there leg)
        let sr = Owner::Series;
        assert_eq!(taunts(&sr, Cruise, &Facts { series_won: true, ..won }), vec![IN]);
        assert!(taunts(&sr, Cruise, &Facts { series_won: true, series_cut: true, ..won }).is_empty());
        assert_eq!(taunts(&sr, Cruise, &won), vec![NOT]);
        assert_eq!(taunts(&sr, Quit, &won), vec![NOT]);
        assert_eq!(taunts(&sr, Garage, &won), vec![IN]);
        assert_eq!(taunts(&sr, Garage, &lost), vec![NOT]);
        assert_eq!(taunts(&sr, Garage, &Facts { bmt: true, ..won }), vec![NOT]);
        assert!(taunts(&Owner::Wrapper, Quit, &won).is_empty());
    }

    /// The story lists are the cores' scripts: every name of `STORY_RELEASE` hashes to a story script that has the
    /// release function ("Opponent Distance = "), every name of `STORY_NO_RELEASE` to one that has the end of race
    /// sequence but not the release (skipped without the extracted data).
    #[test]
    fn the_story_lists_are_the_scripts() {
        let root = std::path::Path::new("D:/MCLA/extracted/cache/script_obj");
        let find = |m: &str| -> Option<Vec<u8>> {
            let h = mcla_formats::hash::joaat(&format!("{}_core.sco", m));
            ["game/career/Story", "sc/career/Story"]
                .iter()
                .find_map(|d| std::fs::read(root.join(d).join(format!("0x{:08x}", h))).ok())
                .map(|f| mcla_formats::sco::decode(&f).expect(m).data)
        };
        if !root.is_dir() {
            return;
        }
        let has = |d: &[u8], s: &str| d.windows(s.len()).any(|w| w == s.as_bytes());
        for m in STORY_RELEASE {
            let d = find(m).unwrap_or_else(|| panic!("{} has no story script", m));
            assert!(has(&d, "Opponent Distance = "), "{} has no release", m);
        }
        for m in STORY_NO_RELEASE {
            let d = find(m).unwrap_or_else(|| panic!("{} has no story script", m));
            assert!(!has(&d, "Opponent Distance = "), "{} has the release", m);
            assert!(has(&d, "returnToCruise") || m == "RACE_SH_01", "{}", m);
        }
        // (the series' reward cut: the story scripts that start OrderedSeriesCore with a cut script are these two)
        let mut cut = Vec::new();
        for e in std::fs::read_dir(root.join("game/career/Story")).unwrap().flatten() {
            let d = mcla_formats::sco::decode(&std::fs::read(e.path()).unwrap()).unwrap().data;
            if has(&d, "Game/Race/OrderedSeriesCore") && has(&d, "Game/CineScripts/generated/cut_reward_") {
                cut.push(e.file_name().to_string_lossy().to_string());
            }
        }
        cut.sort();
        let mut want: Vec<String> = SERIES_REWARD_CUT.iter().map(|m| format!("0x{:08x}", mcla_formats::hash::joaat(&format!("{}_core.sco", m)))).collect();
        want.sort();
        assert_eq!(cut, want);
    }
}
