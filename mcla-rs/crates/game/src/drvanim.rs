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

/// A clip as the player reads it: +12 the duration, +4 bit 0 loop by default (`id` the caller's index).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Clip {
    pub id: ClipId,
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
    /// crmtNodeBlend (type 5): +32 the weight, +36 the rate a second, +48 destroy (keep the child it reaches), +40 the filter;
    /// kids 0 / 1 (a crossfade's 0 is the tree it fades from)
    Blend { weight: f32, rate: f32, destroy: bool, filter: Option<Filter>, kids: [Option<usize>; 2] },
    /// crmtNodeAddSubtract (type 6): +32 the weight, +36 its rate (0 from every builder), +40 the filter; kid 1 added onto 0
    Add { weight: f32, rate: f32, filter: Option<Filter>, kids: [Option<usize>; 2] },
    /// crmtNodeIdentity (type 23): the rest pose
    Identity,
    /// a destroyed node's slot
    Free,
}

impl Node {
    /// The blend node's set weight (vt+56, F-16851): clamped into 0 .. 1.
    fn set_weight(&mut self, w: f32) {
        if let Node::Blend { weight, .. } = self {
            let a = fsel(w, w, 0.0);
            *weight = fsel(a - 1.0, 1.0, a);
        }
    }
}

/// The sequencer's holders: `blend[i]` +20 + 4 i (i < 5), `anim[i]` +44 + 4 i, `pan` +76, `top` +84, each the index of its
/// node (none once the node is destroyed), and its motion tree: `root` (crmtMotionTree +8) over `nodes`.
#[derive(Clone, Debug, Default)]
pub struct Seq {
    pub nodes: Vec<Node>,
    pub blend: [Option<usize>; 5],
    pub anim: [Option<usize>; 5],
    /// +68 / +72: the additive overlays' animation nodes (F-42342 / F-85022)
    pub over: [Option<usize>; 2],
    pub pan: Option<usize>,
    pub top: Option<usize>,
    pub root: Option<usize>,
    free: Vec<usize>,
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

    /// The pan clip's time (holder +76, F-47264 as docs/PEOPLE_TRAFFIC.md reads it): clamp(p x duration, 0, duration).
    pub fn set_pan_time(&mut self, p: f32) {
        if let Some(Node::Anim(pl)) = self.pan.map(|n| &mut self.nodes[n]) {
            if let Some(c) = pl.clip {
                pl.set_time(c.dur * p, c.dur);
            }
        }
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

/// A clip of the driver's packs, by the index the caller gives it.
pub type ClipId = u32;

/// The driver's bone filters: +92 the spine00 subtree, +96 the neck subtree, +100 all but the Facial_root subtree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Spine,
    Neck,
    NoFace,
}

/// A motion-tree request as the builders hand it to the motion tree (crmtRequest*); `hold` lists the sequencer holders
/// (+20 .. +84) that get the node it becomes.
#[derive(Clone, Debug, PartialEq)]
pub struct Req {
    pub kind: Kind,
    pub hold: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// crmtRequestAnimation (F-29648): the clip from time `t` at `rate`; `set` its +32 (the loop mode given: +33 `looping`), else
    /// the generated player takes the clip's own loop flag (the additive overlays' base requests, F-42342 / F-85022)
    Anim { clip: ClipId, t: f32, rate: f32, looping: bool, set: bool },
    /// crmtRequestAddSubtract (F-75019): `b` added onto `a` by weight `w` on the filter's bones
    Add { w: f32, filter: Option<Filter>, a: Box<Req>, b: Box<Req> },
    /// crmtRequestBlend (F-63623): `b` over `a` by `w`, the weight moving at `rate` a second; no `a` blends from the tree in place (a
    /// crossfade)
    Blend { w: f32, rate: f32, filter: Option<Filter>, a: Option<Box<Req>>, b: Box<Req> },
    /// crmtRequestIdentity (F-79659): the rest pose
    Identity,
}

impl Req {
    fn new(kind: Kind) -> Req {
        Req { kind, hold: Vec::new() }
    }

    fn held(mut self, h: u8) -> Req {
        self.hold.push(h);
        self
    }

    /// An animation request as the builders make it (F-29648: time 0, rate 1; +32 = 1, +33 = loop).
    fn anim(clip: ClipId, looping: bool) -> Req {
        Req::new(Kind::Anim { clip, t: 0.0, rate: 1.0, looping, set: true })
    }

    /// A blend request (its ctor clamps the weight into 0 .. 1).
    fn blend(w: f32, rate: f32, filter: Option<Filter>, a: Option<Req>, b: Req) -> Req {
        let c = fsel(w, w, 0.0);
        let w = fsel(c - 1.0, 1.0, c);
        Req::new(Kind::Blend { w, rate, filter, a: a.map(Box::new), b: Box::new(b) })
    }

    fn add(filter: Option<Filter>, a: Req, b: Req) -> Req {
        Req::new(Kind::Add { w: 1.0, filter, a: Box::new(a), b: Box::new(b) })
    }

    /// The crossfade onto `b` over `blend` seconds (rate 1 / blend), unless `noblend` or no blend time.
    fn crossfade(self, blend: f32, noblend: bool) -> Req {
        if noblend || blend == 0.0 {
            self
        } else {
            Req::blend(0.0, 1.0 / blend, None, None, self)
        }
    }
}

/// What the tree builder reads of the driver (the sequencer's owner): +104 the face clip (the human's, F-52181), +108 an
/// extra pan clip (filtered from the spine down; no car sets it), +112 the pan clip (from the neck down, F-56423), and whether
/// the filters exist (+100; a driver without them gets the bare tree).
#[derive(Clone, Copy, Debug, Default)]
pub struct Look {
    pub face: Option<ClipId>,
    pub pan_spine: Option<ClipId>,
    pub pan: Option<ClipId>,
    pub filters: bool,
}

/// The tree builder F-18940: the face (or the rest pose) under the body clip `d` with the face filter, the pan clip added at
/// half its length and rate 0 (holder +76), then the crossfade; the root goes to +84.
pub fn build(look: &Look, dur: &dyn Fn(ClipId) -> f32, d: Req, blend: f32, noblend: bool) -> Req {
    let root = if !look.filters {
        d
    } else {
        let pan = |c: ClipId| Req::new(Kind::Anim { clip: c, t: dur(c) * 0.5, rate: 0.0, looping: false, set: true }).held(76);
        let face = || look.face.map_or(Req::new(Kind::Identity), |c| Req::anim(c, true));
        let nf = Some(Filter::NoFace);
        match (look.pan_spine, look.face, look.pan) {
            (Some(p), Some(_), _) => Req::blend(1.0, 0.0, nf, Some(face()), Req::add(None, d, pan(p))),
            (Some(p), None, _) => Req::add(Some(Filter::Spine), Req::blend(1.0, 0.0, nf, Some(face()), d), pan(p)),
            (None, _, Some(p)) => Req::add(Some(Filter::Neck), Req::blend(1.0, 0.0, nf, Some(face()), d), pan(p)),
            (None, _, None) => Req::blend(1.0, 0.0, nf, Some(face()), d),
        }
    };
    root.crossfade(blend, noblend).held(84)
}

/// Play F-24441: the clip (holder +44) through the tree builder.
pub fn play(look: &Look, dur: &dyn Fn(ClipId) -> f32, clip: ClipId, blend: f32, looping: bool, noblend: bool) -> Req {
    build(look, dur, Req::anim(clip, looping).held(44), blend, noblend)
}

/// Play without the builder F-73665: the clip alone (no face, no pan, no filter), then the crossfade.
pub fn play_cut(clip: ClipId, blend: f32, looping: bool, noblend: bool) -> Req {
    Req::anim(clip, looping).held(44).crossfade(blend, noblend).held(84)
}

/// Pair F-85393: two clips (+44, +48) in a blend of weight 0 (+20), through the tree builder.
pub fn pair(look: &Look, dur: &dyn Fn(ClipId) -> f32, a: ClipId, b: ClipId, looping: bool, blend: f32) -> Req {
    let p = Req::blend(0.0, 0.0, None, Some(Req::anim(a, looping).held(44)), Req::anim(b, looping).held(48)).held(20);
    build(look, dur, p, blend, false)
}

/// Five F-45908: five clips (+44 .. +60, only +44 / +48 held) in nested blends of weight 0, P1 = (A0, A1) .. P4 = (P3, A4)
/// (+20 .. +32), through the tree builder.
pub fn five(look: &Look, dur: &dyn Fn(ClipId) -> f32, c: [ClipId; 5], blend: f32, looping: bool) -> Req {
    let mut p = Req::anim(c[0], looping).held(44);
    for (i, &x) in c[1..].iter().enumerate() {
        let b = Req::anim(x, looping);
        p = Req::blend(0.0, 0.0, None, Some(p), if i == 0 { b.held(48) } else { b }).held(20 + 4 * i as u8);
    }
    build(look, dur, p, blend, false)
}

/// Overlay F-19950: stops the body clip (+44 rate 0), then crossfades (always, at 1 / blend) onto the overlay clip blended
/// fully over a copy of the body clip at its time (rate 1, once) with the face filter; the copy goes to +44, the overlay to
/// +48. Nothing without the filters or without a body clip.
pub fn overlay(s: &mut Seq, look: &Look, clip: ClipId, blend: f32, looping: bool) -> Option<Req> {
    if !look.filters {
        return None;
    }
    s.set_rate(0.0);
    let p = s.player(0)?;
    let (t, body) = (p.t, p.clip?.id);
    let b = Req::new(Kind::Anim { clip: body, t, rate: 1.0, looping: false, set: true }).held(44);
    let p = Req::blend(1.0, 0.0, Some(Filter::NoFace), Some(Req::anim(clip, looping).held(48)), b);
    Some(Req::blend(0.0, 1.0 / blend, None, None, p).held(84))
}

/// The pair's additive overlay F-42342: the pair rebuilt from the clips in +44 / +48 (from 0 at rate 1, the clips' own loop
/// flags; blend +20 of weight 0) plus a second pair of the overlay clips `a` / `b` (+68 / +72, blend +36 of weight 0, `looping`),
/// added at weight 1 with no filter, through the tree builder over `blend` seconds. Nothing without a clip in +44 or +48 (the
/// original builds with no clip then; no caller reaches it).
pub fn pair_overlay(s: &mut Seq, look: &Look, dur: &dyn Fn(ClipId) -> f32, a: ClipId, b: ClipId, looping: bool, blend: f32) -> Option<Req> {
    let base = |s: &mut Seq, i: usize, h: u8| -> Option<Req> {
        let c = s.clipped(i)?.1.id;
        Some(Req::new(Kind::Anim { clip: c, t: 0.0, rate: 1.0, looping: false, set: false }).held(h))
    };
    let (x, y) = (base(s, 0, 44)?, base(s, 1, 48)?);
    let p = Req::blend(0.0, 0.0, None, Some(x), y).held(20);
    let o = Req::blend(0.0, 0.0, None, Some(Req::anim(a, looping).held(68)), Req::anim(b, looping).held(72)).held(36);
    Some(build(look, dur, Req::add(None, p, o), blend, false))
}

/// The single additive overlay F-85022: the clip in +44 rebuilt (from 0 at rate 1, its own loop flag) plus the overlay clip
/// (+68, `looping`) added at weight 1 with no filter, through the tree builder over `blend` seconds.
pub fn one_overlay(s: &mut Seq, look: &Look, dur: &dyn Fn(ClipId) -> f32, clip: ClipId, looping: bool, blend: f32) -> Option<Req> {
    let c = s.clipped(0)?.1.id;
    let x = Req::new(Kind::Anim { clip: c, t: 0.0, rate: 1.0, looping: false, set: false }).held(44);
    Some(build(look, dur, Req::add(None, x, Req::anim(clip, looping).held(68)), blend, false))
}

/// The overlay's end F-72270 (the rider's fidget exit, blend 0): with the filters, the body clip (+44) is stopped (rate 0)
/// and the tree rebuilt as that clip alone from its time (F-12110: max(t, 0)), playing at rate 1 once, under the face
/// filter over the rest pose (+44); at a blend of 0 this is the new root at once (no crossfade), else it fades in at
/// 1 / blend (+84). Nothing without the filters, or without a clip in +44 (the original builds with no clip then; no caller
/// reaches it).
pub fn stop_overlay(s: &mut Seq, look: &Look, blend: f32) -> Option<Req> {
    if !look.filters {
        return None;
    }
    s.set_rate(0.0);
    let p = s.player(0)?;
    let (t, body) = (p.t, p.clip?.id);
    let t = fsel(-t, 0.0, t);
    let b = Req::new(Kind::Anim { clip: body, t, rate: 1.0, looping: false, set: true }).held(44);
    let p = Req::blend(1.0, 0.0, Some(Filter::NoFace), Some(Req::new(Kind::Identity)), b);
    Some(if blend == 0.0 { p.held(84) } else { Req::blend(0.0, 1.0 / blend, None, None, p).held(84) })
}

impl Seq {
    /// The player of overlay holder +68 when it holds an animation node.
    fn over_player(&mut self) -> Option<&mut Player> {
        match self.over[0].map(|n| &mut self.nodes[n]) {
            Some(Node::Anim(p)) => Some(p),
            _ => None,
        }
    }

