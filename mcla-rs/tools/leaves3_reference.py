"""Small leaves of the vehicle code, third batch (verify-leaves3): the generated code unchanged (vector mode with the SDK's flush
mode, like artic2_reference.py, whose harness this extends: f1 .. f6 are register codes 14 .. 19), random and edge cases, the original's
outputs into crates/game/src/testdata/leaves3_ref.txt for the tests next to each routine (`mod offline`). One case a line, the layout of
artic_reference.py: kind | registers | memory in | memory out | r3. Kinds:

  gv  F-90069  the wheel bound's vertices scaled (physics/gjk.rs::scale_corners; the polygon loop off, npoly 0)
  ge  F-17852  a curved edge's centre, normal and radius (physics/gjk.rs::edge_update)
  gf  F-22892  a curved face's cap / band numbers (physics/gjk.rs::face_update; F-95825 inside)
  qu  F-10653  the bumpers' hit queue (damageparts.rs::DamageParts::queue)
  am  F-82909  the ambient type's spring arithmetic (traffic.rs::amb_springs)
  rb  F-12436  the rubber band tune's arithmetic after the xml read (rubber.rs::after_load)
  tf  F-42446  a wheel's tyre effect rule (tyrefx.rs::decide_in; F-92300 and F-63975 / F-33510 stubbed as inputs / records)
  ec  F-14839  the engine's two 12-node curves with the spline builder F-85496 behind them (physics/engine_curve.rs::Curve::new)

Run (from mcla-rs): PYTHONPATH=tools python tools/leaves3_reference.py --recomp D:/MCLA-static --image <original-image.bin>
--out-dir <dir> --data-dir crates/game/src/testdata --tool <mcla-tool.exe> [--only kind,kind]"""
import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import artic_reference as base  # noqa: E402
from artic_reference import Case, Rng, fb, addr, run_cases, write  # noqa: E402
import recomp_closure  # noqa: E402
from cite_safe import cite_safe  # noqa: E402

# f2 .. f6 as register codes 15 .. 19 (the base harness has f1 only)
_old = ('else if (code == 14) { float x; memcpy(&x, &w[0], 4); ctx.f1.f64 = x; }')
_new = ('else if (code >= 14 && code <= 19) { float x; memcpy(&x, &w[0], 4); PPCRegister *F[6] = {&ctx.f1, &ctx.f2, &ctx.f3, &ctx.f4, &ctx.f5, &ctx.f6}; F[code - 14]->f64 = x; }')
assert base.MAIN_HEAD.count(_old) == 1
base.MAIN_HEAD = base.MAIN_HEAD.replace(_old, _new)

_vshim = recomp_closure.vector_shim


def _flush_shim(scratch_end=0x3000):
    s = _vshim(scratch_end)
    fl = 'struct { void disableFlushMode() {} void enableFlushMode() {} void disableFlushModeUnconditional() {} void enableFlushModeUnconditional() {} } fpscr;'
    assert s.count(fl) == 1
    on = '_mm_setcsr(_mm_getcsr() | 0x8040);'
    off = '_mm_setcsr(_mm_getcsr() & ~0x8040);'
    s = s.replace(fl, 'struct { void disableFlushMode() { %s } void enableFlushMode() { %s } void disableFlushModeUnconditional() { %s } '
                  'void enableFlushModeUnconditional() { %s } } fpscr;' % (off, on, off, on), 1)
    return '#include <xmmintrin.h>' + chr(10) + s


recomp_closure.vector_shim = _flush_shim
_scalar = recomp_closure.shim
recomp_closure.shim = lambda scratch_end=0x3000: _scalar(scratch_end).replace('int32_t s32;', 'int32_t s32; int16_t s16; int8_t s8;', 1).replace('Condition cr0,cr6;', 'Condition cr0,cr1,cr6,cr7;', 1)
recomp_closure.EXTRA += 'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}' + chr(10)
base.STACK = 0x5E00

SH, V, VT, EDG, FACE = 0x1000, 0x1100, 0x1280, 0x1400, 0x1600
HAND_MASS, HAND_GRAV = 0xFE01, 0xFE02
SRC = 0x3A00


