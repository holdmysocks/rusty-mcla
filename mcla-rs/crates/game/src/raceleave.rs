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
//! other scripts' taunts (the series', the wager's, the tournament's, TelephoneChallenge's, RACE_GH_06's) have rules of
//! their own and are not ported.

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

    pub fn taunts(&self) -> bool {
        match self {
            Owner::Story(m) => story_taunts(m),
            Owner::LocalHookman => true,
            _ => false,
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

/// The leave for the cruise. `cruise`: the command is RETURN TO CRUISE (else QUIT); `first`: the player's record +164 is 1.
pub fn plan(owner: &Owner, cruise: bool, arrested: bool, first: bool, player: Option<Vec3>, opps: &[Opp]) -> Plan {
    let mut p = Plan::default();
    if owner.taunts() && cruise {
        p.taunts.push(if first { "TAUNT_POST_IN_FIRST" } else { "TAUNT_POST_NOT_IN_FIRST" });
    }
    if owner.releases() {
        for o in opps {
            if crate::brain::racer_distance(player, Some(o.pos)) < KEEP_WITHIN && !arrested {
                p.keep.push((o.uid, o.slot.min(7)));
            }
        }
    }
    if owner.taunts() {
        p.taunts.push("TAUNT_POST_NOT_IN_FIRST");
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
        let opps = [opp(1, 10.0), opp(2, 499.0), opp(3, 500.0), opp(9, 20.0)];
        let p = plan(&Owner::Wrapper, true, false, true, Some(Vec3::ZERO), &opps);
        // (within 500 m, the route by slot - 7 and above take the default, route 7; RaceWrapper has no taunt)
        assert_eq!(p, Plan { keep: vec![(101, 1), (102, 2), (109, 7)], taunts: vec![] });
        // (arrested: nobody stays)
        assert!(plan(&Owner::Wrapper, true, true, false, Some(Vec3::ZERO), &opps).keep.is_empty());
        // (a story core with the include's taunts: by the place on RETURN TO CRUISE, then NOT_IN_FIRST after the loop)
        let s = Owner::Story("BEAT_SH_03".into());
        assert_eq!(plan(&s, true, false, true, Some(Vec3::ZERO), &opps).taunts, vec!["TAUNT_POST_IN_FIRST", "TAUNT_POST_NOT_IN_FIRST"]);
        assert_eq!(plan(&s, false, false, true, Some(Vec3::ZERO), &opps).taunts, vec!["TAUNT_POST_NOT_IN_FIRST"]);
        assert_eq!(plan(&Owner::LocalHookman, true, false, false, Some(Vec3::ZERO), &opps).taunts, vec!["TAUNT_POST_NOT_IN_FIRST", "TAUNT_POST_NOT_IN_FIRST"]);
        // (the cores without the release, and BEAT_SH_04B's release with no taunt)
        let n = plan(&Owner::Story("RACE_SH_01".into()), true, false, true, Some(Vec3::ZERO), &opps);
        assert_eq!(n, Plan::default());
        let b = plan(&Owner::Story("BEAT_SH_04B".into()), true, false, true, Some(Vec3::ZERO), &opps);
        assert_eq!((b.keep.len(), b.taunts.len()), (3, 0));
        assert_eq!(plan(&Owner::Other, true, false, true, Some(Vec3::ZERO), &opps), Plan::default());
        // (no player racer: the distance is 100000)
        assert!(plan(&Owner::Wrapper, true, false, true, None, &opps).keep.is_empty());
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
    }
}
