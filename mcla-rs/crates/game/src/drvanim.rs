//! The in-car driver's animation player (the mcAnim sequencer on the driver object) that the car machine's `Cmd`s drive
//! (`drvmachine.rs`). The sequencer holds its motion-tree nodes through holders: +20 .. +32 the five sweep's nested blends
//! (P1 .. P4, the pair's blend at +20), +36 a blend for PhaseAll, +44 .. +60 the animation nodes (the played clip at +44, the
//! pair's / five's second at +48). A holder's node is used only when its type matches (4 animation, 5 blend) and, for an
//! animation, its clip player has a clip.

/// `fsel(a, b, c)`: b when a >= 0 (a NaN takes c).
fn fsel(a: f32, b: f32, c: f32) -> f32 {
    if a >= 0.0 {
        b
    } else {
        c
    }
}

/// A clip as the player reads it: +12 the duration, +4 bit 0 loop by default.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Clip {
    pub dur: f32,
    pub flags: u16,
}

/// The clip player of an animation node (crmtNodeAnimation +28, F-77198).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Player {
    /// +0: bits 0..1 the loop mode (0 the clip's own, 3 loop, 1 / 2 once)
    pub flags: u32,
    /// +4 the time, +8 the last advance, +12 a pending advance, +16 an extra advance added to +8, +20 the rate
    pub t: f32,
    pub delta: f32,
    pub pending: f32,
    pub extra: f32,
    pub rate: f32,
    pub clip: Option<Clip>,
}

impl Player {
    /// F-55157: loop mode 3 loops, 1 / 2 do not, 0 takes the clip's +4 bit 0.
    pub fn looping(&self) -> bool {
        match self.flags & 3 {
            0 => self.clip.map_or(false, |c| c.flags & 1 != 0),
            m => m == 3,
        }
    }

    /// F-48520: advance by rate x dt (+ the pending advance), wrapping when looping, else stopping at an end. Returns the
    /// time left over of dt when a one-shot clip reached its end (0 otherwise) and the end flag (a loop wrapped, or a
    /// one-shot clip reached its end from inside).
    pub fn advance(&mut self, dt: f32) -> (f32, bool) {
        let (mut left, mut end) = (0.0, false);
        let delta = self.rate.mul_add(dt, self.pending);
        self.pending = 0.0;
        self.delta = 0.0;
        if !(delta.abs() < 1e-6) {
            let t0 = self.t;
            let t1 = t0 + delta;
            self.t = t1;
            if let Some(clip) = self.clip {
                let dur = clip.dur;
                let over = if delta > 0.0 {
                    if t1 < dur {
                        self.delta = delta;
                        None
                    } else {
                        let o = t1 - dur;
                        let over = fsel(delta - o, o, delta);
                        if self.looping() {
                            self.t = if over > dur { (over as f64 % dur as f64) as f32 } else { over };
                            self.delta = delta;
                            end = true;
                            None
                        } else {
                            self.t = dur;
                            let d = delta - over;
                            self.delta = fsel(d, d, 0.0);
                            end = t0 < dur;
                            Some(over)
                        }
                    }
                } else if t1 > 0.0 {
                    self.delta = delta;
                    None
                } else {
                    let over = fsel(delta - t1, delta, t1);
                    if self.looping() {
                        self.t = if -over > dur { dur - ((-over) as f64 % dur as f64) as f32 } else { over + dur };
                        self.delta = delta;
                        end = true;
                        None
                    } else {
                        self.t = 0.0;
                        let d = delta - over;
                        self.delta = fsel(d, 0.0, d);
                        end = t0 > 0.0;
                        Some(over)
                    }
                };
                if let Some(over) = over {
                    let q = over / self.rate;
                    left = fsel(q - dt, dt, q);
                }
            }
        }
        self.delta += self.extra;
        self.extra = 0.0;
        (left, end)
    }

    /// The time clamped into the clip, `fsel` as the phase setters do.
    fn set_time(&mut self, t: f32, dur: f32) {
        let a = fsel(t, t, 0.0);
        self.t = fsel(a - dur, dur, a);
    }
}

/// A motion-tree node the sequencer's holders point at.
#[derive(Clone, Debug)]
pub enum Node {
    /// crmtNodeAnimation (type 4)
    Anim(Player),
    /// crmtNodeBlend (type 5): +32 the weight
    Blend { weight: f32 },
}

impl Node {
    /// The blend node's set weight (vt+56, F-16851): clamped into 0 .. 1.
    fn set_weight(&mut self, w: f32) {
        if let Node::Blend { weight } = self {
            let a = fsel(w, w, 0.0);
            *weight = fsel(a - 1.0, 1.0, a);
        }
    }
}

/// The sequencer's holders: `blend[i]` +20 + 4 i (i < 5), `anim[i]` +44 + 4 i, each the index of its node.
#[derive(Clone, Debug, Default)]
pub struct Seq {
    pub nodes: Vec<Node>,
    pub blend: [Option<usize>; 5],
    pub anim: [Option<usize>; 5],
}

impl Seq {
    /// The player of holder +44 + 4 i when it holds an animation node.
    fn player(&mut self, i: usize) -> Option<&mut Player> {
        match self.anim[i].map(|n| &mut self.nodes[n]) {
            Some(Node::Anim(p)) => Some(p),
            _ => None,
        }
    }

    /// The player of holder +44 + 4 i when it holds an animation node with a clip, and the clip.
    fn clipped(&mut self, i: usize) -> Option<(&mut Player, Clip)> {
        self.player(i).and_then(|p| p.clip.map(|c| (p, c)))
    }