class R(Rng):
    def x(self, lo=-3.0, hi=3.0):
        """a finite single, moderate or wide, sometimes an exact special"""
        r = self.random()
        if r < 0.05:
            return self.choice((0x00000000, 0x80000000, 0x3F800000, 0xBF800000, 0x3F000000))
        if r < 0.25:
            return fb(self.choice((-1, 1)) * 10 ** self.uniform(-3, 3))
        return fb(self.uniform(lo, hi))

    def pos(self, lo=1e-2, hi=4.0):
        r = self.random()
        if r < 0.15:
            return fb(10 ** self.uniform(-2, 2))
        return fb(self.uniform(lo, hi))

    def factor(self):
        """a scale factor: mostly positive, some zero, negative, -0"""
        r = self.random()
        if r < 0.05:
            return 0
        if r < 0.08:
            return 0x80000000
        if r < 0.15:
            return fb(-self.uniform(0.01, 3))
        return self.pos(0.05, 3.0)


def vec(r, lo=-3.0, hi=3.0):
    return [r.x(lo, hi), r.x(lo, hi), r.x(lo, hi), r.x(lo, hi)]


def gen_gjk(r):
    cases = []
    for _ in range(200):                      # F-90069: vertices x factor (a factor not above 0 leaves its axis alone)
        c = Case('gv')
        nv = r.randrange(1, 9)
        c.mem.put(SH + 4, [r.choice((0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 12, 255)) << 24])
        c.mem.put(SH, [VT])
        c.mem.put(VT + 160, [0xFE00])
        c.mem.put(SH + 128, [V])
        c.mem.put(SH + 200, [nv])
        c.mem.put(SH + 204, [0])
        for i in range(nv):
            c.mem.put(V + 16 * i, vec(r))
        sx = r.factor()
        sy = r.factor()
        if r.random() < 0.2:
            sx = sy
        c.reg(3, SH).reg(14, sx).reg(15, sy).reg(16, sy)
        c.out(V, nv * 4)
        cases.append(c)
    for _ in range(600):                      # F-17852: the edge
        c = Case('ge')
        nv = r.randrange(2, 9)
        c.mem.put(SH + 128, [V])
        for i in range(nv):
            c.mem.put(V + 16 * i, vec(r))
        e = 0x1300
        e0, e1 = r.randrange(0, nv), r.randrange(0, nv)
        if r.random() < 0.05:
            e1 = e0
        c.mem.put(e, vec(r))
        c.mem.put(e + 16, vec(r))
        c.mem.put(e + 32, [r.pos(), e0, e1])
        if r.random() < 0.1:                  # a degenerate edge: the three points on a line (cross ~ 0)
            c.mem.put(V + 16 * e0, [fb(0.5), fb(0.25), fb(-1.0), 0])
            c.mem.put(V + 16 * e1, [fb(0.75), fb(0.5), fb(-2.0), 0])
            c.mem.put(e, [fb(0.625), fb(0.375), fb(-1.5), 0])
        sx, sy = r.pos(0.05, 3.0), r.pos(0.05, 3.0)
        c.reg(3, e).reg(7, SH).reg(14, sx).reg(15, sy).reg(16, sy)
        c.out(e, 3).out(e + 16, 3).out(e + 32, 1)
        cases.append(c)
    for _ in range(600):                      # F-22892: the face
        c = Case('gf')
        nv = r.randrange(2, 9)
        c.mem.put(SH + 128, [V])
        for i in range(nv):
            c.mem.put(V + 16 * i, vec(r))
        ne = 6
        for k in range(ne):
            a = EDG + 48 * k
            c.mem.put(a, vec(r))
            c.mem.put(a + 16, vec(r))
            c.mem.put(a + 32, [r.pos(), r.randrange(0, nv), r.randrange(0, nv)])
        cap = r.random() < 0.4
        e = [r.randrange(0, ne) for _ in range(3)]
        cnt = r.choice((2, 2, 3, 4, 4, 1, 5))
        c.mem.put(FACE + 16, vec(r))
        c.mem.put(FACE + 4, [r.randrange(0, nv) << 16])
        c.mem.put(FACE + 48, [r.x(0.2, 3), r.x(-2, 2), r.x(0, 1)])
        c.mem.put(FACE + 60, [(e[0] << 16) | e[1], e[2] << 16])
        c.mem.put(FACE + 76, [cnt])
        c.mem.put(FACE + 80, [(1 if cap else 0) << 8])
        sx, sy = r.pos(0.05, 3.0), r.pos(0.05, 3.0)
        c.reg(3, FACE).reg(7, SH).reg(8, EDG).reg(14, sx).reg(15, sy).reg(16, sy)
        c.out(FACE + 16, 3).out(FACE + 48, 3)
        cases.append(c)
    return cases


