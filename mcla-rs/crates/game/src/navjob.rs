//! The brain's nav-mesh query as the original runs it: the state machine F-49348 (brain +128),
//! the readiness test F-25277, the request F-36902 (store start / goal, submit or wait), the submit
//! F-59389 and the finish F-71636 (decode: docs/reference/ai/nav_jobs.md).
//!
//! The original hands the query to the "[RAGE] Navigation Thread" and reads the result on a later
//! brain update. The frame tick waits for that thread before the brains run again (F-13118: lock,
//! then while a query is queued or running and less than 1000 ms have passed: unlock, yield, lock),
//! so a query submitted in brain update N is always finished by brain update N+1: one update of
//! latency, never more. `NavJob` models exactly that: the search runs at the submit (the port has
//! no thread), the result is held as "submitted" (state 2) until the next `update`, which turns it
//! into 4 (path) or 3 (failed) as the original does.

/// Who holds the query (query +8): no list, the nav level's query list, some other list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Owner {
    None,
    Level,
    Other,
}

/// The parts of the query (navQueryQuickestPath) the state machine reads.
#[derive(Clone, Copy, Debug)]
pub struct Query {
    /// +8
    pub owner: Owner,
    /// +25: 1 queued, 4 running, 5 cancel, 6 done, 2 no handler, 3 check failed
    pub status: u8,
    /// +172: elements in the result path
    pub count: i32,
    /// query vfunc 1 (probe: holds a result to release)
    pub has_result: bool,
}

/// The nav level (global level object): present, and its flag byte +36 (bit 0x80 = initialised).
#[derive(Clone, Copy, Debug)]
pub struct Level {
    pub present: bool,
    pub flags: u8,
}

/// Calls the state machine makes: query vfunc 1 (probe), query vfunc 2 (reset), level vfunc 4 (submit).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Calls {
    pub probe: u32,
    pub reset: u32,
    pub submit: u32,
}

/// F-25277: the level exists and is initialised, and the query is in no list, or in the level's list
/// and neither queued, running nor cancelling (then it is taken out of the list, F-39959 -> F-47249).
pub fn ready(level: Level, q: &mut Query) -> bool {
    if !level.present || level.flags & 0x80 == 0 {
        return false;
    }
    match q.owner {
        Owner::None => true,
        Owner::Other => false,
        Owner::Level => {
            if matches!(q.status, 1 | 4 | 5) {
                false
            } else {
                q.owner = Owner::None;
                true
            }
        }
    }
}

/// F-59389: release the old result (probe, then reset if it holds one), set the query up from the
/// stored start / goal (+208 = 1), hand it to the level, state 2.
pub fn submit(state: &mut i32, q: &mut Query, calls: &mut Calls) {
    calls.probe += 1;
    if q.has_result {
        calls.reset += 1;
    }
    calls.submit += 1;
    *state = 2;
}

/// F-49348, at the top of every brain update. State 0: nothing. Not ready: nothing (the query is
/// still with the thread). 1: submit. 2: failed (status 2 / 3 or an empty path) -> 3, else 4.
/// 5: reset, 0.
pub fn update(state: &mut i32, level: Level, q: &mut Query, calls: &mut Calls) {
    if *state == 0 || !ready(level, q) {
        return;
    }
    match *state {
        1 => submit(state, q, calls),
        2 => *state = if q.status == 3 || q.status == 2 || q.count == 0 { 3 } else { 4 },
        5 => {
            calls.reset += 1;
            *state = 0;
        }
        _ => {}
    }
}

/// F-36902 (start / goal stored by the caller): submit at once when ready, else state 1.
pub fn request(state: &mut i32, level: Level, q: &mut Query, calls: &mut Calls) {
    if ready(level, q) {
        submit(state, q, calls);
    } else {
        *state = 1;
    }
}

/// F-71636 (the path was used, or a re-plan): when ready release the result and go to 0, else
/// state 5 (cancel on a later update).
pub fn finish(state: &mut i32, level: Level, q: &mut Query, calls: &mut Calls) {
    if ready(level, q) {
        calls.probe += 1;
        if q.has_result {
            calls.reset += 1;
        }
        *state = 0;
    } else {
        *state = 5;
    }
}

/// The brain's query with its result, for the port's frame: the search runs at the submit and its
/// result waits as the thread's would until the next `update` (the frame tick's wait, F-13118).
#[derive(Clone, Debug)]
pub struct NavJob<P> {
    /// brain +128: 0 idle, 1 waiting to submit, 2 submitted, 3 failed, 4 path ready, 5 cancel
    pub state: i32,
    q: Query,
    result: Option<P>,
}

impl<P> Default for NavJob<P> {
    fn default() -> Self {
        Self { state: 0, q: Query { owner: Owner::None, status: 0, count: 0, has_result: false }, result: None }
    }
}

