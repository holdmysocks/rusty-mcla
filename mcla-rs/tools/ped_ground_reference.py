"""The walkers' ground probe, up vector and drawn matrix run offline: the walker's slot 80 F-49074 whole (sphere refresh,
ground, matrix tail) with the type's slot 60 F-59837 (set sphere k) and slot 64 F-24029 (ground hit pick, U, place) reached
through its vtable, and the forward ring F-89721 (reset) / F-22858 (push) - the generated code whole and unchanged in
vector mode, on flat guest memory. The flush mode is the SDK's (FTZ and DAZ on the host while the code has it on). The
start-up vector constants the image holds as zero are filled in as the running game has them: 0x828609E0 = +inf (the
normalisations' guard), 0x82860960 = (0, all ones, 0, 0) (the y-lane select the static ctors F-50173 -> F-84238 ->
F-27464 build); 0x828608C0 / 0x828608D0 / 0x828608E0 have no initialiser (zero).
Stubbed, as inputs of the case: the walker's place for the distance F-58860 (returns the case's point), the current matrix
F-33765 (returns the case's matrix), the instance matrix setter F-86858 (empty: it only copies O into the instance).
Guest memory: walker I 0x2000 (+384/+388 its MWC, +412 speed, +448 the type 0x5400 whose vtable 0x5480 has slots 60 / 64,
+452 the behaviour 0x5600 (+1360 its state) or 0, +480 the manager index, +500 byte 0, +944 the sphere response: block k
at +944 + 1216 k, sphere 0's 12 hit slots at +1024 + 96 j (+0 the other instance, +16 the contact, +48 the normal), +4608
the forward ring, +4768 U, +4804 the one-off clip), T 0x4000, O 0x4100, the current matrix 0x4200, the distance point
0x4300, the hit instances 0x6000 + 16 j (+4 their archetype 0x6400 + 32 j, whose +16 holds the type flags), the manager
0x10000 or 0 (+0x6423C the frame counter, focus +5936 i + 0x37330).
Cases: draws on / off (the chance and k by the counter, clip or not, manager or none, the distance about 25 / 85 m), T
facings flat, tilted, zero or scaled, U zero, a ground normal, any vector or tiny, the slots empty, of other types or
0x800, normals about y = 0.70710677 (a float step either side), plan distances about a tie (0.25 m^2 a float step either
side) in random order, the ring empty, part full or full with the index at 0 .. 8, the one-off clip flag, dt 0 or not.
Writes <out-dir>/ped_ground_ref.txt, one case a line: the inputs, '->', the outputs (see `fields` below); and
<out-dir>/ped_ring_ref.txt: sequences of pushes / resets on one ring from empty and the ring after each (F-22858 / F-89721
alone). Words as hex; a word whose top byte is 0x82 is written as two 16-bit halves hhhh_llll."""
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


