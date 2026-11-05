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
    /// crmtRequestAnimation (F-29648): the clip from time `t` at `rate`, +32 set by every builder, +33 loop
    Anim { clip: ClipId, t: f32, rate: f32, looping: bool },
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
        Req::new(Kind::Anim { clip, t: 0.0, rate: 1.0, looping })
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
        let pan = |c: ClipId| Req::new(Kind::Anim { clip: c, t: dur(c) * 0.5, rate: 0.0, looping: false }).held(76);
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
    let b = Req::new(Kind::Anim { clip: body, t, rate: 1.0, looping: false }).held(44);
    let p = Req::blend(1.0, 0.0, Some(Filter::NoFace), Some(Req::anim(clip, looping).held(48)), b);
    Some(Req::blend(0.0, 1.0 / blend, None, None, p).held(84))
}

impl Seq {
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
        for h in self.blend.iter_mut().chain(self.anim.iter_mut()).chain([&mut self.pan, &mut self.top]) {
            if *h == Some(n) {
                *h = None;
            }
        }
    }

    /// The request's generate (F-14509 -> vt+8: animation F-26354 with the node init F-45890, blend F-82335 / add F-71347 through
    /// F-69377, identity F-20211): the node it makes takes the place of `existing`
    /// (the tree's root), which is destroyed, except under a crossfade (a blend without a first source), which keeps it as its
    /// first child.
    fn gen(&mut self, r: &Req, existing: Option<usize>, dur: &dyn Fn(ClipId) -> f32) -> usize {
        let mut keep = false;
        let n = match &r.kind {
            Kind::Anim { clip, t, rate, looping } => {
                let d = dur(*clip);
                let mut p = Player { flags: 1 | (*looping as u32) << 1, rate: *rate, clip: Some(Clip { id: *clip, dur: d, flags: 0 }), ..Player::default() };
                p.set_time(*t, d);
                self.alloc(Node::Anim(p))
            }
            Kind::Identity => self.alloc(Node::Identity),
            Kind::Add { w, filter, a, b } => {
                let (a, b) = (self.gen(a, None, dur), self.gen(b, None, dur));
                self.alloc(Node::Add { weight: *w, rate: 0.0, filter: *filter, kids: [Some(a), Some(b)] })
            }
            Kind::Blend { w, rate, filter, a, b } => {
                let a = match a {
                    Some(a) => Some(self.gen(a, None, dur)),
                    None => {
                        keep = true;
                        existing
                    }
                };
                let b = self.gen(b, None, dur);
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
        let n = self.gen(r, self.root, dur);
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

#[cfg(test)]
mod tests {
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
            Kind::Anim { clip, t, rate, looping } => *o += &format!(" #{id} anim(c{clip} {} {} 1 {} 0)", b(*t), b(*rate), *looping as u8),
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
}