    /// F-72214: the rate of the overlay at +68 (no clip needed).
    pub fn set_overlay_rate(&mut self, r: f32) {
        if let Some(p) = self.over_player() {
            p.rate = r;
        }
    }

    /// F-70114: the overlay's (+68) time left, as F-87745 for +44: -1 when looping or without a clip.
    pub fn overlay_remaining(&mut self) -> f32 {
        match self.over_player() {
            Some(pl) if pl.clip.is_some() && !pl.looping() => {
                let c = pl.clip.unwrap();
                let r = fsel(pl.rate, c.dur - pl.t, pl.t);
                fsel(-r, 0.0, r)
            }
            _ => -1.0,
        }
    }

    fn alloc(&mut self, n: Node) -> usize {
        match self.free.pop() {
            Some(i) => {
                self.nodes[i] = n;
                i
            }
            None => {
                self.nodes.push(n);
                self.nodes.len() - 1
            }
        }
    }

    fn holder(&mut self, h: u8) -> Option<&mut Option<usize>> {
        match h {
            20..=36 => Some(&mut self.blend[(h as usize - 20) / 4]),
            44..=60 => Some(&mut self.anim[(h as usize - 44) / 4]),
            68 | 72 => Some(&mut self.over[(h as usize - 68) / 4]),
            76 => Some(&mut self.pan),
            84 => Some(&mut self.top),
            _ => None,
        }
    }

    fn kids(&self, n: usize) -> [Option<usize>; 2] {
        match &self.nodes[n] {
            Node::Blend { kids, .. } | Node::Add { kids, .. } => *kids,
            _ => [None; 2],
        }
    }

    /// A node's destruction with its subtree (the replace F-54662 / F-91652, the children released); the holders on them let go.
    fn destroy(&mut self, n: usize) {
        for k in self.kids(n).into_iter().flatten() {
            self.destroy(k);
        }
        self.nodes[n] = Node::Free;
        self.free.push(n);
        for h in self.blend.iter_mut().chain(self.anim.iter_mut()).chain(self.over.iter_mut()).chain([&mut self.pan, &mut self.top]) {
            if *h == Some(n) {
                *h = None;
            }
        }
    }

    /// The request's generate (F-14509 -> vt+8: animation F-26354 with the node init F-45890, blend F-82335 / add F-71347 through
    /// F-69377, identity F-20211): the node it makes takes the place of `existing`
    /// (the tree's root), which is destroyed, except under a crossfade (a blend without a first source), which keeps it as its
    /// first child.
    fn gen(&mut self, r: &Req, existing: Option<usize>, dur: &dyn Fn(ClipId) -> f32, looped: &dyn Fn(ClipId) -> bool) -> usize {
        let mut keep = false;
        let n = match &r.kind {
            Kind::Anim { clip, t, rate, looping, set } => {
                let d = dur(*clip);
                // (F-26354: the loop mode cleared, then 1 / 3 when +32 is set; mode 0 reads the clip's +4 bit 0)
                let flags = if *set { 1 | (*looping as u32) << 1 } else { 0 };
                let mut p = Player { flags, rate: *rate, clip: Some(Clip { id: *clip, dur: d, flags: looped(*clip) as u16 }), ..Player::default() };
                p.set_time(*t, d);
                self.alloc(Node::Anim(p))
            }
            Kind::Identity => self.alloc(Node::Identity),
            Kind::Add { w, filter, a, b } => {
                let (a, b) = (self.gen(a, None, dur, looped), self.gen(b, None, dur, looped));
                self.alloc(Node::Add { weight: *w, rate: 0.0, filter: *filter, kids: [Some(a), Some(b)] })
            }
            Kind::Blend { w, rate, filter, a, b } => {
                let a = match a {
                    Some(a) => Some(self.gen(a, None, dur, looped)),
                    None => {
                        keep = true;
                        existing
                    }
                };
                let b = self.gen(b, None, dur, looped);
                self.alloc(Node::Blend { weight: *w, rate: *rate, destroy: *rate != 0.0, filter: *filter, kids: [a, Some(b)] })
            }
        };
        if let (Some(e), false) = (existing, keep) {
            self.destroy(e);
        }
        for h in &r.hold {
            if let Some(x) = self.holder(*h) {
                *x = Some(n);
            }
        }
        n
    }

    /// The motion tree's submit F-47118 with no node named: the request's tree replaces (or crossfades from) the root.
    pub fn submit(&mut self, r: &Req, dur: &dyn Fn(ClipId) -> f32) {
        self.submit_with(r, dur, &|_| false);
    }

    /// The submit with the clips' own loop flags (+4 bit 0), which a request without its +32 leaves the player to.
    pub fn submit_with(&mut self, r: &Req, dur: &dyn Fn(ClipId) -> f32, looped: &dyn Fn(ClipId) -> bool) {
        let n = self.gen(r, self.root, dur, looped);
        self.root = Some(n);
    }

    fn update_node(&mut self, n: usize, dt: f32) -> usize {
        let mut kids = self.kids(n);
        for k in kids.iter_mut() {
            if let Some(c) = *k {
                *k = Some(self.update_node(c, dt));
            }
        }
        match &mut self.nodes[n] {
            // F-57339
            Node::Anim(p) => {
                p.advance(dt);
            }
            // F-42325
            Node::Add { weight, rate, kids: k, .. } => {
                *k = kids;
                *weight = dt.mul_add(*rate, *weight);
            }
            // F-51260
            Node::Blend { weight, rate, destroy, kids: k, .. } => {
                *k = kids;
                let w = dt.mul_add(*rate, *weight);
                let a = fsel(w, w, 0.0);
                *weight = fsel(a - 1.0, 1.0, a);
                let keep = if *rate > 0.0 && *weight == 1.0 {
                    Some(1)
                } else if *rate < 0.0 && *weight == 0.0 {
                    Some(0)
                } else {
                    None
                };
                if let (Some(i), true) = (keep, *destroy) {
                    if let Some(c) = kids[i] {
                        if let Node::Blend { kids: k, .. } = &mut self.nodes[n] {
                            k[i] = None;
                        }
                        self.destroy(n);
                        return c;
                    }
                }
            }
            _ => {}
        }
        n
    }

