"""The walker's path request, delivery and release run offline, frame by frame: at each manager frame's start the
request object's step F-12434 (ped +512 slot 3, from F-12278: submit when asked - F-40518 -, else poll F-88789,
fetch F-60370, deliver F-44435 into the path R, release F-79836); then the walker's F-30466 (the request F-73065
and the take F-23649); sometimes the give-up F-50795 or the release F-75945 (both cancel through the request's slot 4
= F-39771). The generated code whole and unchanged in vector mode (the build of car_dodge_reference.py).
Stubbed, as inputs of the case, and recorded in order (tag, words): the submit F-40518 (t1: the filter word, the
length, the 16 words of the start matrix; returns 1), the poll F-88789 (t2; the case's answer -1 / 0 / 1 for the
frame), the query release F-79836 (t3), the rails' unload F-78813 (t4) and set F-48384 (t5: rail, phase), the
behaviour's slot 15 (+60, the next rail F-95960: t6, the case's rail word and phase), the walker's matrix F-33765 (the
case's 16 words). The frame counter 0x827D752C and the game-wide stamp 0x8294A540 as the case sets them (another
walker took the frame: the stamp equal to the counter before the update).
Also the request's seed F-93347 alone (mode 1): the global MWC 0x82860890 before -> the query's +144, +148 (the
length), +216 / +220 (the seed) and the MWC after.
Writes <out-dir>/walker_path_ref.txt: mode 0 lines = the case's words '->' per frame the record and the state
(R +768 count, +784, +788, +792, +796; the request's +8 / +9 / +10, +80, +84 (R or 0), +88, +416, +16 .. +76; the
stamp; the walker's MWC; the count's points x y z); mode 1 lines = in '->' out. Words as hex; a word whose top byte is
0x82 as two 16-bit halves hhhh_llll."""
import argparse
import random
from pathlib import Path
import car_dodge_reference as cdr
from car_dodge_reference import addr, fb, f32, word
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += 'static int FR;static bool SUB;' + chr(10)

NF = 24      # frames per case
NNODE = 52   # path nodes (past the 48-point cap)
NODEW = 7    # type, a xyz, b xyz
FRW = 9      # poll, fetch ok, M flags, reverse, stamp taken, give-up, rail word, phase, path given (0: null pointer)
P = {}
_n = 0
for name, w in (('scale', 1), ('age', 1), ('mwc', 2), ('maxsp', 1), ('filter', 1), ('mat', 16), ('nnode', 1),
                 ('node', NODEW * NNODE), ('fr', FRW * NF)):
    P[name] = _n
    _n += w
W = _n