def gen_queue(r):
    cases = []
    for _ in range(150):                      # F-10653
        c = Case('qu')
        obj = 0x1000
        n = r.choice((0, 1, 2, 3, 4, 5, 6, 7, 7, 8, 8, 9, 20, 0x7FFFFFFF, r.randrange(0, 12)))
        c.mem.put(obj + 2144, [n])
        for i in range(8):                    # what the slots held before
            c.mem.put(obj + 1888 + 32 * i, vec(r) + [r.x(), r.randrange(0, 256), 0, 0])
        c.mem.put(0x3800, vec(r))
        c.reg(3, obj).reg(4, 0x3800).reg(5, r.choice((0, 1, 2, 255, 0x100, 0x1FF, r.randrange(0, 1 << 16)))).reg(14, r.x(-2, 2))
        for i in range(8):
            c.out(obj + 1888 + 32 * i, 8)
        c.out(obj + 2144, 1)
        cases.append(c)
    return cases


def gen_amb(r):
    cases = []
    for _ in range(500):                      # F-82909
        c = Case('am')
        a, b = 0x1000, 0x1200
        c.mem.put(a + 104, [b])
        c.mem.put(b, [0x1240])
        c.mem.put(0x1240 + 28, [HAND_MASS])
        c.mem.put(0x1240 + 44, [HAND_GRAV])
        c.mem.put(b + 12, [0x1300])
        c.mem.put(0x1300 + 48, vec(r))
        c.mem.put(SRC, [fb(r.uniform(300, 4000)) if r.random() < 0.7 else r.pos(1e-1, 1e4), fb(r.uniform(0.5, 2)) if r.random() < 0.7 else r.pos(1e-1, 5)])
        # the tune: CogSim, CogOffset, Grip, Ts, Sd, Travel, Tt, Td, St (+196 .. +228)
        kind = r.random()
        if kind < 0.5:                        # the shipped files' range
            ws = [fb(r.uniform(0.1, 0.5)), fb(r.uniform(0.5, 1.5)), fb(r.uniform(0.3, 1.0)), fb(r.uniform(0.8, 2.5)), fb(r.uniform(0.1, 0.6)),
                  fb(r.uniform(0.1, 0.4)), fb(r.uniform(0.8, 2.0)), fb(r.uniform(0.02, 0.2)), fb(r.uniform(0.05, 0.3))]
        else:
            ws = [r.pos(0.05, 1.0), r.pos(0.05, 2.0), r.pos(0.05, 2.0), r.pos(0.1, 5), r.pos(0.01, 2), r.pos(0.02, 1), r.pos(0.1, 5), r.pos(0.005, 1), r.pos(0.01, 1)]
        if r.random() < 0.03:
            ws[r.randrange(3, 9)] = r.choice((0, 0x80000000))
        c.mem.put(a + 196, ws)
        c.reg(3, a)
        c.out(a + 236, 6)
        cases.append(c)
    return cases


def gen_rubber(r):
    cases = []
    for _ in range(12):                       # F-12436: eleven records of 35 words at +76
        c = Case('rb')
        obj = 0x1000
        for i in range(11):
            ws = []
            for k in range(35):
                ws.append(r.x(-4, 4) if r.random() < 0.6 else fb(r.uniform(0, 12000)))
            c.mem.put(obj + 76 + 140 * i, ws)
        c.reg(3, obj)
        for i in range(11):
            c.out(obj + 76 + 140 * i, 35)
        cases.append(c)
    return cases


TBL, TUNE, MAT, DRV = 0x1700, 0x1440, 0x1400, 0x14C0
G_TABLE, G_WET, G_DT = 0x82000000 + 0x90AC4C, 0x82000000 + 0x87E130, 0x82000000 + 0x7D7508