    /// The tree's update F-72449 (its updater pass F-52503): post-order, a node's children first; animations advance by dt, blends move
    /// their weight by rate x dt and a finished crossfade gives its place to the child it reached.
    pub fn update(&mut self, dt: f32) {
        if let Some(r) = self.root {
            self.root = Some(self.update_node(r, dt));
        }
    }
}

// ---- the composer's per-DOF frame operations with a filter's weights (crFrame::Blend / AddSubtract through the
// dispatcher F-63377 when the node has a filter). `dst` and `src` are frames of one layout (skin.rs `FrameDof`), `wts` the
// filter's weight per DOF; DOFs whose weight is +-0 are skipped, as are sources without a value. Scalar multiply-adds are
// fused (`fma`), the vector unit's steps flush denormals and are not fused.

use crate::physics::exact::fma;
use crate::skin::{FrameDof, DOF_FLOAT, DOF_NO_VALUE, DOF_QUATERNION, DOF_VECTOR3};

const W_LO: f32 = f32::from_bits(0x3A83_126F); // 0.001
const W_HI: f32 = f32::from_bits(0x3F7F_BE77); // 0.999
/// the add's vector threshold on w squared (image constant, 1e-6)
const W2_ADD: f32 = f32::from_bits(0x3586_37BE);
/// the axis normalisation's epsilon (1e-5)
const AXIS_EPS: f32 = f32::from_bits(0x3727_C5AC);

#[inline]
fn flush(x: f32) -> f32 {
    if x.is_subnormal() { 0.0f32.copysign(x) } else { x }
}
#[inline]
fn vmul(a: f32, b: f32) -> f32 {
    flush(flush(a) * flush(b))
}
#[inline]
fn vadd(a: f32, b: f32) -> f32 {
    flush(flush(a) + flush(b))
}
#[inline]
fn vsub(a: f32, b: f32) -> f32 {
    flush(flush(a) - flush(b))
}

/// The estimate 1 / sqrt in single, two Newton steps in double, 0 when the squared length is not above 0 (fsel).
fn rsqrt2(n2: f32) -> f32 {
    if -(n2 as f64) >= 0.0 {
        return 0.0;
    }
    let n = n2 as f64;
    let mut y = (1.0 / n.sqrt()) as f32 as f64;
    for _ in 0..2 {
        let e = -(y * y).mul_add(n, -1.0);
        y = e.mul_add(y * 0.5, y);
    }
    y as f32
}

/// The filtered blend F-18341: per DOF wi = weight x w; a Quaternion without a value takes the source's, else (z y x w dot
/// not >= 0: the destination negated) d (1 - wi) + s wi normalised; a Vector3 is kept for wi <= 0.001, takes the source's
/// from 0.999 or without a value, else the vector lerp; a Float lerps below 0.999 (fused), else takes the source's when wi >
/// 0.001; other types take the source's word unless wi < 0.5. A written Vector3 / Float / other holds a value afterwards.
pub fn frame_blend_filtered(dst: &mut [FrameDof], src: &[FrameDof], wts: &[f32], w: f32) {
    for ((d, s), &k) in dst.iter_mut().zip(src.iter()).zip(wts.iter()) {
        if s.flags & DOF_NO_VALUE != 0 || k.to_bits() & 0x7FFF_FFFF == 0 {
            continue;
        }
        let wi = k * w;
        let empty = d.flags & DOF_NO_VALUE != 0;
        match d.flags & 0xF {
            DOF_QUATERNION => {
                if empty {
                    d.v = s.v;
                    d.flags &= !DOF_NO_VALUE;
                    continue;
                }
                let (a, b) = (d.v, s.v);
                let dot = fma(a[3], b[3], fma(a[0], b[0], fma(b[1], a[1], b[2] * a[2])));
                if !(dot >= 0.0) {
                    d.v = d.v.map(|x| -x);
                }
                let a = d.v;
                let c = 1.0 - wi;
                let q = [fma(a[0], c, wi * b[0]), fma(a[1], c, b[1] * wi), fma(b[2], wi, a[2] * c), fma(a[3], c, wi * b[3])];
                let n2 = fma(q[3], q[3], fma(q[2], q[2], fma(q[0], q[0], q[1] * q[1])));
                let r = rsqrt2(n2);
                d.v = q.map(|x| x * r);
            }
            DOF_VECTOR3 => {
                if wi <= W_LO {
                    continue;
                }
                d.v = if !(wi >= W_HI) && !empty { std::array::from_fn(|i| vadd(vmul(vsub(s.v[i], d.v[i]), wi), d.v[i])) } else { s.v };
                d.flags &= !DOF_NO_VALUE;
            }
            DOF_FLOAT => {
                if !(wi >= W_HI) && !empty {
                    d.v[0] = fma(s.v[0] - d.v[0], wi, d.v[0]);
                } else if wi > W_LO {
                    d.v[0] = s.v[0];
                } else {
                    continue;
                }
                d.flags &= !DOF_NO_VALUE;
            }
            _ => {
                if wi < 0.5 {
                    continue;
                }
                d.v[0] = s.v[0];
                d.flags &= !DOF_NO_VALUE;
            }
        }
    }
}

/// A quaternion scaled along its rotation (F-90945): angle = `quat_angle(w)`, the axis (x, y, z)
/// normalised (F-41334: above eps 1e-5, else 0); when |axis|^2 > 0.999 and angle^2 > 0 it becomes the rotation of angle x
/// k about the axis (F-25905: w = cos, xyz = axis sin of the half angle), else it is kept.
fn quat_scaled(q: [f32; 4], k: f32) -> [f32; 4] {
    use crate::physics::exact::{guest_cos, guest_sin};
    let ang = crate::skin::quat_angle(q[3]);
    let mut ax = [q[0], q[1], q[2]];
    let n2 = fma(ax[2], ax[2], fma(ax[0], ax[0], ax[1] * ax[1]));
    if n2 > AXIS_EPS {
        let r = rsqrt2(n2);
        ax = ax.map(|x| vmul(x, r));
    } else {
        ax = [0.0; 3];
    }
    let l2 = fma(ax[1], ax[1], fma(ax[0], ax[0], ax[2] * ax[2]));
    if !(l2 > W_HI) || !(ang * ang > 0.0) {
        return q;
    }
    let h = ang * k * 0.5;
    let c = guest_cos(h as f64) as f32;
    let s = guest_sin(h as f64) as f32;
    [ax[0] * s, ax[1] * s, ax[2] * s, c]
}

/// `fnmsubs`: -(a c - b), fused, the negation after the rounding (an exact zero comes out -0).
#[inline]
fn fnms(a: f32, c: f32, b: f32) -> f32 {
    -fma(a, c, -b)
}

/// The filtered add-subtract F-37398: per DOF wi = weight x w, nothing where the destination has no value. A Quaternion
/// takes the source scaled along its rotation by |wi| (`quat_scaled`), then the destination times it (one product for wi
/// > 0, another for wi < 0, nothing for 0); a Vector3 adds s wi when wi^2 > 1e-6; a Float adds s wi (fused); an integer
/// adds the source's when |wi - 1| <= 0.001 (|wi| > 0.001).
pub fn frame_add_filtered(dst: &mut [FrameDof], src: &[FrameDof], wts: &[f32], w: f32) {
    for ((d, s), &k) in dst.iter_mut().zip(src.iter()).zip(wts.iter()) {
        if s.flags & DOF_NO_VALUE != 0 || k.to_bits() & 0x7FFF_FFFF == 0 {
            continue;
        }
        let wi = k * w;
        let empty = d.flags & DOF_NO_VALUE != 0;
        match d.flags & 0xF {
            DOF_QUATERNION => {
                if empty {
                    continue;
                }
                let [dx, dy, dz, dw] = d.v;
                if wi < 0.0 {
                    let [sx, sy, sz, sw] = quat_scaled(s.v, -wi);
                    let f2 = fma(dx, sw, -(dw * sx));
                    let f1 = fma(dx, sx, dy * sy);
                    let f8 = fma(dy, sw, -(dw * sy));
                    let f7 = fma(dz, sw, -(dw * sz));
                    let f5 = fnms(dy, sz, f2);
                    let f3 = fma(dw, sw, f1);
                    let f2 = fnms(dz, sx, f8);
                    let f1 = fnms(dx, sy, f7);
                    d.v = [fma(dz, sy, f5), fma(dx, sz, f2), fma(dy, sx, f1), fma(dz, sz, f3)];
                } else if wi > 0.0 {
                    let [sx, sy, sz, sw] = quat_scaled(s.v, wi);
                    let f2 = fma(dy, sw, dw * sy);
                    let f1 = fma(dw, sw, -(dx * sx));
                    let f6 = fma(dy, sx, dz * sw);
                    let f8 = fma(dw, sx, dz * sy);
                    let f5 = fma(dx, sz, f2);
                    let f3 = fnms(dy, sy, f1);
                    let f1 = fma(dw, sz, f6);
                    let f2 = fma(dx, sw, f8);
                    d.v = [fnms(dy, sz, f2), fnms(dz, sx, f5), fnms(dx, sy, f1), fnms(dz, sz, f3)];
                }
            }
            DOF_VECTOR3 => {
                if wi * wi > W2_ADD && !empty {
                    d.v = std::array::from_fn(|i| vadd(vmul(s.v[i], wi), d.v[i]));
                }
            }
            DOF_FLOAT => {
                if !empty {
                    d.v[0] = fma(s.v[0], wi, d.v[0]);
                }
            }
            _ => {
                if wi.abs() > W_LO && !((wi - 1.0).abs() > W_LO) {
                    d.v[0] = f32::from_bits(d.v[0].to_bits().wrapping_add(s.v[0].to_bits()));
                }
            }
        }
    }
}

// ---- the composer pass: the motion tree to one frame (crmtComposerOptimized, the tree's +12). The tree is walked post-order
// into an op list (1 animation, 12 animation without a clip, 13 identity, 4 blend, 6 add-subtract), the optimizer drops
// what a blend or add cannot show, then the ops run on a stack of frames.

/// What the composer reads of the character. A frame is the skeleton's DOFs in a fixed layout (`len`).
pub trait Poser {
    fn len(&self) -> usize;
    /// The clip's frame at time `t` into `out` (a fresh frame: DOFs the clip has no track for hold no value). `filtered`:
    /// the animation lies under operand 1 of a blend with a filter, which the hand-down F-31817 gives the animation op, so
    /// the sampler F-40465 takes its filtered body F-80057 (`sample_track_in` with `mapped`); otherwise F-12216, whose
    /// same-layout loop or mapped loop is picked by the frame's layout against the clip's +16.
    fn sample(&self, clip: ClipId, t: f32, filtered: bool, out: &mut [FrameDof]);
    /// The identity node's frame: the skeleton's rest pose.
    fn rest(&self, out: &mut [FrameDof]);
    /// A filter's weight per DOF (crFrameFilterBoneBasic: the bone's weight, 1 inside the set and 0 outside; 1 for DOFs
    /// that are not a bone's).
    fn weights(&self, f: Filter) -> &[f32];
    /// A fresh frame of the layout: every DOF without a value (its type kept, so a blend into it treats it by its type).
    fn blank(&self) -> Vec<FrameDof> {
        vec![FrameDof { flags: DOF_NO_VALUE, v: [0.0; 4] }; self.len()]
    }
}

impl Seq {
    /// Op 12: an animation node without a clip (its frame holds no value).
    fn no_clip(&self, n: usize) -> bool {
        matches!(&self.nodes[n], Node::Anim(p) if p.clip.is_none())
    }

    /// The node whose op is on top of the stack for node `n` after the optimizer (F-94730): a
    /// blend whose weight is <= 0.001 in size, or whose child 1 is op 12, leaves child 0; a blend without a filter whose
    /// weight is >= 0.999, or whose child 0 is op 12, leaves child 1; an add-subtract of weight <= 0.001 in size or with
    /// child 1 op 12 leaves child 0; a NaN weight counts as <= 0.001 (checked against the original offline,
    /// test the_optimizer_matches_the_original).
    fn kept(&self, n: usize) -> usize {
        match &self.nodes[n] {
            Node::Blend { weight, filter, kids: [Some(a), Some(b)], .. } => {
                let (a, b) = (self.kept(*a), self.kept(*b));
                // (the original's ble / bge also branch on an unordered compare: a NaN weight leaves child 0)
                if !(weight.abs() > W_LO) || self.no_clip(b) {
                    a
                } else if filter.is_none() && (!(*weight < W_HI) || self.no_clip(a)) {
                    b
                } else {
                    n
                }
            }
            Node::Add { weight, kids: [Some(a), Some(b)], .. } => {
                let (a, b) = (self.kept(*a), self.kept(*b));
                if !(weight.abs() > W_LO) || self.no_clip(b) {
                    a
                } else {
                    n
                }
            }
            _ => n,
        }
    }

    /// The composer pass: the tree's frame, or none without a root.
    pub fn compose(&self, p: &dyn Poser) -> Option<Vec<FrameDof>> {
        self.root.map(|r| self.frame(self.kept(r), false, p))
    }