    fn blend_weight(&mut self, i: usize, w: f32) {
        if let Some(n) = self.blend[i] {
            self.nodes[n].set_weight(w);
        }
    }

    /// F-40742: the played clip's time = phase x duration, clamped into the clip.
    pub fn set_phase(&mut self, p: f32) {
        if let Some((pl, c)) = self.clipped(0) {
            pl.t = c.dur * p;
            pl.set_time(pl.t, c.dur);
        }
    }

    /// F-53423: the played clip's rate (no clip needed).
    pub fn set_rate(&mut self, r: f32) {
        if let Some(pl) = self.player(0) {
            pl.rate = r;
        }
    }

    /// F-84631: the rate of all five animation holders.
    pub fn set_rate5(&mut self, r: f32) {
        for i in 0..5 {
            if let Some((pl, _)) = self.clipped(i) {
                pl.rate = r;
            }
        }
    }

    /// F-62772: the phase of all five animation holders.
    pub fn set_phase2(&mut self, p: f32) {
        for i in 0..5 {
            if let Some((pl, c)) = self.clipped(i) {
                pl.t = p * c.dur;
                pl.set_time(pl.t, c.dur);
            }
        }
    }

    /// F-94065: the five sweep: P1 .. P4 weight = clamp(p - k / 4, 0, 1 / 4) x 4.
    pub fn set_phase5(&mut self, p: f32) {
        let part = |x: f32| {
            let a = fsel(x, x, 0.0);
            fsel(a - 0.25, 0.25, a) * 4.0
        };
        let w = [part(p), part(p - 0.25), part(p - 0.5), part(p - 0.75)];
        for (i, w) in w.into_iter().enumerate() {
            self.blend_weight(i, w);
        }
    }

    /// F-30180: the weight of the blend at +36.
    pub fn set_phase_all(&mut self, p: f32) {
        self.blend_weight(4, p);
    }

    /// F-22341: the weight of the pair's blend (+20).
    pub fn set_weight(&mut self, w: f32) {
        self.blend_weight(0, w);
    }

    /// F-87745: the played clip's time left (in clip time, by the rate's sign), -1 when looping or without a clip.
    pub fn remaining(&mut self) -> f32 {
        match self.clipped(0) {
            Some((pl, c)) if !pl.looping() => {
                let r = fsel(pl.rate, c.dur - pl.t, pl.t);
                fsel(-r, 0.0, r)
            }
            _ => -1.0,
        }
    }

    /// F-96834: the played clip's phase, time / duration (not below 0), -1 without a clip.
    pub fn phase(&mut self) -> f32 {
        match self.clipped(0) {
            Some((pl, c)) => {
                let p = pl.t / c.dur;
                fsel(-p, 0.0, p)
            }
            None => -1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The setters, readers and the clip clock against the original's code run on fake nodes
    /// (`work/drvmachine/player_ref.py`): every output word equal.
    #[test]
    fn the_player_matches_the_original() {
        let b = |x: f32| format!("{:08x}", x.to_bits());
        let mut n = 0;
        for line in include_str!("testdata/drvanim_player_ref.txt").lines() {
            let (case, want) = line.split_once(" -> ").unwrap();
            let mut it = case.split_whitespace();
            let op = it.next().unwrap();
            let w: Vec<u32> = it.map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let mut s = Seq::default();
            for i in 0..5 {
                s.blend[i] = Some(s.nodes.len());
                s.nodes.push(Node::Blend { weight: 0.0 });
            }
            for i in 0..5 {
                let g = |k: usize| w[(6 * i + k) % 24];
                s.anim[i] = Some(s.nodes.len());
                s.nodes.push(Node::Anim(Player {
                    flags: g(0) & 3,
                    t: f32::from_bits(g(1)),
                    rate: f32::from_bits(g(2)),
                    pending: f32::from_bits(g(5)),
                    clip: Some(Clip { dur: f32::from_bits(g(4)), flags: (g(3) & 1) as u16 }),
                    ..Player::default()
                }));
            }
            let x = f32::from_bits(w[23]);
            let got = if op == "adv" {
                let p = s.player(0).unwrap();
                let (left, end) = p.advance(x);
                format!("{} {} {} {} {} {}", b(p.t), b(p.delta), b(p.pending), b(p.extra), end as u8, b(left))
            } else {
                let r = match op {
                    "phase" => (s.set_phase(x), x).1,
                    "rate" => (s.set_rate(x), x).1,
                    "rate5" => (s.set_rate5(x), x).1,
                    "phase2" => (s.set_phase2(x), x).1,
                    "phase5" => (s.set_phase5(x), x).1,
                    "phaseall" => (s.set_phase_all(x), x).1,
                    "weight" => (s.set_weight(x), x).1,
                    "remaining" => s.remaining(),
                    "readphase" => s.phase(),
                    _ => panic!("{op}"),
                };
                let mut o: Vec<String> = Vec::new();
                for i in 0..5 {
                    if let Node::Blend { weight } = s.nodes[s.blend[i].unwrap()] {
                        o.push(b(weight));
                    }
                }
                for i in 0..5 {
                    let p = s.player(i).unwrap();
                    o.push(b(p.t));
                    o.push(b(p.rate));
                }
                o.push(b(r));
                o.join(" ")
            };
            assert_eq!(got, want, "case {n}: {case}");
            n += 1;
        }
        assert!(n > 10_000, "{n} cases");
    }
}