def gen_tyre(r):
    cases = []
    for _ in range(900):                      # F-42446
        c = Case('tf')
        S, W, VEL = 0x1000, 0x1100, 0x3800
        n = r.randrange(1, 7)
        c.mem.put(G_TABLE, [TBL])
        c.mem.put(TBL + 4, [n])
        for col in range(4):
            arr = 0x1780 + 0x40 * col
            c.mem.put(TBL + 8 + 8 * col, [arr])
            c.mem.put(arr, [0 if r.random() < 0.3 else r.getrandbits(32) | 1 for _ in range(n)])
        ix = lambda: r.choice((-1, -1, 0, 1, 2, 3, n - 1, n, n + 1, r.randrange(-3, n + 3))) & 0xFFFF
        c.mem.put(MAT + 44, [(ix() << 16) | ix()])
        thr0 = r.choice((0x3F666666, r.pos(0.02, 0.98), r.pos(0.02, 0.98), 0, 0x3F800000, r.x(-1, 2)))
        thr1 = r.choice((0, 0, 0x3F4CCCCD, r.pos(0.02, 0.98), r.x(-1, 1)))
        c.mem.put(MAT + 48, [thr0, thr1])
        c.mem.put(W + 4, [TUNE])
        c.mem.put(W + 8, [DRV])
        c.mem.put(TUNE + 84, [0 if r.random() < 0.04 else r.pos(0.02, 0.4)])
        c.mem.put(DRV + 372, [r.randrange(0, 6)])
        c.mem.put(W + 292, [r.x(-30, 30) if r.random() < 0.5 else r.x(-9, 9)])
        c.mem.put(W + 296, [r.x(-30, 30) if r.random() < 0.5 else r.x(-9, 9)])
        c.mem.put(W + 420, [r.choice((0x3E3851EC, 0x3F23D70A, r.pos(0.1, 0.9), r.pos(0.1, 0.9), r.x(-0.5, 1)))])
        c.mem.put(W + 464, [MAT])
        c.mem.put(W + 520, [r.x(-0.5, 1.5) if r.random() < 0.8 else r.choice((0, 0x3F800000, thr0))])
        c.mem.put(W + 528, [r.choice((0, 1)) << 16])
        c.mem.put(VEL, [r.x(-40, 40), r.x(-40, 40), r.x(-40, 40)])
        c.mem.put(SRC + 0x10, [r.choice((r.x(-0.1, 0.3), r.x(-0.1, 0.3), 0, 0x80000000, r.x(-2, 2)))])
        active = r.random() < 0.5
        rule0 = r.getrandbits(32) | 1
        c.mem.put(S, [0x1234 if active else 0, (1 << 24) if active or r.random() < 0.5 else 0])
        c.mem.put(S + 32, [r.x(0, 1), r.x(0, 1), r.x(0, 1.1), r.x(), r.choice((-3, -1, 0, 0, 1, 2, 5, 9, 10, 11, r.randrange(-3, 14))) & 0xFFFFFFFF, rule0 if active or r.random() < 0.5 else 0])
        wet = r.choice((0, 0, 0x3F666666, 0x3F666667, 0x3F800000, r.pos(0.0, 1.2), r.x(0, 1)))
        c.mem.put(G_WET, [wet])
        c.mem.put(G_DT, [r.choice((0x3D088889, 0x3D088889, r.pos(0.01, 0.1), 0))])
        c.reg(3, S).reg(4, VEL).reg(5, W).reg(6, 0).reg(7, r.choice((0, 1)))
        c.out(S + 32, 5).out(0x3A20, 7).out(0x3A50, 2)
        cases.append(c)
    return cases


