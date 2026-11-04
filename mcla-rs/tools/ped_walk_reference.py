"""The walkers' path rail, locomotion state, clip start and turn rate run offline - the generated code whole and unchanged
(vector mode), on flat guest memory. The flush mode is the SDK's (FTZ and DAZ on the host while the code has it on); the
start-up vector constant the image holds as zero is filled in as the running game has it (0x828609E0 = +inf, the
normalisations' guard).

Parts (one exe, one mode letter each):
- 'a': the next path rail F-61656 (with the nearest segment F-93741, the closest point on a segment F-17399, the rail to a
  segment's middle F-16360, the arc-line-arc builder F-85722 and its solver, the total turn F-65534, the length F-62306,
  the record copy F-28377) and F-16360 alone on a random segment and turn limit. Guest memory: the path 0x10000 (points +0,
  stride 16, w lanes 0; count +768), P 0x11000, H0 0x11010, the out records 0x12000 (F-61656) and 0x12200 (F-16360),
  zeroed first. Stack 0xFF00.
- 'b': the locomotion state F-33058 (with the clip getter F-71036 and the slide speed F-92375). The behaviour 0x20000
  (+28 the walker 0x21000, whose +456 -> 0x22000 -> +20 the anim set 0x23000 -> +64 0x24000: +28 the clip lengths A28
  0x24100, +36 the clip distances A36 0x24200; +188 the rail, +500 the stop timer, +532 the flags, +1336 the speed
  scale, +1360 the state, +520 the clip holder 0x25000 whose +16 is the clip; the slide +432 / +440 from, +448 / +456
  to, +508 its time), the out request 0x26000 (+152 the wanted speed, +156 its lag).
- 'c': the clip start F-52026, its motion-tree calls replaced by logging stubs (the anim set's clip lookup returns the
  case's clip or 0). The walker 0x30000 (+448 the type 0x31000, +168 its anim set 0x32000; +4784 previous clip, +4788
  current, +4792 parked, +4796 the tree, +4800 the observer, +4804 the one-off clip, +4880 / +4884 the rates, +4888 /
  +4892 the two frame filters).
- 'd': the turn rate's static initialiser F-59108; the word it stores.
- 'e' (a second exe): the walker's anim update F-29547 up to its clip choice, the clip start stubbed (see part e below).
Writes <out-dir>/ped_walk_ref.txt: lines 'a ...', 'b ...', 'c ...', 'e ...', 'd ...': the case's words, '->', the outputs (see the
printing below). Words as hex; a word whose top byte is 0x82 is written as two 16-bit halves hhhh_llll."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build

recomp_closure.shim = lambda: recomp_leaf.shim(0x80000)
recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t){throw std::runtime_error("indirect");}\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n'
                         'static const uint32_t *K;static std::string LOG;\n'
                         'static uint32_t fbits(double d){float f=float(d);uint32_t u;memcpy(&u,&f,4);return u;}\n'
                         'static void lg(const char*fmt,...){char b[64];va_list v;va_start(v,fmt);vsnprintf(b,64,fmt,v);va_end(v);LOG+=b;}\n')
_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <xmmintrin.h>' + chr(10) + '#include <cstdarg>' + chr(10) + '#include <string>' + chr(10) + _vshim()
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
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)


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


NPT = 48


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    nxt, mid, near, loco, start, turn = (addr(a.tool, f) for f in ('F-61656', 'F-16360', 'F-93741', 'F-33058', 'F-52026', 'F-59108'))
    stub_ids = dict(look='F-40227', anim='F-29648', loop='F-31149', rate='F-19443', filt='F-76466', tree='F-47118', fdtor='F-35821',
                    blend='F-13965', bdtor='F-23574', node='F-70228', obs='F-67055', msg='F-51940', hnd='F-83403', reg='F-27465',
                    adtor='F-46389')
    sa = {k: addr(a.tool, v) for k, v in stub_ids.items()}
    rnd = random.Random(16360)
    nl = chr(10)

    # ---- part a: the path rail
    def heading():
        u = rnd.random()
        y = rnd.uniform(-math.pi, math.pi)
        h = [math.sin(y), 0.0, -math.cos(y)]
        if u < 0.04:
            return [0.0, 0.0, 0.0]
        if u < 0.12:
            h[1] = rnd.uniform(-0.3, 0.3)
        elif u < 0.2:
            k = rnd.choice((0.5, 2.0, rnd.uniform(0.01, 5)))
            h = [x * k for x in h]
        return h

    acases = []
    for _ in range(3000):
        n = rnd.choice((0, 1, 2, 2, 3, 4, 6, 8, 12, 20, 32, 48))
        p = [rnd.uniform(-3000, 3000), rnd.uniform(-10, 40), rnd.uniform(-3000, 3000)]
        yaw = rnd.uniform(-math.pi, math.pi)
        pts = []
        for k in range(n):
            pts.append(list(p))
            s = rnd.choice((rnd.uniform(0.3, 8), rnd.uniform(0.3, 8), rnd.uniform(0.01, 0.5), 0.0))
            yaw += rnd.choice((0.0, rnd.uniform(-0.4, 0.4), rnd.uniform(-1.6, 1.6)))
            p = [p[0] + s * math.sin(yaw), p[1] + rnd.uniform(-0.2, 0.2), p[2] - s * math.cos(yaw)]
        pts = [[f32(x) for x in q] for q in pts]
        if n >= 2:
            k = rnd.randrange(n - 1)
            u = rnd.random()
            if u < 0.08:
                P = list(pts[k])
            elif u < 0.14:
                P = [f32((pts[k][j] + pts[k + 1][j]) * 0.5) for j in range(3)]
            else:
                P = [pts[k][j] + rnd.uniform(-3, 3) for j in range(3)]
                P[1] = pts[k][1] + rnd.uniform(-0.6, 0.6)
        else:
            P = [rnd.uniform(-3000, 3000), rnd.uniform(-10, 40), rnd.uniform(-3000, 3000)] if n == 0 else [x + rnd.uniform(-2, 2) for x in pts[0]]
        H0 = heading()
        R = rnd.choice((0.1, 0.4, 0.1, rnd.uniform(0.05, 3), rnd.uniform(0.5, 10)))
        i = rnd.randrange(n - 1) if n >= 2 else 0
        lim = rnd.choice((fb(1.2566371), fb(3.7699115), fb(rnd.uniform(0, 7)), fb(math.pi)))
        c = [n, i, fb(f32(R)), lim] + [fb(f32(x)) for x in P + H0]
        for k in range(NPT):
            c += [fb(x) for x in pts[k]] if k < n else [0, 0, 0]
        acases.append(c)
    AW = len(acases[0])

    # ---- part b: the locomotion state
    def anyf():
        return rnd.choice((0.0, -0.0, rnd.uniform(0.01, 5), rnd.uniform(-2, 0), 1e-30, float('nan'), rnd.uniform(0, 1e-5)))

    bcases = []
    for _ in range(3000):
        flags = rnd.choice((0, 1, 2, 3, 4, 5, 6, 7)) | (rnd.randrange(0, 6) << 3)
        stop = rnd.choice((0.0, -0.0, rnd.uniform(0, 5), rnd.uniform(-1, 0), 1e-40, float('nan')))
        rail = rnd.choice((0, 0x7000, 0x7000, 0x7000))
        scale = rnd.choice((1.0, rnd.uniform(0.85, 1.15), rnd.uniform(0.85, 1.15), 0.0))
        clip = rnd.choice((0, 0, 1, 2))  # 0 no holder, 1 holder with no clip, 2 holder with a clip
        a28 = [rnd.choice((0.0, rnd.uniform(0.5, 30), rnd.uniform(0.5, 2))) for _ in range(3)]
        a36 = [rnd.choice((0.0, rnd.uniform(0.3, 6), rnd.uniform(0.5, 3))) for _ in range(3)]
        if rnd.random() < 0.05:
            a28[1] = anyf()
            a36[2] = anyf()
        fr = [rnd.uniform(-3000, 3000), rnd.uniform(-10, 30), rnd.uniform(-3000, 3000)]
        d = rnd.choice((0.0, rnd.uniform(0.05, 0.8), rnd.uniform(0, 3)))
        ang = rnd.uniform(-math.pi, math.pi)
        to = [fr[0] + d * math.cos(ang), fr[1] + rnd.uniform(-1, 1), fr[2] + d * math.sin(ang)]
        tl = rnd.choice((0.25, 0.25, 0.0, 1e-7, 1e-6, rnd.uniform(0, 1), -0.1))
        c = [flags, rail, clip, fb(f32(stop)) if stop == stop else 0x7FC00000, fb(f32(scale))]
        c += [fb(f32(x)) if x == x else 0x7FC00000 for x in a28 + a36]
        c += [fb(f32(x)) for x in fr + to] + [fb(f32(tl))]
        bcases.append(c)
    BW = len(bcases[0])

    # ---- part c: the clip start
    ccases = []
    for _ in range(3000):
        cur = rnd.choice((-1, 0, 1, 2, 3, -1))
        prev = rnd.choice((-1, 0, 1, 2))
        parked = rnd.choice((-1, -1, -1, 0, 1, 2))
        oneoff = rnd.choice((0, 0, 0, 0x7700))
        r0 = rnd.choice((1.0, rnd.uniform(0.75, 1.25)))
        r1 = rnd.choice((1.0, rnd.uniform(0.75, 1.25)))
        idx = rnd.choice((0, 1, 2))
        rate = rnd.choice((1.0, rnd.uniform(0.75, 1.25), rnd.uniform(0.85, 1.15), 0.0))
        res = rnd.choice((1, 1, 1, 0))
        c = [cur & 0xFFFFFFFF, prev & 0xFFFFFFFF, parked & 0xFFFFFFFF, oneoff, fb(f32(r0)), fb(f32(r1)), idx, fb(f32(rate)), res,
             rnd.choice((0x7100, 0)), 0x7200, 0x7300, 0x7400]
        ccases.append(c)
    CW = len(ccases[0])

    def rows(cs):
        return nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cs)

    pr = 'auto P=[](uint32_t a){printf(" %08x",load(a));};'
    v3 = 'auto V=[](uint32_t a,const uint32_t*s){store(a,s[0]);store(a+4,s[1]);store(a+8,s[2]);store(a+12,0);};'
    rec = 'auto REC=[&](uint32_t r){for(int k=0;k<44;k++)P(r+4*k);}; '
    abody = ('static const uint32_t A[][%d]={' % AW + nl + rows(acases) + nl + '};' + nl +
             'for(auto&c:A){memset(objects+0xF000,0,0x10000);'
             'for(uint32_t k=0;k<c[0];k++)V(0x10000+16*k,c+10+3*k);store(0x10000+768,c[0]);V(0x11000,c+4);V(0x11010,c+7);'
             '{Context ctx{};ctx.r1.u32=0xFF00;ctx.r3.u32=0x10000;ctx.r4.u32=0x11000;sub_%s(ctx,nullptr);printf("%%d",int(ctx.r3.u32));}' % near +
             '{Context ctx{};ctx.r1.u32=0xFF00;ctx.r3.u32=0x12000;ctx.r4.u32=0x10000;ctx.r5.u32=0x11000;ctx.r6.u32=0x11010;'
             'float r;memcpy(&r,&c[2],4);ctx.f1.f64=r;sub_%s(ctx,nullptr);REC(0x12000);}' % nxt +
             'if(c[0]>=2){Context ctx{};ctx.r1.u32=0xFF00;ctx.r3.u32=0x12200;ctx.r4.u32=0x10000;ctx.r5.u32=0x11000;ctx.r6.u32=0x11010;'
             'ctx.r9.u32=c[1];float r,l;memcpy(&r,&c[2],4);memcpy(&l,&c[3],4);ctx.f1.f64=r;ctx.f2.f64=l;sub_%s(ctx,nullptr);printf(" |");REC(0x12200);}' % mid +
             'printf("\\n");}' + nl)
    bbody = ('static const uint32_t B[][%d]={' % BW + nl + rows(bcases) + nl + '};' + nl +
             'for(auto&c:B){memset(objects+0x1F000,0,0x8000);uint32_t M=0x20000;'
             'store(M+28,0x21000);store(0x21000+456,0x22000);store(0x22000+20,0x23000);store(0x23000+64,0x24000);'
             'store(0x24000+28,0x24100);store(0x24000+36,0x24200);'
             'for(int k=0;k<3;k++){store(0x24100+4*k,c[5+k]);store(0x24200+4*k,c[8+k]);}'
             'store(M+532,c[0]);store(M+188,c[1]);store(M+520,c[2]?0x25000:0);store(0x25000+16,c[2]==2?0x5100:0);'
             'store(M+500,c[3]);store(M+1336,c[4]);store(M+1360,0xEEEE);store(M+432,c[11]);store(M+436,c[12]);store(M+440,c[13]);'
             'store(M+448,c[14]);store(M+452,c[15]);store(M+456,c[16]);store(M+508,c[17]);store(0x26000+152,0xAAAA);store(0x26000+156,0xBBBB);'
             'Context ctx{};ctx.r1.u32=0xFF00;ctx.r3.u32=M;ctx.r5.u32=0x26000;sub_%s(ctx,nullptr);' % loco +
             'P(M+1360);P(0x26000+152);P(0x26000+156);printf("\\n");}' + nl)
    I = 0x30000
    cbody = ('static const uint32_t C[][%d]={' % CW + nl + rows(ccases) + nl + '};' + nl +
             'for(auto&c:C){memset(objects+0x2F000,0,0x8000);K=c;LOG.clear();'
             'store(%d,0x31000);store(0x31000+168,0x32000);store(%d,c[1]);store(%d,c[0]);store(%d,c[2]);store(%d,c[10]);store(%d,c[11]);'
             % (I + 448, I + 4784, I + 4788, I + 4792, I + 4796, I + 4800) +
             'store(%d,c[3]);store(%d,c[4]);store(%d,c[5]);store(%d,c[12]);store(%d,c[9]);'
             % (I + 4804, I + 4880, I + 4884, I + 4888, I + 4892) +
             'Context ctx{};ctx.r1.u32=0xFF00;ctx.r3.u32=%d;ctx.r4.u32=c[6];float r;memcpy(&r,&c[7],4);ctx.f1.f64=r;sub_%s(ctx,nullptr);'
             % (I, start) +
             'for(uint32_t o:{4784u,4788u,4792u,4804u,4880u,4884u})P(%d+o);printf(" %%s\\n",LOG.c_str());}' % I + nl)
    dbody = ('{Context ctx{};ctx.r1.u32=0xFF00;sub_%s(ctx,nullptr);printf("%%08x\\n",load(0x8294A554));}' % turn + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
                'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};'
                'for(int k=0;k<4;k++)put(0x828609E0+4*k,0x7F800000);' + pr + v3 + rec + nl +
                'char m=argv[2][0];' + nl +
                'if(m==0x61){' + abody + '}' + nl + 'if(m==0x62){' + bbody + '}' + nl + 'if(m==0x63){' + cbody + '}' + nl +
                'if(m==0x64){' + dbody + '}' + nl + 'return 0;}' + nl)
    s = sa
    stubs = {
        'sub_' + s['look']: 'lg(" G%d",int(ctx.r4.u32));ctx.r3.u32=K[8]?0x33000:0;',
        'sub_' + s['anim']: 'lg(" A%08x/%08x",fbits(ctx.f1.f64),fbits(ctx.f2.f64));',
        'sub_' + s['loop']: 'lg(" L%d%d",int(ctx.r4.u32),int(ctx.r5.u32));',
        'sub_' + s['rate']: 'lg(" R%08x",fbits(ctx.f1.f64));',
        'sub_' + s['filt']: 'lg(" F%x",ctx.r5.u32);',
        'sub_' + s['tree']: 'lg(" T%x/%d",ctx.r3.u32,int(ctx.r5.u32));',
        'sub_' + s['fdtor']: 'lg(" d");',
        'sub_' + s['blend']: 'lg(" B%08x/%08x",fbits(ctx.f1.f64),fbits(ctx.f2.f64));',
        'sub_' + s['bdtor']: 'lg(" e");',
        'sub_' + s['node']: 'lg(" o");ctx.r3.u32=0;',
        'sub_' + s['obs']: 'lg(" s%x",ctx.r3.u32);',
        'sub_' + s['msg']: 'lg(" m%x",ctx.r4.u32);',
        'sub_' + s['hnd']: 'lg(" h");',
        'sub_' + s['reg']: 'lg(" n%x",ctx.r4.u32);',
        'sub_' + s['adtor']: 'lg(" z");',
    }
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + x for x in (nxt, mid, near, loco, start, turn)], main_src, a.out_dir, 'ped_walk', stubs=stubs, vector=True)
    out = {}
    for m in 'abcd':
        r = subprocess.run([str(exe), str(a.image), m], capture_output=True, text=True, check=True).stdout
        out[m] = [x for x in r.split(nl) if x]

    # ---- part e: the walker's anim update F-29547 up to its clip choice (a second exe: the clip start is a logging stub
    # here). The walker 0x30000 (+452 the behaviour 0x20000 laid out as in part b, +456 -> the anim set as in part b,
    # +4788 the current clip, +4792 parked, +4804 the one-off clip, +4880 the rate); r13 -> 0x40000 -> 0x40100 (the
    # thread byte +72 = 0), the manager 0x41000 (+40 dt). Stubbed: the shared update F-50624, the one-off start F-20899
    # (logged), the clip start F-52026 (logged: index, rate), the rate setter F-66231 (logged), the parked retry F-40288
    # (logged), the bone LOD F-62015, the scheduler F-42913.
    upd, oneoff, setrate, retry, lod, sched, shared = (addr(a.tool, f) for f in ('F-29547', 'F-20899', 'F-66231', 'F-40288', 'F-62015', 'F-42913', 'F-50624'))
    ecases = []
    for _ in range(3000):
        b = bcases[rnd.randrange(len(bcases))]
        st = rnd.choice((0, 1, 2, 3, 1, 2, 4))
        cur = rnd.choice((-1, 0, 1, 2, 1, 2))
        rate = rnd.choice((1.0, rnd.uniform(0.75, 1.25), 0.75, 1.25, f32(bf(b[4])) if b[4] != 0x7FC00000 else 1.0))
        one = rnd.choice((0, 0, 0x5100, 0x5200))
        parked = rnd.choice((-1, -1, -1, 1))
        ecases.append([st, cur & 0xFFFFFFFF, fb(f32(rate)), one, parked & 0xFFFFFFFF] + b)
    EW = len(ecases[0])
    ebody = ('static const uint32_t E[][%d]={' % EW + nl + rows(ecases) + nl + '};' + nl +
             'for(auto&e:E){const uint32_t*c=e+5;memset(objects+0x1F000,0,0x30000);LOG.clear();uint32_t M=0x20000,I=0x30000;'
             'store(I+452,M);store(I+456,0x22000);store(0x22000+20,0x23000);store(0x23000+64,0x24000);'
             'store(0x24000+28,0x24100);store(0x24000+36,0x24200);'
             'for(int k=0;k<3;k++){store(0x24100+4*k,c[5+k]);store(0x24200+4*k,c[8+k]);}'
             'store(M+532,c[0]);store(M+188,c[1]);store(M+520,c[2]?0x25000:0);store(0x25000+16,c[2]==2?0x5100:0);'
             'store(M+500,c[3]);store(M+1336,c[4]);store(M+1360,e[0]);store(M+432,c[11]);store(M+436,c[12]);store(M+440,c[13]);'
             'store(M+448,c[14]);store(M+452,c[15]);store(M+456,c[16]);store(M+508,c[17]);'
             'store(I+4788,e[1]);store(I+4880,e[2]);store(I+4804,e[3]);store(I+4792,e[4]);store(0x40000,0x40100);'
             'store(0x41000+40,0x3d088889);'
             'Context ctx{};ctx.r1.u32=0xFF00;ctx.r13.u32=0x40000;ctx.r3.u32=I;ctx.r4.u32=0x41000;sub_%s(ctx,nullptr);' % upd +
             'printf("%s\\n",LOG.c_str());}' + nl)
    emain = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
             'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + pr + nl + ebody + 'return 0;}' + nl)
    estubs = {'sub_' + shared: '', 'sub_' + oneoff: 'lg(" O%x",ctx.r4.u32);', 'sub_' + start: 'lg(" S%d/%08x",int(ctx.r4.u32),fbits(ctx.f1.f64));',
              'sub_' + setrate: 'lg(" Q%08x",fbits(ctx.f1.f64));', 'sub_' + retry: 'lg(" K");', 'sub_' + lod: '', 'sub_' + sched: ''}
    exe2 = build(idx, ['sub_' + upd], emain, a.out_dir, 'ped_walk_e', stubs=estubs, vector=True)
    r = subprocess.run([str(exe2), str(a.image), 'e'], capture_output=True, text=True, check=True).stdout
    out['e'] = r.split(nl)[:len(ecases)]
    assert len(out['e']) == len(ecases)
    assert len(out['a']) == len(acases) and len(out['b']) == len(bcases) and len(out['c']) == len(ccases), {k: len(v) for k, v in out.items()}
    hexw = lambda t: word(int(t, 16)) if len(t) == 8 and all(ch in '0123456789abcdef' for ch in t) else t
    fix = lambda o: ' '.join(hexw(t) for t in o.split())
    lines = []
    for tag, cs, res in (('a', acases, out['a']), ('b', bcases, out['b']), ('c', ccases, out['c']), ('e', ecases, out['e'])):
        for c, o in zip(cs, res):
            # (a path case: only its n points)
            c = c[:10 + 3 * c[0]] if tag == 'a' else c
            lines.append(tag + ' ' + ' '.join(word(x) for x in c) + ' -> ' + fix(o))
    lines.append('d -> ' + out['d'][0])
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'ped_walk_ref.txt').write_text(nl.join(lines) + nl)
    valid = sum(int(o.split()[1 + 40], 16) >> 24 & 0x20 != 0 for o in out['a'])
    states = [int(o.split()[0], 16) for o in out['b']]
    kinds = [('B' if ' B' in o else 'D' if ' T' in o else 'P') for o in out['c']]
    print(len(acases), 'path rail cases,', valid, 'with a valid rail;', len(bcases), 'locomotion cases, states',
          {k: states.count(k) for k in sorted(set(states))}, ';', len(ccases), 'clip starts', {k: kinds.count(k) for k in 'PDB'},
          '; turn rate', out['d'][0])


if __name__ == '__main__':
    main()