    /// `under`: the node lies under operand 1 of a kept blend with a filter. The hand-down F-31817 (after the optimizer, on
    /// the ops left) gives each animation op in the range of a filtered blend's operand 1 the blend's filter (more than one
    /// make a combined record); the animation is then sampled through the filter. Its values on the filter's DOFs are the
    /// filtered body's (`sample_track_in`, mapped) and its other DOFs hold no value, which no blend above reads (each
    /// filtered blend skips a DOF of weight 0, and the masks are 0 / 1), so the port samples them all.
    fn frame(&self, n: usize, under: bool, p: &dyn Poser) -> Vec<FrameDof> {
        let mut f = p.blank();
        match &self.nodes[n] {
            Node::Anim(pl) => {
                if let Some(c) = pl.clip {
                    p.sample(c.id, pl.t, under, &mut f);
                }
            }
            Node::Identity => p.rest(&mut f),
            // (a kept blend: the frame of child 1 blended onto child 0's; F-27951 clamps the weight)
            Node::Blend { weight, filter, kids: [Some(a), Some(b)], .. } => {
                f = self.frame(self.kept(*a), under, p);
                let s = self.frame(self.kept(*b), under || filter.is_some(), p);
                match filter {
                    Some(x) => frame_blend_filtered(&mut f, &s, p.weights(*x), crate::skin::frame_weight(*weight)),
                    None => crate::skin::frame_blend(&mut f, &s, *weight),
                }
            }
            Node::Add { weight, filter, kids: [Some(a), Some(b)], .. } => {
                f = self.frame(self.kept(*a), under, p);
                let s = self.frame(self.kept(*b), under, p);
                // (the unfiltered add F-89611 is the filtered one with every weight 1)
                match filter {
                    Some(x) => frame_add_filtered(&mut f, &s, p.weights(*x), *weight),
                    None => frame_add_filtered(&mut f, &s, &vec![1.0; s.len()], *weight),
                }
            }
            _ => {}
        }
        f
    }
}

// ---- the clip sampler: a clip at time t into a frame (F-47152 -> F-40465 -> without a filter F-12216), DOF by track.

/// The key index and the fraction for time `t` (F-47152, F-10710 / F-30330, F-12216), in the block `sample_block` picks.
pub fn sample_pos(a: &mcla_formats::rage::anim::Animation, t: f32) -> (usize, f32) {
    let (_, i, fr) = sample_block(a, t);
    (i, fr)
}

/// The tracks of block `b` (a clip of one block: its tracks).
pub fn block_tracks(a: &mcla_formats::rage::anim::Animation, b: usize) -> &[mcla_formats::rage::anim::Track] {
    a.block_tracks.get(b).filter(|_| a.blocks > 1).map_or(&a.tracks[..], |t| &t[..])
}

/// The block, the key index and the fraction for time `t` (F-47152, F-10710 / F-30330, F-12216): frame = (frames - 1) /
/// duration x t, clamped to [0, frames - 1]; the block F-10710 = the frame / the block length (+0a, single), truncated, its
/// low 16 bits, at most blocks - 1; less the block's start (F-30330: fnms(block length, block, frame)), clamped to [0, block
/// length + 1]; the key index the floor of it clamped to [0, the block's frames (+10) - 2], the fraction what is left
/// clamped to [0, 1].
pub fn sample_block(a: &mcla_formats::rage::anim::Animation, t: f32) -> (usize, usize, f32) {
    let last = (a.frames as i64 - 1) as f32;
    let pos = (last / a.duration) * t;
    let pos = fsel(pos, pos, 0.0);
    let pos = fsel(pos - last, last, pos);
    // (F-10710: fdivs, fctidz - a NaN gives the integer 0x8000_0000_0000_0000 - then lhz of the low half, cmplw)
    let q = pos / a.block_len as f32;
    let iv: i64 = if q.is_nan() { i64::MIN } else { q as i64 };
    let b = (iv as u16).min(a.blocks.wrapping_sub(1));
    let blk = b as f32;
    let local = fnms(a.block_len as f32, blk, pos);
    let local = fsel(local, local, 0.0);
    let hi = (a.block_len as i64 + 1) as f32;
    let local = fsel(local - hi, hi, local);
    let fl = (local as f64).floor() as f32;
    let fl = fsel(fl, fl, 0.0);
    let bf = a.block_frames.get(b as usize).copied().unwrap_or(a.block0_frames);
    let top = (bf as i64 - 2) as f32;
    let fl = fsel(fl - top, top, fl);
    let i = fl as i64 as i32;
    let fr = local - i as f32;
    let fr = fsel(fr, fr, 0.0);
    (b as usize, i.max(0) as usize, fsel(fr - 1.0, 1.0, fr))
}

/// A float channel at key `i` and fraction `fr`: a static float; a raw one a + (b - a) fr (fused); a quantised one the same on
/// fma(q, scale, offset) of keys i and i + 1 (F-35923, F-53505, F-62929 / F-60697).
fn chan_at(c: &mcla_formats::rage::anim::Chan, i: usize, fr: f32) -> f32 {
    use mcla_formats::rage::anim::Chan;
    let at = |v: usize, n: usize| v.min(n.saturating_sub(1));
    match c {
        Chan::Static(v) => *v,
        Chan::Raw(v) if !v.is_empty() => {
            let (a, b) = (v[at(i, v.len())], v[at(i + 1, v.len())]);
            fma(b - a, fr, a)
        }
        Chan::Quant(q, sc, off) if !q.is_empty() => {
            let a = fma(q[at(i, q.len())] as f32, *sc, *off);
            let b = fma(q[at(i + 1, q.len())] as f32, *sc, *off);
            fma(b - a, fr, a)
        }
        Chan::Static4(v) => v[0],
        _ => 0.0,
    }
}

/// One track into its DOF (F-12216): a Quaternion of one channel per component (track byte +1 high nibble 0x10) is the four
/// channels normalised (z y w x order of the squares, the estimate rsqrt with two double Newton steps); of one channel, its
/// 16-byte value (F-14184). A Vector3 of one channel per component sets x y z (w kept); of one channel, its 16 bytes. The DOF
/// holds a value afterwards. (The three-channel quaternion with w rebuilt is not in the driver clips: not ported.)
pub fn sample_track(tr: &mcla_formats::rage::anim::Track, i: usize, fr: f32, d: &mut FrameDof) {
    sample_track_in(tr, i, fr, d, false)
}

/// `sample_track` as the sampler's loops differ: the squared norm of a quaternion is summed x, w, y, z (fused) in F-12216's
/// same-layout loop and its per-DOF lookup loop, and y, x, z, w in its mapped loop and in all of the filtered body F-80057's
/// loops (`mapped`); the rest is the same (checked offline against all five, work/driverlight/fsamp).
pub fn sample_track_in(tr: &mcla_formats::rage::anim::Track, i: usize, fr: f32, d: &mut FrameDof, mapped: bool) {
    use mcla_formats::rage::anim::Chan;
    let per = tr.flags & 0xF0 == 0x10;
    match d.flags & 0xF {
        DOF_QUATERNION if per && tr.chans.len() >= 4 => {
            let [x, y, z, w] = std::array::from_fn(|k| chan_at(&tr.chans[k], i, fr));
            let n2 = if mapped { fma(w, w, fma(z, z, fma(x, x, y * y))) } else { fma(z, z, fma(y, y, fma(w, w, x * x))) };
            let r = rsqrt2(n2);
            d.v = [r * x, r * y, r * z, r * w];
        }
        DOF_VECTOR3 if per && tr.chans.len() >= 3 => {
            for k in 0..3 {
                d.v[k] = chan_at(&tr.chans[k], i, fr);
            }
        }
        DOF_QUATERNION | DOF_VECTOR3 if !per => match tr.chans.first() {
            Some(Chan::Static4(v)) => d.v = *v,
            _ => return,
        },
        // (a float DOF - the camera's FOV, the face's float tracks: F-15435 asks the track's first channel, slot +36)
        DOF_FLOAT => match tr.chans.first() {
            Some(c) => d.v[0] = chan_at(c, i, fr),
            None => return,
        },
        _ => return,
    }
    d.flags &= !DOF_NO_VALUE;
}

/// One DOF of a clip at time `t` as F-15435 samples it (r6 0: no frame rounding) - the scene camera, the scene cars, the
/// scenes' movers and roots read their tracks with it: the block, key and fraction of `sample_block`, the track of that
/// block with the DOF's kind and id (the block's sorted list, F-75513's search), its value by the DOF's type (`DOF_VECTOR3`
/// x y z, `DOF_QUATERNION` normalised in the mapped order, `DOF_FLOAT` x); None when the block has no such track. Checked
/// against the original run offline on the 239 cine packs (test `the_dof_sampler_matches_the_original`).
pub fn sample_dof(a: &mcla_formats::rage::anim::Animation, kind: u8, id: u16, ty: u8, t: f32) -> Option<[f32; 4]> {
    let (b, i, fr) = sample_block(a, t);
    let tr = block_tracks(a, b).iter().find(|x| x.kind == kind && x.bone_id == id)?;
    let mut d = FrameDof { flags: ty | DOF_NO_VALUE, v: [0.0; 4] };
    sample_track_in(tr, i, fr, &mut d, true);
    (d.flags & DOF_NO_VALUE == 0).then_some(d.v)
}

#[cfg(test)]
mod tests {
    /// F-15435 run offline on the original's code over the 239 cine packs (work/driverlight/cinesamp/samp_ref.py: every clip's
    /// root / mover / camera tracks and three others, 12 times each, block edges and the ends included - 61,860 cases; the
    /// committed 4,000 of them, `MCLA_CINESAMP_FULL=<samp_ref.txt>` for all): "<pack> <anim> <kind> <id> <type> <t> -> <ret>
    /// <flags> <4 words>"; `sample_dof` gives every value word (x y z of a vector, the four of a quaternion, x of a float)
    /// bit for bit. Skipped without the disc's data.
    #[test]
    fn the_dof_sampler_matches_the_original() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let full = std::env::var("MCLA_CINESAMP_FULL").ok().and_then(|p| std::fs::read_to_string(p).ok());
        let text = full.as_deref().unwrap_or(include_str!("testdata/cine_dof_ref.txt"));
        let mut packs: std::collections::HashMap<String, Option<mcla_formats::rsc::Resource>> = Default::default();
        let mut anims: std::collections::HashMap<(String, usize), mcla_formats::rage::anim::Animation> = Default::default();
        let (mut n, mut bad) = (0, 0);
        for line in text.lines() {
            let (a, b) = line.split_once(" -> ").unwrap();
            let w: Vec<&str> = a.split(' ').collect();
            let o: Vec<&str> = b.split(' ').collect();
            let h = |s: &str| u32::from_str_radix(s, 16).unwrap();
            let pack = if w[0].contains('/') { w[0].to_string() } else { format!("resources/animation/cineanimpacks/{}.r1", w[0]) };
            let res = packs.entry(pack.clone()).or_insert_with(|| crate::assets::load_resource_file(&std::path::Path::new(&root).join(&pack)).ok());
            let Some(res) = res.as_ref() else { return };
            let off = h(w[1]) as usize;
            let an = anims.entry((pack, off)).or_insert_with(|| mcla_formats::rage::anim::animation_named(&mcla_formats::rage::Rd::new(res), off).unwrap());
            let (kind, id, ty, t) = (h(w[2]) as u8, h(w[3]) as u16, h(w[4]) as u8, f32::from_bits(h(w[5])));
            let words = match ty { DOF_QUATERNION => 4, DOF_FLOAT => 1, _ => 3 };
            let want: Vec<u32> = o[2..2 + words].iter().map(|s| h(s)).collect();
            let got = sample_dof(an, kind, id, ty, t).map(|v| v[..words].iter().map(|x| x.to_bits()).collect::<Vec<u32>>());
            if (o[0] == "1") != got.is_some() || got.as_ref().is_some_and(|g| *g != want) {
                if bad < 10 {
                    eprintln!("{line}
  port {got:08x?}");
                }
                bad += 1;
            }
            n += 1;
        }
        assert_eq!(bad, 0, "{bad} of {n} differ");
        assert!(n >= 4000, "{n}");
    }