/// The level is built at start-up (F-89841) and initialised before any brain runs.
const LEVEL: Level = Level { present: true, flags: 0x80 };

impl<P> NavJob<P> {
    /// F-49348 at the top of the brain update.
    pub fn update(&mut self) {
        let mut c = Calls::default();
        update(&mut self.state, LEVEL, &mut self.q, &mut c);
        if c.reset > 0 {
            self.release();
        }
    }

    /// F-36902: plan the route to the next goal; `search` is the quickest-path query (None: no path).
    pub fn request(&mut self, search: impl FnOnce() -> Option<P>) {
        let mut c = Calls::default();
        request(&mut self.state, LEVEL, &mut self.q, &mut c);
        if c.submit > 0 {
            self.release();
            self.result = search();
            // the thread's work, done before the next brain update: status 6, the path's elements
            // (the goal point at least, so never 0 for a path)
            self.q = Query { owner: Owner::Level, status: 6, count: self.result.is_some() as i32, has_result: true };
        }
    }

    /// F-71636: the path was handed on, or the brain plans again.
    pub fn finish(&mut self) {
        let mut c = Calls::default();
        finish(&mut self.state, LEVEL, &mut self.q, &mut c);
        if c.reset > 0 {
            self.release();
        }
    }

    /// The path while the state is 4 (taken out; `put_back` when the aiv has no room yet).
    pub fn take(&mut self) -> Option<P> {
        if self.state == 4 {
            self.result.take()
        } else {
            None
        }
    }

    pub fn put_back(&mut self, p: P) {
        self.result = Some(p);
    }

    /// The driver's "route on its way" test (F-17058 case 5: state 1 or 2).
    pub fn pending(&self) -> bool {
        matches!(self.state, 1 | 2)
    }

    fn release(&mut self) {
        self.result = None;
        self.q.has_result = false;
        self.q.count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(k: u32) -> Owner {
        match k {
            0 => Owner::None,
            1 => Owner::Level,
            _ => Owner::Other,
        }
    }

    /// Every case of the original's F-49348 / F-36902 / F-71636 (with F-25277, F-59389 and the
    /// list removal) run offline: state, query owner and the calls made.
    #[test]
    fn nav_state_machine_matches_the_original() {
        let mut n = 0;
        for line in include_str!("testdata/navjob_sm_ref.txt").lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let v: Vec<i64> = line.split_whitespace().map(|s| s.parse().unwrap()).collect();
            let (op, state0) = (v[0], v[1] as i32);
            let level = Level { present: v[2] != 0, flags: v[3] as u8 };
            let mut q = Query { owner: owner(v[4] as u32), status: v[5] as u8, count: v[6] as i32, has_result: v[7] != 0 };
            let mut state = state0;
            let mut c = Calls::default();
            match op {
                0 => update(&mut state, level, &mut q, &mut c),
                1 => request(&mut state, level, &mut q, &mut c),
                _ => finish(&mut state, level, &mut q, &mut c),
            }
            assert_eq!(state as i64, v[8], "state: {line}");
            assert_eq!(q.owner, owner(v[9] as u32), "owner: {line}");
            assert_eq!((c.probe as i64, c.reset as i64, c.submit as i64), (v[10], v[11], v[12]), "calls: {line}");
            // the submit's set-up: one query set-up, two navPoints, +208 = 1
            assert_eq!((c.submit as i64, 2 * c.submit as i64, c.submit as i64), (v[13], v[14], v[15]), "set-up: {line}");
            assert_eq!(v[16], 1, "start / goal stored: {line}");
            n += 1;
        }
        assert_eq!(n, 1500);
    }

    /// One update of latency: requested in update N, path used in update N+1, driver sees "pending"
    /// for the rest of N; a failed search gives 3 at N+1 and is requested again.
    #[test]
    fn path_arrives_on_the_next_update() {
        let mut j: NavJob<u32> = NavJob::default();
        j.update();
        assert_eq!(j.state, 0);
        j.request(|| Some(7));
        assert_eq!(j.state, 2);
        assert!(j.pending());
        assert_eq!(j.take(), None);
        j.update();
        assert_eq!(j.state, 4);
        assert!(!j.pending());
        assert_eq!(j.take(), Some(7));
        j.finish();
        assert_eq!(j.state, 0);
        j.request(|| None);
        assert_eq!(j.state, 2);
        j.update();
        assert_eq!(j.state, 3);
        j.request(|| Some(9));
        assert_eq!(j.state, 2);
        j.update();
        assert_eq!(j.take(), Some(9));
        // the aiv has no room: the path waits in state 4, the driver does not see it as pending
        j.put_back(9);
        j.update();
        assert_eq!(j.state, 4);
        assert!(!j.pending());
        assert_eq!(j.take(), Some(9));
    }
}
