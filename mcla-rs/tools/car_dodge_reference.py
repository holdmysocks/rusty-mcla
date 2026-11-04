"""The walkers' car dodge run offline: the threat F-15352 whole (with the direction search F-94813, the probe F-78951,
the lane limit F-50861, the slide start F-83234, the evade clip pick F-37443, the inside-0x800 test F-88694, the
segment F-17399, F-41334, F-32060 with the C library's acos, the guest's sin / cos, F-98427 and the obstacle's own
getters F-82464 / F-71030 / F-63122, the probe query's ctor F-13499) - the generated code whole and unchanged in vector mode, on flat guest memory,
with the flush mode the SDK's (FTZ and DAZ on the host while the code has it on). Also F-94813 and F-83234 called on
their own (the sidestep's arguments: no car, +463 = 1).
Stubbed, as inputs of the case, and recorded in order (tag, words): the world's queries - F-88694's sphere run F-66549
(t1: centre, radius; its ctor F-43397 empty), the stop timer F-84843 (t2), the swept ball F-27454 (the probe's vt+16,
t3: from, to, radius, a car left out), the ground probe F-57005 (t4: the segment; its ctor F-10576 empty), the lane
snap F-59579 (t5: point, radius), the mover sample F-21043 (t6: clip, time; the case's two moves of the clip, the
first call the one at the pick's time), the speech F-39461 (t7: kind), the temp rails F-99671 (t8) and the path
F-50795 (t9). Also as inputs: the walker's place F-58860 and matrix F-33765, the grid's cell F-88455 and 9 list heads
F-99153 (the objects in cell order), F-96472 / F-17857 of an ambient car, the object's matrix (vt+4).
The type / mask / exclusion words of the queries are checked here (an ERR token otherwise): F-66549 1 instance, the
walker's, 640, 0x800; F-27454 640, 0x381E, byte 7, the walker's and the car's instances; F-57005 512, 0x381E.
Start-up constants the image holds as zero are filled in as the running game has them: 0x828609E0 = +inf.
Writes <out-dir>/car_dodge_ref.txt: per case the input words, '->', the outputs (mode 0 / 2: the avoidance part's
words, the two MWCs; mode 1: the result, the displacement, T, the clear way, the 0x800 flag) and the record. Words as
hex; a word whose top byte is 0x82 as two 16-bit halves hhhh_llll."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build, run

recomp_closure.shim = lambda: recomp_leaf.shim(0x80000)
recomp_closure.EXTRA += r'''
#define ctr r2
static void dispatch(Context&,const uint8_t*,uint32_t);
#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)
static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}
namespace rex{namespace ppc{static inline simde__m128 simde_mm_cvtepu32_ps_(simde__m128i v){alignas(16) uint32_t u[4];
simde_mm_store_si128((simde__m128i*)u,v);return simde_mm_set_ps(float(u[3]),float(u[2]),float(u[1]),float(u[0]));}}}
static std::string LOG;static const uint32_t*KC;static int NSW,NGR,NLN,MC[8];static uint32_t HEADS[9];
static void logt(int t){LOG+=" t"+std::to_string(t);}
static void lw(uint32_t w){char b[12];snprintf(b,12," %08x",w);LOG+=b;}
static void err(int k){LOG+=" ERR"+std::to_string(k);}
'''
_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/avx2.h>' + chr(10) + '#include <xmmintrin.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    s = s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                  'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)
    fp = ('struct { void disableFlushMode() {} void enableFlushMode() {} void disableFlushModeUnconditional() {}'
          ' void enableFlushModeUnconditional() {} } fpscr;')
    assert s.count(fp) == 1
    on, off = '{_mm_setcsr(_mm_getcsr()|0x8040);}', '{_mm_setcsr(_mm_getcsr()&~0x8040u);}'
    return s.replace(fp, 'struct { void disableFlushMode() %s void enableFlushMode() %s void disableFlushModeUnconditional() %s'
                     ' void enableFlushModeUnconditional() %s } fpscr;' % (off, on, off, on), 1)


recomp_closure.vector_shim = vector_shim
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3', '-mavx2'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def bf(u):
    return struct.unpack('>f', struct.pack('>I', u))[0]


def f32(x):
    return bf(fb(x))


def word(x):
    return ('%04x_%04x' % (x >> 16, x & 0xFFFF)) if x >> 24 == 0x82 else '%x' % x


NSWEEP = 6
NENT = 6
ENTW = 11  # type, dir, audio, clip present, duration, move at t xyz, move at the end xyz
NOBJ = 5
OBJW = 26  # cell, kind, rail, beh, active, a, c, q, vel, prev xyz, e0, e1, rv, has +48, burn, inst
# the case's words
P = {}
_n = 0
for name, w in (('mode', 1), ('p', 3), ('r', 1), ('u', 3), ('rows', 9), ('female', 1), ('flags', 1), ('mdir', 3),
                 ('clip', 1), ('time', 1), ('len', 1), ('yaw', 1), ('stretch', 1), ('t', 1), ('stop', 3), ('from', 3),
                 ('to', 3), ('mwc', 4), ('dt', 1), ('in800', 1), ('sw', 5 * NSWEEP), ('gr', NSWEEP), ('ln', 5 * NSWEEP),
                 ('nent', 1), ('ent', ENTW * NENT), ('nobj', 1), ('cell', 1), ('obj', OBJW * NOBJ), ('x', 11)):
    P[name] = _n
    _n += w
W = _n


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    ids = ('F-15352', 'F-94813', 'F-83234', 'F-58860', 'F-88455', 'F-99153', 'F-96472', 'F-17857', 'F-43397', 'F-66549',
           'F-84843', 'F-10576', 'F-57005', 'F-59579', 'F-33765', 'F-21043', 'F-39461', 'F-99671', 'F-50795', 'F-98427',
           'F-27454')
    A = {f: addr(a.tool, f) for f in ids}
    getters = [addr(a.tool, i) for i in ('F-98427', 'F-82464', 'F-71030', 'F-63122')]
    rnd = random.Random(15352)

    def unit(v):
        l = math.sqrt(sum(x * x for x in v))
        return [x / l for x in v] if l > 0 else v

    def yrot(h, tilt=0.0):
        # rows a (x), up, c (z) of a matrix turned by h about y, then a little about x
        a_ = [math.cos(h), 0.0, -math.sin(h)]
        c_ = [math.sin(h), 0.0, math.cos(h)]
        up = [0.0, 1.0, 0.0]
        if tilt:
            ct, st = math.cos(tilt), math.sin(tilt)
            up, c_ = [up[i] * ct + c_[i] * st for i in range(3)], [c_[i] * ct - up[i] * st for i in range(3)]
        return a_, up, c_

    def vec(v):
        return [fb(f32(x)) for x in v]

    cases = []
    for ci in range(a.cases):
        mode = 0 if ci < a.cases * 3 // 4 else (1 if ci % 2 else 2)
        c = [0] * W
        c[P['mode']] = mode
        pw = [rnd.uniform(-2000, 2000), rnd.uniform(-10, 40), rnd.uniform(-2000, 2000)]
        r = rnd.choice((0.3, 0.35, rnd.uniform(0.2, 0.5)))
        u = unit([rnd.uniform(-0.2, 0.2), 1.0, rnd.uniform(-0.2, 0.2)]) if rnd.random() < 0.5 else [0.0, 1.0, 0.0]
        if rnd.random() < 0.03:
            u = rnd.choice(([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]))
        wa, wu, wc = yrot(rnd.uniform(-math.pi, math.pi), rnd.choice((0.0, 0.0, rnd.uniform(-0.2, 0.2))))
        c[P['p']:P['p'] + 3] = vec(pw)
        c[P['r']] = fb(f32(r))
        c[P['u']:P['u'] + 3] = vec(u)
        c[P['rows']:P['rows'] + 9] = vec(wa + wu + wc)
        c[P['female']] = rnd.getrandbits(1)
        c[P['flags']] = rnd.choice((0, 0, 0, 4, 4, 2, 6)) | rnd.choice((0, 0, 1, 8, 0x40))
        c[P['mdir']:P['mdir'] + 3] = vec(unit([rnd.uniform(-1, 1), 0.0, rnd.uniform(-1, 1)]))
        nent = rnd.choice((0, 2, 4, 6, 6, 6))
        c[P['nent']] = nent
        with_clip = []
        for i in range(NENT):
            e = P['ent'] + ENTW * i
            ty = rnd.choice((0, 1, 1, 2, 2, 3, 4))
            dr = rnd.choice((0, 0, 0, 1, 2, 3, 4))
            au = rnd.choice((0, 1, 1, 2))
            pres = int(rnd.random() < 0.85)
            dur = rnd.choice((rnd.uniform(0.3, 3.0), 0.2, 1.0))
            ang = rnd.uniform(-math.pi, math.pi)
            rr = rnd.uniform(0, 3)
            dt_ = [-math.sin(ang) * rr, rnd.uniform(-0.05, 0.05), -math.cos(ang) * rr]
            re_ = rnd.uniform(0, 5)
            de = [rnd.uniform(-1, 1) * re_, 0.0, rnd.uniform(-1, 1) * re_]
            c[e:e + ENTW] = [ty, dr, au, pres, fb(f32(dur))] + vec(dt_) + vec(de)
            if pres and i < nent:
                with_clip.append(i)
        c[P['clip']] = rnd.choice(with_clip) if with_clip and rnd.random() < 0.35 else 0xFFFFFFFF
        for k in ('time', 'len', 'yaw', 'stretch', 't'):
            c[P[k]] = fb(f32(rnd.uniform(0, 1.5)))
        for k in ('stop', 'from', 'to'):
            c[P[k]:P[k] + 3] = vec([pw[i] + rnd.uniform(-2, 2) for i in range(3)])
        c[P['mwc']:P['mwc'] + 4] = [rnd.getrandbits(32) for _ in range(4)]
        c[P['dt']] = fb(f32(rnd.choice((1 / 30, 1 / 30, 1 / 60, 0.0, 0.05))))
        c[P['in800']] = int(rnd.random() < 0.08)
        for k in range(NSWEEP):
            s = P['sw'] + 5 * k
            hit = int(rnd.random() < 0.45)
            d = rnd.uniform(0, 4) if rnd.random() < 0.7 else rnd.uniform(0, 14)
            h = rnd.uniform(-math.pi, math.pi)
            hp = [pw[0] + d * math.cos(h), pw[1] + rnd.uniform(0, 1.2), pw[2] + d * math.sin(h)]
            c[s:s + 5] = [hit] + vec(hp) + [rnd.choice((0x800, 0x800, 0x10, 0x804, 0x2))]
            c[P['gr'] + k] = int(rnd.random() < 0.85)
            s = P['ln'] + 5 * k
            off = [rnd.uniform(-10, 10), rnd.uniform(-1, 1), rnd.uniform(-10, 10)]
            if rnd.random() < 0.05:
                off = [0.0, 0.5, 0.0]
            d2 = sum(x * x for x in off) if rnd.random() < 0.5 else off[0] ** 2 + off[2] ** 2
            # (the nearest point is relative to the probe's end, unknown here: the stub adds the case's offset to it)
            c[s:s + 5] = [int(rnd.random() < 0.5)] + vec(off) + [fb(f32(d2))]
        # the objects, in cell order
        nobj = rnd.choice((0, 1, 1, 2, 3, 5)) if mode == 0 else rnd.choice((1, 2))
        c[P['nobj']] = nobj
        c[P['cell']] = rnd.getrandbits(16)
        cells = sorted(rnd.randrange(9) for _ in range(nobj))
        for j in range(NOBJ):
            o = P['obj'] + OBJW * j
            kind = rnd.choice((0, 0, 1, 2, 2, 2, 3))
            h = rnd.uniform(-math.pi, math.pi)
            oa, ou, oc = yrot(h, rnd.choice((0.0, 0.0, 0.0, rnd.uniform(-0.1, 0.1))))
            u_ = rnd.random()
            speed = rnd.choice((0.0, 0.005, rnd.uniform(1, 9), rnd.uniform(9, 11), rnd.uniform(10, 35)))
            if u_ < 0.6:
                vd = [-x for x in oc]  # forward
            elif u_ < 0.7:
                vd = oc
            elif u_ < 0.85:
                vd = rnd.choice((oa, [-x for x in oa]))
            else:
                vd = unit([oa[i] * rnd.uniform(-1, 1) + oc[i] * rnd.uniform(-1, 1) for i in range(3)])
            vel = [vd[i] * speed + rnd.uniform(-0.3, 0.3) * (i != 1) for i in range(3)]
            if rnd.random() < 0.1:
                vel[1] = rnd.uniform(-3, 3)
            # placed so that its path passes near the walker: back along its move, a little to the side
            back = rnd.uniform(-2, 1.2 * speed + 6)
            side = rnd.choice((rnd.uniform(-1.5, 1.5), rnd.uniform(-5, 5)))
            vn = unit([vel[0], 0.0, vel[2]]) if speed > 0 else [0.0, 0.0, 1.0]
            sd = [-vn[2], 0.0, vn[0]]
            q = [pw[i] - vn[i] * back + sd[i] * side for i in range(3)]
            q[1] = pw[1] + rnd.choice((rnd.uniform(-1, 1), 5.0, -5.0, rnd.uniform(-7, 7)))
            prev = [vel[i] - rnd.choice((0.0, rnd.uniform(-2, 2), rnd.uniform(-0.1, 0.1))) for i in range(3)]
            e0, e1 = rnd.uniform(1.6, 2.6), rnd.uniform(3.5, 6.0)
            rv = math.sqrt((e0 / 2) ** 2 + (e1 / 2) ** 2) if rnd.random() < 0.7 else rnd.uniform(0.5, 4)
            has48 = int(kind <= 1 and rnd.random() < 0.8 or rnd.random() < 0.3)
            burn = rnd.choice((0.0, 0.0, 0.5, 1.0, -0.0))
            c[o:o + OBJW] = ([cells[j] if j < nobj else 0, kind, int(rnd.random() < 0.3), int(rnd.random() < 0.5),
                              int(rnd.random() < 0.5)] + vec(oa) + vec(oc) + vec(q) + vec(vel) + vec(prev) +
                             [fb(f32(e0)), fb(f32(e1)), fb(f32(rv)), has48, fb(f32(burn)), int(rnd.random() < 0.7)])
        x = P['x']
        if mode == 1:
            ang = rnd.uniform(-math.pi, math.pi)
            dv = [math.cos(ang), 0.0, math.sin(ang)]
            if rnd.random() < 0.1:
                dv = unit([dv[0], rnd.uniform(-0.3, 0.3), dv[2]])
            near = rnd.uniform(-1, 3)
            c[x:x + 11] = [int(rnd.random() < 0.7)] + vec(dv) + vec([near, near + rnd.uniform(0, 4)]) + \
                [int(rnd.random() < 0.6), fb(f32(rnd.uniform(1e-6, 1.0))), fb(f32(rnd.uniform(0, 3))), 0, 0]
        elif mode == 2:
            j = rnd.randrange(nobj) if rnd.random() < 0.7 else 0xFFFFFFFF
            ang = rnd.uniform(-math.pi, math.pi)
            ln = rnd.uniform(0, 3)
            disp = [math.cos(ang) * ln, rnd.uniform(-0.1, 0.1), math.sin(ang) * ln]
            v8 = [rnd.uniform(-10, 10), rnd.uniform(-1, 1), rnd.uniform(-10, 10)]
            c[x:x + 11] = [j] + vec(disp) + vec(v8) + [fb(f32(rnd.choice((0.25, rnd.uniform(0, 1))))),
                                                       fb(f32(rnd.uniform(0, 4))), rnd.getrandbits(1), rnd.getrandbits(1)]
        cases.append(c)
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    O = lambda j, k: 'c[%d+%d*(%s)+%d]' % (P['obj'], OBJW, j, k)
    # object j: 0x8000 + 0x400 j (its matrix +0x200), C 0xA000 + 0x400 j, S 0xC000 + 0x100 j, S2 0xD000 + 0x400 j,
    # its physics instance 0xF000 + 0x200 j; hit object 0x12000; avoid entries 0x13000, clips 0x13400; manager 0x14000 (+0x64228
    # the avoid list 0x13F00: +8 the entries, +12 the count)
    setup = (
        'memset(objects,0,sizeof(objects));LOG.clear();KC=c;NSW=NGR=NLN=0;memset(MC,0,sizeof(MC));'
        'auto v3=[&](uint32_t a,int k,uint32_t w){store(a,c[k]);store(a+4,c[k+1]);store(a+8,c[k+2]);store(a+12,w);};'
        # the walker: I 0x2000 (+112 its instance, matrix at +128: place +176), type 0x3400, matrix 0x3500
        'store(0x2000+448,0x3400);objects[0x3400-0x1000+109]=c[%d]?1:2;' % P['female'] +
        'store(0x2000+492,c[%d]);v3(0x2000+4768,%d,0);v3(0x2000+176,%d,0x3f800000);' % (P['r'], P['u'], P['p']) +
        'v3(0x3500,%d,0);v3(0x3510,%d,0);v3(0x3520,%d,0);v3(0x3530,%d,0x3f800000);' % (P['rows'], P['rows'] + 3, P['rows'] + 6, P['p']) +
        # M 0x1000
        'store(0x1000+224,0x2000);store(0x1000+260,c[%d]);v3(0x1000+128,%d,0);' % (P['flags'], P['mdir']) +
        'store(0x1000+248,c[%d]==0xFFFFFFFFu?0:0x13000+20*c[%d]);' % (P['clip'], P['clip']) +
        'store(0x1000+240,c[%d]);store(0x1000+244,c[%d]);store(0x1000+252,c[%d]);store(0x1000+256,c[%d]);store(0x1000+236,c[%d]);'
        % (P['time'], P['len'], P['yaw'], P['stretch'], P['t']) +
        'v3(0x1000+96,%d,0);v3(0x1000+160,%d,0);v3(0x1000+176,%d,0);' % (P['stop'], P['from'], P['to']) +
        'put(0x82860880,c[%d]);put(0x82860884,c[%d]);put(0x82860890,c[%d]);put(0x82860894,c[%d]);'
        % (P['mwc'], P['mwc'] + 1, P['mwc'] + 2, P['mwc'] + 3) +
        # the avoid list
        'store(0x14000+0x64228,0x13F00);store(0x13F00+8,0x13000);store16(0x13F00+12,c[%d]);' % P['nent'] +
        'for(int i=0;i<%d;i++){const uint32_t*e=c+%d+%d*i;store(0x13000+20*i+4,e[0]);store(0x13000+20*i+8,e[1]);'
        'store(0x13000+20*i+12,e[2]);store(0x13000+20*i+16,e[3]?0x13400+32*i:0);store(0x13400+32*i+12,e[4]);}'
        % (NENT, P['ent'], ENTW) +
        # the objects and the grid's heads
        'for(int i=0;i<9;i++)HEADS[i]=0;int n=c[%d];' % P['nobj'] +
        'for(int j=n-1;j>=0;j--){uint32_t o=0x8000+0x400*j;int cl=%s;store(o+12,HEADS[cl]);HEADS[cl]=o;' % O('j', 0) +
        'store(o,0x4000);objects[o-0x1000+6]=%s;store(o+16,0xA000+0x400*j);store(0xA000+0x400*j+452,%s);' % (O('j', 1), O('j', 3)) +
        'v3(o+0x200,%d+%d*j+5,0);store(o+0x210,0);store(o+0x214,0x3f800000);store(o+0x218,0);store(o+0x21C,0);'
        % (P['obj'], OBJW) +
        'v3(o+0x220,%d+%d*j+8,0);v3(o+0x230,%d+%d*j+11,0x3f800000);v3(o+432,%d+%d*j+14,0);v3(o+416,%d+%d*j+17,0);'
        % (P['obj'], OBJW, P['obj'], OBJW, P['obj'], OBJW, P['obj'], OBJW) +
        'store(o+472,%s);store(o+476,%s);store(o+480,%s);' % (O('j', 20), O('j', 21), O('j', 22)) +
        'if(%s){uint32_t S=0xC000+0x100*j,S2=0xD000+0x400*j,IN=0xF000+0x200*j;store(o+48,S);store(S+8,S2);'
        'store(S+28,%s?0x18000+16*j:0);store(S2+128,IN);store(S2+580,%s);v3(IN+272,%d+%d*j+14,0);}}'
        % (O('j', 23), O('j', 25), O('j', 24), P['obj'], OBJW) +
        'store(0x16000,0x16100);'
    )
    stub = {
        A['F-58860']: 'ctx.r3.u32=0x3530;',
        A['F-88455']: 'ctx.r3.u32=KC[%d];' % P['cell'],
        A['F-99153']: 'for(int i=0;i<9;i++)store(ctx.r5.u32+4*i,HEADS[i]);',
        A['F-96472']: 'int j=(ctx.r3.u32-0xA000)/0x400;ctx.r3.u32=%s;' % O('j', 2).replace('c[', 'KC['),
        A['F-17857']: 'int j=(ctx.r3.u32-0xA000)/0x400;ctx.r3.u32=%s;' % O('j', 4).replace('c[', 'KC['),
        A['F-43397']: '',
        A['F-66549']: ('uint32_t q=ctx.r3.u32;if(load(q+4)!=1||load(q+8)!=0x2070||load(q+16)!=640||load(q+20)!=0x800)err(1);'
                       'logt(1);lw(load(q+32));lw(load(q+36));lw(load(q+40));lw(load(q+48));store(q+52,KC[%d]);' % P['in800']),
        A['F-84843']: 'logt(2);',
        A['F-10576']: '',
        A['F-57005']: ('uint32_t q=ctx.r3.u32;if(load(q+16)!=512||load(q+20)!=0x381E||load(q+8)!=0x2070||load(q+4)!=(load(q+12)?2u:1u))err(4);'
                       'logt(4);for(int k=0;k<3;k++)lw(load(q+32+4*k));for(int k=0;k<3;k++)lw(load(q+48+4*k));'
                       'store(q+96,KC[%d+NGR++]?0x12000:0);' % P['gr']),
        A['F-59579']: ('uint32_t x=ctx.r4.u32;logt(5);for(int k=0;k<4;k++)lw(load(x+4*k));const uint32_t*s=KC+%d+5*NLN++;'
                       'if(s[0]){for(int k=0;k<3;k++){float a,b;uint32_t u=load(x+4*k);memcpy(&a,&u,4);memcpy(&b,&s[1+k],4);'
                       'a+=b;memcpy(&u,&a,4);store(ctx.r6.u32+4*k,u);}store(ctx.r8.u32,s[4]);ctx.r3.u32=1;}else ctx.r3.u32=0;' % P['ln']),
        A['F-33765']: 'ctx.r3.u32=0x3500;',
        A['F-21043']: ('int i=(ctx.r3.u32-0x13400)/32;float t=float(ctx.f1.f64);uint32_t tb;memcpy(&tb,&t,4);logt(6);lw(i);lw(tb);'
                       'const uint32_t*e=KC+%d+%d*i;int k=MC[i]++?1:0;store(ctx.r7.u32+16,e[5+3*k]);store(ctx.r7.u32+20,e[6+3*k]);'
                       'store(ctx.r7.u32+24,e[7+3*k]);store(ctx.r7.u32+28,0);ctx.r3.u32=1;' % (P['ent'], ENTW)),
        A['F-39461']: 'if(ctx.r3.u32!=0x2000)err(7);logt(7);lw(ctx.r4.u32);',
        A['F-99671']: 'if(ctx.r3.u32!=0x17000||ctx.r4.u32||ctx.r5.u32)err(8);logt(8);',
        A['F-50795']: 'if(ctx.r3.u32!=0x17100)err(9);logt(9);',
    }
    sweep = ('static void sweep(Context&ctx){uint32_t q=ctx.r3.u32;'
             'if(load(q+16)!=640||load(q+20)!=0x381E||*pointer(q+24)!=7||load(q+8)!=0x2070'
             '||load(q+4)!=(load(q+12)?2u:1u))err(3);'
             'logt(3);for(int k=0;k<3;k++)lw(load(q+32+4*k));for(int k=0;k<3;k++)lw(load(q+48+4*k));lw(load(q+64));lw(load(q+4)==2);'
             'const uint32_t*s=KC+%d+5*NSW++;if(s[0]){store(q+112,0x12000);store(0x12004,0x12100);store(0x12110,s[4]);'
             'store(q+128,s[1]);store(q+132,s[2]);store(q+136,s[3]);}else store(q+112,0);}' % P['sw'])
    disp = (sweep + nl + 'static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){if(a==0x%s)sweep(ctx);' % A['F-27454'] +
            
            'else if(a==0x7F000004)ctx.r3.u32+=0x200;' +
            ''.join('else if(a==0x%s)sub_%s(ctx,base);' % (g, g) for g in getters) +
            'else throw std::runtime_error("unexpected indirect call");}')
    x = P['x']
    pr = 'auto PW=[](uint32_t w){printf(" %08x",w);};auto PV=[&](uint32_t a){PW(load(a));PW(load(a+4));PW(load(a+8));};'
    out_m = ('for(int k=0;k<5;k++)PV(0x1000+16*k);{uint32_t o=load(0x1000+80);PW(o?(o-0x8000)/0x400:0xFFFFFFFFu);}'
             'PV(0x1000+96);PV(0x1000+128);PV(0x1000+160);PV(0x1000+176);PW(load(0x1000+236));PW(load(0x1000+240));'
             'PW(load(0x1000+244));{uint32_t e=load(0x1000+248);PW(e?(e-0x13000)/20:0xFFFFFFFFu);}PW(load(0x1000+252));'
             'PW(load(0x1000+256));PW(load(0x1000+260));PW(get(0x82860880));PW(get(0x82860884));PW(get(0x82860890));'
             'PW(get(0x82860894));')
    body = ('static const uint32_t C[][%d]={' % W + nl + rows + nl + '};' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};'
            'auto get=[](uint32_t a){const uint8_t*p=image.data()+(a-0x82000000u);return uint32_t(p[0])<<24|p[1]<<16|p[2]<<8|p[3];};'
            'for(int k=0;k<4;k++)put(0x828609E0+4*k,0x7F800000);put(0x8286D8E4,0x14000);put(0x8286F1B4,0x15000);' + pr + nl +
            'for(auto&c:C){' + setup +
            'store(0x4000+4,0x7F000004);' +
            ''.join('store(0x4000+%d,0x%s);' % (off, g) for off, g in zip((8, 12, 28, 32), getters)) +
            'Context ctx{};ctx.r1.u32=0x1FF00;ctx.r13.u32=0x16000;ctx.r3.u32=0x1000;' +
            ('if(c[0]==0){ctx.r4.u32=0x17000;ctx.r5.u32=0x17100;float d;memcpy(&d,&c[%d],4);ctx.f1.f64=d;'
             'try{sub_%s(ctx,nullptr);}catch(std::exception&e){LOG+=" EXC";}' % (P['dt'], A['F-15352'])) +
            ('}else if(c[0]==1){ctx.r4.u32=c[%d]?0x18000:0;store(0x3700,c[%d]);store(0x3704,c[%d]);'
             'store(0x3708,c[%d]);store(0x370C,0);float d;memcpy(&d,&c[%d],4);ctx.f1.f64=d;memcpy(&d,&c[%d],4);ctx.f2.f64=d;'
             'ctx.r5.u32=0x3700;ctx.r8.u32=c[%d];ctx.r9.u32=0x3710;ctx.r10.u32=0x3714;store(0x3710,c[%d]);store(0x3714,c[%d]);'
             'store(0x1FF00+84,0x3718);objects[0x3718-0x1000]=0x55;sub_%s(ctx,nullptr);'
             % (x, x + 1, x + 2, x + 3, x + 4, x + 5, x + 6, x + 7, x + 8, A['F-94813'])) +
            ('}else{ctx.r4.u32=0x17000;ctx.r5.u32=0x17100;ctx.r6.u32=c[%d]==0xFFFFFFFFu?0:0x8000+0x400*c[%d];'
             'store(0x3700,c[%d]);store(0x3704,c[%d]);store(0x3708,c[%d]);store(0x370C,0);'
             'store(0x3720,c[%d]);store(0x3724,c[%d]);store(0x3728,c[%d]);store(0x372C,0);ctx.r7.u32=0x3700;ctx.r8.u32=0x3720;'
             'float d;memcpy(&d,&c[%d],4);ctx.f1.f64=d;memcpy(&d,&c[%d],4);ctx.f2.f64=d;objects[0x1FF00+87-0x1000]=c[%d];'
             'objects[0x1FF00+95-0x1000]=c[%d];sub_%s(ctx,nullptr);}'
             % (x, x, x + 1, x + 2, x + 3, x + 4, x + 5, x + 6, x + 7, x + 8, x + 9, x + 10, A['F-83234'])) +
            'if(c[0]==1){PW(ctx.r3.u32&0xFF);PV(0x3700);PW(load(0x3710));PW(load(0x3714));PW(objects[0x3718-0x1000]);}'
            'else{' + out_m + '}printf("%s' + NL + '",LOG.c_str());}' + nl)
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    stubs = {'sub_' + k: v for k, v in stub.items()}
    names = ['sub_' + A[f] for f in ('F-15352', 'F-94813', 'F-83234')] + ['sub_' + g for g in getters]
    exe = build(idx, names, main_src, a.out_dir, 'car_dodge', stubs=stubs, vector=True)
    res = [x for x in run(exe, a.image).split(nl) if x]
    assert len(res) == len(cases), (len(res), len(cases))
    bad = [i for i, o in enumerate(res) if 'ERR' in o or 'EXC' in o]
    assert not bad, (len(bad), res[bad[0]])
    fix = lambda o: ' '.join(word(int(t, 16)) if len(t) == 8 and not t.startswith('t') else t for t in o.split())
    lines = [' '.join(word(x) for x in c) + ' -> ' + fix(o) for c, o in zip(cases, res)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'car_dodge_ref.txt').write_text(nl.join(lines) + nl)
    n = lambda m, t: sum(c[0] == m and (' %s ' % t in o + ' ') for c, o in zip(cases, res))
    print(len(lines), 'cases:', sum(c[0] == 0 for c in cases), 'threat runs (stops', n(0, 't2'), 'searches', n(0, 't3'),
          'slides', n(0, 't8'), 'speech', n(0, 't7'), 'mover', n(0, 't6'), '),', sum(c[0] == 1 for c in cases),
          'searches alone (lane', n(1, 't5'), 'ground', n(1, 't4'), '),', sum(c[0] == 2 for c in cases), 'slide starts alone')


if __name__ == '__main__':
    main()
