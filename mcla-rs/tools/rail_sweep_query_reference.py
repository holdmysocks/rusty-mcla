"""What the rail sweep F-23371 asks the level, run offline: the cases of rail_sweep_reference.py (a rail built by
F-85722, swept by F-23371), but now with the sphere query F-66549 and the swept capsule F-27454 themselves run whole
(their shape set-ups F-38096 / the capsule's, the result list's reset and box), and only the level's query drivers
stubbed and logged: the sphere driver (s: centre xyz +144, radius +176, max +36, material mask +88, include r5, type
flags r6, states r7, r8, r9, r10, the excluded count +2448 and words +1680..) and the sweep driver (c: from +144, to
+160, radius +208, the directed byte +212, max +36, +88, r5 .. r10, count +2480, words +1712..). The material
manager's cast (the CRT's dynamic cast) answers an object whose flag shift (+16) is 8 (phMaterialMgrFlag's ctor
default); the query objects' ctors and the free are empty. Each driver answers hit or not by the case's hit index (it
writes the result record's instance word, the query's +64 / params +112). Test `rail_sweep_query_matches_the_original`.
Params (+0 the width, +4 .. +12 the excluded words, +16 the mask - 0 means 0x812 -, +24 the start phase, +28 the length
left, in and out (< 0: the rest of the rail), +32 the height of the spheres' bottom).
Writes <out-dir>/rail_sweep_query_ref.txt, one case a line: P H0 T H1 (xyz each), R, s0, s1, width, excl x3, mask, start,
length, height, hit index; '->' the build's valid flag, the result, the length left after and the query records."""
import argparse
import math
import random
import re
from pathlib import Path
import tcp_rail_reference as trr
from tcp_rail_reference import addr, fb, f32
from recomp_closure import index, build, run
import recomp_closure

