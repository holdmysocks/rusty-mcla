"""The articulated body's leaves, second batch (verify-artic2): the same harness as artic_reference.py (read its header for the
case-file layout) with the SDK's flush mode on (MXCSR FTZ | DAZ while the generated code has it on; the first batch ran without
it and so saw denormals the console never has), fewer cases per kind, and more kinds. Output: crates/game/src/testdata/artic2_ref.txt.
Run (from mcla-rs): PYTHONPATH=tools python tools/artic2_reference.py --recomp D:/MCLA-static --image <original-image.bin>
--out-dir <dir> --data-dir crates/game/src/testdata --tool <mcla-tool.exe>"""
import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import recomp_closure  # noqa: E402
import artic_reference as base  # noqa: E402
from artic_reference import *  # noqa: E402,F401,F403

_frame = base._frame
base.STACK = 0x7E00                       # (the whole-body passes need a deeper stack than the leaves: the scratch is 0x1000 .. 0x8000)
_build = base.build


def _build8(*a, **k):
    k['scratch_end'] = 0x8000
    return _build(*a, **k)


base.build = _build8
_prev = recomp_closure.vector_shim


def _flush_shim(scratch_end=0x3000):
    s = _prev(scratch_end)
    fl = 'struct { void disableFlushMode() {} void enableFlushMode() {} void disableFlushModeUnconditional() {} void enableFlushModeUnconditional() {} } fpscr;'
    assert s.count(fl) == 1
    on = '_mm_setcsr(_mm_getcsr() | 0x8040);'
    off = '_mm_setcsr(_mm_getcsr() & ~0x8040);'
    s = s.replace(fl, 'struct { void disableFlushMode() { %s } void enableFlushMode() { %s } void disableFlushModeUnconditional() { %s } '
                  'void enableFlushModeUnconditional() { %s } } fpscr;' % (off, on, off, on), 1)
    return '#include <xmmintrin.h>' + chr(10) + s


recomp_closure.vector_shim = _flush_shim
_closure = recomp_closure.closure
recomp_closure.closure = lambda idx, names, stubs=(), limit=60: _closure(idx, names, stubs, 300)   # (the whole-body passes pull in many joint maps)


def thin(cases, keep, rnd):
    """at most keep[kind] cases of each kind (a random pick, order kept)"""
    out = []
    for kind in dict.fromkeys(c.kind for c in cases):
        cs = [c for c in cases if c.kind == kind]
        n = keep.get(kind, len(cs))
        pick = set(rnd.sample(range(len(cs)), min(n, len(cs))))
        out += [c for i, c in enumerate(cs) if i in pick]
    return out


KEEP = {'pr': 60, 'am': 0, 'cp': 0, 'ar': 15, 'pi': 15, 'bc': 80, 'hd': 60, 'rs': 0, 'sv': 40, 'tr': 100, 'gr': 40, 'si': 50,
        'm1': 40, 'm2': 40, 'jb': 0, 'hj': 1, 'lb': 0, 'rc': 50, 'pv': 12, 'rv': 18, 'ri': 18, 'wk': 25, 'su': 10, 'sd': 10, 'rt': 25,
        'pg': 10, 'bf': 8, 'fv': 7, 'ax': 12, 'bi': 40}


def island2(c, rnd):
    """island_state plus the words the rows' axes read (a ball's n, m, e2; a hinge's state)"""
    island_state(c, rnd)
    for j in range(NL - 1):
        a = JOINT0 + JSZ * j
        if c.mem.w[a] == 1:
            c.mem.put(a + 768, rnd.v(12, 0.03))
        else:
            c.mem.put(a + 632, [rnd.choice((0, 1, 0xFFFFFFFF))])
    c.mem.put(BUF, rnd.v(4, 0.04))


