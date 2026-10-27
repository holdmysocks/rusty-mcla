//! The movie runtime: instances (`swfINSTANCE`), script objects, timelines and
//! the AVM1 interpreter, following the original player in the executable.
//!
//! Timeline (verified in the recomp):
//! * per tick, pre-order over the instance tree (`F-94080`): an instance whose
//!   alpha byte is 0 is skipped with its subtree; otherwise its `onEnterFrame`
//!   runs, then a sprite processes its current frame (`F-27252`): playing ->
//!   fire frame events (mask 2) and execute every command of the current frame
//!   including actions; stopped -> re-apply PlaceObject commands and fire frame
//!   events of named children only. Then pending stop/play requests apply and a
//!   playing sprite steps to the next frame executing only display commands
//!   (`F-48063`, `F-41374`, `F-85946`). Children follow.
//! * `goto` only steps forward, wrapping through the last frame; at the wrap the
//!   children's "placed" marks are cleared so frame 0 places them again. Nothing
//!   is removed except by RemoveObject commands.
//! * `_x`/`_y` are offsets added to the timeline translation; `_xscale`/`_yscale`
//!   replace the timeline scale when either differs from 100; `_rotation`
//!   replaces the timeline rotation when non-zero (`F-74625`).
//!
//! Scripts: values are single precision, a function object is its own
//! activation (parameters in registers, other names forward to the scope it was
//! called on), unknown names fall back to the native registry and `_global`,
//! and a missing variable is created as integer 0.

use crate::value::{hash, InstId, NativeFn, ObjId, Ref, Value};
use mcla_formats::avm1::{self, Action, PushValue};
use mcla_formats::rage::swf::{Character, Cmd, Code, CxForm, Matrix, Movie, Place};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

type Members = Vec<(u32, Rc<str>, Value)>;

fn mget(m: &Members, h: u32) -> Option<&Value> {
    m.iter().find(|e| e.0 == h).map(|e| &e.2)
}

fn mset(m: &mut Members, h: u32, name: &Rc<str>, v: Value) {
    if let Some(e) = m.iter_mut().find(|e| e.0 == h) {
        e.2 = v;
    } else {
        m.push((h, name.clone(), v));
    }
}

#[derive(Debug, Clone)]
pub struct Func {
    pub code: Code,
    pub start: usize,
    pub len: usize,
    pub pool: Rc<Vec<Rc<str>>>,
    /// DefineFunction2: register of each parameter
    pub param_regs: Vec<u8>,
    /// DefineFunction (v1): parameter names
    pub param_names: Vec<Rc<str>>,
    pub v2: bool,
    // call state kept in the object, as the original does
    pub args: Vec<Value>,
    pub scope: Option<Ref>,
}

#[derive(Debug, Clone)]
pub enum ObjKind {
    Script,
    Array(Vec<Value>),
    Func(Box<Func>),
}

#[derive(Debug, Clone)]
pub struct Obj {
    pub kind: ObjKind,
    pub members: Members,
    pub prototype: Option<ObjId>,
    mark: bool,
}

#[derive(Debug, Clone)]
pub struct Inst {
    pub character: u16,
    pub depth: u16,
    pub parent: Option<InstId>,
    pub children: BTreeMap<u16, InstId>,
    // transform state (instance +84..+136)
    pub placed_rot: f32,
    pub rotation: f32,
    pub placed_tx: f32,
    pub placed_ty: f32,
    pub placed_sx: f32,
    pub placed_sy: f32,
    pub xscale: f32,
    pub yscale: f32,
    pub x: f32,
    pub y: f32,
    pub skew: [f32; 4],
    /// composed local matrix
    pub matrix: Matrix,
    pub cxform: CxForm,
    /// +76..+79: RGB multiplier for children (`setRGB`, `textColor`) and alpha (`_alpha`)
    pub tint: [u8; 4],
    /// a texture the host supplies in place of this clip's bitmaps (the game's
    /// `__texturename` clips: car pictures); 0 = none
    pub tex_swap: u32,
    pub clip_depth: u16,
    pub ratio: u16,
    pub name: Rc<str>,
    pub text: Option<Rc<str>>,
    pub frame: u16,
    pub playing: bool,
    play_req: bool,
    stop_req: bool,
    placed_by: Option<u64>,
    pub members: Option<ObjId>,
    pub on_enter_frame: Option<Ref>,
    pub on_release: Option<Ref>,
    pub events: Vec<(u32, Code)>,
}

impl Inst {
    fn new(character: u16, depth: u16, parent: Option<InstId>) -> Inst {
        Inst {
            character,
            depth,
            parent,
            children: BTreeMap::new(),
            placed_rot: 0.0,
            rotation: 0.0,
            placed_tx: 0.0,
            placed_ty: 0.0,
            placed_sx: 100.0,
            placed_sy: 100.0,
            xscale: 100.0,
            yscale: 100.0,
            x: 0.0,
            y: 0.0,
            skew: [1.0, 0.0, 0.0, 1.0],
            matrix: Matrix::IDENTITY,
            cxform: CxForm::IDENTITY,
            tint: [255; 4],
            tex_swap: 0,
            clip_depth: 0,
            ratio: 0,
            name: Rc::from(""),
            text: None,
            frame: 0,
            playing: true,
            play_req: false,
            stop_req: false,
            placed_by: None,
            members: None,
            on_enter_frame: None,
            on_release: None,
            events: Vec::new(),
        }
    }

    /// `F-74625`
    pub fn compose(&mut self) {
        let rot = if self.rotation != 0.0 { self.rotation } else { self.placed_rot };
        let (s, c) = rotation(rot);
        let (sx, sy) = if self.xscale == 100.0 && self.yscale == 100.0 { (self.placed_sx, self.placed_sy) } else { (self.xscale, self.yscale) };
        let (sx, sy) = (sx * 0.01, sy * 0.01);
        // (the placed 2x2 times the rotation, fused as the original, then the scale; the terms of the
        // rotation's zero translation stay in: x * 0 added)
        let [k0, k1, k2, k3] = self.skew;
        let p0 = k1.mul_add(-s, k0 * c);
        let p1 = k0.mul_add(s, k1 * c);
        let p2 = k3.mul_add(-s, k2 * c);
        let p3 = k2.mul_add(s, k3 * c);
        self.matrix = Matrix {
            a: p0.mul_add(sx, p2 * 0.0),
            b: p1.mul_add(sx, p3 * 0.0),
            c: p2.mul_add(sy, p0 * 0.0),
            d: p3.mul_add(sy, p1 * 0.0),
            tx: self.placed_tx + self.x,
            ty: self.placed_ty + self.y,
        };
    }