# the case's words (input), in order
NSLOT = 12
SLOTW = 8  # present, flags, contact xyz, normal xyz


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    upd, setk, pick, reset, push, dpos, curm, setm = (addr(a.tool, f) for f in
                                                      ('F-49074', 'F-59837', 'F-24029', 'F-89721', 'F-22858', 'F-58860', 'F-33765', 'F-86858'))
    rnd = random.Random(49074)
    S2 = f32(math.sqrt(0.5))

    def unit(v):
        l = math.sqrt(sum(x * x for x in v))
        return [x / l for x in v] if l > 0 else v

    def facing():
        u = rnd.random()
        y = rnd.uniform(-math.pi, math.pi)
        v = [math.sin(y), 0.0, -math.cos(y)]
        if u < 0.05:
            return [0.0, 0.0, 0.0]
        if u < 0.25:
            v[1] = rnd.uniform(-0.5, 0.5)
            v = unit(v)
        elif u < 0.35:
            k = rnd.uniform(0.01, 30)
            v = [x * k for x in v]
        return v

    def normal():
        u = rnd.random()
        if u < 0.3:
            # about y = 0.70710677: a float step either side
            ny = bf(fb(S2) + rnd.choice((-2, -1, 0, 1, 2)))
            h = math.sqrt(max(0.0, 1 - ny * ny))
            ang = rnd.uniform(-math.pi, math.pi)
            return [h * math.cos(ang), ny, h * math.sin(ang)]
        if u < 0.6:
            return [0.0, 1.0, 0.0]
        if u < 0.85:
            return unit([rnd.uniform(-0.6, 0.6), 1.0, rnd.uniform(-0.6, 0.6)])
        return unit([rnd.uniform(-1, 1), rnd.uniform(-1, 1), rnd.uniform(-1, 1)])

    def upv():
        u = rnd.random()
        if u < 0.25:
            return [0.0, 0.0, 0.0]
        if u < 0.7:
            return unit([rnd.uniform(-0.5, 0.5), 1.0, rnd.uniform(-0.5, 0.5)])
        if u < 0.75:
            return [rnd.choice((1e-39, 3e-39, 1e-20, 0.0)) for _ in range(3)]
        return [rnd.uniform(-2, 2) for _ in range(3)]

    cases = []
    for _ in range(3000):
        beh = rnd.random() < 0.9
        state = rnd.choice((3, 3, 1, 0, 2)) if beh else 0
        mgr = rnd.random() < 0.9
        idx = rnd.getrandbits(1)
        counter = rnd.randrange(0, 1 << 20)
        lo, carry = rnd.getrandbits(32), rnd.getrandbits(32)
        speed = rnd.choice((0.0, rnd.uniform(0, 3), 1.2))
        dt = rnd.choice((1 / 30, 1 / 60, 0.05, 0.0, 1 / 30))
        place = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 60), rnd.uniform(-3000, 3000)]
        r2 = facing()
        r0 = [rnd.uniform(-1, 1) for _ in range(3)]
        r1 = [rnd.uniform(-1, 1) for _ in range(3)]
        cur = [place[i] + rnd.uniform(-0.3, 0.3) for i in range(3)]
        dist = rnd.choice((25.0, 85.0, rnd.uniform(0, 30), rnd.uniform(0, 100), rnd.uniform(80, 200)))
        ang = rnd.uniform(-math.pi, math.pi)
        dpt = [place[0] + rnd.uniform(-1, 1), 0.0, place[2] + rnd.uniform(-1, 1)]
        focus = [dpt[0] + dist * math.cos(ang), rnd.uniform(-5, 5), dpt[2] + dist * math.sin(ang)]
        up = upv()
        c0 = [place[i] + rnd.uniform(-1.5, 1.5) for i in range(3)] if rnd.random() < 0.8 else list(place)
        # the hit slots
        n = rnd.choice((0, 1, 2, 3, 5, 8, 12, 12))
        slots = []
        base = rnd.uniform(0, 3)
        for j in range(NSLOT):
            if j >= n or rnd.random() < 0.1:
                slots.append([0] * SLOTW)
                continue
            fl = rnd.choice((0x800, 0x800, 0x800, 0x803, 0x8800, 0x2, 0x20, 0))
            u = rnd.random()
            if u < 0.5:
                # plan distance squared about base or base +- 0.25 (ties), a few float steps either side
                d = rnd.choice((base, base + 0.25, base - 0.25, base + 0.5, base + rnd.uniform(-0.3, 0.3)))
                d = max(0.0, d)
                d = bf(max(0, fb(f32(d)) + rnd.choice((-1, 0, 0, 1))))
            else:
                d = rnd.uniform(0, 4)
            r = math.sqrt(d)
            t = rnd.uniform(-math.pi, math.pi)
            pt = [c0[0] + r * math.cos(t), c0[1] + rnd.uniform(-2, 2), c0[2] + r * math.sin(t)]
            if rnd.random() < 0.15:
                pt[1] = c0[1] - 0.5
            slots.append([1, fl] + [fb(f32(x)) for x in pt] + [fb(f32(x)) for x in normal()])
        # the forward ring
        u = rnd.random()
        cnt = 0 if u < 0.15 else (rnd.randint(1, 7) if u < 0.45 else 8)
        ix = cnt if cnt < 8 else rnd.randint(0, 8)
        ents = []
        for _ in range(8):
            y = rnd.uniform(-math.pi, math.pi)
            ents += [fb(f32(x)) for x in (math.sin(y), rnd.choice((0.0, 0.0, rnd.uniform(-0.3, 0.3))), -math.cos(y))]
        ssum = [fb(f32(rnd.uniform(-8, 8))) for _ in range(3)]
        oneoff = rnd.getrandbits(1)
        c = [int(beh), state, int(mgr), idx, counter, lo, carry] + [fb(f32(x)) for x in [speed, dt] + r0 + r1 + r2 + place + cur
                                                                      + dpt + focus + up + c0]
        c += [cnt, ix] + ssum + ents + [oneoff]
        for s in slots:
            c += s
        cases.append(c)
    W = len(cases[0])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){'
            'if(a==0x%s)sub_%s(ctx,base);else if(a==0x%s)sub_%s(ctx,base);else throw std::runtime_error("unexpected indirect call");}'
            % (setk, setk, pick, pick))
    # input word positions
    P = dict(beh=0, state=1, mgr=2, idx=3, counter=4, lo=5, carry=6, speed=7, dt=8, r0=9, r1=12, r2=15, place=18, cur=21,
             dpt=24, focus=27, up=30, c0=33, cnt=36, ix=37, sum=38, ents=41, oneoff=65, slots=66)
    assert P['slots'] + NSLOT * SLOTW == W, W
    I = 0x2000
    v3 = lambda adr, k, w='0': 'store(%d,c[%d]);store(%d,c[%d]);store(%d,c[%d]);store(%d,%s);' % (adr, k, adr + 4, k + 1, adr + 8, k + 2, adr + 12, w)
    setup = ('memset(objects,0,sizeof(objects));'
             'store(%d,c[%d]);store(%d,c[%d]);store(%d,c[%d]);store(%d,0x5400);store(0x5400,0x5480);store(0x5480+60,0x%s);store(0x5480+64,0x%s);'
             % (I + 384, P['lo'], I + 388, P['carry'], I + 412, P['speed'], I + 448, setk, pick) +
             'store(%d,c[0]?0x5600:0);store(0x5600+1360,c[1]);store(%d,c[3]);' % (I + 452, I + 480) +
             v3(0x4000, P['r0']) + v3(0x4010, P['r1']) + v3(0x4020, P['r2']) + v3(0x4030, P['place'], '0x3f800000') +
             v3(0x4230, P['cur'], '0x3f800000') + v3(0x4300, P['dpt']) + v3(I + 4768, P['up']) + v3(I + 992, P['c0'], '0x3f800000') +
             'if(c[2]){store(0x10000+0x6423C,c[4]);uint32_t f=0x10000+c[3]*5936+0x37330;store(f,c[%d]);store(f+4,c[%d]);store(f+8,c[%d]);}'
             % (P['focus'], P['focus'] + 1, P['focus'] + 2) +
             'store(%d,c[%d]);store(%d,c[%d]);' % (I + 4608, P['cnt'], I + 4612, P['ix']) + v3(I + 4624, P['sum']) +
             'for(int e=0;e<8;e++){store(%d+16*e,c[%d+3*e]);store(%d+16*e,c[%d+3*e]);store(%d+16*e,c[%d+3*e]);}'
             % (I + 4640, P['ents'], I + 4644, P['ents'] + 1, I + 4648, P['ents'] + 2) +
             'store(%d,c[%d]);' % (I + 4804, P['oneoff']) +
             'for(int j=0;j<12;j++){const uint32_t*s=c+%d+8*j;uint32_t S=%d+96*j;if(s[0]){store(S,0x6000+16*j);store(0x6000+16*j+4,0x6400+32*j);'
             'store(0x6400+32*j+16,s[1]);}store(S+16,s[2]);store(S+20,s[3]);store(S+24,s[4]);store(S+48,s[5]);store(S+52,s[6]);store(S+56,s[7]);}'
             % (P['slots'], I + 1024))
    pr = 'auto P=[](uint32_t a){printf(" %08x",load(a));};'
    out = ('printf("%d",int(ctx.r3.u32));for(int k=0;k<16;k++)P(0x4100+4*k);for(int k=0;k<4;k++)P(' + str(I + 4768) + '+4*k);'
           'P(' + str(I + 4608) + ');P(' + str(I + 4612) + ');for(int k=0;k<36;k++)P(' + str(I + 4624) + '+4*k);'
           'for(int b=0;b<3;b++){uint32_t B=' + str(I + 944) + '+1216*b;P(B+20);P(B+64);P(B+48);P(B+52);P(B+56);}'
           'P(' + str(I + 384) + ');P(' + str(I + 388) + ');printf("' + NL + '");')
    body = ('static const uint32_t C[][%d]={' % W + nl + rows + nl + '};' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};'
            'for(int k=0;k<4;k++)put(0x828609E0+4*k,0x7F800000);put(0x82860960,0);put(0x82860964,0xFFFFFFFF);put(0x82860968,0);put(0x8286096C,0);'
            + pr + nl +
            'for(auto&c:C){' + setup + 'Context ctx{};ctx.r1.u32=0x7F00;ctx.r3.u32=%d;ctx.r5.u32=0x4000;ctx.r6.u32=0x4100;' % I +
            'ctx.r7.u32=c[2]?0x10000:0;{float d;memcpy(&d,&c[8],4);ctx.f1.f64=d;}sub_%s(ctx,nullptr);' % upd + out + '}' + nl)
    # the ring alone: sequences of push (1) / reset (2) from empty
    seqs = []
    for _ in range(600):
        ops = []
        for _ in range(rnd.randint(1, 24)):
            y = rnd.uniform(-math.pi, math.pi)
            v = [math.sin(y), rnd.choice((0.0, 0.0, rnd.uniform(-0.3, 0.3))), -math.cos(y)]
            if rnd.random() < 0.05:
                v = [rnd.choice((1e-39, 0.0, -2e-39)) for _ in range(3)]
            ops.append([2 if rnd.random() < 0.12 else 1] + [fb(f32(x)) for x in v])
        seqs.append(ops)
    rrows = nl.join('{%d,%s},' % (len(s), ','.join('0x%08x' % x for op in s for x in op)) for s in seqs)
    rbody = ('static const uint32_t R[][1+24*4]={' + nl + rrows + nl + '};' + nl +
             'for(auto&s:R){memset(objects,0,0x400);for(uint32_t i=0;i<s[0];i++){const uint32_t*o=s+1+4*i;'
             'store(0x1200,o[1]);store(0x1204,o[2]);store(0x1208,o[3]);store(0x120C,0);'
             'Context ctx{};ctx.r1.u32=0x7F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1200;if(o[0]==2)sub_%s(ctx,nullptr);else sub_%s(ctx,nullptr);'
             % (reset, push) +
             'if(i)printf(" |");P(0x1000);P(0x1004);for(int k=0;k<36;k++)P(0x1010+4*k);}printf("' + NL + '");}' + nl)
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
                'if(argc>2&&argv[2][0]==0x72){' + pr + rbody + 'return 0;}' + nl + body + '}' + nl)
    idx = index(a.recomp)
    stubs = {'sub_' + dpos: 'ctx.r3.u32=0x4300;', 'sub_' + curm: 'ctx.r3.u32=0x4200;', 'sub_' + setm: ''}
    exe = build(idx, ['sub_' + upd, 'sub_' + setk, 'sub_' + pick, 'sub_' + reset, 'sub_' + push], main_src, a.out_dir, 'ped_ground',
                stubs=stubs, vector=True)
    res = [x for x in run(exe, a.image).split(nl) if x]
    assert len(res) == len(cases), (len(res), len(cases))
    r = subprocess.run([str(exe), str(a.image), 'r'], capture_output=True, text=True, check=True).stdout
    rres = [x for x in r.split(nl) if x]
    assert len(rres) == len(seqs), (len(rres), len(seqs))
    fix = lambda o: ' '.join(word(int(t, 16)) if t not in ('|',) and len(t) == 8 else t for t in o.split())
    lines = [' '.join(word(x) for x in c) + ' -> ' + fix(o) for c, o in zip(cases, res)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'ped_ground_ref.txt').write_text(nl.join(lines) + nl)
    rl = [' '.join(word(x) for op in s for x in op) + ' -> ' + fix(o) for s, o in zip(seqs, rres)]
    (a.out_dir / 'ped_ring_ref.txt').write_text(nl.join(rl) + nl)
    hits = sum(o.split()[0] == '1' for o in res)
    print(len(lines), 'cases,', hits, 'with a ground hit;', len(rl), 'ring sequences,', sum(len(s) for s in seqs), 'ring steps')


if __name__ == '__main__':
    main()