def gen_new(rnd, tool):
    cases = []
    for _ in range(150):                      # F-50660: the point velocity of link k
        c = Case('pv')
        island2(c, rnd)
        c.reg(3, BODY).reg(4, rnd.randrange(0, NL)).reg(5, BUF).reg(6, OUT)
        c.out(OUT, 4)
        island_outs(c)
        cases.append(c)
    for _ in range(150):                      # F-94941: the row's velocity
        c = Case('rv')
        island2(c, rnd)
        j = rnd.randrange(0, NL - 1)
        kind = rnd.randrange(0, 4) if c.mem.w[JOINT0 + JSZ * j] == 1 else rnd.randrange(0, 4)
        c.reg(3, OUT).reg(4, BODY).reg(5, j).reg(6, kind)
        c.out(OUT, 4)
        island_outs(c)
        cases.append(c)
    for _ in range(150):                      # F-17591: an impulse s along the row
        c = Case('ri')
        island2(c, rnd)
        j = rnd.randrange(0, NL - 1)
        c.reg(3, BODY).reg(4, j).reg(5, rnd.randrange(0, 4)).reg(14, rnd.fin() if rnd.random() < 0.8 else rnd.f())
        island_outs(c)
        cases.append(c)
    for _ in range(200):                      # F-55744: the pivot walked to a link (F-31346, F-81602 inside)
        c = Case('wk')
        island2(c, rnd)
        c.reg(3, BODY).reg(4, rnd.randrange(0, NL)).reg(5, rnd.choice((-1, 0, 1, 2, 3, rnd.randrange(0, NL))) & 0xFFFFFFFF)
        island_outs(c)
        cases.append(c)
    for kind in ('su', 'sd'):                 # F-31346 up (child c to its parent), F-81602 down
        for _ in range(150):
            c = Case(kind)
            parents = island_state(c, rnd)
            ch = rnd.randrange(1, NL)
            c.reg(3, BODY).reg(4, ch).reg(5, parents[ch - 1])
            island_outs(c)
            cases.append(c)
    for _ in range(60):                       # F-52649: every child put back on its parent through its joint
        c = Case('rt')
        parents = rand_tree(rnd, NL)
        body_frame(c, rnd, NL, parents)
        for i in range(NL):
            a = LINK0 + LSZ * i
            c.mem.put(a + 416, _frame(rnd))
            c.mem.put(a + 464, rnd.v(4, 0.02))
            c.mem.put(a + 608, rnd.v(8, 0.02))
        for j in range(NL - 1):
            a = JOINT0 + JSZ * j
            c.mem.put(a, [rnd.choice((0, 1))])
            c.mem.put(a + 8, [LINK0 + LSZ * parents[j], LINK0 + LSZ * (j + 1)])
            c.mem.put(a + 432, _frame(rnd))
            c.mem.put(a + 496, rnd.v(4, 0.02))
            c.mem.put(a + 512, rnd.v(4, 0.02))
            c.mem.put(a + 544, _frame(rnd))
        c.reg(3, BODY)
        for i in range(NL):
            c.out(LINK0 + LSZ * i + 416, 16).out(LINK0 + LSZ * i + 608, 16)
        cases.append(c)
    return cases


def full_island(c, rnd):
    """island_state plus every word the whole-body passes read: links' inertia, position, velocities, forces, impulses, bias;
    joints' two inertias and the maps"""
    parents = island_state(c, rnd)
    for i in range(NL):
        a = LINK0 + LSZ * i
        c.mem.put(a + 128, rnd.v(36, 0.02))
        c.mem.put(a + 464, rnd.v(4, 0.02))
        c.mem.put(a + 608, rnd.v(32, 0.02))      # +608 .. +735: velocities, motion, force, torque, impulses
        c.mem.put(a + 880, rnd.v(8, 0.02))
        c.mem.put(a + 912, rnd.v(16, 0.02))
    for j in range(NL - 1):
        a = JOINT0 + JSZ * j
        c.mem.put(a + 16, rnd.v(72, 0.02))
        c.mem.put(a + 336, rnd.v(16, 0.02))
        c.mem.put(a + 368, rnd.v(16, 0.02))
    return parents


def whole_outs(c, links, joints):
    for i in range(NL):
        for off, n in links:
            c.out(LINK0 + LSZ * i + off, n)
    for j in range(NL - 1):
        for off, n in joints:
            c.out(JOINT0 + JSZ * j + off, n)