    use super::*;

    /// The request tree in the offline harness's words (preorder ids) and the holders in the builders' order.
    fn dump(r: &Req, ids: &mut u32, holds: &mut Vec<(u8, u32)>, o: &mut String) {
        let b = |x: f32| format!("{:08x}", x.to_bits());
        let fl = |f: &Option<Filter>| match f {
            None => "-",
            Some(Filter::Spine) => "F92",
            Some(Filter::Neck) => "F96",
            Some(Filter::NoFace) => "F100",
        };
        let id = *ids;
        *ids += 1;
        holds.extend(r.hold.iter().map(|&h| (h, id)));
        match &r.kind {
            Kind::Anim { clip, t, rate, looping, set } => *o += &format!(" #{id} anim(c{clip} {} {} {} {} 0)", b(*t), b(*rate), *set as u8, (*looping && *set) as u8),
            Kind::Add { w, filter, a, b: x } => {
                *o += &format!(" #{id} add({} {}", b(*w), fl(filter));
                dump(a, ids, holds, o);
                dump(x, ids, holds, o);
                *o += ")";
            }
            Kind::Blend { w, rate, filter, a, b: x } => {
                *o += &format!(" #{id} blend({} {} {} {} 00000000", b(*w), b(*rate), (*rate != 0.0) as u8, fl(filter));
                match a {
                    Some(a) => dump(a, ids, holds, o),
                    None => *o += " null",
                }
                dump(x, ids, holds, o);
                *o += ")";
            }
            Kind::Identity => *o += &format!(" #{id} identity"),
        }
    }