    /// Split a PlaceObject matrix into the timeline rotation, scale and the
    /// remaining 2x2 factor (polar decomposition, as `F-54074` does), then
    /// compose again.
    pub fn set_placed(&mut self, m: &Matrix) {
        let det = m.a * m.d - m.b * m.c;
        if det == 1.0 && m.a + m.d == 2.0 {
            self.placed_rot = 0.0;
            self.placed_sx = 100.0;
            self.placed_sy = 100.0;
            self.skew = [1.0, 0.0, 0.0, 1.0];
        } else {
            // rotation of the polar decomposition: angle of (a + d, b - c)
            let flip = det < 0.0;
            let (a, b, c, d) = if flip { (m.a, m.b, -m.c, -m.d) } else { (m.a, m.b, m.c, m.d) };
            let theta = (b - c).atan2(a + d);
            let (s, co) = theta.sin_cos();
            // P = R^T * M  (columns x: (a,b), y: (c,d))
            let p0 = co * a + s * b;
            let p1 = -s * a + co * b;
            let p2 = co * c + s * d;
            let p3 = -s * c + co * d;
            let sx = (p0 * p0 + p1 * p1).sqrt();
            let mut sy = (p2 * p2 + p3 * p3).sqrt();
            let k = [if sx > 0.0 { p0 / sx } else { 1.0 }, if sx > 0.0 { p1 / sx } else { 0.0 }, if sy > 0.0 { p2 / sy } else { 0.0 }, if sy > 0.0 { p3 / sy } else { 1.0 }];
            if flip {
                sy = -sy;
            }
            self.placed_rot = theta * 180.0 / std::f32::consts::PI;
            self.placed_sx = sx * 100.0;
            self.placed_sy = sy * 100.0;
            self.skew = k;
        }
        self.placed_tx = m.tx;
        self.placed_ty = m.ty;
        self.compose();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameMode {
    /// command slot 1: everything
    Full,
    /// slot 2: display list only
    Display,
    /// slot 3: stopped sprite
    Idle,
}

pub type FsCommand = Box<dyn FnMut(&mut Player, &str, &str)>;

pub struct Player {
    pub movie: Arc<Movie>,
    pub insts: Vec<Option<Inst>>,
    pub objs: Vec<Option<Obj>>,
    free: Vec<ObjId>,
    pub root: InstId,
    pub global: ObjId,
    registry: Members,
    stack: Vec<Value>,
    ret: Vec<Value>,
    pub fscommand: Option<FsCommand>,
    accum: f32,
    rng: u32,
    depth: u32,
    alloc_since_gc: usize,
    pub warnings: Vec<String>,
    /// false while building previews: no ActionScript runs
    pub scripts: bool,
    /// The host's key names drawn in place of clips, by instance name, with their ARGB
    /// colour (a pad button's picture shown as a keyboard key: `draw::Builder::key_cap`);
    /// empty = as authored.
    pub key_caps: Vec<(String, String, u32)>,
}

const MAX_WARNINGS: usize = 200;

impl Player {
    pub fn new(movie: Arc<Movie>) -> Player {
        let mut p = Player {
            movie,
            insts: Vec::new(),
            objs: Vec::new(),
            free: Vec::new(),
            root: 0,
            global: 0,
            registry: Vec::new(),
            stack: Vec::new(),
            ret: Vec::new(),
            fscommand: None,
            accum: 0.0,
            rng: 0x1234_5678,
            depth: 0,
            alloc_since_gc: 0,
            warnings: Vec::new(),
            scripts: true,
            key_caps: Vec::new(),
        };
        p.global = p.alloc(ObjKind::Script);
        p.root = p.alloc_inst(Inst::new(0, 0, None));
        p.install_registry();
        p
    }

    fn warn(&mut self, s: String) {
        log::debug!("swf: {}", s);
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(s);
        }
    }

    // ---------------------------------------------------------------- heap

    fn alloc(&mut self, kind: ObjKind) -> ObjId {
        self.alloc_since_gc += 1;
        let o = Obj { kind, members: Vec::new(), prototype: None, mark: false };
        if let Some(id) = self.free.pop() {
            self.objs[id as usize] = Some(o);
            id
        } else {
            self.objs.push(Some(o));
            (self.objs.len() - 1) as ObjId
        }
    }

    fn alloc_inst(&mut self, i: Inst) -> InstId {
        if let Some(pos) = self.insts.iter().position(|x| x.is_none()) {
            self.insts[pos] = Some(i);
            pos as InstId
        } else {
            self.insts.push(Some(i));
            (self.insts.len() - 1) as InstId
        }
    }

    pub fn inst(&self, id: InstId) -> Option<&Inst> {
        self.insts.get(id as usize).and_then(|i| i.as_ref())
    }

    pub fn inst_mut(&mut self, id: InstId) -> Option<&mut Inst> {
        self.insts.get_mut(id as usize).and_then(|i| i.as_mut())
    }

    fn obj(&self, id: ObjId) -> Option<&Obj> {
        self.objs.get(id as usize).and_then(|o| o.as_ref())
    }

    fn obj_mut(&mut self, id: ObjId) -> Option<&mut Obj> {
        self.objs.get_mut(id as usize).and_then(|o| o.as_mut())
    }

    fn free_inst_tree(&mut self, id: InstId) {
        if let Some(i) = self.insts.get_mut(id as usize).and_then(|i| i.take()) {
            for c in i.children.values() {
                self.free_inst_tree(*c);
            }
        }
    }

    /// Mark-and-sweep over script objects reachable from the global object,
    /// the registry, instances, the stacks.
    pub fn gc(&mut self) {
        let mut work: Vec<ObjId> = vec![self.global];
        let push_val = |v: &Value, w: &mut Vec<ObjId>| {
            if let Value::Obj(Ref::Obj(o)) = v {
                w.push(*o);
            }
        };
        for e in &self.registry {
            push_val(&e.2, &mut work);
        }
        for v in self.stack.iter().chain(self.ret.iter()) {
            push_val(v, &mut work);
        }
        for i in self.insts.iter().flatten() {
            if let Some(m) = i.members {
                work.push(m);
            }
            for r in [i.on_enter_frame, i.on_release].into_iter().flatten() {
                if let Ref::Obj(o) = r {
                    work.push(o);
                }
            }
        }
        while let Some(o) = work.pop() {
            let Some(obj) = self.objs.get_mut(o as usize).and_then(|x| x.as_mut()) else { continue };
            if obj.mark {
                continue;
            }
            obj.mark = true;
            for e in &obj.members {
                push_val(&e.2, &mut work);
            }
            if let Some(p) = obj.prototype {
                work.push(p);
            }
            match &obj.kind {
                ObjKind::Array(v) => v.iter().for_each(|x| push_val(x, &mut work)),
                ObjKind::Func(f) => {
                    f.args.iter().for_each(|x| push_val(x, &mut work));
                    if let Some(Ref::Obj(s)) = f.scope {
                        work.push(s);
                    }
                }
                ObjKind::Script => {}
            }
        }
        for (i, slot) in self.objs.iter_mut().enumerate() {
            match slot {
                Some(o) if o.mark => o.mark = false,
                Some(_) => {
                    *slot = None;
                    self.free.push(i as ObjId);
                }
                None => {}
            }
        }
        self.alloc_since_gc = 0;
    }

    // ------------------------------------------------------------ registry

    fn reg_native(m: &mut Members, name: &str, f: NativeFn) {
        mset(m, hash(name), &Rc::from(name), Value::Native(f));
    }

    fn install_registry(&mut self) {
        let math = self.alloc(ObjKind::Script);
        {
            let m = &mut self.obj_mut(math).unwrap().members;
            fn un(a: &[Value], k: usize) -> Value {
                Value::Float((MATH[k].1)(a.first().map_or(0.0, |v| v.to_f32())))
            }
            Self::reg_native(m, MATH[0].0, |_, _, a| un(a, 0));
            Self::reg_native(m, MATH[1].0, |_, _, a| un(a, 1));
            Self::reg_native(m, MATH[2].0, |_, _, a| un(a, 2));
            Self::reg_native(m, MATH[3].0, |_, _, a| un(a, 3));
            Self::reg_native(m, MATH[4].0, |_, _, a| un(a, 4));
            Self::reg_native(m, MATH[5].0, |_, _, a| un(a, 5));
            Self::reg_native(m, MATH[6].0, |_, _, a| un(a, 6));
            Self::reg_native(m, MATH[7].0, |_, _, a| un(a, 7));
            Self::reg_native(m, "random", |p, _, _| Value::Float(p.random() as f32 / 4_294_967_296.0));
        }
        let key = self.alloc(ObjKind::Script);
        {
            let m = &mut self.obj_mut(key).unwrap().members;
            for n in ["addListener", "removeListener"] {
                Self::reg_native(m, n, |_, _, _| Value::Undef);
            }
            for n in ["getAscii", "getCode"] {
                Self::reg_native(m, n, |_, _, _| Value::Int(0));
            }
            for n in ["isDown", "isToggled"] {
                Self::reg_native(m, n, |_, _, _| Value::Bool(false));
            }
        }
        let stage = self.alloc(ObjKind::Script);
        let movieclip = self.alloc(ObjKind::Script);
        let shared = self.alloc(ObjKind::Script);
        let g = self.global;
        let mut reg: Members = Vec::new();
        for (n, o) in [("Math", math), ("Key", key), ("Stage", stage), ("MovieClip", movieclip), ("_global", g), ("SharedObject", shared)] {
            mset(&mut reg, hash(n), &Rc::from(n), Value::Obj(Ref::Obj(o)));
        }
        Self::reg_native(&mut reg, "Array", native_array);
        Self::reg_native(&mut reg, "Sound", |_, _, _| Value::Undef);
        Self::reg_native(&mut reg, "Color", native_color);
        self.registry = reg;
    }

    /// Register a game native (the registry the original fills from C++).
    pub fn register_native(&mut self, name: &str, f: NativeFn) {
        Self::reg_native(&mut self.registry, name, f);
    }

    fn random(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    // ----------------------------------------------------------- host API

    /// `_global.<name> = v` (the game's `F-59146` / `F-66095` / `F-16600`).
    pub fn set_global(&mut self, name: &str, v: Value) {
        let g = self.global;
        let n: Rc<str> = Rc::from(name);
        if let Some(o) = self.obj_mut(g) {
            mset(&mut o.members, hash(name), &n, v);
        }
    }

    pub fn get_global(&self, name: &str) -> Option<Value> {
        self.obj(self.global).and_then(|o| mget(&o.members, hash(name)).cloned())
    }

    /// A new empty script object (for building the structures the game hands to movies).
    pub fn new_object(&mut self) -> Value {
        Value::Obj(Ref::Obj(self.alloc(ObjKind::Script)))
    }

    /// A new array object with the given elements.
    pub fn new_array(&mut self, v: Vec<Value>) -> Value {
        Value::Obj(Ref::Obj(self.alloc(ObjKind::Array(v))))
    }

    /// Read `_global.a.b.c` (numeric segments index arrays).
    pub fn get_path(&mut self, path: &str) -> Option<Value> {
        let mut cur = Value::Obj(Ref::Obj(self.global));
        for p in path.split('.') {
            let r = cur.obj()?;
            let key = p.parse::<i32>().map(Value::Int).unwrap_or_else(|_| Value::str(p));
            cur = self.get_member(r, &key)?;
        }
        Some(cur)
    }

    /// `_global.a.b.c = v` for an existing object path.
    pub fn set_path(&mut self, path: &str, v: Value) {
        let mut parts: Vec<&str> = path.split('.').collect();
        let last = parts.pop().unwrap_or("");
        let mut cur = Ref::Obj(self.global);
        for p in parts {
            let key = p.parse::<i32>().map(Value::Int).unwrap_or_else(|_| Value::str(p));
            match self.get_member(cur, &key) {
                Some(Value::Obj(r)) => cur = r,
                _ => return,
            }
        }
        // a numeric last segment addresses an array element
        let key = last.parse::<i32>().map(Value::Int).unwrap_or_else(|_| Value::str(last));
        self.set_member(cur, &key, v);
    }

    /// Call `_global.<name>(args)` or a method of any object reachable by a dotted path.
    pub fn call_path(&mut self, path: &str, args: &[Value]) -> Value {
        let mut parts: Vec<&str> = path.split('.').collect();
        let method = parts.pop().unwrap_or("");
        let mut cur = Ref::Obj(self.global);
        for p in parts {
            match self.get_member(cur, &Value::str(p)) {
                Some(Value::Obj(r)) => cur = r,
                _ => return Value::Undef,
            }
        }
        self.call_method(cur, method, args)
    }

    /// Instance by dotted path of instance names from the root.
    pub fn find(&self, path: &str) -> Option<InstId> {
        let mut cur = self.root;
        for p in path.split('.').filter(|s| !s.is_empty()) {
            let i = self.inst(cur)?;
            cur = *i.children.values().find(|c| self.inst(**c).map_or(false, |x| x.name.eq_ignore_ascii_case(p)))?;
        }
        Some(cur)
    }

    /// Instantiate a character under the root without running any script:
    /// sprites show their first frame, recursively (for previews).
    pub fn preview(&mut self, ch: u16) -> InstId {
        let root = self.root;
        let mut i = Inst::new(ch, 0xffff, Some(root));
        if let Some(Character::EditText(e)) = self.movie.char(ch) {
            i.text = Some(Rc::from(e.initial.as_str()));
        }
        let id = self.alloc_inst(i);
        self.inst_mut(root).unwrap().children.insert(0xffff, id);
        fn fill(p: &mut Player, id: InstId, depth: u32) {
            if depth > 32 || !p.is_sprite(id) {
                return;
            }
            p.exec_frame(id, FrameMode::Display);
            let kids: Vec<InstId> = p.inst(id).map(|i| i.children.values().copied().collect()).unwrap_or_default();
            for k in kids {
                fill(p, k, depth + 1);
            }
        }
        let s = self.scripts;
        self.scripts = false;
        fill(self, id, 0);
        self.scripts = s;
        id
    }

    // ------------------------------------------------------------ timeline

    fn frame_count(&self, id: InstId) -> usize {
        let Some(i) = self.inst(id) else { return 0 };
        self.movie.sprite(i.character).map_or(0, |s| s.frames.len())
    }

    fn is_sprite(&self, id: InstId) -> bool {
        // character 0 is the movie's own timeline (dictionary entry 0 is unused)
        self.inst(id).map_or(false, |i| i.character == 0 || matches!(self.movie.char(i.character), Some(Character::Sprite(_))))
    }

    /// Advance the movie by `dt` seconds at its frame rate (`swfCONTEXT` update).
    pub fn update(&mut self, dt: f32) {
        let step = 1.0 / self.movie.fps.max(1) as f32;
        self.accum += dt;
        let mut n = 0;
        while self.accum >= step && n < 8 {
            self.accum -= step;
            self.tick();
            n += 1;
        }
        if n == 8 {
            self.accum = 0.0;
        }
    }

    /// One frame of the whole movie.
    pub fn tick(&mut self) {
        let r = self.root;
        self.tick_inst(r);
        if self.alloc_since_gc > 4096 {
            self.gc();
        }
    }

    fn tick_inst(&mut self, id: InstId) {
        let Some(i) = self.inst(id) else { return };
        if i.tint[3] == 0 {
            return;
        }
        if let Some(f) = i.on_enter_frame {
            self.call_function(f, &[], Ref::Inst(id));
        }
        if self.is_sprite(id) {
            self.sprite_frame(id);
        }
        let kids: Vec<InstId> = self.inst(id).map(|i| i.children.values().copied().collect()).unwrap_or_default();
        for k in kids {
            // a child removed by an earlier sibling's script is skipped
            if self.inst(k).map_or(false, |c| c.parent == Some(id)) {
                self.tick_inst(k);
            }
        }
    }

    /// `F-27252` + `F-48063`
    fn sprite_frame(&mut self, id: InstId) {
        let playing = self.inst(id).map_or(false, |i| i.playing);
        if playing {
            self.fire_events(id, 2);
            self.exec_frame(id, FrameMode::Full);
        } else {
            self.exec_frame(id, FrameMode::Idle);
        }
        let Some(i) = self.inst_mut(id) else { return };
        if i.stop_req {
            i.playing = false;
            i.stop_req = false;
        }
        if i.play_req {
            i.play_req = false;
            i.playing = true;
        }
        if i.playing {
            let n = self.frame_count(id);
            if n > 0 {
                let next = (self.inst(id).unwrap().frame as usize + 1) % n;
                self.goto(id, next);
            }
        }
    }

    /// `F-85946` with display-only frames.
    pub fn goto(&mut self, id: InstId, target: usize) {
        let n = self.frame_count(id);
        let Some(cur) = self.inst(id).map(|i| i.frame as usize) else { return };
        if target >= n || target == cur {
            return;
        }
        let mut guard = 0;
        loop {
            let i = self.inst_mut(id).unwrap();
            i.frame += 1;
            if i.frame as usize >= n {
                let kids: Vec<InstId> = i.children.values().copied().collect();
                i.frame = 0;
                for k in kids {
                    if let Some(c) = self.inst_mut(k) {
                        c.placed_by = None;
                    }
                }
            }
            self.exec_frame(id, FrameMode::Display);
            let f = self.inst(id).map_or(target, |i| i.frame as usize);
            guard += 1;
            if f == target || guard > n + 1 {
                break;
            }
        }
    }

    fn exec_frame(&mut self, id: InstId, mode: FrameMode) {
        let movie = self.movie.clone();
        let Some(i) = self.inst(id) else { return };
        let (ch, frame) = (i.character, i.frame as usize);
        let Some(sprite) = movie.sprite(ch) else { return };
        let Some(f) = sprite.frames.get(frame) else { return };
        for (ci, cmd) in f.cmds.iter().enumerate() {
            if self.inst(id).is_none() {
                return;
            }
            let key = (ch as u64) << 40 | (frame as u64) << 20 | ci as u64;
            match (cmd, mode) {
                (Cmd::Place(p), FrameMode::Full | FrameMode::Display) => {
                    let child = self.place(id, p, key);
                    if let (Some(c), true) = (child, p.name.is_some() || !p.events.is_empty()) {
                        // swfCMD_PlaceObject2ClipEvent (F-33189)
                        if p.character.is_some() {
                            if let Some(n) = &p.name {
                                let name: Rc<str> = Rc::from(n.as_str());
                                self.set_member(Ref::Inst(id), &Value::Str(name.clone()), Value::Obj(Ref::Inst(c)));
                                if let Some(ci) = self.inst_mut(c) {
                                    ci.name = name;
                                }
                            }
                        }
                        if !p.events.is_empty() {
                            if let Some(ci) = self.inst_mut(c) {
                                ci.events = p.events.iter().map(|e| (e.flags, e.actions.clone())).collect();
                            }
                        }
                        self.fire_events(c, 1);
                    }
                }
                (Cmd::Place(p), FrameMode::Idle) => {
                    if p.name.is_some() || !p.events.is_empty() {
                        // slot 3 of the named PlaceObject: events only
                        if !p.events.is_empty() {
                            if let Some(c) = self.child_at(id, p.depth) {
                                if let Some(ci) = self.inst_mut(c) {
                                    ci.events = p.events.iter().map(|e| (e.flags, e.actions.clone())).collect();
                                }
                                self.fire_events(c, 2);
                            }
                        }
                    } else {
                        self.place(id, p, key);
                    }
                }
                (Cmd::Remove(d), FrameMode::Full | FrameMode::Display) => self.remove_child(id, *d),
                (Cmd::DoAction(code) | Cmd::DoInitAction(code), FrameMode::Full) => {
                    self.run_block(code, Ref::Inst(id));
                }
                _ => {}
            }
        }
    }

    fn child_at(&self, parent: InstId, depth: u16) -> Option<InstId> {
        self.inst(parent).and_then(|i| i.children.get(&depth).copied())
    }

    /// `F-70772` / `F-89132`
    fn place(&mut self, parent: InstId, p: &Place, key: u64) -> Option<InstId> {
        let existing = self.child_at(parent, p.depth);
        let id = match (existing, p.character) {
            (Some(e), Some(ch)) => {
                self.inst_mut(e)?.character = ch;
                e
            }
            (Some(e), None) => e,
            (None, Some(ch)) => {
                let mut i = Inst::new(ch, p.depth, Some(parent));
                i.clip_depth = p.clip_depth;
                i.ratio = p.ratio.unwrap_or(0);
                if let Some(Character::EditText(e)) = self.movie.char(ch) {
                    i.text = Some(Rc::from(e.initial.as_str()));
                }
                let id = self.alloc_inst(i);
                self.inst_mut(parent)?.children.insert(p.depth, id);
                id
            }
            (None, None) => {
                self.warn(format!("PlaceObject modifies empty depth {}", p.depth));
                return None;
            }
        };
        let i = self.inst_mut(id)?;
        if i.placed_by == Some(key) {
            return Some(id);
        }
        i.placed_by = Some(key);
        if let Some(m) = &p.matrix {
            i.set_placed(m);
        }
        if let Some(c) = p.cxform {
            i.cxform = c;
        }
        Some(id)
    }

    /// `F-12139`
    fn remove_child(&mut self, parent: InstId, depth: u16) {
        let Some(c) = self.child_at(parent, depth) else { return };
        let name = self.inst(c).map(|i| i.name.clone()).unwrap_or_else(|| Rc::from(""));
        if !name.is_empty() {
            if let Some(m) = self.inst(parent).and_then(|i| i.members) {
                if let Some(o) = self.obj_mut(m) {
                    let h = hash(&name);
                    o.members.retain(|e| e.0 != h);
                }
            }
        }
        if let Some(pi) = self.inst_mut(parent) {
            pi.children.remove(&depth);
        }
        self.free_inst_tree(c);
    }

    /// `F-27448`
    fn fire_events(&mut self, id: InstId, mask: u32) {
        let ev: Vec<Code> = match self.inst(id) {
            Some(i) => i.events.iter().filter(|e| e.0 & mask != 0).map(|e| e.1.clone()).collect(),
            None => return,
        };
        for code in ev {
            self.run_block(&code, Ref::Inst(id));
        }
    }

    // -------------------------------------------------------- object model

    pub fn get_member(&mut self, r: Ref, name: &Value) -> Option<Value> {
        match r {
            Ref::Inst(id) => self.inst_get(id, name),
            Ref::Obj(o) => self.obj_get(o, name),
        }
    }

    pub fn set_member(&mut self, r: Ref, name: &Value, v: Value) {
        match r {
            Ref::Inst(id) => self.inst_set(id, name, v),
            Ref::Obj(o) => self.obj_set(o, name, v),
        }
    }

    fn key(name: &Value) -> (u32, Rc<str>) {
        let s = name.to_string();
        (hash(&s), s)
    }

    /// `F-41319`
    fn inst_get(&mut self, id: InstId, name: &Value) -> Option<Value> {
        let (h, s) = Self::key(name);
        let i = self.inst(id)?;
        let v = match s.as_ref() {
            _ if h == hash("_name") => Some(Value::Str(i.name.clone())),
            _ if h == hash("_y") => Some(Value::Float(i.y)),
            _ if h == hash("_x") => Some(Value::Float(i.x)),
            _ if h == hash("_global") => Some(Value::Obj(Ref::Obj(self.global))),
            _ if h == hash("this") => Some(Value::Obj(Ref::Inst(id))),
            _ if h == hash("_alpha") => Some(Value::Float(i.tint[3] as f32 * (100.0 / 255.0))),
            _ if h == hash("_root") => Some(Value::Obj(Ref::Inst(self.root))),
            _ if h == hash("_parent") => Some(Value::Obj(Ref::Inst(i.parent.unwrap_or(id)))),
            _ if h == hash("_rotation") => Some(Value::Float(i.rotation)),
            _ if h == hash("_xscale") => Some(Value::Float(i.xscale)),
            _ if h == hash("_yscale") => Some(Value::Float(i.yscale)),
            _ if h == hash("textColor") => Some(Value::Int((i.tint[0] as i32) << 16 | (i.tint[1] as i32) << 8 | i.tint[2] as i32)),
            _ if h == hash("onEnterFrame") => Some(i.on_enter_frame.map_or(Value::Undef, Value::Obj)),
            _ if h == hash("_width") || h == hash("_height") => {
                let b = crate::draw::bounds(self, id);
                Some(Value::Float(match b {
                    Some(b) if h == hash("_width") => b[2] - b[0],
                    Some(b) => b[3] - b[1],
                    None => 0.0,
                }))
            }
            _ if h == hash("text") && i.text.is_some() => Some(Value::Str(i.text.clone().unwrap())),
            _ => None,
        };
        if v.is_some() {
            return v;
        }
        let m = match self.inst(id)?.members {
            Some(m) => m,
            None => {
                let m = self.alloc(ObjKind::Script);
                self.inst_mut(id)?.members = Some(m);
                m
            }
        };
        self.obj_get(m, name)
    }

    /// `F-26264`
    fn inst_set(&mut self, id: InstId, name: &Value, v: Value) {
        let (h, _) = Self::key(name);
        let Some(i) = self.inst_mut(id) else { return };
        let mut dirty = true;
        if h == hash("_y") {
            i.y = v.to_f32();
        } else if h == hash("_x") {
            i.x = v.to_f32();
        } else if h == hash("_alpha") {
            let a = v.to_i32();
            i.tint[3] = (((a << 8) - a) / 100) as u8;
            dirty = false;
        } else if h == hash("_rotation") {
            i.rotation = v.to_f32() % 360.0;
        } else if h == hash("textColor") {
            let c = v.to_i32();
            i.tint[0] = (c >> 16) as u8;
            i.tint[1] = (c >> 8) as u8;
            i.tint[2] = c as u8;
            dirty = false;
        } else if h == hash("_xscale") {
            i.xscale = v.to_f32();
        } else if h == hash("_yscale") {
            i.yscale = v.to_f32();
        } else if h == hash("onEnterFrame") {
            i.on_enter_frame = v.obj();
            dirty = false;
        } else if h == hash("onRelease") {
            i.on_release = v.obj();
            dirty = false;
        } else if h == hash("text") {
            i.text = Some(v.to_string());
            dirty = false;
        } else {
            let m = match i.members {
                Some(m) => m,
                None => {
                    let m = self.alloc(ObjKind::Script);
                    self.inst_mut(id).unwrap().members = Some(m);
                    m
                }
            };
            self.obj_set(m, name, v);
            return;
        }
        if dirty {
            i.compose();
        }
    }

    /// `F-42184` (script objects) / `F-39441` (arrays) / `F-41439` (functions)
    fn obj_get(&mut self, o: ObjId, name: &Value) -> Option<Value> {
        let obj = self.obj(o)?;
        match &obj.kind {
            ObjKind::Array(v) if !name.is_str() => {
                let k = name.to_i32();
                return if k >= 0 && (k as usize) < v.len() { Some(v[k as usize].clone()) } else { self.obj_get_script(o, name) };
            }
            ObjKind::Array(v) => {
                if hash(&name.to_string()) == hash("length") {
                    return Some(Value::Int(v.len() as i32));
                }
            }
            ObjKind::Func(f) => {
                let s = name.to_string();
                if let Some(k) = f.param_names.iter().position(|n| **n == *s) {
                    return Some(f.args.get(k).cloned().unwrap_or(Value::Undef));
                }
                let scope = f.scope;
                return match scope {
                    Some(sc) => self.get_member(sc, name),
                    None => self.registry_or_global(hash(&s)),
                };
            }
            ObjKind::Script => {}
        }
        self.obj_get_script(o, name)
    }

    fn obj_get_script(&mut self, o: ObjId, name: &Value) -> Option<Value> {
        let (h, _) = Self::key(name);
        if h == hash("this") {
            return Some(Value::Obj(Ref::Obj(o)));
        }
        if h == hash("prototype") {
            let p = match self.obj(o)?.prototype {
                Some(p) => p,
                None => {
                    let p = self.alloc(ObjKind::Script);
                    self.obj_mut(o)?.prototype = Some(p);
                    p
                }
            };
            return Some(Value::Obj(Ref::Obj(p)));
        }
        if let Some(v) = mget(&self.obj(o)?.members, h) {
            return Some(v.clone());
        }
        self.registry_or_global(h)
    }

    fn registry_or_global(&self, h: u32) -> Option<Value> {
        if let Some(v) = mget(&self.registry, h) {
            return Some(v.clone());
        }
        self.obj(self.global).and_then(|g| mget(&g.members, h).cloned())
    }

    fn obj_set(&mut self, o: ObjId, name: &Value, v: Value) {
        let Some(obj) = self.obj_mut(o) else { return };
        match &mut obj.kind {
            ObjKind::Array(arr) if !name.is_str() => {
                let k = name.to_i32();
                if k >= 0 && (k as usize) < 1 << 16 {
                    let k = k as usize;
                    if k >= arr.len() {
                        arr.resize(k + 1, Value::Undef);
                    }
                    arr[k] = v;
                }
                return;
            }
            ObjKind::Func(f) => {
                if let Some(sc) = f.scope {
                    self.set_member(sc, name, v);
                }
                return;
            }
            _ => {}
        }
        let (h, s) = Self::key(name);
        mset(&mut obj.members, h, &s, v);
    }

    fn delete_member(&mut self, r: Ref, name: &Value) {
        let h = hash(&name.to_string());
        let o = match r {
            Ref::Inst(i) => self.inst(i).and_then(|x| x.members),
            Ref::Obj(o) => Some(o),
        };
        if let Some(obj) = o.and_then(|o| self.obj_mut(o)) {
            obj.members.retain(|e| e.0 != h);
        }
    }

    /// Method call on an object (`callMethod` vtable slot 16 of each class).
    pub fn call_method(&mut self, r: Ref, name: &str, args: &[Value]) -> Value {
        match r {
            Ref::Inst(id) => self.inst_call(id, name, args),
            Ref::Obj(o) => {
                if let Some(ObjKind::Func(f)) = self.obj(o).map(|x| &x.kind) {
                    // an activation forwards to the scope it runs in
                    return match f.scope {
                        Some(sc) => self.call_method(sc, name, args),
                        None => Value::Undef,
                    };
                }
                // F-44997
                match self.obj_get(o, &Value::str(name)) {
                    Some(Value::Native(f)) => f(self, r, args),
                    Some(Value::Obj(fr)) if self.is_func(fr) => self.call_function(fr, args, r),
                    Some(_) => Value::Int(0),
                    None => {
                        self.warn(format!("method {} not found", name));
                        Value::Int(0)
                    }
                }
            }
        }
    }

    fn is_func(&self, r: Ref) -> bool {
        matches!(r, Ref::Obj(o) if matches!(self.obj(o).map(|x| &x.kind), Some(ObjKind::Func(_))))
    }

    /// `F-36699`
    fn inst_call(&mut self, id: InstId, name: &str, args: &[Value]) -> Value {
        let frame_arg = |a: &[Value]| (a.first().map_or(0, |v| v.to_i32()) - 1) as u16 as usize;
        match name {
            "gotoAndPlay" => {
                if !self.inst(id).map_or(false, |i| i.playing) {
                    self.goto(id, frame_arg(args));
                    self.play(id);
                }
                Value::Undef
            }
            "gotoAndStop" => {
                self.goto(id, frame_arg(args));
                self.stop(id);
                Value::Undef
            }
            "play" => {
                self.play(id);
                Value::Undef
            }
            "stop" => {
                self.stop(id);
                Value::Undef
            }
            "nextFrame" => {
                let f = self.inst(id).map_or(0, |i| i.frame as usize) + 1;
                let n = self.frame_count(id);
                self.goto(id, if n > 0 { f % n } else { 0 });
                Value::Undef
            }
            "prevFrame" => {
                let f = self.inst(id).map_or(0, |i| i.frame as usize);
                let n = self.frame_count(id);
                self.goto(id, if f == 0 { n.saturating_sub(1) } else { f - 1 });
                Value::Undef
            }
            "getBytesLoaded" | "getBytesTotal" => Value::Int(1),
            "attachMovie" => self.attach_movie(id, args),
            "removeMovieClip" => {
                if let Some((p, d)) = self.inst(id).and_then(|i| i.parent.map(|p| (p, i.depth))) {
                    self.remove_child(p, d);
                }
                Value::Undef
            }
            _ => {
                let h = hash(name);
                if let Some(Value::Native(f)) = mget(&self.registry, h).cloned() {
                    return f(self, Ref::Inst(id), args);
                }
                // MovieClip.prototype
                if let Some(Value::Obj(Ref::Obj(mc))) = mget(&self.registry, hash("MovieClip")).cloned() {
                    if let Some(p) = self.obj(mc).and_then(|o| o.prototype) {
                        match self.obj(p).and_then(|o| mget(&o.members, h)).cloned() {
                            Some(Value::Native(f)) => return f(self, Ref::Inst(id), args),
                            Some(Value::Obj(fr)) if self.is_func(fr) => return self.call_function(fr, args, Ref::Inst(id)),
                            _ => {}
                        }
                    }
                }
                self.warn(format!("GlobalMethod {} does not exist", name));
                Value::Undef
            }
        }
    }

    /// `play` (slot 4): a request applied after the instance's frame; a
    /// non-sprite forwards to its parent.
    fn play(&mut self, id: InstId) {
        let sprite = self.is_sprite(id);
        let Some(i) = self.inst_mut(id) else { return };
        i.play_req = true;
        if !sprite {
            if let Some(p) = i.parent {
                self.play(p);
            }
        }
    }

    fn stop(&mut self, id: InstId) {
        if let Some(i) = self.inst_mut(id) {
            i.stop_req = true;
        }
    }

    /// attachMovie(linkage, name, depth)
    fn attach_movie(&mut self, parent: InstId, args: &[Value]) -> Value {
        let link = args.first().map(|v| v.to_string()).unwrap_or_else(|| Rc::from(""));
        let name = args.get(1).map(|v| v.to_string()).unwrap_or_else(|| Rc::from(""));
        let depth = args.get(2).map_or(0, |v| v.to_i32()) as u16;
        let Some(ch) = self.movie.export(&link) else {
            self.warn(format!("attachMovie: no export {:?}", link));
            return Value::Undef;
        };
        self.remove_child(parent, depth);
        let mut i = Inst::new(ch, depth, Some(parent));
        i.name = name.clone();
        let id = self.alloc_inst(i);
        if let Some(p) = self.inst_mut(parent) {
            p.children.insert(depth, id);
        }
        self.set_member(Ref::Inst(parent), &Value::Str(name), Value::Obj(Ref::Inst(id)));
        Value::Obj(Ref::Inst(id))
    }

    /// Call a function object (`F-66672`).
    pub fn call_function(&mut self, f: Ref, args: &[Value], scope: Ref) -> Value {
        let Ref::Obj(fo) = f else { return Value::Undef };
        let Some(Obj { kind: ObjKind::Func(func), .. }) = self.obj_mut(fo) else { return Value::Undef };
        func.args = args.to_vec();
        func.scope = Some(scope);
        let (code, start, len, pool, v2, regs_map) = (func.code.clone(), func.start, func.len, func.pool.clone(), func.v2, func.param_regs.clone());
        if self.depth > 64 {
            self.warn("script recursion too deep".into());
            return Value::Undef;
        }
        let mut regs = vec![Value::Undef; 256];
        if v2 {
            regs[1] = Value::Obj(f);
            for (k, r) in regs_map.iter().enumerate() {
                regs[*r as usize] = args.get(k).cloned().unwrap_or(Value::Undef);
            }
            let n = regs_map.len();
            // a failed lookup leaves the register alone (with `_global` as the scope,
            // `_root`/`_parent` are not found and the parameter registers survive)
            for (k, nm) in ["_root", "_parent", "_global"].into_iter().enumerate() {
                if let Some(v) = self.get_member(f, &Value::str(nm)) {
                    if 3 + n + k < 256 {
                        regs[3 + n + k] = v;
                    }
                }
            }
        }
        let before = self.ret.len();
        self.depth += 1;
        self.exec(&code, start, start + len, f, &mut regs, pool);
        self.depth -= 1;
        if self.ret.len() > before {
            self.ret.pop().unwrap()
        } else {
            Value::Undef
        }
    }

    // ---------------------------------------------------------- interpreter

    /// Run a frame or event block with `scope` as `this`.
    pub fn run_block(&mut self, code: &Code, scope: Ref) {
        if !self.scripts {
            return;
        }
        let mut regs = vec![Value::Undef; 256];
        let before = self.ret.len();
        self.exec(code, 0, code.len(), scope, &mut regs, Rc::new(Vec::new()));
        self.ret.truncate(before);
    }

    fn pop(&mut self) -> Value {
        // the original warns on underflow and yields integer 0
        self.stack.pop().unwrap_or(Value::Int(0))
    }

    fn pop_args(&mut self) -> Vec<Value> {
        let n = self.pop().to_i32().clamp(0, 88) as usize;
        (0..n).map(|_| self.pop()).collect()
    }

    fn target_inst(&self, scope: Ref) -> Option<InstId> {
        let mut r = scope;
        for _ in 0..16 {
            match r {
                Ref::Inst(i) => return Some(i),
                Ref::Obj(o) => match self.obj(o).map(|x| &x.kind) {
                    Some(ObjKind::Func(f)) => r = f.scope?,
                    _ => return None,
                },
            }
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn exec(&mut self, code: &Code, start: usize, end: usize, scope: Ref, regs: &mut [Value], mut pool: Rc<Vec<Rc<str>>>) {
        let mut pc = start;
        let mut steps = 0u32;
        let end = end.min(code.len());
        while pc < end {
            steps += 1;
            if steps > 2_000_000 {
                self.warn("script step limit".into());
                return;
            }
            let Some((a, next)) = avm1::decode(code, pc) else { return };
            pc = next;
            match a {
                Action::End => return,
                Action::Op(op) => {
                    if self.op(op, scope) {
                        return;
                    }
                }
                Action::ConstantPool(v) => pool = Rc::new(v.into_iter().map(|s| Rc::from(s.as_str())).collect()),
                Action::Push(vals) => {
                    for v in vals {
                        let x = match v {
                            PushValue::Str(s) => Value::str(&s),
                            PushValue::Float(f) => Value::Float(f),
                            PushValue::Null | PushValue::Undefined => Value::Int(0),
                            PushValue::Register(r) => regs.get(r as usize).cloned().unwrap_or(Value::Undef),
                            PushValue::Bool(b) => Value::Bool(b),
                            PushValue::Double(d) => Value::Float(d as f32),
                            PushValue::Int(i) => Value::Int(i),
                            PushValue::Constant(c) => pool.get(c as usize).map(|s| Value::Str(s.clone())).unwrap_or(Value::Int(0)),
                        };
                        self.stack.push(x);
                    }
                }
                Action::StoreRegister(r) => {
                    let v = self.stack.last().cloned().unwrap_or(Value::Int(0));
                    if let Some(slot) = regs.get_mut(r as usize) {
                        *slot = v;
                    }
                }
                Action::Jump(o) => pc = (pc as i64 + o as i64).max(0) as usize,
                Action::If(o) => {
                    if self.pop().to_bool() {
                        pc = (pc as i64 + o as i64).max(0) as usize;
                    }
                }
                Action::GotoFrame(f) => {
                    if let Some(t) = self.target_inst(scope) {
                        self.goto(t, f as usize);
                    }
                }
                Action::GetUrl(url, target) => {
                    if let Some(cmd) = url.strip_prefix("FSCommand:") {
                        if let Some(mut h) = self.fscommand.take() {
                            h(self, cmd, &target);
                            if self.fscommand.is_none() {
                                self.fscommand = Some(h);
                            }
                        }
                    }
                }
                Action::DefineFunction2(f) => {
                    let func = Func {
                        code: code.clone(),
                        start: f.body,
                        len: f.body_len,
                        pool: pool.clone(),
                        param_regs: f.params.iter().map(|p| p.0).collect(),
                        param_names: Vec::new(),
                        v2: true,
                        args: Vec::new(),
                        scope: None,
                    };
                    let id = self.alloc(ObjKind::Func(Box::new(func)));
                    self.stack.push(Value::Obj(Ref::Obj(id)));
                    pc = f.body + f.body_len;
                }
                Action::DefineFunction { params, body, body_len, .. } => {
                    let func = Func {
                        code: code.clone(),
                        start: body,
                        len: body_len,
                        pool: pool.clone(),
                        param_regs: Vec::new(),
                        param_names: params.iter().map(|s| Rc::from(s.as_str())).collect(),
                        v2: false,
                        args: Vec::new(),
                        scope: None,
                    };
                    let id = self.alloc(ObjKind::Func(Box::new(func)));
                    self.stack.push(Value::Obj(Ref::Obj(id)));
                    pc = body + body_len;
                }
                other => self.warn(format!("unsupported action {:?}", other)),
            }
        }
    }

    /// Payload-free actions. Returns true on Return.
    fn op(&mut self, op: u8, scope: Ref) -> bool {
        let fbin = |p: &mut Player, f: fn(f32, f32) -> f32| {
            let b = p.pop().to_f32();
            let a = p.pop().to_f32();
            p.stack.push(Value::Float(f(a, b)));
        };
        let ibin = |p: &mut Player, f: fn(i32, i32) -> i32| {
            let b = p.pop().to_i32();
            let a = p.pop().to_i32();
            p.stack.push(Value::Int(f(a, b)));
        };
        match op {
            0x04 | 0x05 | 0x06 | 0x07 => {
                if let Some(t) = self.target_inst(scope) {
                    match op {
                        0x04 => {
                            self.inst_call(t, "nextFrame", &[]);
                        }
                        0x05 => {
                            self.inst_call(t, "prevFrame", &[]);
                        }
                        0x06 => self.play(t),
                        _ => self.stop(t),
                    }
                }
            }
            0x0a => fbin(self, |a, b| a + b),
            0x0b => fbin(self, |a, b| a - b),
            0x0c => fbin(self, |a, b| a * b),
            0x0d => fbin(self, |a, b| a / b),
            0x0e => {
                let b = self.pop().to_f32();
                let a = self.pop().to_f32();
                self.stack.push(Value::Bool(a == b));
            }
            0x0f => {
                let b = self.pop().to_f32();
                let a = self.pop().to_f32();
                self.stack.push(Value::Bool(a < b));
            }
            0x10 => {
                let b = self.pop().to_bool();
                let a = self.pop().to_bool();
                self.stack.push(Value::Bool(a && b));
            }
            0x11 => {
                let b = self.pop().to_bool();
                let a = self.pop().to_bool();
                self.stack.push(Value::Bool(a || b));
            }
            0x12 => {
                let a = self.pop().to_bool();
                self.stack.push(Value::Bool(!a));
            }
            0x13 => {
                let b = self.pop().to_string();
                let a = self.pop().to_string();
                self.stack.push(Value::Bool(a == b));
            }
            0x14 | 0x31 => {
                let a = self.pop().to_string();
                self.stack.push(Value::Int(a.len() as i32));
            }
            0x15 | 0x35 => {
                // StringExtract(string, index, count) — `A-75332`: the index is
                // 0-based (not Flash's 1-based); an index outside the string gives ""
                // with a warning; the count is clipped to the end
                let count = self.pop().to_i32();
                let index = self.pop().to_i32();
                let s = self.pop().to_string();
                let b = s.as_bytes();
                if index < 0 || index >= b.len() as i32 {
                    self.warn(format!("StringExtract index {} outside {:?}", index, s));
                    self.stack.push(Value::str(""));
                } else {
                    let from = index as usize;
                    let to = if count < 0 { from } else { (from + count as usize).min(b.len()) };
                    self.stack.push(Value::str(&String::from_utf8_lossy(&b[from..to])));
                }
            }
            0x17 | 0x26 => {
                self.pop();
            }
            0x18 => {
                let a = self.pop().to_i32();
                self.stack.push(Value::Int(a));
            }
            0x1c => {
                let name = self.pop();
                let v = match self.get_member(scope, &name) {
                    Some(v) => v,
                    None => {
                        self.warn(format!("variable {:?} not found, created", name));
                        self.set_member(scope, &name, Value::Int(0));
                        Value::Int(0)
                    }
                };
                self.stack.push(v);
            }
            0x1d | 0x3c => {
                let v = self.pop();
                let name = self.pop();
                self.set_member(scope, &name, v);
            }
            0x41 => {
                let name = self.pop();
                self.set_member(scope, &name, Value::Undef);
            }
            0x21 => {
                let b = self.pop().to_string();
                let a = self.pop().to_string();
                self.stack.push(Value::str(&format!("{}{}", a, b)));
            }
            0x29 => {
                let b = self.pop().to_string();
                let a = self.pop().to_string();
                self.stack.push(Value::Bool(a < b));
            }
            0x30 => {
                let n = self.pop().to_i32();
                let r = self.random();
                self.stack.push(Value::Int(if n > 0 { (r % n as u32) as i32 } else { 0 }));
            }
            0x32 | 0x36 => {
                let s = self.pop().to_string();
                self.stack.push(Value::Int(s.bytes().next().map_or(0, |b| b as i32)));
            }
            0x33 | 0x37 => {
                let c = self.pop().to_i32() as u8;
                self.stack.push(Value::str(&String::from_utf8_lossy(&[c])));
            }
            0x34 => {
                let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
                self.stack.push(Value::Int(t as i32));
            }
            0x3a => {
                let name = self.pop();
                let o = self.pop();
                if let Some(r) = o.obj() {
                    self.delete_member(r, &name);
                }
                self.stack.push(Value::Bool(true));
            }
            0x3d => {
                let name = self.pop().to_string();
                let args = self.pop_args();
                let v = if &*name == "Object" {
                    Value::Undef
                } else {
                    match self.get_member(scope, &Value::Str(name.clone())) {
                        Some(Value::Obj(f)) if self.is_func(f) => self.call_function(f, &args, scope),
                        Some(Value::Native(f)) => f(self, scope, &args),
                        _ => {
                            self.warn(format!("function {} not found", name));
                            Value::Undef
                        }
                    }
                };
                self.stack.push(v);
            }
            0x3e => {
                let v = self.pop();
                self.ret.push(v);
                return true;
            }
            0x3f => {
                let b = self.pop().to_i32();
                let a = self.pop().to_i32();
                self.stack.push(Value::Int(if b == 0 { 0 } else { a.wrapping_rem(b) }));
            }
            0x40 => {
                let name = self.pop().to_string();
                let args = self.pop_args();
                let kind = if &*name == "Array" { ObjKind::Array(Vec::new()) } else { ObjKind::Script };
                let o = self.alloc(kind);
                // no class is registered under "Object": a plain object
                if &*name != "Object" {
                    self.call_method(Ref::Obj(o), &name, &args);
                }
                self.stack.push(Value::Obj(Ref::Obj(o)));
            }
            0x42 => {
                let elems = self.pop_args();
                let o = self.alloc(ObjKind::Array(elems));
                self.stack.push(Value::Obj(Ref::Obj(o)));
            }
            0x43 => {
                let n = self.pop().to_i32().clamp(0, 1000);
                let o = self.alloc(ObjKind::Script);
                for _ in 0..n {
                    let v = self.pop();
                    let k = self.pop();
                    self.obj_set(o, &k, v);
                }
                self.stack.push(Value::Obj(Ref::Obj(o)));
            }
            0x44 => {
                let v = self.pop();
                let t = match &v {
                    Value::Str(_) => "string",
                    Value::Int(_) | Value::Float(_) => "number",
                    Value::Bool(_) => "boolean",
                    Value::Obj(Ref::Inst(_)) => "movieclip",
                    Value::Obj(r) if self.is_func(*r) => "function",
                    Value::Native(_) => "function",
                    Value::Obj(_) => "object",
                    Value::Undef => "undefined",
                };
                self.stack.push(Value::str(t));
            }
            0x47 => {
                let b = self.pop();
                let a = self.pop();
                if a.is_str() || b.is_str() {
                    self.stack.push(Value::str(&format!("{}{}", a.to_string(), b.to_string())));
                } else {
                    self.stack.push(Value::Float(a.to_f32() + b.to_f32()));
                }
            }
            0x48 | 0x67 => {
                let b = self.pop();
                let a = self.pop();
                let (a, b) = if op == 0x67 { (b, a) } else { (a, b) };
                let r = if a.is_str() || b.is_str() { *a.to_string() < *b.to_string() } else { a.to_f32() < b.to_f32() };
                self.stack.push(Value::Bool(r));
            }
            0x49 => {
                let b = self.pop();
                let a = self.pop();
                let r = equals2(&a, &b);
                self.stack.push(Value::Bool(r));
            }
            0x66 => {
                let b = self.pop();
                let a = self.pop();
                let r = match (&a, &b) {
                    (Value::Str(x), Value::Str(y)) => x == y,
                    (Value::Int(x), Value::Int(y)) => x == y,
                    (Value::Float(x), Value::Float(y)) => x == y,
                    (Value::Bool(x), Value::Bool(y)) => x == y,
                    (Value::Obj(x), Value::Obj(y)) => x == y,
                    (Value::Undef, Value::Undef) => true,
                    _ => false,
                };
                self.stack.push(Value::Bool(r));
            }
            0x4c => {
                let v = self.stack.last().cloned().unwrap_or(Value::Int(0));
                self.stack.push(v);
            }
            0x4d => {
                let b = self.pop();
                let a = self.pop();
                self.stack.push(b);
                self.stack.push(a);
            }
            0x4e => {
                let name = self.pop();
                let o = self.pop();
                let v = match &o {
                    Value::Str(s) if hash(&name.to_string()) == hash("length") => Value::Int(s.len() as i32),
                    Value::Obj(r) => match self.get_member(*r, &name) {
                        Some(v) => v,
                        None => {
                            self.warn(format!("member {:?} not found", name));
                            Value::Int(0)
                        }
                    },
                    _ => {
                        self.warn(format!("GetMember {:?} on a non-object {:?}", name, o));
                        Value::Int(0)
                    }
                };
                self.stack.push(v);
            }
            0x4f => {
                let v = self.pop();
                let name = self.pop();
                let o = self.pop();
                if let Some(r) = o.obj() {
                    self.set_member(r, &name, v);
                }
            }
            0x50 | 0x51 => {
                let a = self.pop().to_f32();
                self.stack.push(Value::Float(if op == 0x50 { a + 1.0 } else { a - 1.0 }));
            }
            0x52 => {
                let name = self.pop().to_string();
                let o = self.pop();
                let args = self.pop_args();
                let v = match o.obj() {
                    Some(r) => self.call_method(r, &name, &args),
                    None => {
                        self.warn(format!("CallMethod {} on a non-object", name));
                        Value::Int(0)
                    }
                };
                self.stack.push(v);
            }
            0x60 => ibin(self, |a, b| a & b),
            0x61 => ibin(self, |a, b| a | b),
            0x62 => ibin(self, |a, b| a ^ b),
            0x63 => ibin(self, |a, b| a.wrapping_shl(b as u32 & 31)),
            0x64 => ibin(self, |a, b| a >> (b & 31)),
            0x65 => ibin(self, |a, b| ((a as u32) >> (b & 31)) as i32),
            _ => self.warn(format!("unsupported action {:#04x}", op)),
        }
        false
    }
}

/// Width in pixels of `s` in the movie font named `font` at `size` pixels
/// (`FSCommand:MeasureString`): the glyph advances (em 1024) scaled by the size.
/// An unknown font name falls back to the movie's first font.
pub fn measure_string(movie: &Movie, font: &str, s: &str, size: f32) -> f32 {
    let mut first = None;
    for c in movie.chars.iter().flatten() {
        if let Character::Font(f) = c {
            if first.is_none() {
                first = Some(f);
            }
            if f.name.eq_ignore_ascii_case(font) {
                first = Some(f);
                break;
            }
        }
    }
    let Some(f) = first else { return 0.0 };
    let em: f32 = s.chars().map(|ch| f.glyph_index(ch).and_then(|g| f.advances.get(g).copied()).unwrap_or(0.0)).sum();
    em / 1024.0 * size
}

/// FSCommands every host answers the same way: `getvar` of the platform
/// constants and `MeasureString`. Returns false for anything else.
pub fn common_fscommand(p: &mut Player, cmd: &str, arg: &str) -> bool {
    match cmd.to_ascii_lowercase().as_str() {
        "getvar" => match arg {
            "aspect" => p.set_global("aspect", Value::Int(1)),
            "platform" => p.set_global("platform", Value::Int(0)),
            "lang" => p.set_global("lang", Value::Int(0)),
            _ => return false,
        },
        "measurestring" => {
            let s = p.get_global("fs_measure_string").map(|v| v.to_string().to_string()).unwrap_or_default();
            let size = p.get_global("fs_measure_size").map_or(20.0, |v| v.to_f32());
            let font = p.get_global("fs_measure_font").map(|v| v.to_string().to_string()).unwrap_or_default();
            let w = measure_string(&p.movie, &font, &s, size);
            p.set_global("fs_measure_width", Value::Float(w));
        }
        // `F-33151`: how many lines `fs_measure_string` takes in a field
        // `fs_measure_slotWidth` wide (an integer; the movies multiply it by the text
        // size to make room for a description). Lines break at spaces and at CR/LF.
        "measureslotheight" => {
            let s = p.get_global("fs_measure_string").map(|v| v.to_string().to_string()).unwrap_or_default();
            let size = p.get_global("fs_measure_size").map_or(20.0, |v| v.to_f32());
            let font = p.get_global("fs_measure_font").map(|v| v.to_string().to_string()).unwrap_or_default();
            let width = p.get_global("fs_measure_slotWidth").map_or(400.0, |v| v.to_f32()).max(1.0);
            let space = measure_string(&p.movie, &font, " ", size);
            let mut lines = 0;
            for para in s.split(|c: char| c == char::from(13) || c == char::from(10)) {
                lines += 1;
                let mut x = 0.0;
                for word in para.split(' ') {
                    let w = measure_string(&p.movie, &font, word, size);
                    if x > 0.0 && x + space + w > width {
                        lines += 1;
                        x = w;
                    } else {
                        x += if x > 0.0 { space } else { 0.0 } + w;
                    }
                }
            }
            p.set_global("fs_measure_height", Value::Int(if s.is_empty() { 0 } else { lines }));
        }
        _ => return false,
    }
    true
}

/// The Math natives (F-25737 sin, F-86511 cos, F-18756 tan, F-13576 atan, F-57109 asin, F-54807 acos, F-58006 sqrt,
/// F-84682 abs): the argument to float, the C library's double function of it, rounded to float
pub const MATH: [(&str, fn(f32) -> f32); 8] = [
    ("sin", |x| (x as f64).sin() as f32),
    ("cos", |x| (x as f64).cos() as f32),
    ("tan", |x| (x as f64).tan() as f32),
    ("atan", |x| (x as f64).atan() as f32),
    ("asin", |x| (x as f64).asin() as f32),
    ("acos", |x| (x as f64).acos() as f32),
    ("sqrt", |x| (x as f64).sqrt() as f32),
    ("abs", |x| x.abs()),
];

/// The sine and cosine of a rotation in degrees (F-98882): the angle times the float constant pi/180, the C library's
/// double sine and cosine of it rounded to float
pub fn rotation(deg: f32) -> (f32, f32) {
    let a = (deg * (std::f32::consts::PI / 180.0)) as f64;
    (a.sin() as f32, a.cos() as f32)
}

#[cfg(test)]
#[path = "original_tests.rs"]
mod original_tests;

#[cfg(test)]
#[path = "player_dispatch_tests.rs"]
mod dispatch_tests;

#[cfg(test)]
#[path = "player_inst_tests.rs"]
mod inst_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use mcla_formats::rage::swf::Sprite;

    fn empty_movie() -> Arc<Movie> {
        Arc::new(Movie { stage: [0.0, 1280.0, 0.0, 720.0], fps: 30, root: Sprite { frames: Vec::new() }, chars: Vec::new(), exports: Vec::new() })
    }

    fn push_str(code: &mut Vec<u8>, s: &str) {
        let n = s.len() + 2;
        code.extend([0x96, n as u8, 0, 0]);
        code.extend(s.as_bytes());
        code.push(0);
    }

    fn push_int(code: &mut Vec<u8>, i: i32) {
        code.extend([0x96, 5, 0, 7]);
        code.extend(i.to_le_bytes());
    }

    /// r = StringExtract(s, index, count) run by the interpreter
    fn extract(s: &str, index: i32, count: i32) -> String {
        let mut p = Player::new(empty_movie());
        let mut code = Vec::new();
        push_str(&mut code, "r");
        push_str(&mut code, s);
        push_int(&mut code, index);
        push_int(&mut code, count);
        code.extend([0x15, 0x1d, 0x00]);
        let root = p.root;
        p.run_block(&Code::from(&code[..]), Ref::Inst(root));
        p.get_member(Ref::Inst(root), &Value::str("r")).unwrap().to_string().to_string()
    }

    #[test]
    fn string_extract_is_zero_based_like_the_original() {
        // the disc's formatmoney: StringExtract(s, 0, 1) + "," + StringExtract(s, 1, 20)
        assert_eq!(extract("5000", 0, 1), "5");
        assert_eq!(extract("5000", 1, 20), "000");
        assert_eq!(extract("abcdef", 2, 3), "cde");
        // outside the string: empty
        assert_eq!(extract("abc", 3, 1), "");
        assert_eq!(extract("abc", -1, 2), "");
    }

    #[test]
    fn measure_string_without_fonts_is_zero() {
        assert_eq!(measure_string(&empty_movie(), "Dev Gothic", "ABC", 20.0), 0.0);
    }
}

/// Equals2 (`A-88557`): strings compare as strings, otherwise floats, ints,
/// booleans in that order; two objects compare equal (the original warns).
fn equals2(a: &Value, b: &Value) -> bool {
    if a.is_str() || b.is_str() {
        return a.to_string() == b.to_string();
    }
    let is = |v: &Value, k: u8| match (v, k) {
        (Value::Float(_), 4) | (Value::Int(_), 3) | (Value::Bool(_), 2) => true,
        _ => false,
    };
    if is(a, 4) || is(b, 4) {
        a.to_f32() == b.to_f32()
    } else if is(a, 3) || is(b, 3) {
        a.to_i32() == b.to_i32()
    } else if is(a, 2) || is(b, 2) {
        a.to_bool() == b.to_bool()
    } else {
        true
    }
}

/// `Array` constructor (`F-77768`): the arguments become the elements.
fn native_array(p: &mut Player, this: Ref, args: &[Value]) -> Value {
    if let Ref::Obj(o) = this {
        if let Some(Obj { kind: ObjKind::Array(v), .. }) = p.obj_mut(o) {
            *v = args.to_vec();
        }
    }
    Value::Undef
}

/// `Color` constructor (`F-82626`): `setRGB` native plus `_target`.
fn native_color(p: &mut Player, this: Ref, args: &[Value]) -> Value {
    p.set_member(this, &Value::str("setRGB"), Value::Native(native_set_rgb));
    p.set_member(this, &Value::str("_target"), args.first().cloned().unwrap_or(Value::Int(0)));
    Value::Undef
}

/// `setRGB` (`F-96389`): the target's RGB multiplier bytes.
fn native_set_rgb(p: &mut Player, this: Ref, args: &[Value]) -> Value {
    let t = p.get_member(this, &Value::str("_target"));
    let c = args.first().map_or(0, |v| v.to_i32());
    if let Some(Value::Obj(Ref::Inst(i))) = t {
        if let Some(inst) = p.inst_mut(i) {
            inst.tint[0] = (c >> 16) as u8;
            inst.tint[1] = (c >> 8) as u8;
            inst.tint[2] = c as u8;
        }
    }
    Value::Undef
}