def gen_curve(r):
    cases = []
    for _ in range(40):                       # F-14839 (+ F-85496 behind its virtual call)
        c = Case('ec')
        E, VTC = 0x1000, 0x1300
        n = r.randrange(3, 12)
        c.mem.put(VTC + 28, [0xFE10])
        c.mem.put(E + 4, [TUNE])
        c.mem.put(E + 68, [fb(r.uniform(400, 900))])
        c.mem.put(E + 76, [fb(r.uniform(30, 200))])
        c.mem.put(E + 112, [VTC, 0x1500, 12 << 16])
        c.mem.put(E + 192, [VTC, 0x1600, 12 << 16])
        c.mem.put(TUNE + 52, [n])
        vals = [r.randrange(-3000, 3000) if r.random() < 0.3 else r.randrange(0, 400) for _ in range(n)]
        c.mem.put(TUNE + 56, [v & 0xFFFFFFFF for v in vals])
        c.mem.put(TUNE + 100, [r.randrange(-1500, 400) & 0xFFFFFFFF for _ in range(n)])
        c.reg(3, E)
        c.out(0x4000, 192).out(0x4300, 192)
        cases.append(c)
    return cases


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--data-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--only', default='')
    a = ap.parse_args()
    a.out_dir.mkdir(parents=True, exist_ok=True)
    ids = ('F-90069', 'F-17852', 'F-22892', 'F-10653', 'F-82909', 'F-12436', 'F-95825', 'F-42446', 'F-14839', 'F-85496', 'F-92300', 'F-63975', 'F-33510', 'F-15488', 'F-83058', 'F-12484', 'F-36408', 'F-32585', 'F-32036', 'F-25937')
    fid = {f: addr(a.tool, f) for f in ids}
    sub = {f: 'sub_%08X' % x for f, x in fid.items()}
    r = R(20261110)
    call = lambda f: '%s(ctx,nullptr);' % sub[f]
    groups = [
        ('gjk', gen_gjk, {'gv': call('F-90069'), 'ge': call('F-17852'), 'gf': call('F-22892')}),
        ('queue', gen_queue, {'qu': call('F-10653')}),
        ('amb', gen_amb, {'am': call('F-82909')}),
        ('rubber', gen_rubber, {'rb': call('F-12436')}),
        ('tyre', gen_tyre, {'tf': call('F-42446')}),
        ('curve', gen_curve, {'ec': call('F-14839')}),
    ]
    only = [x for x in a.only.split(',') if x]
    cases, calls = [], {}
    for name, gen, cs in groups:
        if only and name not in only:
            continue
        cases += gen(r)
        calls.update(cs)
    names = [sub[f] for f in ('F-90069', 'F-17852', 'F-22892', 'F-10653', 'F-82909', 'F-12436', 'F-42446', 'F-14839', 'F-85496')]
    dispatch = ('if (a == 0xFE00) return; if (a == 0x%X) { ctx.f1.f64 = getf(0x%X); return; } if (a == 0x%X) { ctx.f1.f64 = getf(0x%X); return; } '
                'if (a == 0xFE10) { %s(ctx, base); return; } throw std::runtime_error("indirect");') % (HAND_MASS, SRC, HAND_GRAV, SRC + 4, sub['F-85496'])
    stubs = {}
    # (the callees that are not the rule: the spring set-up's physics hand-over F-83058, the rubber tune's reader and name formatting,
    # F-12484 F-36408 F-32585 F-32036 F-25937)
    for f in ('F-83058', 'F-12484', 'F-36408', 'F-32585', 'F-32036', 'F-25937'):
        stubs[sub[f]] = ''
    # F-92300 (the held compression) answers from the case; F-63975 (start / continue the effect) and F-33510 (stop it) write down what they got
    stubs[sub['F-92300']] = 'ctx.f1.f64 = getf(0x%X);' % (SRC + 0x10)
    stubs[sub['F-63975']] = ('store(0x3A54, load(0x3A54) + 1); store(0x3A20, ctx.r4.u32); store(0x3A24, ctx.r10.u32); '
                             '{ float x[5] = {(float)ctx.f1.f64, (float)ctx.f2.f64, (float)ctx.f3.f64, (float)ctx.f4.f64, (float)ctx.f5.f64}; '
                             'for (int k = 0; k < 5; k++) { uint32_t w; memcpy(&w, &x[k], 4); store(0x3A28 + 4 * k, w); } }')
    stubs[sub['F-33510']] = 'store(0x3A50, load(0x3A50) + 1);'
    # the allocator: 16-aligned blocks from 0x4000 on (the next free address kept in the guest memory, zero at the start of a case)
    stubs[sub['F-15488']] = 'uint32_t b = load(0x3FF0); if (b < 0x4000) b = 0x4000; uint32_t n = ctx.r3.u32; ctx.r3.u32 = b; store(0x3FF0, b + ((n + 15) & ~15u));'
    res = run_cases(a, cases, calls, dispatch, names, 'leaves3', stubs=stubs)
    # (the data file: words that look like addresses of the original get an h in front)
    text = []
    for c, rr in zip(cases, res):
        text.append(base.data_line(c, rr))
    (a.data_dir / 'leaves3_ref.txt').write_text(cite_safe('\n'.join(text) + '\n'))
    print('leaves3_ref.txt', len(cases), 'cases')


if __name__ == '__main__':
    main()