# (the two query objects are some 2.5 KB each and sit on F-23371's stack: a larger scratch, the sweep's stack at 0x3F000)
_shim = recomp_closure.shim
recomp_closure.shim = lambda scratch_end=0x3000: _shim(0x40000)
recomp_closure.EXTRA += ('static int NQ_,HIT_;static void pw_(uint32_t w){printf(" %08x",w);}' + chr(10))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=1000)
    a = ap.parse_args()
    A = {f: addr(a.tool, f) for f in ('F-85722', 'F-23371', 'F-66549', 'F-27454', 'F-43397')}
    rnd = random.Random(23371)

    def heading():
        y = rnd.uniform(-math.pi, math.pi)
        h = [math.sin(y), 0.0, -math.cos(y)]
        if rnd.random() < 0.1:
            k = rnd.choice((0.5, 2.0, rnd.uniform(0.01, 30)))
            h = [x * k for x in h]
        return h

    cases = []
    for _ in range(a.cases):
        R = rnd.choice((0.1, 0.4, 0.4, 2.0, 5.0, 12.0, rnd.uniform(0.05, 30)))
        P = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 60), rnd.uniform(-3000, 3000)]
        H0, H1 = heading(), heading()
        s0, s1 = rnd.getrandbits(1), rnd.getrandbits(1)
        dist = rnd.choice((rnd.uniform(0, 3), rnd.uniform(0, 30), rnd.uniform(0, 80), 2 * R))
        ang = rnd.uniform(-math.pi, math.pi)
        dy = rnd.choice((0.0, 0.0, rnd.uniform(-3, 3)))
        T = [P[0] + dist * math.cos(ang), P[1] + dy, P[2] + dist * math.sin(ang)]
        width = rnd.choice((0.6, 0.6, 1.8, 2.2, rnd.uniform(0.05, 4)))
        # (the count +4: the game's 1 or 2, none, or any word - the copy stops at 64)
        excl = [rnd.choice((0, 1, 1, 2, 2, rnd.getrandbits(32)))] + [rnd.choice((0, rnd.getrandbits(32))) for _ in range(2)]
        mask = rnd.choice((0, 0x383E, 0x2106, rnd.getrandbits(16)))
        start = rnd.choice((0.0, 0.0, 0.5, 1.0, rnd.random(), rnd.random()))
        left = rnd.choice((-1.0, -1.0, 0.0, 1e-7, 2e-6, rnd.uniform(0, 5), rnd.uniform(0, 60)))
        lift = rnd.choice((0.6, 0.0, 0.35, rnd.uniform(-1, 2)))
        hit = rnd.choice((-1, -1, -1, 0, 1, 2, rnd.randrange(0, 40)))
        cases.append([fb(f32(x)) for x in P + H0 + T + H1 + [R]] + [s0, s1, fb(f32(width))] + excl +
                     [mask, fb(f32(start)), fb(f32(left)), fb(f32(lift)), hit & 0xFFFFFFFF])
    W = len(cases[0])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    # (the drivers and helpers by the calls F-66549 / F-27454 make: ctor, set-up, cast, driver, free, dtor)
    from recomp_leaf import extract
    idx0 = index(a.recomp)

    def calls(fn):
        s = extract(idx0['sub_' + fn], 'sub_' + fn)
        out = []
        for m in re.finditer(r'\b(sub_[0-9A-F]{8})\(ctx, base\)', s):
            if m.group(1) not in out:
                out.append(m.group(1))
        return out
    cs_, cc_ = calls(A['F-66549']), calls(A['F-27454'])
    assert len(cs_) == 6 and len(cc_) == 6, (cs_, cc_)
    s_ctor, s_setup, cast, s_drv, free, dtor = cs_
    c_ctor, c_setup, cast2, c_drv, free2, dtor2 = cc_
    assert (cast, free, dtor) == (cast2, free2, dtor2)
    common = 'pw_(load(q+36));pw_(load(q+88));pw_(ctx.r5.u32);pw_(ctx.r6.u32);pw_(ctx.r7.u32&0xFF);pw_(ctx.r8.u32);pw_(ctx.r9.u32);pw_(ctx.r10.u32);'
    stubs = {
        s_ctor: '', c_ctor: '', free: '', dtor: '',
        cast: 'store(0x6000+16,8);ctx.r3.u64=0x6000;',
        s_drv: 'printf(" s");uint32_t q=ctx.r3.u32;for(int k=0;k<3;k++)pw_(load(q+144+4*k));pw_(load(q+176));' + common +
               'uint32_t n=load(q+2448);pw_(n);for(uint32_t k=0;k<n&&k<4;k++)pw_(load(q+1680+4*k));'
               'store(load(q+32),NQ_++==HIT_?0x9000:0);ctx.r3.u64=0;',
        c_drv: 'printf(" c");uint32_t q=ctx.r3.u32;for(int k=0;k<3;k++)pw_(load(q+144+4*k));for(int k=0;k<3;k++)pw_(load(q+160+4*k));'
               'pw_(load(q+208));pw_(*pointer(q+212));' + common +
               'uint32_t n=load(q+2480);pw_(n);for(uint32_t k=0;k<n&&k<4;k++)pw_(load(q+1712+4*k));'
               'store(load(q+32),NQ_++==HIT_?0x9000:0);ctx.r3.u64=0;',
    }
    body = ('static const uint32_t C[][%d]={' % W + nl + rows + nl + '};' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};'
            'for(int k=0;k<4;k++)put(0x828609E0+4*k,0x7F800000);'
            'auto bf=[](uint32_t u){float f;memcpy(&f,&u,4);return f;};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));NQ_=0;HIT_=int(c[23]);'
            'for(int i=0;i<4;i++)for(int k=0;k<3;k++)store(0x1100+16*i+4*k,c[3*i+k]);'
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;ctx.r5.u32=0x1110;ctx.r6.u32=c[13];'
            'ctx.r7.u32=0x1120;ctx.r8.u32=0x1130;ctx.r9.u32=c[14];ctx.f1.f64=bf(c[12]);'
            'sub_%s(ctx,nullptr);printf("%%d",int(ctx.r3.u32));' % A['F-85722'] +
            # the params at 0x1400
            'store(0x1400,c[15]);for(int k=0;k<3;k++)store(0x1404+4*k,c[16+k]);store(0x1410,c[19]);store(0x1414,0);'
            'store(0x1418,c[20]);store(0x141C,c[21]);store(0x1420,c[22]);'
            'Context x{};x.r1.u32=0x3F000;x.r3.u32=0x1000;x.r4.u32=0x1400;'
            'try{sub_%s(x,nullptr);printf(" %%d %%08x",int(x.r3.u32&0xFF),load(0x141C));}catch(std::exception&e){printf(" EXC %%s",e.what());}' % A['F-23371'] +
            'printf("' + NL + '");}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + A['F-85722'], 'sub_' + A['F-23371']], main_src, a.out_dir, 'rail_sweep_query', stubs=stubs, vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    bad = [o for o in out if 'EXC' in o]
    assert not bad, (len(bad), bad[0][:300])
    lines = [' '.join('%08x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that would read as addresses of the original are split by '_'; the test removes it)
    (a.out_dir / 'rail_sweep_query_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases,', sum(o.split()[0] == '1' for o in out), 'valid,', sum(o.split().count('s') for o in out), 'spheres,',
          sum(o.split().count('c') for o in out), 'capsules,', sum(o.split()[0] == '1' and o.split()[-2] == '1' for o in out), 'blocked')


if __name__ == '__main__':
    main()
