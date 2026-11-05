"""Riding traffic's off-lane rails run offline: the builders of the original, whole and unchanged in vector mode, on flat
guest memory, with what they ask of the world (the lanes, the sweep, the pool) as stubs that record their arguments and
answer from the case. The arc-line-arc class itself (F-85722 and the rest) is in the closure and runs as it is.

Modes (--mode, default all):
- recovery: the rail builder F-26466 (recovery and rejoin rails). Stubs: the lane query F-56141 (the entries:
  lane, phase t0, the valid byte), the lane's slot 8 (point and tangent at a phase; the stub's answer is printed, the
  port reads it back), the car bound's slot 4 (the sweep radius), the sweep F-23371 (its answer from the case, printed
  with the rail it was asked about), the pool F-88792 (records the pool index and the speed). The car's own MWC
  (+384), the radius F-36195, the copy and the attach F-70405 run as they are.
- request: the two request builders in front of it: the recovery request (F-26749: start and heading from the
  follower's rail at its phase, flattened and made unit; else the car's place and -row 2) and the rejoin request F-89026
  (from the passing rail: its end T, the unit tangent at phase 1, 0.7 x its speed at least 4.4704 m/s); F-26466 is a stub
  that prints the request.
- backing: the backing-up rail F-54867 (the four control points from the car's place and row 2, the pool index 0,
  speed 4, the attach at phase 1 reversed).
- lanechange: the cruising ghost's route lane-change rail F-11487 (two lanes' points and unit derivatives, the four
  control points P0, P0 + W T0, P1 - W T1, P1, the pool index 0 at the target lane's slot 17 speed, the attach).
- lcinstall: its install F-32118 whole (F-11487 and the attach F-70405 inside it): the follower's old rail unloaded,
  its attached holders released, the new rail set at phase 0, the holder's words copied into the follower (+36 ..).
- join: a race path's join rail F-45519 (four rails, the shortest).
- pass: the passing rail F-92334 whole (with the lane test F-50817 and the arc-line-arc class). Stubs: the obstacle's
  slots 1 / 4 / 6 (matrix, width, length) and the own obstacle's slot 4 (width) answer from the case; the sweep
  F-23371, the lane query F-56141 (the 48-byte entries: +16 tangent, +32 lane -> +8 owner -> +4 flags / +8 parent,
  +36 d2), the pavement test F-50927 and the probe F-57005 answer from a per-case generator and print what they were
  asked and what they answered; the pool F-88792 and the attach F-70405 record their arguments.
Writes <out-dir>/traffic_rails_<mode>_ref.txt. Floats as hex bits."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build, run

recomp_closure.shim = lambda: recomp_leaf.shim(0x10000)
recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n')
_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <xmmintrin.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    s = s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                  'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)
    # (the SDK's flush mode: FTZ | DAZ in MXCSR while the code has it on)
    fp = ('struct { void disableFlushMode() {} void enableFlushMode() {} void disableFlushModeUnconditional() {}'
          ' void enableFlushModeUnconditional() {} } fpscr;')
    assert s.count(fp) == 1
    on, off = '{_mm_setcsr(_mm_getcsr()|0x8040);}', '{_mm_setcsr(_mm_getcsr()&~0x8040u);}'
    return s.replace(fp, 'struct { void disableFlushMode() %s void enableFlushMode() %s void disableFlushModeUnconditional() %s'
                     ' void enableFlushModeUnconditional() %s } fpscr;' % (off, on, off, on), 1)


recomp_closure.vector_shim = vector_shim
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)

NL = chr(92) + 'n'
nl = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


COMMON = r'''
static auto bf=[](uint32_t u){float f;memcpy(&f,&u,4);return f;};
static auto fbits=[](double d){float f=float(d);uint32_t u;memcpy(&u,&f,4);return u;};
static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}
static void vec(uint32_t a,const uint32_t*c){for(int k=0;k<3;k++)store(a+4*k,c[k]);store(a+12,0);}
static void pv(uint32_t a){printf(" %08x %08x %08x",load(a),load(a+4),load(a+8));}
static void rec(uint32_t r){for(int o:{0,16,48,64,96,112,128,144})pv(r+o);for(int o:{32,36,80})printf(" %08x",load(r+o));printf(" %02x",*pointer(r+160));}
static void consts(){for(int k=0;k<4;k++)put(0x828609E0+4*k,0x7F800000);for(int k=0;k<3;k++)put(0x82860950+4*k,0xFFFFFFFF);put(0x8286095C,0);}
'''
recomp_closure.EXTRA += 'static const uint32_t*cur;static uint32_t hs;' + COMMON


def recovery(a, idx, tool):
    """F-26466 whole. Guest memory: holder 0x1000, request 0x1100 (+0 car 0x1200, +4 route 0x1500 with +124 = 0, +16 P,
    +32 H0, +48 v, +52 the turn limit, +56..+64 sweep words, +68 the graph word 0x1580, +72 flags), car 0x1200 (+384 MWC,
    +448 type 0x1400: +160 fMinTurningRadius, +164 fPivotTurningRadius), the car bound object 0x1600 (vtable 0x1620,
    slot 4 = the radius), lanes 0x1700 + 16 k (vtable 0x1680, slot 8 = point and tangent), the pool's rail 0x1A00."""
    build_ = addr(tool, 'F-26466')
    query, bound, sweep, pool = addr(tool, 'F-56141'), addr(tool, 'F-88657'), addr(tool, 'F-23371'), addr(tool, 'F-88792')
    rnd = random.Random(26466)
    cases = []
    for _ in range(1500):
        yaw = rnd.uniform(-math.pi, math.pi)
        h0 = [math.sin(yaw), 0.0, -math.cos(yaw)]
        u = rnd.random()
        if u < 0.1:
            h0 = [x * rnd.uniform(0.2, 3) for x in h0]
        elif u < 0.2:
            h0[1] = rnd.uniform(-0.3, 0.3)
        p = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 60), rnd.uniform(-3000, 3000)]
        v = rnd.choice((0.0, rnd.uniform(0, 4), rnd.uniform(4, 30), 4.4704, 4.0))
        ready = int(rnd.random() < 0.3)
        lim = rnd.choice((2.44346, 2.44346, 2.44346, rnd.uniform(0.5, 6.3)))
        mint, piv = rnd.uniform(3, 9), rnd.uniform(2, 6)
        rad = rnd.uniform(0.7, 1.6)
        n = rnd.choice((0, 1, 2, 3, 5, 8, 12, rnd.randint(1, 48)))
        lanes = []
        for _k in range(n):
            ang = rnd.uniform(-math.pi, math.pi)
            dist = rnd.uniform(0, 25)
            b = [p[0] + dist * math.cos(ang), p[1] + rnd.choice((0.0, rnd.uniform(-6, 6))), p[2] + dist * math.sin(ang)]
            ty = rnd.uniform(-math.pi, math.pi)
            s = rnd.choice((1.0, rnd.uniform(5, 300)))
            d = [math.sin(ty) * s, rnd.choice((0.0, rnd.uniform(-0.1, 0.1))) * s, -math.cos(ty) * s]
            if rnd.random() < 0.03:
                d = [0.0, 0.0, 0.0]
            lanes.append([f32(x) for x in b + d] + [f32(rnd.choice((0.0, rnd.random(), rnd.random(), 0.999))), int(rnd.random() > 0.05)])
        hitp = rnd.choice((0.0, 0.2, 0.5, 0.9))
        seed = [rnd.getrandbits(32), rnd.getrandbits(32)]
        cases.append(dict(p=[f32(x) for x in p], h0=[f32(x) for x in h0], v=f32(v), ready=ready, lim=f32(lim), mint=f32(mint),
                          piv=f32(piv), rad=f32(rad), lanes=lanes, hitp=hitp, seed=seed, hseed=rnd.getrandbits(31)))
    rows = []
    for c in cases:
        w = [fb(x) for x in c['p'] + c['h0'] + [c['v'], c['lim'], c['mint'], c['piv'], c['rad']]]
        w += [c['ready'], c['seed'][0], c['seed'][1], int(c['hitp'] * 1000), c['hseed'], len(c['lanes'])]
        for l in c['lanes']:
            w += [fb(x) for x in l[:7]] + [l[7]]
        rows.append(w)
    data = ','.join('0x%08x' % x for r in rows for x in r)
    disp = (r'''static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){
 if(a==0x82FF0100){ctx.f1.f64=bf(cur[10]);return;}
 if(a==0x82FF0200){uint32_t lane=(ctx.r4.u32-0x1700)/16;const uint32_t*L=cur+17+8*lane;float ph=float(ctx.f1.f64);
  float px=bf(L[0])+bf(L[3])*(ph*20.0f),py=bf(L[1])+bf(L[4])*ph,pz=bf(L[2])+bf(L[5])*(ph*20.0f);
  float tx=bf(L[3])+ph,ty=bf(L[4]),tz=bf(L[5])-ph*0.5f;
  putf(ctx.r3.u32,px);putf(ctx.r3.u32+4,py);putf(ctx.r3.u32+8,pz);store(ctx.r3.u32+12,0);
  putf(ctx.r6.u32,tx);putf(ctx.r6.u32+4,ty);putf(ctx.r6.u32+8,tz);store(ctx.r6.u32+12,0);
  printf(" T %d %08x %08x %08x %08x %08x %08x %08x",lane,fbits(ph),load(ctx.r3.u32),load(ctx.r3.u32+4),load(ctx.r3.u32+8),load(ctx.r6.u32),load(ctx.r6.u32+4),load(ctx.r6.u32+8));return;}
 if(a==0x82FF0300){printf(" P %08x",fbits(ctx.f1.f64));return;}
 throw std::runtime_error("unexpected indirect call");}
''')
    stubs = {
        'sub_' + query: ('int n=int(cur[16]);for(int k=0;k<n;k++){uint32_t e=ctx.r5.u32+48*k;store(e+32,0x1700+16*k);'
                         'store(e+40,cur[17+8*k+6]);*pointer(e+44)=uint8_t(cur[17+8*k+7]);}'
                         'printf(" Q %08x %08x %08x %08x %d",load(ctx.r4.u32),load(ctx.r4.u32+4),load(ctx.r4.u32+8),load(ctx.r4.u32+12),int(ctx.r6.u32));ctx.r3.u64=n;'),
        'sub_' + bound: 'ctx.r3.u64=0x1600;',
        'sub_' + sweep: ('hs=hs*1103515245u+12345u;int hit=((hs>>8)%1000)<cur[14];'
                         'printf(" S %08x %08x %08x %02x %d",load(ctx.r3.u32+32),load(ctx.r3.u32+36),load(ctx.r3.u32+80),*pointer(ctx.r3.u32+160),hit);'
                         'printf(" %08x %08x %08x %08x %08x",load(ctx.r4.u32),load(ctx.r4.u32+4),load(ctx.r4.u32+8),load(ctx.r4.u32+12),load(ctx.r4.u32+16));ctx.r3.u64=hit;'),
        'sub_' + pool: ('printf(" A %d %08x %08x",int(ctx.r4.u32),fbits(ctx.f1.f64),ctx.r3.u32);memset(objects+0xA00,0,0x200);store(0x1A00,0x1B80);'
                        'ctx.r3.u64=0x1A00;'),
        'sub_' + addr(tool, 'F-74061'): 'throw std::runtime_error("route");', 'sub_' + addr(tool, 'F-50269'): 'throw std::runtime_error("route");',
    }
    body = ('static const uint32_t D[]={' + data + '};' + nl + 'consts();' +
            'for(size_t i=0;i<sizeof(D)/4;){const uint32_t*c=D+i;cur=c;i+=17+8*c[16];memset(objects,0,0xE000);hs=c[15];' +
            'uint32_t req=0x1100,car=0x1200;store(req,car);store(req+4,0x1500);vec(req+16,c);vec(req+32,c+3);store(req+48,c[6]);'
            'store(req+52,c[7]);store(req+56,0x11);store(req+60,0x22);store(req+64,0x33);store(req+68,0x1580);*pointer(req+72)=c[11]?0x80:0;'
            'store(car+384,c[12]);store(car+388,c[13]);store(car+448,0x1400);store(0x1400+160,c[8]);store(0x1400+164,c[9]);'
            'store(0x1600,0x1620);store(0x1620+16,0x82FF0100);store(0x1680+32,0x82FF0200);for(int k=0;k<48;k++)store(0x1700+16*k,0x1680);'
            'store(0x1B80+92,0x82FF0300);'
            'Context ctx{};ctx.r1.u32=0xF000;ctx.r3.u32=0x1000;ctx.r4.u32=req;sub_' + build_ + '(ctx,nullptr);'
            'printf(" R %d",int(ctx.r3.u32));if(ctx.r3.u32){printf(" %08x %d %08x %08x %02x",load(0x1004),int(load(0x100C)-0x1700)/16,load(0x1014),load(0x1018),*pointer(0x101C));rec(0x1A80);}'
            'printf(" M %u %u' + NL + '",load(car+384),load(car+388));}' + nl)
    main_src = disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(idx, ['sub_' + build_], main_src, a.out_dir, 'rails_recovery', stubs=stubs, vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    # (the car's MWC seeds in decimal)
    lines = [' '.join(('%d' % x) if k in (12, 13) else '%08x' % x for k, x in enumerate(r)) + ' ->' + (' ' + o if o else '') for r, o in zip(rows, out)]
    return lines


def join(a, idx, tool):
    """F-45519 whole: the path object 0x1000 (+16 the file's vertices 0x2000, +20 their count, +32 the snapped points
    0x3000, +48 the lane's direction, +112 the spawn matrix's row 2, +144 the type 0x1400 with +160 fMinTurningRadius,
    +180 the rail made). The allocator F-15488 hands out 0x4000 (zeroed); the rail's constructor F-36658 is a stub (its
    vtable does not matter here); the record copy F-28377 and the four arc-line-arc builds run as they are."""
    build_, alloc, ctor = addr(tool, 'F-45519'), addr(tool, 'F-15488'), addr(tool, 'F-36658')
    rnd = random.Random(45519)
    rows = []
    for _ in range(2000):
        n = rnd.choice((2, 2, 3, 4, 5, 8))
        o = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 60), rnd.uniform(-3000, 3000)]
        verts, snap = [], []
        q = list(o)
        for _k in range(n):
            step = rnd.choice((0.0, rnd.uniform(0.5, 40), rnd.uniform(0.5, 40), rnd.uniform(0.5, 40)))
            ang = rnd.uniform(-math.pi, math.pi)
            q = [q[0] + step * math.cos(ang), q[1] + rnd.choice((0.0, rnd.uniform(-2, 2))), q[2] + step * math.sin(ang)]
            verts.append(list(q))
            sp = list(q) if rnd.random() < 0.3 else [q[0] + rnd.uniform(-4, 4), q[1] + rnd.uniform(-1, 1), q[2] + rnd.uniform(-4, 4)]
            snap.append(sp)
        if rnd.random() < 0.05:
            snap[-1] = list(snap[-2])
        ty = rnd.uniform(-math.pi, math.pi)
        s = rnd.choice((1.0, 1.0, rnd.uniform(0.2, 3)))
        ld = [math.sin(ty) * s, rnd.choice((0.0, rnd.uniform(-0.1, 0.1))), -math.cos(ty) * s]
        ry = rnd.uniform(-math.pi, math.pi)
        r2 = [math.sin(ry), rnd.choice((0.0, rnd.uniform(-0.2, 0.2))), math.cos(ry)]
        rr = rnd.choice((rnd.uniform(3, 9), rnd.uniform(0.5, 30)))
        w = [n, fb(f32(rr))] + [fb(f32(x)) for x in ld + r2]
        for v in verts + [[0.0] * 3] * (8 - n) + snap + [[0.0] * 3] * (8 - n):
            w += [fb(f32(x)) for x in v]
        rows.append(w)
    data = ','.join('0x%08x' % x for r in rows for x in r)
    disp = 'static void dispatch(Context&,const uint8_t*,uint32_t){throw std::runtime_error("unexpected indirect call");}' + nl
    stubs = {'sub_' + alloc: 'memset(pointer(0x4000),0,0x200);ctx.r3.u64=0x4000;', 'sub_' + ctor: ''}
    body = ('static const uint32_t D[]={' + data + '};' + nl + 'consts();' +
            'for(size_t i=0;i<sizeof(D)/4;i+=56){const uint32_t*c=D+i;memset(objects,0,0xE000);uint32_t o=0x1000;'
            'store(o+16,0x2000);store16(o+20,uint16_t(c[0]));store(o+32,0x3000);vec(o+48,c+2);vec(o+112,c+5);store(o+144,0x1400);'
            'store(0x1400+160,c[1]);for(int k=0;k<8;k++){vec(0x2000+16*k,c+8+3*k);vec(0x3000+16*k,c+32+3*k);}'
            'Context ctx{};ctx.r1.u32=0xF000;ctx.r3.u32=o;sub_' + build_ + '(ctx,nullptr);'
            'printf("R %d",int(ctx.r3.u32));if(ctx.r3.u32){if(load(o+180)!=0x4000)throw std::runtime_error("rail");rec(0x4000+128);}'
            'printf("' + NL + '");}' + nl)
    main_src = disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(idx, ['sub_' + build_], main_src, a.out_dir, 'rails_join', stubs=stubs, vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(rows), (len(out), len(rows))
    return [' '.join('%08x' % x for x in r) + ' -> ' + o for r, o in zip(rows, out)]


def backing(a, idx, tool):
    """F-54867 whole: holder 0x1000, the car 0x1200 (its matrix through F-33765 at +128: row 2 at +160, the place at
    +176; +500 bit 0x20 clear; the thread block r13 0x5000 with +72 = 0), the graph word 0x1580. The pool F-88792 records
    the pool index and the speed and hands out the rail 0x1A00 (vtable 0x1B80, slot 16 -> the control point array
    0x1C00); the attach F-70405 records its arguments. The four control points are read back from the array."""
    build_, pool, attach = addr(tool, 'F-54867'), addr(tool, 'F-88792'), addr(tool, 'F-70405')
    rnd = random.Random(54867)
    rows = []
    for _ in range(2000):
        p = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 60), rnd.uniform(-3000, 3000)]
        yaw = rnd.uniform(-math.pi, math.pi)
        r2 = [math.sin(yaw), rnd.choice((0.0, rnd.uniform(-0.3, 0.3))), math.cos(yaw)]
        if rnd.random() < 0.1:
            r2 = [x * rnd.uniform(0.2, 3) for x in r2]
        rows.append([fb(f32(x)) for x in p + r2])
    data = ','.join('0x%08x' % x for r in rows for x in r)
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){'
            'if(a==0x82FF0400){store(0x1D00+4,0x1C00);ctx.r3.u64=0x1D00;return;}'
            'throw std::runtime_error("unexpected indirect call");}' + nl)
    stubs = {
        'sub_' + pool: ('printf(" A %d %08x",int(ctx.r4.u32),fbits(ctx.f1.f64));memset(pointer(0x1A00),0,0x400);store(0x1A00,0x1B80);'
                        'store(0x1B80+64,0x82FF0400);ctx.r3.u64=0x1A00;'),
        'sub_' + attach: ('printf(" X %d %d %d %d %d %08x %08x",ctx.r3.u32==0x1000,ctx.r4.u32==0x1A00,int(ctx.r6.u32),int(ctx.r8.u32),'
                          'int(ctx.r9.u32),fbits(ctx.f1.f64),fbits(ctx.f2.f64));'),
    }
    body = ('static const uint32_t D[]={' + data + '};' + nl + 'consts();' +
            'for(size_t i=0;i<sizeof(D)/4;i+=6){const uint32_t*c=D+i;memset(objects,0,0xE000);uint32_t car=0x1200;'
            'vec(car+176,c);store(car+188,0x3F800000);vec(car+160,c+3);store(0x5000,0x5100);'
            'Context ctx{};ctx.r1.u32=0xF000;ctx.r13.u32=0x5000;ctx.r3.u32=0x1000;ctx.r4.u32=car;ctx.r5.u32=0x1580;'
            'sub_' + build_ + '(ctx,nullptr);printf(" R %d",int(ctx.r3.u32));for(int k=0;k<4;k++)pv(0x1C00+16*k);'
            'printf("' + NL + '");}' + nl)
    main_src = disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(idx, ['sub_' + build_], main_src, a.out_dir, 'rails_backing', stubs=stubs, vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(rows), (len(out), len(rows))
    return [' '.join('%08x' % x for x in r) + ' -> ' + o for r, o in zip(rows, out)]


def lanechange(a, idx, tool):
    """The cruising ghost's route lane-change rail F-11487 whole (called by F-32118 from F-24481): holder 0x1000, the
    car 0x1200 (+496 its obstacle 0x1300, vtable 0x1380: slot 12 records lane and f1), the current lane 0x1700 and
    the target lane 0x1740 (vtable 0x1680: slot 15 -> their curves 0x1780 / 0x17C0, slot 17 -> the target's speed from the
    case), the curve's point and derivative F-71868 (`eval_pair`, proven) a stub answering the case's for (curve,
    phase); the road of the lane (F-86691) and the cast (F-75805) stubs that
    record their argument; the pool F-88792 and the attach F-70405 record their arguments. Stack: the pool pointer at
    caller sp+92, the byte at caller sp+87. Prints the call log, the four control points and the holder's +32 .. +52."""
    nm = {k: addr(tool, k) for k in ('F-11487', 'F-88792', 'F-70405', 'F-86691', 'F-75805', 'F-71868')}
    ev = nm['F-71868']
    rnd = random.Random(11487)
    rows = []
    for _ in range(3000):
        p0 = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 60), rnd.uniform(-3000, 3000)]
        dist, ang = rnd.uniform(0, 80), rnd.uniform(-math.pi, math.pi)
        p1 = [p0[0] + dist * math.cos(ang), p0[1] + rnd.choice((0.0, rnd.uniform(-3, 3))), p0[2] + dist * math.sin(ang)]

        def tan():
            y = rnd.uniform(-math.pi, math.pi)
            s = rnd.choice((1.0, rnd.uniform(5, 300), rnd.uniform(0.001, 1)))
            d = [math.sin(y) * s, rnd.choice((0.0, rnd.uniform(-0.1, 0.1))) * s, -math.cos(y) * s]
            if rnd.random() < 0.02:
                d = [0.0, 0.0, 0.0]
            return d
        d0, d1 = tan(), tan()
        t = rnd.choice((0.0, 1.0, rnd.random(), rnd.random()))
        ph = rnd.choice((0.0, 1.0, rnd.random(), rnd.random()))
        w = rnd.choice((4.0, 10.0, rnd.uniform(4, 10), rnd.uniform(4, 10)))
        sp = rnd.choice((rnd.uniform(4, 30), 13.4112, 0.0))
        byte = rnd.choice((0, 0, 0, 1))
        rows.append([fb(f32(x)) for x in p0 + d0 + p1 + d1 + [t, ph, w, sp]] + [byte])
    data = ','.join('0x%08x' % x for r in rows for x in r)
    disp = (r'''static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){
 if(a==0x82FF0700){printf(" S15 %d",ctx.r3.u32==0x1740);ctx.r3.u64=ctx.r3.u32+0x80;return;}
 if(a==0x82FF0701){printf(" S17 %d",ctx.r3.u32==0x1740);ctx.f1.f64=bf(cur[15]);return;}
 if(a==0x82FF0400){store(0x1D00+4,0x1C00);ctx.r3.u64=0x1D00;return;}
 if(a==0x82FF0800){printf(" V %d %08x",int(ctx.r4.u32),fbits(ctx.f1.f64));return;}
 throw std::runtime_error("unexpected indirect call");}
''')
    stubs = {
        'sub_' + ev: ('{uint32_t c=ctx.r4.u32;const uint32_t*q=c==0x1780?cur:cur+6;printf(" E %d %08x",c==0x1780,fbits(ctx.f1.f64));'
                      'vec(ctx.r3.u32,q);vec(ctx.r6.u32,q+3);}'),
        'sub_' + nm['F-86691']: 'printf(" O %d",ctx.r3.u32==0x1700);ctx.r3.u64=0x1900;',
        'sub_' + nm['F-75805']: 'printf(" C %d %d",ctx.r3.u32==0x1900,int(ctx.r4.u32));',
        'sub_' + nm['F-88792']: ('printf(" A %d %d %08x",ctx.r3.u32==0x6000,int(ctx.r4.u32),fbits(ctx.f1.f64));memset(pointer(0x1A00),0,0x400);'
                                 'store(0x1A00,0x1B80);store(0x1B80+64,0x82FF0400);ctx.r3.u64=0x1A00;'),
        'sub_' + nm['F-70405']: ('printf(" X %d %d %d %d %d %d %08x %08x",ctx.r3.u32==0x1000,ctx.r4.u32==0x1A00,int(ctx.r6.u32==0x1740),int(ctx.r8.u32),'
                                 'int(ctx.r9.u32==0x1900),int(ctx.r10.u32==0x6000),fbits(ctx.f1.f64),fbits(ctx.f2.f64));'),
    }
    body = ('static const uint32_t D[]={' + data + '};' + nl + 'consts();' +
            'for(size_t i=0;i<sizeof(D)/4;i+=17){cur=D+i;memset(objects,0,0xE000);uint32_t car=0x1200;'
            'store(car+496,0x1300);store(0x1300,0x1380);store(0x1380+48,0x82FF0800);'
            'store(0x1700,0x1680);store(0x1740,0x1680);store(0x1680+60,0x82FF0700);store(0x1680+68,0x82FF0701);'
            'store(0xF000+92,0x6000);*pointer(0xF000+87)=uint8_t(cur[16]);store(0x5000,0x5100);'
            'Context ctx{};ctx.r1.u32=0xF000;ctx.r13.u32=0x5000;ctx.r3.u32=0x1000;ctx.r4.u32=car;ctx.r5.u32=0x1700;ctx.r7.u32=0x1740;'
            'ctx.f1.f64=bf(cur[12]);ctx.f2.f64=bf(cur[13]);ctx.f3.f64=bf(cur[14]);ctx.f4.f64=bf(cur[14]);'
            'sub_' + nm['F-11487'] + '(ctx,nullptr);printf(" R %d",int(ctx.r3.u32&0xFF));for(int k=0;k<4;k++)pv(0x1C00+16*k);'
            'printf(" H %d %d %d %08x %08x %d",load(0x1000+32)==car,load(0x1000+36)==0x1700,load(0x1000+40)==0x1740,load(0x1000+44),load(0x1000+48),int(*pointer(0x1000+52)));'
            'printf("' + NL + '");}' + nl)
    main_src = disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(idx, ['sub_' + nm['F-11487']], main_src, a.out_dir, 'rails_lanechange', stubs=stubs, vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(rows), (len(out), len(rows))
    return [' '.join('%08x' % x for x in r) + ' -> ' + o for r, o in zip(rows, out)]


def lcinstall(a, idx, tool):
    """The route lane change's install F-32118 whole with F-11487 and the attach F-70405 inside it (their world stubbed as
    in `lanechange`): the follower 0x3000 (+168 the car 0x1200, +172 the pool 0x6000; its four rail holders +4 / +36 /
    +92 / +124 with vtable 0x3800, the attached bit 0x80 of +32 / +64 / +120 / +152 from the case; +156 .. +164 from the
    case). Stubs: the rail unload F-78813 and the set-rail F-48384 record their arguments, a holder's slot 1 (its
    release) records which holder. Prints the call log and the follower's words +36 .. +88, +156 .. +164 after."""
    nm = {k: addr(tool, k) for k in ('F-32118', 'F-88792', 'F-86691', 'F-75805', 'F-71868', 'F-78813', 'F-48384', 'F-65393')}
    rnd = random.Random(32118)
    rows = []
    for _ in range(1000):
        p0 = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 60), rnd.uniform(-3000, 3000)]
        p1 = [p0[0] + rnd.uniform(-60, 60), p0[1] + rnd.uniform(-2, 2), p0[2] + rnd.uniform(-60, 60)]
        d0 = [rnd.uniform(-30, 30), rnd.uniform(-1, 1), rnd.uniform(-30, 30)]
        d1 = [rnd.uniform(-30, 30), rnd.uniform(-1, 1), rnd.uniform(-30, 30)]
        t, ph = rnd.random(), rnd.random()
        ln, w, sp = rnd.uniform(15, 75), rnd.uniform(4, 10), rnd.uniform(4, 30)
        bits = rnd.getrandbits(4)
        old = [rnd.getrandbits(16), fb(f32(rnd.random())), rnd.getrandbits(16)]
        rows.append([fb(f32(x)) for x in p0 + d0 + p1 + d1 + [t, ph, w, sp, ln]] + [bits] + old)
    data = ','.join('0x%08x' % x for r in rows for x in r)
    disp = (r'''static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){
 if(a==0x82FF0700){ctx.r3.u64=ctx.r3.u32+0x80;return;}
 if(a==0x82FF0701){ctx.f1.f64=bf(cur[15]);return;}
 if(a==0x82FF0400){store(0x1D00+4,0x1C00);ctx.r3.u64=0x1D00;return;}
 if(a==0x82FF0800){printf(" V %d %08x",int(ctx.r4.u32),fbits(ctx.f1.f64));return;}
 if(a==0x82FF0900){printf(" D %d",int(ctx.r3.u32-0x3000));return;}
 throw std::runtime_error("unexpected indirect call");}
''')
    stubs = {
        'sub_' + nm['F-71868']: '{const uint32_t*q=ctx.r4.u32==0x1780?cur:cur+6;vec(ctx.r3.u32,q);vec(ctx.r6.u32,q+3);}',
        'sub_' + nm['F-86691']: 'ctx.r3.u64=0x1900;',
        'sub_' + nm['F-75805']: '',
        'sub_' + nm['F-88792']: ('printf(" A %d %08x",int(ctx.r4.u32),fbits(ctx.f1.f64));memset(pointer(0x1A00),0,0x400);'
                                 'store(0x1A00,0x1B80);store(0x1B80+64,0x82FF0400);ctx.r3.u64=0x1A00;'),
        'sub_' + nm['F-78813']: 'printf(" U %d",ctx.r3.u32==0x3000);',
        'sub_' + nm['F-65393']: 'printf(" G");',
        'sub_' + nm['F-48384']: 'printf(" B %d %d %08x",ctx.r3.u32==0x3000,ctx.r4.u32==0x1A00,fbits(ctx.f1.f64));',
    }
    body = ('static const uint32_t D[]={' + data + '};' + nl + 'consts();' +
            'for(size_t i=0;i<sizeof(D)/4;i+=21){cur=D+i;memset(objects,0,0xE000);uint32_t car=0x1200,f=0x3000;'
            'store(car+496,0x1300);store(0x1300,0x1380);store(0x1380+48,0x82FF0800);'
            'store(0x1700,0x1680);store(0x1740,0x1680);store(0x1680+60,0x82FF0700);store(0x1680+68,0x82FF0701);'
            'store(0x5000,0x5100);store(0x3800+4,0x82FF0900);'
            'store(f+168,car);store(f+172,0x6000);int ho[4]={4,36,92,124};for(int k=0;k<4;k++){store(f+ho[k],0x3800);'
            '*pointer(f+ho[k]+28)=((cur[17]>>k)&1)?0x80:0x05;}'
            'store(f+156,cur[18]);store(f+160,cur[18]+1);store(f+164,cur[19]);store(f+176,cur[20]);'
            'Context ctx{};ctx.r1.u32=0xF000;ctx.r13.u32=0x5000;ctx.r3.u32=f;ctx.r4.u32=0x1700;ctx.r5.u32=0;ctx.r6.u32=0x1740;ctx.r10.u32=0;'
            'ctx.f1.f64=bf(cur[12]);ctx.f2.f64=bf(cur[13]);ctx.f3.f64=bf(cur[16]);'
            'ctx.f4.f64=bf(cur[14]);'
            'sub_' + nm['F-32118'] + '(ctx,nullptr);printf(" R %d",int(ctx.r3.u32&0xFF));for(int k=0;k<4;k++)pv(0x1C00+16*k);printf(" F");'
            'for(int o=36;o<92;o+=4)printf(" %08x",load(f+o));printf(" %08x %08x %08x",load(f+156),load(f+160),load(f+164));'
            'printf("' + NL + '");}' + nl)
    main_src = disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(idx, ['sub_' + nm['F-32118']], main_src, a.out_dir, 'rails_lcinstall', stubs=stubs, vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(rows), (len(out), len(rows))
    return [' '.join('%08x' % x for x in r) + ' -> ' + o for r, o in zip(rows, out)]


def request(a, idx, tool):
    """The two request builders run whole, F-26466 a stub that prints the request (+16 P, +32 H0, +48 v, +52 the turn
    limit, +72 the flags). The passing / follower rail is a real arc-line-arc record built first by F-85722 at 0x2080 (the
    rail 0x2000, +304 its speed); its vtable's slots 8, 10, 13 and 17 are the class's own (F-53036, F-94625, F-56412,
    F-65027) reached through the dispatch. Kind 0: the rejoin request F-89026 (the passing object 0x1800, +8 the rail);
    kind 1: the recovery request F-26749 with the follower 0x1700 (car +452; +188 the rail, +208 its phase); kind 2: the
    same with no follower (the car's place F-58860 and 0 - row 2 F-50349: the matrix at car +128 through the thread block
    r13 0x5000, +72 = 0). The car's speed at +412, the offset argument a zero vector, the ready bit from its argument."""
    names = {k: addr(tool, k) for k in ('F-89026', 'F-26749', 'F-26466', 'F-85722', 'F-53036', 'F-94625', 'F-56412', 'F-65027')}
    rnd = random.Random(26749)
    rows = []
    for _ in range(3000):
        p = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 60), rnd.uniform(-3000, 3000)]
        y0, y1 = rnd.uniform(-math.pi, math.pi), rnd.uniform(-math.pi, math.pi)
        h0 = [math.sin(y0), 0.0, -math.cos(y0)]
        dist, ang = rnd.uniform(0, 60), rnd.uniform(-math.pi, math.pi)
        t = [p[0] + dist * math.cos(ang), p[1] + rnd.choice((0.0, rnd.uniform(-3, 3))), p[2] + dist * math.sin(ang)]
        h1 = [math.sin(y1), rnd.choice((0.0, rnd.uniform(-0.1, 0.1))), -math.cos(y1)]
        rr = rnd.uniform(2, 12)
        sp = rnd.choice((0.0, rnd.uniform(0, 6.4), rnd.uniform(6.4, 30), 4.4704 / 0.7))
        ph = rnd.choice((0.0, 1.0, rnd.random(), rnd.random(), rnd.uniform(-0.1, 1.1)))
        kind = rnd.choice((0, 1, 2))
        pos = [rnd.uniform(-3000, 3000), rnd.choice((-0.0, rnd.uniform(-20, 60))), rnd.uniform(-3000, 3000)]
        yr = rnd.uniform(-math.pi, math.pi)
        r2 = [math.sin(yr), rnd.choice((0.0, -0.0, rnd.uniform(-0.2, 0.2))), math.cos(yr)]
        v = rnd.choice((0.0, rnd.uniform(0, 30)))
        ready = int(rnd.random() < 0.3)
        w = [fb(f32(x)) for x in p + h0 + t + h1 + [rr]] + [rnd.getrandbits(1), rnd.getrandbits(1)]
        w += [fb(f32(sp)), fb(f32(ph)), kind] + [fb(f32(x)) for x in pos + r2 + [v]] + [ready]
        rows.append(w)
    data = ','.join('0x%08x' % x for r in rows for x in r)
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){'
            'if(a==0x82FF0508){sub_' + names['F-53036'] + '(ctx,base);return;}'
            'if(a==0x82FF050A){sub_' + names['F-94625'] + '(ctx,base);return;}'
            'if(a==0x82FF050D){sub_' + names['F-56412'] + '(ctx,base);return;}'
            'if(a==0x82FF0511){sub_' + names['F-65027'] + '(ctx,base);return;}'
            'throw std::runtime_error("unexpected indirect call");}' + nl)
    stubs = {'sub_' + names['F-26466']: ('uint32_t q=ctx.r4.u32;pv(q+16);pv(q+32);printf(" %08x %08x %02x",load(q+48),load(q+52),*pointer(q+72));')}
    body = ('static const uint32_t D[]={' + data + '};' + nl + 'consts();' +
            'for(size_t i=0;i<sizeof(D)/4;i+=26){const uint32_t*c=D+i;memset(objects,0,0xE000);'
            'vec(0x6000,c);vec(0x6010,c+3);vec(0x6020,c+6);vec(0x6030,c+9);'
            'Context b{};b.r1.u32=0xF000;b.r3.u32=0x2080;b.r4.u32=0x6000;b.r5.u32=0x6010;b.r6.u32=c[13];b.r7.u32=0x6020;'
            'b.r8.u32=0x6030;b.r9.u32=c[14];b.f1.f64=bf(c[12]);sub_' + names['F-85722'] + '(b,nullptr);'
            'store(0x2000,0x1B00);store(0x1B00+32,0x82FF0508);store(0x1B00+40,0x82FF050A);store(0x1B00+52,0x82FF050D);'
            'store(0x1B00+68,0x82FF0511);store(0x2000+304,c[15]);'
            'uint32_t car=0x1200;store(0x5000,0x5100);vec(car+176,c+18);store(car+188,0x3F800000);vec(car+160,c+21);store(car+412,c[24]);'
            'Context ctx{};ctx.r1.u32=0xF000;ctx.r13.u32=0x5000;ctx.r3.u32=0x1000;ctx.r4.u32=car;ctx.r5.u32=0x1500;ctx.r6.u32=0x1580;'
            'printf("K %d",int(c[17]));'
            'if(c[17]==0){store(0x1800+8,0x2000);ctx.r7.u32=0x1800;sub_' + names['F-89026'] + '(ctx,nullptr);}'
            'else{if(c[17]==1){store(car+452,0x1700);store(0x1700+188,0x2000);store(0x1700+208,c[16]);}'
            'ctx.r7.u32=0;ctx.r8.u32=c[25];ctx.r9.u32=0x1900;sub_' + names['F-26749'] + '(ctx,nullptr);}'
            'printf("' + NL + '");}' + nl)
    main_src = disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(idx, ['sub_' + names[k] for k in ('F-89026', 'F-26749', 'F-85722', 'F-53036', 'F-94625', 'F-56412', 'F-65027')],
                main_src, a.out_dir, 'rails_request', stubs=stubs, vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(rows), (len(out), len(rows))
    return [' '.join('%08x' % x for x in r) + ' -> ' + o for r, o in zip(rows, out)]


def passing(a, idx, tool):
    """F-92334 whole. Guest memory: holder 0x1000, request 0x1100 (+0 the offset vector 0, +16 car 0x3000, +20 the
    obstacle 0x1300, +24 the graph word 0x1580, +28 clearance, +32 the sweep's extra word, +36 the turn gate, +40 the mask
    word, +44 flags: 0x80 left, 0x40 right, 0x20 lane test, 0x10 ready). Car 0x3000: +412 v, +448 type 0x1400 (+160 / +164
    the radii), +452 the follower 0x1700 (+188 a real arc-line-arc rail 0x2000, built first by F-85722, its slot 8 the
    class's F-53036; +208 the phase) or 0 (the car's place F-58860 and 0 - row 2 F-50349: the matrix at car +128, thread
    block r13 0x5000), +496 its own obstacle 0x1600 (slot 4 = its width). The obstacle 0x1300: slot 1 the matrix 0x1340
    (row 0, row 2 +32, place +48), slots 4 / 6 its width / length."""
    nm = {k: addr(tool, k) for k in ('F-92334', 'F-85722', 'F-53036', 'F-23371', 'F-56141', 'F-50927', 'F-57005', 'F-88792',
                                     'F-70405', 'F-86691', 'F-75805')}
    rnd = random.Random(92334)
    PASS = struct.unpack('>f', bytes.fromhex('4011361e'))[0]
    rows = []
    for _ in range(4000):
        yaw = rnd.uniform(-math.pi, math.pi)
        h = [math.sin(yaw), 0.0, -math.cos(yaw)]
        rt = [math.cos(yaw), 0.0, math.sin(yaw)]
        p = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 60), rnd.uniform(-3000, 3000)]
        u = rnd.random()
        if u < 0.85:
            dist, side = rnd.uniform(2, 20), rnd.choice((0.0, rnd.uniform(-1, 1), rnd.uniform(-4, 4)))
            o = [p[k] + h[k] * dist + rt[k] * side for k in range(3)]
            o[1] = p[1] + rnd.choice((0.0, 0.0, rnd.uniform(-1.5, 1.5)))
            oy = yaw + rnd.choice((0.0, rnd.uniform(-0.3, 0.3), rnd.uniform(-math.pi, math.pi)))
        elif u < 0.95:
            o = [p[0] + rnd.uniform(-30, 30), p[1] + rnd.uniform(-5, 5), p[2] + rnd.uniform(-30, 30)]
            oy = rnd.uniform(-math.pi, math.pi)
        else:
            o = list(p) if rnd.random() < 0.5 else [p[0], p[1] + 1.0, p[2]]
            oy = rnd.uniform(-math.pi, math.pi)
        r0 = [math.cos(oy), 0.0, math.sin(oy)]
        r2 = [-math.sin(oy), 0.0, math.cos(oy)]
        if rnd.random() < 0.1:
            r0[1], r2[1] = rnd.uniform(-0.2, 0.2), rnd.uniform(-0.2, 0.2)
        w, ln = rnd.choice((rnd.uniform(1.5, 2.6), rnd.uniform(1.5, 2.6), 3.0, 0.0)), rnd.choice((rnd.uniform(3.5, 6), rnd.uniform(3.5, 6), 6.0, rnd.uniform(6, 16)))
        if rnd.random() < 0.08:
            # (a point obstacle: the four corners on the line, picked by rounding or by the start values)
            w, ln = rnd.choice(((0.0, 0.0), (1e-3, 1e-3), (0.0, 1e-4)))
        wown = rnd.choice((rnd.uniform(1.5, 2.6), rnd.uniform(1.5, 2.6), 3.0, rnd.uniform(0.5, 1.0)))
        c = rnd.choice((1.1, 1.5, 0.8, rnd.uniform(0, 3)))
        extra = rnd.choice((0.0, 0.0, 0.0, rnd.uniform(0, 1)))
        gate = rnd.choice((PASS, PASS, PASS, rnd.uniform(0.3, 6.3)))
        g = rnd.random()
        fl = 0xE0 if g < 0.75 else 0xC0 if g < 0.9 else rnd.choice((0x80, 0x40, 0xA0, 0x60, 0x20, 0x00))
        fl |= (rnd.random() < 0.3) << 4 | rnd.getrandbits(4)
        v = rnd.choice((0.0, rnd.uniform(0, 4), rnd.uniform(4, 30), 4.4704, 4.4704 / 0.7))
        mint, piv = rnd.uniform(3, 9), rnd.uniform(2, 6)
        kind = int(rnd.random() < 0.5)
        ry = yaw + rnd.choice((0.0, rnd.uniform(-0.2, 0.2)))
        cr2 = [-math.sin(ry), rnd.choice((0.0, rnd.uniform(-0.1, 0.1))), math.cos(ry)]
        rp = [p[k] - h[k] * rnd.uniform(0, 10) for k in range(3)]
        rtg = [p[k] + h[k] * rnd.uniform(0, 10) + rt[k] * rnd.uniform(-2, 2) for k in range(3)]
        rh1 = [math.sin(ry), 0.0, -math.cos(ry)]
        rr = rnd.uniform(2, 12)
        ph = rnd.choice((0.0, 1.0, rnd.random(), rnd.random()))
        probs = [rnd.choice((0, 100, 300, 600)), rnd.getrandbits(31), rnd.choice((0, 200, 500)), rnd.choice((1000, 800, 300, 0))]
        wd = [fb(f32(x)) for x in o + r0 + r2 + [w, ln, wown, c, extra, gate]] + [fl] + [fb(f32(x)) for x in [v, mint, piv]] + [kind]
        wd += [fb(f32(x)) for x in p + cr2 + rp + h + rtg + rh1 + [rr]] + [rnd.getrandbits(1), rnd.getrandbits(1), fb(f32(ph))]
        wd += probs
        assert len(wd) == 46
        rows.append(wd)
    data = ','.join('0x%08x' % x for r in rows for x in r)
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){'
            'if(a==0x82FF0100){ctx.f1.f64=bf(cur[11]);return;}'
            'if(a==0x82FF0201){ctx.r3.u64=0x1340;return;}'
            'if(a==0x82FF0204){ctx.f1.f64=bf(cur[9]);return;}'
            'if(a==0x82FF0206){ctx.f1.f64=bf(cur[10]);return;}'
            'if(a==0x82FF0508){sub_' + nm['F-53036'] + '(ctx,base);return;}'
            'throw std::runtime_error("unexpected indirect call");}' + nl)
    lcg = 'hs=hs*1103515245u+12345u;'
    stubs = {
        'sub_' + nm['F-23371']: (lcg + 'int hit=((hs>>8)%1000)<cur[42];uint32_t q=ctx.r3.u32,s=ctx.r4.u32;'
                                 'printf(" S %08x %08x %08x %02x %d",load(q+32),load(q+36),load(q+80),*pointer(q+160),hit);'
                                 'for(int k=0;k<9;k++)printf(" %08x",load(s+4*k));ctx.r3.u64=hit;'),
        'sub_' + nm['F-56141']: (lcg + 'static const int NS[]={0,0,1,1,2,3,4,5,6,9};int n=NS[(hs>>8)%10];uint32_t s=ctx.r4.u32;'
                                 'printf(" Q %08x %08x %08x %08x %d %d %d",load(s),load(s+4),load(s+8),load(s+12),int(ctx.r6.u32),ctx.r3.u32==0x1580,n);'
                                 'for(int k=0;k<n;k++){uint32_t e=ctx.r5.u32+48*k,l=0x2400+16*k,ow=0x2800+32*k,pa=0x2810+32*k;float t[3];'
                                 'for(int j=0;j<3;j++){' + lcg + 't[j]=((hs>>8)%5==0)?0.0f:float(int((hs>>12)%2001)-1000)/250.0f;}'
                                 + lcg + 'float d2=float((hs>>8)%40)*0.25f;' + lcg + 'int code=(hs>>8)%5;' + lcg + 'uint32_t f=(hs>>4)&0xFFFFFFFBu;'
                                 'putf(e+16,t[0]);putf(e+20,t[1]);putf(e+24,t[2]);store(e+32,l);putf(e+36,d2);'
                                 'store(l+8,code?ow:0);store(ow+4,code==2?(f|4):f);store(ow+8,code>=3?pa:0);store(pa+4,code==3?(f|4):f);'
                                 'printf(" %08x %08x %08x %08x %d",load(e+16),load(e+20),load(e+24),load(e+36),code);}ctx.r3.u64=n;'),
        'sub_' + nm['F-50927']: (lcg + 'int on=((hs>>8)%1000)<cur[44];printf(" N %08x %08x %08x %08x %08x %d",load(ctx.r3.u32),'
                                 'load(ctx.r3.u32+4),load(ctx.r3.u32+8),fbits(ctx.f1.f64),fbits(ctx.f2.f64),on);ctx.r3.u64=on;'),
        'sub_' + nm['F-57005']: (lcg + 'int g=((hs>>8)%1000)<cur[45];uint32_t q=ctx.r3.u32;printf(" G");pv(q+32);pv(q+48);'
                                 'printf(" %08x %02x %d %d",load(q+20),*pointer(q+24),int(ctx.r4.u32),g);store(q+96,g?0x10:0);'),
        'sub_' + nm['F-88792']: ('printf(" A %d %08x %d",int(ctx.r4.u32),fbits(ctx.f1.f64),ctx.r3.u32==0x1580);memset(objects+0xA00,0,0x200);'
                                 'store(0x1A00,0x1B80);ctx.r3.u64=0x1A00;'),
        'sub_' + nm['F-70405']: ('printf(" X %d %d %d %08x %08x %d %d",ctx.r3.u32==0x1000,ctx.r4.u32==0x1A00,ctx.r9.u32==0x2000,fbits(ctx.f1.f64),'
                                 'fbits(ctx.f2.f64),int(ctx.r6.u32),int(ctx.r8.u32));'),
        'sub_' + nm['F-86691']: '', 'sub_' + nm['F-75805']: '',
    }
    body = ('static const uint32_t D[]={' + data + '};' + nl + 'consts();' +
            'for(size_t i=0;i<sizeof(D)/4;i+=46){const uint32_t*c=D+i;cur=c;memset(objects,0,0xE000);hs=c[43];'
            'uint32_t car=0x3000,req=0x1100;'
            'if(c[19]){vec(0x6000,c+26);vec(0x6010,c+29);vec(0x6020,c+32);vec(0x6030,c+35);'
            'Context b{};b.r1.u32=0xF000;b.r3.u32=0x2080;b.r4.u32=0x6000;b.r5.u32=0x6010;b.r6.u32=c[39];b.r7.u32=0x6020;'
            'b.r8.u32=0x6030;b.r9.u32=c[40];b.f1.f64=bf(c[38]);sub_' + nm['F-85722'] + '(b,nullptr);'
            'store(0x2000,0x2300);store(0x2300+32,0x82FF0508);store(0x1700+188,0x2000);store(0x1700+208,c[41]);store(car+452,0x1700);}'
            'store(0x5000,0x5100);vec(car+176,c+20);store(car+188,0x3F800000);vec(car+160,c+23);store(car+412,c[16]);'
            'store(car+448,0x1400);store(0x1400+160,c[17]);store(0x1400+164,c[18]);store(car+496,0x1600);store(0x1600,0x1620);store(0x1620+16,0x82FF0100);'
            'store(0x1300,0x1320);store(0x1320+4,0x82FF0201);store(0x1320+16,0x82FF0204);store(0x1320+24,0x82FF0206);'
            'vec(0x1340,c+3);vec(0x1340+32,c+6);vec(0x1340+48,c);store(0x1340+60,0x3F800000);'
            'store(req+16,car);store(req+20,0x1300);store(req+24,0x1580);store(req+28,c[12]);store(req+32,c[13]);store(req+36,c[14]);'
            'store(req+40,0xFFFFFFFF);*pointer(req+44)=uint8_t(c[15]);'
            'Context ctx{};ctx.r1.u32=0xF000;ctx.r13.u32=0x5000;ctx.r3.u32=0x1000;ctx.r4.u32=req;sub_' + nm['F-92334'] + '(ctx,nullptr);'
            'printf(" R %d",int(ctx.r3.u32&0xFF));if(ctx.r3.u32&0xFF)rec(0x1A80);printf("' + NL + '");}' + nl)
    main_src = disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(idx, ['sub_' + nm[k] for k in ('F-92334', 'F-85722', 'F-53036')], main_src, a.out_dir, 'rails_pass', stubs=stubs, vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(rows), (len(out), len(rows))
    return [' '.join('%08x' % x for x in r) + ' -> ' + o for r, o in zip(rows, out)]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--mode', default='all')
    a = ap.parse_args()
    idx = index(a.recomp)
    modes = {'recovery': recovery, 'join': join, 'backing': backing, 'request': request, 'pass': passing, 'lanechange': lanechange,
             'lcinstall': lcinstall}
    for m, f in modes.items():
        if a.mode not in ('all', m):
            continue
        lines = f(a, idx, a.tool)
        # (no word that reads as an address of the image: such a case is left out)
        lines = [x for x in lines if not any(len(t) == 8 and t.startswith('82') for t in x.split())]
        a.out_dir.mkdir(parents=True, exist_ok=True)
        (a.out_dir / ('traffic_rails_%s_ref.txt' % m)).write_text(nl.join(lines) + nl)
        print(m, len(lines), 'cases,', sum(' R 1' in x or '> R 1' in x for x in lines), 'built')


if __name__ == '__main__':
    main()