PED, REQ, R, M, FOL, BEH, BVT, TYP, NODES = 0x10000, 0x10200, 0x18000, 0x19000, 0x19800, 0x19C00, 0x19E00, 0x1A000, 0x14000
MAT = 0x1B000
NEXT_RAIL = 0x00ABC000  # the behaviour's slot 15 as the dispatch sees it


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=600)
    ap.add_argument('--seeds', type=int, default=1000)
    a = ap.parse_args()
    ids = ('F-30466', 'F-12434', 'F-50795', 'F-75945', 'F-39771', 'F-93347', 'F-40518', 'F-88789', 'F-79836', 'F-78813',
           'F-48384', 'F-33765', 'F-60370', 'F-44435', 'F-73065', 'F-23649')
    A = {f: addr(a.tool, f) for f in ids}
    rnd = random.Random(73065)

    def vec(v):
        return [fb(f32(x)) for x in v]

    cases = []
    for ci in range(a.cases):
        c = [0] * W
        c[P['scale']] = fb(f32(rnd.uniform(0.85, 1.15)))
        c[P['age']] = rnd.choice((0, 3, 15, 16, 40, rnd.randrange(0, 100)))
        c[P['mwc']] = rnd.getrandbits(32)
        c[P['mwc'] + 1] = rnd.getrandbits(32) if rnd.random() < 0.5 else rnd.randrange(0, 0x5CDCFAA7)
        c[P['maxsp']] = fb(f32(rnd.choice((20.0, 20.0, rnd.uniform(1, 30)))))
        c[P['filter']] = rnd.choice((0, 0x1C000, rnd.getrandbits(32)))
        h = rnd.uniform(-3.2, 3.2)
        import math
        ch, sh = math.cos(h), math.sin(h)
        rows = [[ch, 0.0, -sh, 0.0], [0.0, 1.0, 0.0, 0.0], [sh, 0.0, ch, 0.0],
                [rnd.uniform(-2000, 2000), rnd.uniform(-5, 60), rnd.uniform(-2000, 2000), 1.0]]
        if rnd.random() < 0.1:
            rows[2][1] = 0.0  # (a zero lane: 0 - 0)
            rows[0] = [0.0, 0.0, 0.0, 0.0]
        c[P['mat']:P['mat'] + 16] = vec(sum(rows, []))
        nn = rnd.choice((0, 1, 2, 5, 12, 31, 32, 33, 47, 48, 49, 52))
        c[P['nnode']] = nn
        for k in range(NNODE):
            o = P['node'] + NODEW * k
            t = rnd.choice((0, 1, 2, 2, 2, 2, 3, 2))
            pa = [rnd.uniform(-2000, 2000), rnd.uniform(-5, 60), rnd.uniform(-2000, 2000)]
            pb = [x + rnd.uniform(-4, 4) for x in pa] if rnd.random() < 0.9 else pa
            c[o:o + NODEW] = [t] + vec(pa) + vec(pb)
        for f in range(NF):
            o = P['fr'] + FRW * f
            c[o + 0] = rnd.choice((0xFFFFFFFF, 0xFFFFFFFF, 0, 1, 1, 1))
            c[o + 1] = rnd.choice((0, 1, 1, 1))
            c[o + 2] = rnd.choice((1, 1, 1, 1, 3, 5, 7, 9, 0x11, 1 | rnd.getrandbits(8)))
            c[o + 3] = rnd.choice((0, 0, 0, 1))
            c[o + 4] = rnd.random() < 0.15
            c[o + 5] = rnd.choice((0,) * 14 + (1, 2))
            c[o + 6] = rnd.choice((0, 0x1D000, 0x1D100))
            c[o + 7] = fb(f32(rnd.choice((0.0, rnd.uniform(0, 1)))))
            c[o + 8] = rnd.random() < 0.9
        cases.append(c)
    seeds = [[rnd.getrandbits(32), rnd.choice((rnd.getrandbits(32), rnd.randrange(0, 0x5CDCFAA7), 0, 1)),
              fb(f32(rnd.uniform(1, 30)))] for _ in range(a.seeds)]

    nl = chr(10)
    NL = chr(92) + 'n'
    rows_c = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    rows_s = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in seeds)
    stubs = {
        'sub_' + A['F-40518']: 'logt(1);lw(ctx.r4.u32);{float x=float(ctx.f1.f64);uint32_t u;memcpy(&u,&x,4);lw(u);}'
                                'for(int k=0;k<16;k++)lw(load(ctx.r5.u32+4*k));ctx.r3.u64=1;SUB=true;',
        'sub_' + A['F-88789']: 'logt(2);ctx.r3.s64=SUB?int32_t(KC[%d+%d*FR]):-1;' % (P['fr'], FRW),
        'sub_' + A['F-79836']: 'logt(3);SUB=false;',
        'sub_' + A['F-78813']: 'logt(4);if(ctx.r3.u32!=0x%x)err(4);' % FOL,
        'sub_' + A['F-48384']: 'logt(5);if(ctx.r3.u32!=0x%x)err(5);lw(ctx.r4.u32);{float x=float(ctx.f1.f64);uint32_t u;'
                                'memcpy(&u,&x,4);lw(u);}' % FOL,
        'sub_' + A['F-33765']: 'if(ctx.r3.u32!=0x%x)err(33);ctx.r3.u64=0x%x;' % (PED, MAT),
    }
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){'
            'if(a==0x%s){sub_%s(ctx,base);return;}' % (A['F-39771'], A['F-39771']) +
            'if(a==0x%x){logt(6);if(ctx.r3.u32!=0x%x||ctx.r5.u32!=0)err(6);store(ctx.r4.u32,KC[%d+%d*FR+7]);'
            'ctx.r3.u64=KC[%d+%d*FR+6];return;}' % (NEXT_RAIL, BEH, P['fr'], FRW, P['fr'], FRW) +
            'throw std::runtime_error("unexpected indirect call");}')
    setup = (
        'memset(objects,0,sizeof(objects));LOG.clear();KC=c;SUB=false;'
        'store(0x%x,0x820C0F68);store(0x%x+800,0x%x);store(0x%x+804,0x%x);store(0x%x,0x%x);store(0x%x+60,0x%x);'
        % (REQ, R, PED, R, BEH, BEH, BVT, BVT, NEXT_RAIL) +
        'store(0x%x+792,c[%d]);store(0x%x+796,c[%d]);store(0x%x+384,c[%d]);store(0x%x+388,c[%d+1]);'
        % (R, P['scale'], R, P['age'], PED, P['mwc'], PED, P['mwc']) +
        'store(0x%x+448,0x%x);store(0x%x+124,c[%d]);store(0x%x+96,c[%d]);store(0x%x+224,0x%x);'
        % (PED, TYP, TYP, P['maxsp'], TYP, P['filter'], M, PED) +
        'for(int k=0;k<16;k++)store(0x%x+4*k,c[%d+k]);' % (MAT, P['mat']) +
        # the path object at query +168 = req +264: its +12 the first node; nodes chained by +48
        'store(0x%x+276,c[%d]?0x%x:0);for(uint32_t k=0;k<c[%d];k++){uint32_t n=0x%x+64*k;const uint32_t*o=c+%d+%d*k;'
        'store(n,o[0]);for(int j=0;j<3;j++){store(n+4+4*j,o[1+j]);store(n+16+4*j,o[4+j]);}store(n+48,k+1<c[%d]?n+64:0);}'
        % (REQ, P['nnode'], NODES, P['nnode'], NODES, P['node'], NODEW, P['nnode'])
    )
    state = ('PW(load(0x{R:x}+768));PW(*pointer(0x{R:x}+784));PW(load(0x{R:x}+788));PW(load(0x{R:x}+792));PW(load(0x{R:x}+796));'
             'PW(*pointer(0x{Q:x}+8));PW(*pointer(0x{Q:x}+9));PW(*pointer(0x{Q:x}+10));PW(load(0x{Q:x}+80));'
             'PW(load(0x{Q:x}+84)==0x{R:x}?1:load(0x{Q:x}+84));PW(load(0x{Q:x}+88));PW(load(0x{Q:x}+416));'
             'for(int k=0;k<16;k++)PW(load(0x{Q:x}+16+4*k));PW(get(0x8294A540));PW(load(0x{P:x}+384));PW(load(0x{P:x}+388));'
             'for(uint32_t k=0;k<load(0x{R:x}+768)&&k<48;k++)for(int j=0;j<3;j++)PW(load(0x{R:x}+16*k+4*j));'
             ).format(R=R, Q=REQ, P=PED)
    frame = (
        'for(FR=0;FR<%d;FR++){const uint32_t*fr=c+%d+%d*FR;LOG.clear();' % (NF, P['fr'], FRW) +
        'store(0x%x+188,fr[1]);if(!fr[8])store(0x%x+276,0);else store(0x%x+276,c[%d]?0x%x:0);'
        % (REQ + 96, REQ, REQ, P['nnode'], NODES) +
        # the frame's start: F-12434 (r4 = 0)
        'Context ctx{};ctx.r1.u32=0x7FF00;ctx.r13.u32=0x7E000;ctx.r3.u32=0x%x;ctx.r4.u32=0;sub_%s(ctx,nullptr);'
        % (REQ, A['F-12434']) +
        # the walker's update: the counter, another walker's stamp, M's flags, F-30466 (R, M, P, reverse)
        'put(0x827D752C,100+FR);if(fr[4])put(0x8294A540,100+FR);store(0x%x+260,fr[2]);' % M +
        'ctx.r1.u32=0x7FF00;ctx.r3.u32=0x%x;ctx.r4.u32=0x%x;ctx.r5.u32=0x%x;ctx.r6.u32=fr[3];sub_%s(ctx,nullptr);'
        % (R, M, FOL, A['F-30466']) +
        'if(fr[5]==1){ctx.r1.u32=0x7FF00;ctx.r3.u32=0x%x;sub_%s(ctx,nullptr);}' % (R, A['F-50795']) +
        'if(fr[5]==2){ctx.r1.u32=0x7FF00;ctx.r3.u32=0x%x;sub_%s(ctx,nullptr);}' % (R, A['F-75945']) +
        'printf(" |%s");' + state + '}'
    )
    body = ('static const uint32_t C[][%d]={' % W + nl + rows_c + nl + '};' + nl +
            'static const uint32_t S[][3]={' + nl + rows_s + nl + '};' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};'
            'auto get=[](uint32_t a){uint8_t*p=image.data()+(a-0x82000000u);return uint32_t(p[0])<<24|uint32_t(p[1])<<16|uint32_t(p[2])<<8|p[3];};'
            'auto PW=[](uint32_t w){printf(" %08x",w);};' + nl +
            'for(auto&c:C){put(0x8294A540,0);' + setup + 'try{' + frame + '}catch(std::exception&e){printf(" EXC %s",e.what());}'
            'printf("' + NL + '");}' + nl +
            # mode 1: the seed F-93347 (r3 the query +16, r4 a mesh, r5 the heading, r7 no filter, r8 none, f1 the length)
            'for(auto&s:S){memset(objects,0,sizeof(objects));put(0x82860890,s[0]);put(0x82860894,s[1]);store(0x2000+4,0);'
            'Context ctx{};ctx.r1.u32=0x7FF00;ctx.r13.u32=0x7E000;ctx.r3.u32=0x3000;ctx.r4.u32=0x2000;ctx.r5.u32=0x2100;'
            'ctx.r7.u32=0;ctx.r8.u32=0x2200;{float x;memcpy(&x,&s[2],4);ctx.f1.f64=x;}'
            'try{sub_%s(ctx,nullptr);}catch(std::exception&e){printf(" EXC %%s",e.what());}' % A['F-93347'] +
            'PW(load(0x3000+144));PW(load(0x3000+148));PW(load(0x3000+216));PW(load(0x3000+220));PW(get(0x82860890));'
            'PW(get(0x82860894));printf("' + NL + '");}' + nl)
    body = body.replace('printf(" |%s");', 'printf(" |%s",LOG.c_str());')
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    names = ['sub_' + A[f] for f in ('F-30466', 'F-12434', 'F-50795', 'F-75945', 'F-39771', 'F-93347')]
    # (F-93347's copy of the query's filter block F-63209 at r8: an empty block here)
    exe = build(idx, names, main_src, a.out_dir, 'walker_path', stubs=stubs, vector=True)
    res = [x for x in run(exe, a.image).split(nl) if x]
    assert len(res) == len(cases) + len(seeds), (len(res), len(cases), len(seeds))
    bad = [i for i, o in enumerate(res) if 'EXC' in o or 'ERR' in o]
    assert not bad, (len(bad), [t for t in res[bad[0]].split("|") if "EXC" in t or "ERR" in t][:2])
    lines = []
    for c, o in zip(cases, res):
        head = c[:P['node'] + NODEW * c[P['nnode']]] + c[P['fr']:]
        lines.append('0 ' + ' '.join(word(x) for x in head) + ' ->' +
                     ' |'.join(' '.join(t if t.startswith('t') else word(int(t, 16)) for t in fr.split()) for fr in o.split('|')[1:]))
    for s, o in zip(seeds, res[len(cases):]):
        lines.append('1 ' + ' '.join(word(x) for x in s) + ' -> ' + ' '.join(word(int(t, 16)) for t in o.split()))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'walker_path_ref.txt').write_text(nl.join(lines) + nl)
    n1 = sum(o.count('t1') for o in res)
    n6 = sum(o.count('t6') for o in res)
    nd = sum(1 for o in res[:len(cases)] for fr in o.split('|')[1:] if fr.split() and False)
    print(len(cases), 'cases x', NF, 'frames,', len(seeds), 'seeds;', n1, 'submits,', sum(o.count('t2') for o in res), 'polls,',
          sum(o.count('t3') for o in res), 'releases,', n6, 'rails taken')


if __name__ == '__main__':
    main()
