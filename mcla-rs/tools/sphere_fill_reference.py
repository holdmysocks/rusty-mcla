"""The walkers' sphere-1 contact fill run offline: a sphere shape test set up by F-38096 (-> F-96489 -> F-16848), its
material mask (+88) set as F-66549 does, then for each object in the case's order the steps of the query driver F-49353:
the centre into the object's frame (F-51913), the bound test F-15615 on a capsule bound (the walker's: vtable 0x8207FF34,
type 1, material read through its slot 12 = F-60139), and on a hit the commit F-77105 (with F-57315 to world space and the
slot allocator F-89028 inside F-15615) - the generated code whole and unchanged in vector mode with the SDK's flush mode
(the build of car_dodge_reference.py). Only the helpers of F-15615's box and curved-geometry cases are left out (they
throw if reached; a capsule never reaches them). The level's cull order (F-49353's candidate lists) is not run:
the case gives the order.
Writes <out-dir>/sphere_fill_ref.txt: per case the input words (centre, radius, material mask, count, then per object
its rows, place, radius, length, centroid and material), '->', the 12 slots (instance index + 1 or 0, contact xyz,
normal xyz, the bound's normal xyz, t, depth, material, part, +78, +80 byte, +82) and the list's +44, +52, +80, +84.
Words as hex; a word whose top byte is 0x82 as two 16-bit halves hhhh_llll."""
import argparse
import math
import random
from pathlib import Path
import car_dodge_reference as cdr
from car_dodge_reference import addr, fb, f32, word
from recomp_closure import index, build, run

NOBJ = 20
OBJW = 18  # rows (9), place (3), radius, length, centroid xyz, material
P = {}
_n = 0
for name, w in (('c', 3), ('R', 1), ('mask', 1), ('nobj', 1), ('obj', OBJW * NOBJ)):
    P[name] = _n
    _n += w