    /// The rider's additive overlays (F-42342 pair, F-85022 single), the overlay's rate (F-72214) and its time left (F-70114)
    /// against the original's run offline (`work/driverlight/ovl/ovl_ref.py`, 2,000 cases): every request, its fields and the
    /// holders, the rate and the time left equal.
    #[test]
    fn the_additive_overlays_match_the_original() {
        let text = include_str!("testdata/drvanim_ovl_ref.txt");
        let mut n = 0;
        for case in text.split("= ").skip(1) {
            let (head, want) = case.split_once('\n').unwrap();
            let want = want.replace('\r', "");
            let mut it = head.split_whitespace();
            let op = it.next().unwrap();
            let w: Vec<u32> = it.map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let look = Look { face: (w[0] != 0).then_some(14), pan_spine: (w[1] != 0).then_some(13), pan: (w[2] != 0).then_some(15), filters: w[14] == 0 };
            let dur = |c: ClipId| if c == 13 || c == 15 || c == 12 || c == 11 || c == 10 { f(13) } else { f32::from_bits(0x3f80_0000 + 0x0010_0000 * c) };
            let clip = |i: usize| w[6 + i];
            let mut s = Seq::default();
            for (i, c) in [12, 11].into_iter().enumerate() {
                s.anim[i] = Some(s.nodes.len());
                s.nodes.push(Node::Anim(Player { t: f(11), rate: f(12), clip: Some(Clip { id: c, dur: f(13), flags: 0 }), ..Player::default() }));
            }
            s.over[0] = Some(s.nodes.len());
            let oc = (w[16] != 0).then_some(Clip { id: 10, dur: f(13), flags: w[17] as u16 });
            s.nodes.push(Node::Anim(Player { flags: w[15], t: f(11), rate: f(12), clip: oc, ..Player::default() }));
            let mut got = format!("{op}\n");
            let req = match op {
                "addpair" => pair_overlay(&mut s, &look, &dur, clip(0), clip(1), w[4] != 0, f(3)),
                "addone" => one_overlay(&mut s, &look, &dur, clip(0), w[4] != 0, f(3)),
                "stopg" => stop_overlay(&mut s, &look, f(3)),
                "ovrate" => {
                    s.set_overlay_rate(f(3));
                    if let Some(Node::Anim(p)) = s.nodes.get(2) {
                        got += &format!("rate68 {:08x}
", p.rate.to_bits());
                    }
                    None
                }
                "ovrem" => {
                    got += &format!("rem68 {:08x}
", s.overlay_remaining().to_bits());
                    None
                }
                _ => panic!("{op}"),
            };
            if let Some(r) = &req {
                let (mut ids, mut holds, mut o) = (0, Vec::new(), String::new());
                dump(r, &mut ids, &mut holds, &mut o);
                got += &format!("submit 0{o}\n");
                let order = [84u8, 76, 44, 48, 20, 68, 72, 36];
                holds.sort_by_key(|(h, _)| order.iter().position(|x| x == h));
                for (h, id) in holds {
                    got += &format!("hold {h} {id}\n");
                }
            }
            if op == "stopg" {
                if let Some(Node::Anim(p)) = s.nodes.first() {
                    got += &format!("rate44 {:08x}\n", p.rate.to_bits());
                }
            }
            // (F-72270 at a blend of 0 hands +44 the rest pose's request, then the clip's: the last one holds)
            let lines: Vec<&str> = want.lines().collect();
            let want: String = lines
                .iter()
                .enumerate()
                .filter(|(i, l)| !(l.starts_with("hold ") && lines[i + 1..].iter().any(|m| m.split(' ').nth(1) == l.split(' ').nth(1) && m.starts_with("hold "))))
                .map(|(_, l)| format!("{l}
"))
                .collect();
            assert_eq!(got, want, "case {n}: {head}");
            n += 1;
        }
        assert_eq!(n, 2500);
    }

    /// The tree builders against the original's run offline (`work/drvmachine/tree_ref.py`): every request, its fields and
    /// the holders equal, over the face / pan / filter / blend / loop combinations.
    #[test]
    fn the_trees_match_the_original() {
        let text = include_str!("testdata/drvanim_tree_ref.txt");
        let mut n = 0;
        for case in text.split("= ").skip(1) {
            let (head, want) = case.split_once('\n').unwrap();
            // (a checkout with autocrlf has the file in CRLF)
            let want = want.replace('\r', "");
            let mut it = head.split_whitespace();
            let op = it.next().unwrap();
            let w: Vec<u32> = it.map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let f = |i: usize| f32::from_bits(w[i]);
            let look = Look {
                face: (w[0] != 0).then_some(14),
                pan_spine: (w[1] != 0).then_some(13),
                pan: (w[2] != 0).then_some(15),
                filters: w[14] == 0,
            };
            let dur = |c: ClipId| if c == 13 || c == 15 || c == 12 { f(13) } else { f32::from_bits(0x3f80_0000 + 0x0010_0000 * c) };
            let clip = |i: usize| w[6 + i];
            let mut s = Seq::default();
            s.anim[0] = Some(0);
            s.nodes.push(Node::Anim(Player { t: f(11), rate: f(12), clip: Some(Clip { id: 12, dur: f(13), flags: 0 }), ..Player::default() }));
            let req = match op {
                "play" => Some(play(&look, &dur, clip(0), f(3), w[4] != 0, w[5] != 0)),
                "play2" => Some(play_cut(clip(0), f(3), w[4] != 0, w[5] != 0)),
                "pair" => Some(pair(&look, &dur, clip(0), clip(1), w[4] != 0, f(3))),
                "five" => Some(five(&look, &dur, [clip(0), clip(1), clip(2), clip(3), clip(4)], f(3), w[4] != 0)),
                "overlay" => overlay(&mut s, &look, clip(0), f(3), w[4] != 0),
                _ => panic!("{op}"),
            };
            let mut got = format!("{op}\n");
            if let Some(r) = &req {
                let (mut ids, mut holds, mut o) = (0, Vec::new(), String::new());
                dump(r, &mut ids, &mut holds, &mut o);
                got += &format!("submit 0{o}\n");
                let order = [84u8, 76, 44, 48, 20, 24, 28, 32];
                holds.sort_by_key(|(h, _)| order.iter().position(|x| x == h));
                for (h, id) in holds {
                    got += &format!("hold {h} {id}\n");
                }
            }
            if op == "overlay" {
                if let Some(Node::Anim(p)) = s.nodes.first() {
                    got += &format!("rate44 {:08x}\n", p.rate.to_bits());
                }
            }
            assert_eq!(got, want, "case {n}: {head}");
            n += 1;
        }
        assert!(n >= 2000, "{n} cases");
    }

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
                s.nodes.push(Node::Blend { weight: 0.0, rate: 0.0, destroy: false, filter: None, kids: [None; 2] });
            }
            for i in 0..5 {
                let g = |k: usize| w[(6 * i + k) % 24];
                s.anim[i] = Some(s.nodes.len());
                s.nodes.push(Node::Anim(Player {
                    flags: g(0) & 3,
                    t: f32::from_bits(g(1)),
                    rate: f32::from_bits(g(2)),
                    pending: f32::from_bits(g(5)),
                    clip: Some(Clip { id: 0, dur: f32::from_bits(g(4)), flags: (g(3) & 1) as u16 }),
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
                    if let Node::Blend { weight, .. } = s.nodes[s.blend[i].unwrap()] {
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

    fn dump_node(s: &Seq, n: usize, ids: &mut Vec<usize>, o: &mut String) {
        let b = |x: f32| format!("{:08x}", x.to_bits());
        let fl = |f: &Option<Filter>| match f {
            None => "-",
            Some(Filter::Spine) => "F92",
            Some(Filter::Neck) => "F96",
            Some(Filter::NoFace) => "F100",
        };
        let id = ids.len();
        ids.push(n);
        match &s.nodes[n] {
            Node::Anim(p) => {
                *o += &format!(" #{id} t4({} {} {} {} c{})", p.flags & 3, b(p.t), b(p.delta), b(p.rate), p.clip.map_or(0, |c| c.id));
                return;
            }
            Node::Blend { weight, rate, destroy, filter, .. } => *o += &format!(" #{id} t5({} {} {} {}", b(*weight), b(*rate), *destroy as u8, fl(filter)),
            Node::Add { weight, rate, filter, .. } => *o += &format!(" #{id} t6({} {} {} 00000000", b(*weight), b(*rate), fl(filter)),
            Node::Identity => *o += &format!(" #{id} t23("),
            Node::Free => *o += " FREE(",
        }
        for k in s.kids(n).into_iter().flatten() {
            dump_node(s, k, ids, o);
        }
        *o += ")";
    }

    /// The motion tree against the original's run whole offline (`work/drvmachine/tree_run.py`): real tree, holders, builders,
    /// submit and updater; after every update the node tree (types, weights, rates, destroy flags, filters, clip players) and
    /// the holders equal.
    #[test]
    fn the_motion_tree_matches_the_original() {
        let want = include_str!("testdata/drvanim_run_ref.txt").replace('\r', "");
        let f = |x: &str| f32::from_bits(u32::from_str_radix(x, 16).unwrap());
        let u = |x: &str| u32::from_str_radix(x, 16).unwrap();
        let dur = |c: ClipId| f32::from_bits(0x3f80_0000 + 0x0010_0000 * c);
        let (mut s, mut look) = (Seq::default(), Look::default());
        let mut got = String::new();
        let mut updates = 0;
        for line in include_str!("testdata/drvanim_run_cases.txt").lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            if w.is_empty() {
                continue;
            }
            let op = w[0];
            if op == "=" {
                s = Seq::default();
                look = Look {
                    face: (w[1] == "1").then_some(14),
                    pan_spine: (w[2] == "1").then_some(13),
                    pan: (w[3] == "1").then_some(15),
                    filters: true,
                };
                got += &format!("= {} {} {}\n", w[1], w[2], w[3]);
                continue;
            }
            let req = match op {
                "play" => Some(play(&look, &dur, u(w[1]), f(w[2]), w[3] == "1", w[4] == "1")),
                "play2" => Some(play_cut(u(w[1]), f(w[2]), w[3] == "1", w[4] == "1")),
                "pair" => Some(pair(&look, &dur, u(w[1]), u(w[2]), w[3] == "1", f(w[4]))),
                "five" => Some(five(&look, &dur, [u(w[1]), u(w[2]), u(w[3]), u(w[4]), u(w[5])], f(w[6]), w[7] == "1")),
                "overlay" => overlay(&mut s, &look, u(w[1]), f(w[2]), w[3] == "1"),
                _ => None,
            };
            if let Some(r) = req {
                s.submit(&r, &dur);
            }
            match op {
                "phase" => s.set_phase(f(w[1])),
                "rate" => s.set_rate(f(w[1])),
                "rate5" => s.set_rate5(f(w[1])),
                "phase2" => s.set_phase2(f(w[1])),
                "phase5" => s.set_phase5(f(w[1])),
                "phaseall" => s.set_phase_all(f(w[1])),
                "weight" => s.set_weight(f(w[1])),
                _ => {}
            }
            if op == "upd" {
                s.update(f(w[1]));
                let mut ids = Vec::new();
                got += "tree";
                match s.root {
                    Some(r) => dump_node(&s, r, &mut ids, &mut got),
                    None => got += " -",
                }
                got += "\nhold";
                let hs = [s.blend[0], s.blend[1], s.blend[2], s.blend[3], s.blend[4], None, s.anim[0], s.anim[1], s.anim[2], s.anim[3], s.anim[4], s.pan, s.top];
                for h in hs {
                    got += &format!(" {}", h.map_or(-1, |n| ids.iter().position(|&x| x == n).map_or(-2, |i| i as i64)));
                }
                got += "\n";
                updates += 1;
            } else {
                got += &format!("{op}\n");
            }
        }
        for (i, (g, w)) in got.lines().zip(want.lines()).enumerate() {
            assert_eq!(g, w, "line {}", i + 1);
        }
        assert_eq!(got.lines().count(), want.lines().count());
        assert!(updates > 3000, "{updates} updates");
    }

    /// The filtered blend F-18341 and add-subtract F-37398 against the original's code run offline
    /// (work/drvmachine/composer_ref.py): 3,000 cases of up to 8 DOFs of every type.
    #[test]
    fn the_filtered_blend_and_add_match_the_original() {
        let text = include_str!("testdata/drvanim_composer_ref.txt");
        let (mut n, mut bad) = (0, 0);
        for line in text.lines() {
            let (case, want) = line.split_once(" ->").unwrap();
            let mut it = case.split_whitespace();
            let m = it.next().unwrap();
            let w: Vec<u32> = it.map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let want: Vec<u32> = want.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let k = w[1] as usize;
            let f = |x: u32| f32::from_bits(x);
            let (mut d, mut s, mut wts) = (vec![], vec![], vec![]);
            for i in 0..k {
                let q = &w[2 + 11 * i..];
                wts.push(f(q[0]));
                d.push(FrameDof { flags: q[1] as u8, v: [f(q[2]), f(q[3]), f(q[4]), f(q[5])] });
                s.push(FrameDof { flags: q[6] as u8, v: [f(q[7]), f(q[8]), f(q[9]), f(q[10])] });
            }
            if m == "L" {
                frame_blend_filtered(&mut d, &s, &wts, f(w[0]));
            } else {
                frame_add_filtered(&mut d, &s, &wts, f(w[0]));
            }
            for i in 0..k {
                let wi = wts[i] * f(w[0]);
                // (the add's scaled integer path reads a stack word it never wrote: not compared)
                if m == "A" && d[i].flags & 0xF > 2 && s[i].flags & DOF_NO_VALUE == 0 && wts[i].to_bits() & 0x7FFF_FFFF != 0
                    && wi.abs() > W_LO && (wi - 1.0).abs() > W_LO
                {
                    continue;
                }
                let got = [d[i].flags as u32, d[i].v[0].to_bits(), d[i].v[1].to_bits(), d[i].v[2].to_bits(), d[i].v[3].to_bits()];
                n += 1;
                if got[..] != want[5 * i..5 * i + 5] {
                    if bad < 8 {
                        eprintln!("{line}
  dof {i}: got {got:08x?}");
                    }
                    bad += 1;
                }
            }
        }
        assert_eq!(bad, 0, "{bad} of {n} DOFs differ");
        assert!(n > 10_000, "{n}");
    }

    /// The optimizer F-94730 against the original's code run offline (work/driverlight/opt/opt_ref.py): 4,000 random
    /// op lists of blends, adds, clip / clip-less animations and identities, the weights around 0.001 / 0.999 and NaN,
    /// with and without a filter; the ops the original leaves in its list are the port's tree after `kept`, in post-order.
    #[test]
    fn the_optimizer_matches_the_original() {
        let text = include_str!("testdata/drvanim_opt_ref.txt");
        fn emit(s: &Seq, n: usize, out: &mut Vec<usize>) {
            let n = s.kept(n);
            if let Node::Blend { kids: [Some(a), Some(b)], .. } | Node::Add { kids: [Some(a), Some(b)], .. } = &s.nodes[n] {
                emit(s, *a, out);
                emit(s, *b, out);
            }
            out.push(n);
        }
        let (mut cases, mut dropped) = (0, 0);
        for line in text.lines() {
            let (ops, left) = line.split_once(" -> ").unwrap();
            let mut s = Seq::default();
            let mut st: Vec<usize> = Vec::new();
            for tok in ops.split(' ') {
                let f: Vec<&str> = tok.split(':').collect();
                let w = f32::from_bits(u32::from_str_radix(f[1], 16).unwrap());
                let filter = (f[2] == "1").then_some(Filter::Spine);
                let node = match u32::from_str_radix(f[0], 16).unwrap() {
                    1 => Node::Anim(Player { clip: Some(Clip { id: 1, dur: 1.0, flags: 0 }), ..Default::default() }),
                    12 => Node::Anim(Player::default()),
                    13 => Node::Identity,
                    op => {
                        let b = st.pop().unwrap();
                        let a = st.pop().unwrap();
                        if op == 4 {
                            Node::Blend { weight: w, rate: 0.0, destroy: false, filter, kids: [Some(a), Some(b)] }
                        } else {
                            Node::Add { weight: w, rate: 0.0, filter, kids: [Some(a), Some(b)] }
                        }
                    }
                };
                s.nodes.push(node);
                st.push(s.nodes.len() - 1);
            }
            assert_eq!(st.len(), 1, "{line}");
            let mut got = Vec::new();
            emit(&s, st[0], &mut got);
            let want: Vec<usize> = left.split(' ').map(|x| x.parse().unwrap()).collect();
            assert_eq!(got, want, "{line}");
            cases += 1;
            dropped += (s.nodes.len() != want.len()) as usize;
        }
        assert!(cases == 4000 && dropped > 1000, "{cases} {dropped}");
    }

    /// The flush prelude F-92880 (the optimizer F-94730, then the filter hand-down F-31817) against the original's code run
    /// offline (work/driverlight/hand/hand_ref.py): 4,000 random op lists with three real bone filters. An animation op the
    /// original leaves with a filter (or a combined record of several) is one the port samples `under` a filtered blend,
    /// and its filters are the filtered blends above it whose operand 1 holds it, innermost first.
    #[test]
    fn the_filter_hand_down_matches_the_original() {
        let text = include_str!("testdata/drvanim_hand_ref.txt");
        fn emit(s: &Seq, n: usize, up: &mut Vec<Filter>, out: &mut Vec<String>) {
            let n = s.kept(n);
            let name = |f: &Filter| match f {
                Filter::Spine => '0',
                Filter::Neck => '1',
                Filter::NoFace => '2',
            };
            match &s.nodes[n] {
                Node::Blend { filter, kids: [Some(a), Some(b)], .. } => {
                    emit(s, *a, up, out);
                    up.extend(*filter);
                    emit(s, *b, up, out);
                    if filter.is_some() {
                        up.pop();
                    }
                    out.push(n.to_string());
                }
                Node::Add { kids: [Some(a), Some(b)], .. } => {
                    emit(s, *a, up, out);
                    emit(s, *b, up, out);
                    out.push(n.to_string());
                }
                Node::Anim(p) if p.clip.is_some() && !up.is_empty() => out.push(format!("{n}:{}", up.iter().rev().map(name).collect::<String>())),
                _ => out.push(n.to_string()),
            }
        }
        let (mut cases, mut handed) = (0, 0);
        for line in text.lines() {
            let (ops, left) = line.split_once(" -> ").unwrap();
            let mut s = Seq::default();
            let mut st: Vec<usize> = Vec::new();
            for tok in ops.split(' ') {
                let f: Vec<&str> = tok.split(':').collect();
                let w = f32::from_bits(u32::from_str_radix(f[1], 16).unwrap());
                let filter = match f[2] {
                    "1" => Some(Filter::Spine),
                    "2" => Some(Filter::Neck),
                    "3" => Some(Filter::NoFace),
                    _ => None,
                };
                let node = match u32::from_str_radix(f[0], 16).unwrap() {
                    1 => Node::Anim(Player { clip: Some(Clip { id: 1, dur: 1.0, flags: 0 }), ..Default::default() }),
                    12 => Node::Anim(Player::default()),
                    13 => Node::Identity,
                    op => {
                        let b = st.pop().unwrap();
                        let a = st.pop().unwrap();
                        if op == 4 {
                            Node::Blend { weight: w, rate: 0.0, destroy: false, filter, kids: [Some(a), Some(b)] }
                        } else {
                            Node::Add { weight: w, rate: 0.0, filter, kids: [Some(a), Some(b)] }
                        }
                    }
                };
                s.nodes.push(node);
                st.push(s.nodes.len() - 1);
            }
            let mut got = Vec::new();
            emit(&s, st[0], &mut Vec::new(), &mut got);
            let want: Vec<&str> = left.split(' ').collect();
            assert_eq!(got, want, "{line}");
            cases += 1;
            handed += want.iter().filter(|x| x.contains(':')).count();
        }
        assert!(cases == 4000 && handed > 1500, "{cases} {handed}");
    }

    /// The unfiltered add-subtract F-89611 ('U') and the add dispatcher F-97665 with no filter and one layout ('D')
    /// against the original's code run offline (work/driverlight/addu/addu_ref.py): both are the filtered add with
    /// every weight 1, the port's op 6 without a filter.
    #[test]
    fn the_unfiltered_add_matches_the_original() {
        let text = include_str!("testdata/drvanim_addu_ref.txt");
        let (mut n, mut bad, mut modes) = (0, 0, [0; 2]);
        for line in text.lines() {
            let (case, want) = line.split_once(" ->").unwrap();
            let mut it = case.split_whitespace();
            let m = it.next().unwrap();
            modes[(m == "D") as usize] += 1;
            let w: Vec<u32> = it.map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let want: Vec<u32> = want.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let k = w[1] as usize;
            let f = |x: u32| f32::from_bits(x);
            let (mut d, mut s) = (vec![], vec![]);
            for i in 0..k {
                let q = &w[2 + 11 * i..];
                d.push(FrameDof { flags: q[1] as u8, v: [f(q[2]), f(q[3]), f(q[4]), f(q[5])] });
                s.push(FrameDof { flags: q[6] as u8, v: [f(q[7]), f(q[8]), f(q[9]), f(q[10])] });
            }
            let wts = vec![1.0; k];
            frame_add_filtered(&mut d, &s, &wts, f(w[0]));
            for i in 0..k {
                let wi = f(w[0]);
                // (the add's scaled integer path reads a stack word it never wrote: not compared)
                if d[i].flags & 0xF > 2 && s[i].flags & DOF_NO_VALUE == 0 && wi.abs() > W_LO && (wi - 1.0).abs() > W_LO {
                    continue;
                }
                let got = [d[i].flags as u32, d[i].v[0].to_bits(), d[i].v[1].to_bits(), d[i].v[2].to_bits(), d[i].v[3].to_bits()];
                n += 1;
                if got[..] != want[5 * i..5 * i + 5] {
                    if bad < 8 {
                        eprintln!("{line}\n  dof {i}: got {got:08x?}");
                    }
                    bad += 1;
                }
            }
        }
        assert_eq!(bad, 0, "{bad} of {n} DOFs differ");
        assert!(n > 8_000 && modes[0] > 1400 && modes[1] > 1400, "{n} {modes:?}");
    }

    /// The composer's whole flush F-66956 against the original's code run offline (work/driverlight/flush/flush_ref.py):
    /// random op lists of real clips of the Challenger 70 player pack (op 1 at random times, op 12, op 4 / op 6 with weights
    /// about the optimizer's thresholds and one of three real bone filters or none), every pooled frame of one clip's DOFs
    /// and bound to a real accelerator. The original runs the optimizer, the filter hand-down and every op (F-62814: F-42575,
    /// F-84707, F-29370); the port's `Seq::compose` must leave the same top frame, every DOF's flags and words. Skipped
    /// without the disc's data.
    #[test]
    fn the_whole_flush_matches_the_original() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let Ok(d) = std::fs::read(std::path::Path::new(&root).join("resources/animation/0xe4f29864/player/male/0x953f13ee.r2")) else {
            return;
        };
        let vs = u32::from_le_bytes(d[12..16].try_into().unwrap()) as usize;
        let data = d[16..].to_vec();
        let res = mcla_formats::rsc::Resource { rtype: 2, flags: 0, virtual_size: vs, physical_size: data.len() - vs, data };
        let rd = mcla_formats::rage::Rd::new(&res);
        let text = include_str!("testdata/drvanim_flush_ref.txt");
        let mut lines = text.lines();
        let rf = usize::from_str_radix(lines.next().unwrap().strip_prefix("# ref ").unwrap(), 16).unwrap();
        let r = mcla_formats::rage::anim::animation_named(&rd, rf).unwrap();
        // the frame: the reference clip's tracks (a rotation for kinds 1 / 6, else a vector); the filters' bones as the
        // harness gave them (bone i of the distinct ids of kinds 0 / 1, k = 0..2: (7 i + 3 k) % (k + 2) == 0)
        let dofs: Vec<(u8, u16)> = r.tracks.iter().map(|t| (t.kind, t.bone_id)).collect();
        let mut ids: Vec<u16> = Vec::new();
        for t in &r.tracks {
            if t.kind <= 1 && !ids.contains(&t.bone_id) {
                ids.push(t.bone_id);
            }
        }
        let wts: Vec<Vec<f32>> = (0..3)
            .map(|k| dofs.iter().map(|&(kind, id)| if kind > 1 { 1.0 } else { let i = ids.iter().position(|&x| x == id).unwrap(); if (i * 7 + k * 3) % (k + 2) == 0 { 1.0 } else { 0.0 } }).collect())
            .collect();
        struct P<'a> {
            dofs: &'a [(u8, u16)],
            layout: u32,
            clips: Vec<mcla_formats::rage::anim::Animation>,
            wts: &'a [Vec<f32>],
        }
        impl Poser for P<'_> {
            fn len(&self) -> usize {
                self.dofs.len()
            }
            fn blank(&self) -> Vec<FrameDof> {
                self.dofs.iter().map(|&(k, _)| FrameDof { flags: if matches!(k, 1 | 6) { DOF_QUATERNION } else { DOF_VECTOR3 } | DOF_NO_VALUE, v: [0.0; 4] }).collect()
            }
            fn sample(&self, clip: ClipId, t: f32, filtered: bool, out: &mut [FrameDof]) {
                let a = &self.clips[clip as usize];
                let (i, fr) = sample_pos(a, t);
                let mapped = filtered || a.layout != self.layout;
                for tr in &a.tracks {
                    if let Some(k) = self.dofs.iter().position(|&x| x == (tr.kind, tr.bone_id)) {
                        sample_track_in(tr, i, fr, &mut out[k], mapped);
                    }
                }
            }
            fn rest(&self, _: &mut [FrameDof]) {
                unreachable!()
            }
            fn weights(&self, f: Filter) -> &[f32] {
                &self.wts[match f {
                    Filter::Spine => 0,
                    Filter::Neck => 1,
                    Filter::NoFace => 2,
                }]
            }
        }
        let mut p = P { dofs: &dofs, layout: r.layout, clips: Vec::new(), wts: &wts };
        let mut clip_of: std::collections::HashMap<usize, ClipId> = Default::default();
        let (mut cases, mut n, mut bad, mut ops) = (0, 0, 0, [0usize; 4]);
        for line in lines {
            let (case, want) = line.split_once(" -> ").unwrap();
            let v: Vec<u32> = case.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let mut s = Seq::default();
            let mut st: Vec<usize> = Vec::new();
            for o in v[1..].chunks(3) {
                let node = match o[0] {
                    1 => {
                        let id = *clip_of.entry(o[1] as usize).or_insert_with(|| {
                            p.clips.push(mcla_formats::rage::anim::animation_named(&rd, o[1] as usize).unwrap());
                            (p.clips.len() - 1) as ClipId
                        });
                        ops[0] += 1;
                        Node::Anim(Player { t: f32::from_bits(o[2]), clip: Some(Clip { id, dur: p.clips[id as usize].duration, flags: 0 }), ..Default::default() })
                    }
                    12 => {
                        ops[1] += 1;
                        Node::Anim(Player::default())
                    }
                    op => {
                        let b = st.pop().unwrap();
                        let a = st.pop().unwrap();
                        let filter = [None, Some(Filter::Spine), Some(Filter::Neck), Some(Filter::NoFace)][o[2] as usize];
                        let w = f32::from_bits(o[1]);
                        if op == 4 {
                            ops[2] += 1;
                            Node::Blend { weight: w, rate: 0.0, destroy: false, filter, kids: [Some(a), Some(b)] }
                        } else {
                            ops[3] += 1;
                            Node::Add { weight: w, rate: 0.0, filter, kids: [Some(a), Some(b)] }
                        }
                    }
                };
                s.nodes.push(node);
                st.push(s.nodes.len() - 1);
            }
            s.root = Some(st[0]);
            let want: Vec<u32> = want.split_whitespace().skip(2).map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            assert_eq!(want.len(), 5 * dofs.len(), "{line}");
            let f = s.compose(&p).unwrap();
            cases += 1;
            for (k, d) in f.iter().enumerate() {
                let w = &want[5 * k..5 * k + 5];
                // (a vector's fourth word is not one the ops define)
                let m = if d.flags & 0xF == DOF_VECTOR3 { 4 } else { 5 };
                let got = [d.flags as u32, d.v[0].to_bits(), d.v[1].to_bits(), d.v[2].to_bits(), d.v[3].to_bits()];
                // (a DOF without a value: only the flag is defined)
                let m = if w[0] & DOF_NO_VALUE as u32 != 0 { 1 } else { m };
                n += 1;
                if got[..m] != w[..m] {
                    if bad < 8 {
                        eprintln!("{case}\n  dof {k} {:?}: got {got:08x?} want {w:08x?}", dofs[k]);
                    }
                    bad += 1;
                }
            }
        }
        assert_eq!(bad, 0, "{bad} of {n} DOFs differ");
        assert!(cases > 1000 && ops.iter().all(|&x| x > 300), "{cases} {ops:?}");
    }

    /// The block step of the sampler (F-10710 / F-30330) against the original's code run offline on clips of several blocks
    /// (work/driverlight/msamp/msample_ref.py: the 456 driver clips of more than one block are in 279 packs; 12 of those packs,
    /// times on and about the block edges, on keys, at the ends and random): 'S' the same-layout loop and 'M' the mapped loop
    /// kept in the testdata (the harness also runs the lookup and filtered ones), each DOF of the clip's first block as the
    /// frame; 864 samples, every DOF's words. Skipped without the disc's data.
    #[test]
    fn the_block_step_matches_the_original() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let text = include_str!("testdata/drvanim_msample_ref.txt");
        let mut packs: std::collections::HashMap<&str, mcla_formats::rsc::Resource> = Default::default();
        let (mut n, mut bad, mut off, mut lines, mut blocks) = (0, 0, 0, 0, std::collections::BTreeSet::new());
        for line in text.lines() {
            let (case, want) = line.split_once(" ->").unwrap();
            let mut it = case.split_whitespace();
            let pack = it.next().unwrap();
            if !packs.contains_key(pack) {
                let Ok(d) = std::fs::read(std::path::Path::new(&root).join("resources/animation").join(pack)) else {
                    return;
                };
                let vs = u32::from_le_bytes(d[12..16].try_into().unwrap()) as usize;
                let data = d[16..].to_vec();
                packs.insert(pack, mcla_formats::rsc::Resource { rtype: 2, flags: 0, virtual_size: vs, physical_size: data.len() - vs, data });
            }
            let rd = mcla_formats::rage::Rd::new(&packs[pack]);
            let o = usize::from_str_radix(it.next().unwrap(), 16).unwrap();
            let t = f32::from_bits(u32::from_str_radix(it.next().unwrap(), 16).unwrap());
            let mode = it.next().unwrap();
            let mask: Vec<u16> = it.next().map_or(vec![], |m| m.split(',').filter(|x| !x.is_empty()).map(|x| u16::from_str_radix(x, 16).unwrap()).collect());
            let want: Vec<u32> = want.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let a = mcla_formats::rage::anim::animation_named(&rd, o).unwrap();
            assert!(a.blocks > 1, "{line}");
            let (b, i, fr) = sample_block(&a, t);
            blocks.insert((pack, o, b));
            let frame = block_tracks(&a, 0);
            let tracks = block_tracks(&a, b);
            assert_eq!(want.len(), 5 * frame.len(), "{line}");
            lines += 1;
            for (k, f) in frame.iter().enumerate() {
                let quat = matches!(f.kind, 1 | 6);
                let w = &want[5 * k..5 * k + 5];
                if mode == "F" && f.kind <= 1 && !mask.contains(&f.bone_id) {
                    off += 1;
                    continue;
                }
                let mut dof = FrameDof { flags: quat as u8 | DOF_NO_VALUE, v: [0.0; 4] };
                if let Some(tr) = tracks.iter().find(|x| (x.kind, x.bone_id) == (f.kind, f.bone_id)) {
                    sample_track_in(tr, i, fr, &mut dof, mode == "M" || mode == "F");
                }
                let got = [dof.flags as u32, dof.v[0].to_bits(), dof.v[1].to_bits(), dof.v[2].to_bits(), dof.v[3].to_bits()];
                let m = if dof.flags & DOF_NO_VALUE != 0 { 1 } else if quat || f.flags & 0xF0 != 0x10 { 5 } else { 4 };
                n += 1;
                if got[..m] != w[..m] {
                    if bad < 8 {
                        eprintln!("{mode} {case} block {b} dof {k} kind {}: got {got:08x?} want {w:08x?}", f.kind);
                    }
                    bad += 1;
                }
            }
        }
        assert_eq!(bad, 0, "{bad} of {n} DOFs differ");
        assert!(lines > 800 && blocks.iter().filter(|x| x.2 > 0).count() > 20, "{lines} {}", blocks.len());
        let _ = off;
    }

    /// The sampler's other loops against the original's code run offline (work/driverlight/fsamp/fsample_ref.py, the
    /// Challenger 70 player pack, 360 samples of 180 clips, every frame bound to a real accelerator as a scheduled tree's
    /// are): 'S' the frame's layout = the clip's +16 (F-12216's same-layout loop), 'M' another layout (its mapped loop), 'F'
    /// with a bone filter on a random half of the clip's bones (F-40465 -> F-61066 -> F-80057). S is `sample_track_in`
    /// as before, M and F with `mapped`; F's DOFs off the filter (bone kinds 0 / 1) hold no value in the original, which the
    /// port does not model (no blend above reads them). Skipped without the disc's data.
    #[test]
    fn the_sampler_loops_match_the_original() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let Ok(d) = std::fs::read(std::path::Path::new(&root).join("resources/animation/0xe4f29864/player/male/0x953f13ee.r2")) else {
            return;
        };
        let vs = u32::from_le_bytes(d[12..16].try_into().unwrap()) as usize;
        let data = d[16..].to_vec();
        let res = mcla_formats::rsc::Resource { rtype: 2, flags: 0, virtual_size: vs, physical_size: data.len() - vs, data };
        let rd = mcla_formats::rage::Rd::new(&res);
        let text = include_str!("testdata/drvanim_fsample_ref.txt");
        let (mut n, mut bad, mut off, mut modes) = (0, 0, 0, [0usize; 3]);
        for line in text.lines() {
            let (case, want) = line.split_once(" ->").unwrap();
            let mut it = case.split_whitespace();
            let o = usize::from_str_radix(it.next().unwrap(), 16).unwrap();
            let t = f32::from_bits(u32::from_str_radix(it.next().unwrap(), 16).unwrap());
            let mode = it.next().unwrap();
            let mask: Vec<u16> = it.next().map_or(vec![], |m| m.split(',').filter(|x| !x.is_empty()).map(|x| u16::from_str_radix(x, 16).unwrap()).collect());
            let want: Vec<u32> = want.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let a = mcla_formats::rage::anim::animation_named(&rd, o).unwrap();
            let (i, fr) = sample_pos(&a, t);
            assert_eq!(want.len(), 5 * a.tracks.len(), "{line}");
            modes[match mode { "S" => 0, "M" => 1, _ => 2 }] += 1;
            for (k, tr) in a.tracks.iter().enumerate() {
                let quat = matches!(tr.kind, 1 | 6);
                let w = &want[5 * k..5 * k + 5];
                if mode == "F" && tr.kind <= 1 && !mask.contains(&tr.bone_id) {
                    assert!(w[0] & DOF_NO_VALUE as u32 != 0, "{case} track {k}: a DOF off the filter holds a value");
                    off += 1;
                    continue;
                }
                let mut dof = FrameDof { flags: quat as u8 | DOF_NO_VALUE, v: [0.0; 4] };
                sample_track_in(tr, i, fr, &mut dof, mode != "S");
                let got = [dof.flags as u32, dof.v[0].to_bits(), dof.v[1].to_bits(), dof.v[2].to_bits(), dof.v[3].to_bits()];
                let m = if quat || tr.flags & 0xF0 != 0x10 { 5 } else { 4 };
                n += 1;
                if got[..m] != w[..m] {
                    if bad < 8 {
                        eprintln!("{mode} {case} track {k} kind {}: got {got:08x?} want {w:08x?}", tr.kind);
                    }
                    bad += 1;
                }
            }
        }
        assert_eq!(bad, 0, "{bad} of {n} DOFs differ");
        assert!(n > 60_000 && off > 10_000 && modes == [360, 360, 360], "{n} {off} {modes:?}");
    }

    /// F-59824 over a clip's tracks (type 1 for kinds 1 / 6, else 0; in track order) is the clip's stored +16, for every
    /// clip of the Challenger 70 player pack (182). Skipped without the disc's data.
    #[test]
    fn the_layout_signature_is_the_clips() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let Ok(d) = std::fs::read(std::path::Path::new(&root).join("resources/animation/0xe4f29864/player/male/0x953f13ee.r2")) else {
            return;
        };
        let vs = u32::from_le_bytes(d[12..16].try_into().unwrap()) as usize;
        let data = d[16..].to_vec();
        let res = mcla_formats::rsc::Resource { rtype: 2, flags: 0, virtual_size: vs, physical_size: data.len() - vs, data };
        let rd = mcla_formats::rage::Rd::new(&res);
        let mut n = 0;
        let mut o = 4;
        while let Some(a) = rd.ptr(o) {
            o += 4;
            let Some(an) = mcla_formats::rage::anim::animation_named(&rd, a) else { continue };
            if an.blocks != 1 {
                continue;
            }
            let dofs: Vec<(u8, u8, u16)> = an.tracks.iter().map(|t| (matches!(t.kind, 1 | 6) as u8, t.kind, t.bone_id)).collect();
            assert_eq!(mcla_formats::rage::skeleton::layout_signature(&dofs), an.layout, "{}", an.name);
            n += 1;
        }
        assert!(n >= 180, "{n}");
    }

    /// The clip sampler (F-47152 -> F-12216) against the original's code run offline on the Challenger 70 player pack
    /// (work/drvmachine/sample_ref.py): 720 samples of 180 clips, every DOF (vectors' x y z, quaternions' four words).
    /// Skipped without the disc's data.
    #[test]
    fn the_clip_sampler_matches_the_original() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let Ok(d) = std::fs::read(std::path::Path::new(&root).join("resources/animation/0xe4f29864/player/male/0x953f13ee.r2")) else {
            return;
        };
        let vs = u32::from_le_bytes(d[12..16].try_into().unwrap()) as usize;
        let data = d[16..].to_vec();
        let res = mcla_formats::rsc::Resource { rtype: 2, flags: 0, virtual_size: vs, physical_size: data.len() - vs, data };
        let rd = mcla_formats::rage::Rd::new(&res);
        let text = include_str!("testdata/drvanim_sample_ref.txt");
        let (mut n, mut bad) = (0, 0);
        for line in text.lines() {
            let (case, want) = line.split_once(" ->").unwrap();
            let mut it = case.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap());
            let (o, t) = (it.next().unwrap() as usize, f32::from_bits(it.next().unwrap()));
            let want: Vec<u32> = want.split_whitespace().map(|x| u32::from_str_radix(x, 16).unwrap()).collect();
            let a = mcla_formats::rage::anim::animation_named(&rd, o).unwrap();
            let (i, fr) = sample_pos(&a, t);
            assert_eq!(want.len(), 5 * a.tracks.len(), "{line}");
            for (k, tr) in a.tracks.iter().enumerate() {
                let quat = matches!(tr.kind, 1 | 6);
                let mut dof = FrameDof { flags: quat as u8 | DOF_NO_VALUE, v: [0.0; 4] };
                sample_track(tr, i, fr, &mut dof);
                let w = &want[5 * k..5 * k + 5];
                let got = [dof.flags as u32, dof.v[0].to_bits(), dof.v[1].to_bits(), dof.v[2].to_bits(), dof.v[3].to_bits()];
                // (a vector's fourth word: the static channel's stored word, else what the frame held)
                let m = if quat || tr.flags & 0xF0 != 0x10 { 5 } else { 4 };
                n += 1;
                if got[..m] != w[..m] {
                    if bad < 8 {
                        eprintln!("{case} track {k} kind {} flags {:x}: got {got:08x?} want {w:08x?}", tr.kind, tr.flags);
                    }
                    bad += 1;
                }
            }
        }
        assert_eq!(bad, 0, "{bad} of {n} DOFs differ");
    }
}