def gen_whole(rnd, tool):
    cases = []
    for _ in range(20):                       # F-39401: the impulses through the inverses, up and down the joints
        c = Case('pg')
        full_island(c, rnd)
        c.reg(3, BODY)
        whole_outs(c, [(704, 8), (912, 16)], [(304, 16)])
        cases.append(c)
    for _ in range(20):                       # F-60999: the articulated bias forces
        c = Case('bf')
        full_island(c, rnd)
        c.reg(3, BODY)
        whole_outs(c, [(880, 8)], [(368, 16)])
        cases.append(c)
    for _ in range(20):                       # F-90112: velocities made relative, bias, forces, impulses committed
        c = Case('fv')
        full_island(c, rnd)
        dt = rnd.choice((fb(1 / 60), fb(1 / 30), fb(1 / 120), 0, rnd.fin()))
        c.reg(3, BODY).reg(11, dt, dt, dt, dt)
        whole_outs(c, [(608, 32), (880, 24)], [(304, 28)])
        cases.append(c)
    for _ in range(40):                       # F-58091: a . response(a) of a joint
        c = Case('ax')
        full_island(c, rnd)
        j = rnd.randrange(0, NL - 1)
        c.mem.put(BUF, rnd.v(4, 0.04))
        c.reg(3, JOINT0 + JSZ * j).reg(4, BUF)
        c.out(OUT, 1)
        cases.append(c)
    for _ in range(40):                       # F-11734: the 6x6 block inverse
        c = Case('bi')
        c.mem.put(0x3A00, rnd.v(36, 0.02))
        c.reg(3, 0x3A00).reg(4, 0x3B00)
        c.out(0x3B00, 36)
        cases.append(c)
    return cases


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--data-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--group', default='all')
    a = ap.parse_args()
    a.out_dir.mkdir(parents=True, exist_ok=True)
    ids = ('F-78078', 'F-67069', 'F-48604', 'F-10668', 'F-96841', 'F-79699', 'F-47802', 'F-91185', 'F-38496', 'F-21563', 'F-29294',
           'F-53726', 'F-25250', 'F-93538', 'F-77258', 'F-33001', 'F-19400', 'F-50660', 'F-94941', 'F-17591', 'F-55744', 'F-31346', 'F-81602', 'F-52649', 'F-39401', 'F-60999', 'F-90112', 'F-58091', 'F-11734')
    fid = {f: addr(a.tool, f) for f in ids}
    sub = {f: 'sub_%08X' % x for f, x in fid.items()}
    rnd = Rng(20261110)
    call = lambda f: '%s(ctx,nullptr);' % sub[f]
    isl = thin(base.gen_island(rnd, fid), KEEP, rnd)
    art = thin(base.gen_artic(rnd, fid), KEEP, rnd)
    bld = thin(base.gen_build(rnd, fid), KEEP, rnd)
    new = thin(gen_new(rnd, fid), KEEP, rnd)
    new += thin(gen_whole(rnd, fid), KEEP, rnd)
    cases = isl + art + bld + new
    calls = {'pr': call('F-78078'), 'ar': call('F-48604'), 'pi': call('F-96841'), 'bc': call('F-79699'), 'hd': call('F-47802'),
             'sv': call('F-91185'), 'tr': call('F-38496'), 'gr': call('F-21563'), 'si': call('F-53726'), 'm1': call('F-25250'),
             'm2': call('F-93538'), 'hj': call('F-77258'), 'rc': call('F-33001'),
             'pv': call('F-50660'), 'rv': call('F-94941'), 'ri': call('F-17591'), 'wk': call('F-55744'), 'su': call('F-31346'), 'sd': call('F-81602'), 'rt': call('F-52649'), 'pg': call('F-39401'), 'bf': call('F-60999'), 'fv': call('F-90112'),
             'ax': call('F-58091') + ' { float f = (float)ctx.f1.f64; uint32_t u; memcpy(&u, &f, 4); store(0x3900, u); }', 'bi': call('F-11734')}
    names = [sub[f] for f in ('F-78078', 'F-48604', 'F-96841', 'F-79699', 'F-47802', 'F-91185', 'F-38496', 'F-21563', 'F-53726',
                              'F-25250', 'F-93538', 'F-77258', 'F-33001', 'F-50660', 'F-94941', 'F-17591', 'F-55744', 'F-31346', 'F-81602', 'F-52649', 'F-39401', 'F-60999', 'F-90112', 'F-58091', 'F-11734')]
    res = run_cases(a, cases, calls, 'throw std::runtime_error("indirect");', names, 'artic2', stubs={sub['F-29294']: MEMSET_BODY})
    write(a.data_dir, 'artic2_ref.txt', cases, res)


if __name__ == '__main__':
    main()