W = _n
SLOTW = 17


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    ids = ('F-38096', 'F-51913', 'F-15615', 'F-77105', 'F-60139')
    A = {f: addr(a.tool, f) for f in ids}
    rnd = random.Random(15615)

    def vec(v):
        return [fb(f32(x)) for x in v]

    cases = []
    for ci in range(a.cases):
        c = [0] * W
        far = rnd.random() < 0.5
        cw = [rnd.uniform(-2000, 2000), rnd.uniform(-10, 40), rnd.uniform(-2000, 2000)] if far else \
            [rnd.uniform(-5, 5), rnd.uniform(-1, 1), rnd.uniform(-5, 5)]
        R = rnd.choice((0.667, 0.667, 0.667, 2.0, rnd.uniform(0.05, 1.5)))
        c[P['c']:P['c'] + 3] = vec(cw)
        c[P['R']] = fb(f32(R))
        c[P['mask']] = rnd.choice((0, 0, 512 << rnd.randrange(0, 20), 0xFFFFFFFF, rnd.getrandbits(32)))
        nobj = rnd.choice((0, 1, 1, 2, 3, 5, 8, 12, 13, 16, 20))
        crowd = rnd.random() < 0.3
        c[P['nobj']] = nobj
        for k in range(NOBJ):
            o = P['obj'] + OBJW * k
            h = rnd.uniform(-math.pi, math.pi)
            tilt = rnd.choice((0.0, 0.0, rnd.uniform(-0.3, 0.3)))
            ch, sh, ct, st = math.cos(h), math.sin(h), math.cos(tilt), math.sin(tilt)
            r0 = [ch, 0.0, -sh]
            r1 = [sh * st, ct, ch * st]
            r2 = [sh * ct, -st, ch * ct]
            rad, ln = rnd.choice(((0.3, 1.2), (0.3, 1.2), (0.3, 1.2), (rnd.uniform(0.05, 1.0), rnd.uniform(0.0, 3.0))))
            mid = rnd.choice(([0.0, 0.9, 0.0], [0.0, 0.9, 0.0], [rnd.uniform(-1, 1), rnd.uniform(-1, 2), rnd.uniform(-1, 1)]))
            reach = rad + R
            kind = rnd.random()
            if crowd:
                # (crowded: most objects touch, more than 12 contacts fill the list)
                loc = [rnd.uniform(-reach, reach) * 0.7, rnd.uniform(-ln * 0.5 - reach * 0.5, ln * 0.5 + reach * 0.5),
                       rnd.uniform(-reach, reach) * 0.7]
            elif kind < 0.1:
                # the centre on the capsule's axis (normal +x, part by height)
                a_y = rnd.choice((rnd.uniform(-ln, ln), ln * 0.5, -ln * 0.5, ln, -ln))
                loc = [0.0, a_y, 0.0]
            elif kind < 0.2:
                # at the axis' ends
                loc = [rnd.uniform(-reach, reach) * 0.5, rnd.choice((ln * 0.5, -ln * 0.5)), rnd.uniform(-reach, reach) * 0.5]
            else:
                loc = [rnd.uniform(-reach * 1.2, reach * 1.2), rnd.uniform(-ln - reach, ln + reach) * 0.6,
                       rnd.uniform(-reach * 1.2, reach * 1.2)]
            loc = [loc[i] + mid[i] for i in range(3)]
            # place = centre - loc in world (rows as the instance matrix)
            wl = [loc[0] * r0[j] + loc[1] * r1[j] + loc[2] * r2[j] for j in range(3)]
            pos = [cw[j] - wl[j] for j in range(3)]
            mat = rnd.choice((0, 0, 0, 512 << rnd.randrange(0, 20), rnd.getrandbits(32)))
            c[o:o + OBJW] = vec(r0 + r1 + r2 + pos) + [fb(f32(rad)), fb(f32(ln))] + vec(mid) + [mat]
        cases.append(c)
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    # the test 0x4000 (results 0x6000, 12 x 96), the centre vector 0x3F00, instances 0x8000 + 0x100 k (matrix at +16),
    # bounds 0xA000 + 0x100 k
    setup = (
        'memset(objects,0,sizeof(objects));'
        'for(int k=0;k<3;k++)store(0x3F00+4*k,c[%d+k]);store(0x3F0C,0);' % P['c'] +
        'for(int k=0;k<%d;k++){const uint32_t*o=c+%d+%d*k;uint32_t in=0x8000+0x100*k,b=0xA000+0x100*k;'
        'for(int r=0;r<4;r++){for(int j=0;j<3;j++)store(in+16+16*r+4*j,o[3*r+j]);store(in+16+16*r+12,r==3?0x3f800000:0);}'
        'store(in+4,0xC000);store(0xC000+12,b);'
        'store(b,0x8207FF34);*pointer(b+4)=1;for(int j=0;j<4;j++){store(b+128+4*j,o[12]);store(b+144+4*j,o[13]);}'
        'for(int j=0;j<3;j++)store(b+48+4*j,o[14+j]);store(b+60,0);store(b+208,o[17]);}' % (NOBJ, P['obj'], OBJW)
    )
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){if(a==0x%s){sub_%s(ctx,base);return;}'
            'throw std::runtime_error("unexpected indirect call");}' % (A['F-60139'], A['F-60139']))
    body = ('static const uint32_t C[][%d]={' % W + nl + rows + nl + '};' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};'
            'for(int k=0;k<3;k++){put(0x82860950+4*k,0xFFFFFFFF);put(0x82860930+4*k,0);}put(0x8286095C,0);put(0x8286093C,0x3F800000);'
            'for(int k=0;k<4;k++)put(0x828609E0+4*k,0x7F800000);'
            'auto PW=[](uint32_t w){printf(" %08x",w);};' + nl +
            'for(auto&c:C){' + setup +
            'Context ctx{};ctx.r1.u32=0x1FF00;ctx.r13.u32=0x16000;ctx.r3.u32=0x4000;ctx.r4.u32=0x3F00;ctx.r6.u32=0x6000;ctx.r7.u32=12;'
            '{float R;memcpy(&R,&c[%d],4);ctx.f1.f64=R;}sub_%s(ctx,nullptr);store(0x4000+88,c[%d]);' % (P['R'], A['F-38096'], P['mask']) +
            'std::string out;try{for(uint32_t k=0;k<c[%d];k++){uint32_t in=0x8000+0x100*k;' % P['nobj'] +
            'ctx.r1.u32=0x1FF00;ctx.r3.u32=0x4000;ctx.r4.u32=in+16;sub_%s(ctx,nullptr);' % A['F-51913'] +
            'ctx.r1.u32=0x1FF00;ctx.r3.u32=0x4000;ctx.r4.u32=0xA000+0x100*k;ctx.r5.u32=in;sub_%s(ctx,nullptr);' % A['F-15615'] +
            'if(ctx.r3.u32&0xFF){ctx.r1.u32=0x1FF00;ctx.r3.u32=0x4000;ctx.r4.u32=in+16;ctx.r5.u32=0;sub_%s(ctx,nullptr);}}' % A['F-77105'] +
            '}catch(std::exception&e){printf(" EXC");}'
            'for(int s=0;s<12;s++){uint32_t a=0x6000+96*s;uint32_t o=load(a);PW(o?(o-0x8000)/0x100+1:0);'
            'for(int j=0;j<3;j++)PW(load(a+16+4*j));for(int j=0;j<3;j++)PW(load(a+32+4*j));for(int j=0;j<3;j++)PW(load(a+48+4*j));'
            'PW(load(a+64));PW(load(a+68));PW(load(a+72));PW(load16(a+76));PW(load16(a+78));PW(*pointer(a+80));PW(load16(a+82));}'
            'PW(load(0x4000+44));PW(load(0x4000+52));PW(load(0x4000+80));PW(load(0x4000+84));printf("' + NL + '");}' + nl)
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    names = ['sub_' + A[f] for f in ids]
    # the box (F-15615 case 3) and curved-geometry (case 5) helpers: never reached by a capsule; they throw if they were
    other = {'sub_' + addr(a.tool, f) for f in ('F-25936', 'F-48308', 'F-80030', 'F-96503')}
    stubs = {n: 'throw std::runtime_error("other bound type");' for n in other}
    exe = build(idx, names, main_src, a.out_dir, 'sphere_fill', stubs=stubs, vector=True)
    res = [x for x in run(exe, a.image).split(nl) if x]
    assert len(res) == len(cases), (len(res), len(cases))
    bad = [i for i, o in enumerate(res) if 'EXC' in o]
    assert not bad, (len(bad), res[bad[0]])
    # (the input as written: the case's head and its objects only)
    lines = [' '.join(word(x) for x in c[:P['obj'] + OBJW * c[P['nobj']]]) + ' -> ' + ' '.join(word(int(t, 16)) for t in o.split()) for c, o in zip(cases, res)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'sphere_fill_ref.txt').write_text(nl.join(lines) + nl)
    filled = [sum(int(o.split()[SLOTW * s], 16) != 0 for s in range(12)) for o in res]
    print(len(lines), 'cases,', sum(c[P['nobj']] for c in cases), 'objects;', sum(filled), 'slots filled;',
          sum(f == 12 for f in filled), 'lists full;', sum(c[P['nobj']] > 12 for c in cases), 'cases with more than 12 objects')


if __name__ == '__main__':
    main()
