"""The torn-off bumper's contact callback run offline: F-80340 (its impact and scrape sounds from the piece's contacts
of the tick, DAMAGE_PARTS_RULE.md 2.5) on the generated code unchanged. The bodies' velocity at a point (their vtable
+84) is dispatched to a stand-in that writes the body's velocity (+16); the sound calls are stubbed and log their level.

Damage parts object (r3) at 0x1000: +1264 -> the TUNERS_DAMAGE map at 0x3000 (the disc's numbers, 4.3; the two hashes
stand-ins), +16 the sound object. The piece (r4) 0x1800: +528 -> its instance 0x2000 (+64 / +80 places, +96 the scrape's
time, +100 its loop). The contact list (r5) 0x1900: +0 -> pointers at 0x1C00, +4 the instance, +12 the count. Contact k at
0x4000 + 0x100 k (+48 / +64 the two points, +80 the normal, +160 -> the pair 0x4800 + 0x40 k, +165 the flags); the pair:
+0 / +4 the instances, +8 / +12 the bodies (0x5000 + 0x40 j: +0 -> the vtable 0x5800, +16 the velocity), +28 / +36
components. The clock 0x8286CF34.

Writes <out-dir>/piece_contact_ref.txt: one case a line: "loop now n | flags first my other vx vy vz ox oy oz nx ny nz
(x n) -> imp vol | scr vol pitch | time loop" (hex words; '-' where nothing was played). Test
`caraudio::tests::piece_contacts_match_the_original`."""
import argparse
import math
import random
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import bumper_step_reference as bs  # (sets up the closure build: the shim, the vector unit, the recomp's x86 flags)

recomp_closure.EXTRA = recomp_closure.EXTRA.replace('#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("unexpected indirect call")\n', '')
recomp_closure.EXTRA += 'static void dispatch(Context&,const uint8_t*,uint32_t);\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
fb, addr = bs.fb, bs.addr
TD = {127: 0x11111111, 131: 3.0, 135: 1.5, 139: 70.0, 143: 2.0, 147: 0x22222222, 151: 5.0, 155: 60.0, 159: 0.25, 163: 0.5,
      167: 5.0, 171: 100.0, 175: 0.7, 179: 1.5}
MAXK = 6


def unit(rnd):
    while True:
        v = [rnd.uniform(-1, 1) for _ in range(3)]
        n = math.sqrt(sum(x * x for x in v))
        if 0.1 < n <= 1:
            return [x / n for x in v]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    fn = 'sub_' + addr(a.tool, 'F-80340')
    init, scrape, impact = ('sub_' + addr(a.tool, k) for k in ('F-35212', 'F-30971', 'F-55796'))
    rnd = random.Random(19)
    cases = []
    for _ in range(a.cases):
        n = rnd.choice((0, 1, 1, 2, 3, 4, 5, MAXK))
        rows = []
        for k in range(n):
            flags = 0x80 | 0x40 if rnd.random() < 0.9 else rnd.choice((0x40, 0))
            first = rnd.choice((0, 1, 1, 2))   # 1: the piece is the pair's first, 0: second, 2: neither
            my = 0 if rnd.random() < 0.08 else 1
            other = 0 if rnd.random() < 0.3 else 1
            s = rnd.choice((0.5, 2.0, 5.0, 20.0))
            mv = [rnd.uniform(-s, s) for _ in range(3)]
            ov = [rnd.uniform(-s, s) for _ in range(3)] if rnd.random() < 0.5 else [0.0, 0.0, 0.0]
            nn = unit(rnd)
            if rnd.random() < 0.1:
                mv = [0.0, 0.0, 0.0]
                ov = [0.0, 0.0, 0.0]
            rows.append([flags, first, my, other] + mv + ov + nn)
        loop = 1 if rnd.random() < 0.3 else 0
        now = rnd.uniform(0, 500)
        cases.append((loop, now, rows))
    nl = chr(10)
    flat = []
    for loop, now, rows in cases:
        w = ['%d' % loop, '0x%08x' % fb(now), '%d' % len(rows)]
        for r in rows + [[0] * 13] * (MAXK - len(rows)):
            w += ['%d' % x for x in r[:4]] + ['0x%08x' % fb(float(x)) for x in r[4:]]
        flat.append('{' + ','.join(w) + '}')
    tdset = ''.join('store8s(0x3000+%d,0x%08x);' % (o, v if isinstance(v, int) else fb(v)) for o, v in TD.items())
    setup = ('memset(objects,0,0x7000);store(0x1000+1264,0x3000);' + tdset +
             'store(0x1800+528,0x2000);store(0x2000+100,c[0]?0x7777:0);store(0x2000+96,0);store(0x8286CF34,c[1]);'
             'store(0x1900,0x1C00);store(0x1904,0x2000);store(0x190C,c[2]);store(0x6000,0);store(0x6010,0);'
             'for(int k=0;k<c[2];k++){const uint32_t*r=c+3+13*k;uint32_t ct=0x4000+0x100*k,pr=0x4800+0x40*k,mb=0x5000+0x80*k,ob=mb+0x40;'
             'store(0x1C00+4*k,ct);*pointer(ct+165)=uint8_t(r[0]);store(ct+160,pr);'
             'store(pr+0,r[1]==1?0x2000:0x2400);store(pr+4,r[1]==0?0x2000:0x2400);'
             'store(pr+8,r[2]?mb:0);store(pr+12,r[3]?ob:0);store(pr+28,1);store(pr+36,2);'
             'store(mb,0x5800);store(ob,0x5800);store(0x5800+84,0x90000000);'
             'for(int i=0;i<3;i++){store(mb+16+4*i,r[4+i]);store(ob+16+4*i,r[7+i]);store(ct+80+4*i,r[10+i]);}store(mb+28,0);store(ob+28,0);}')
    # (the pair's +8 is the body of whichever instance is +0; the code takes +8 for itself when it is first, else +12:
    # so when the piece is second its body must be at +12 - the rows say which body is the piece's)
    setup += ('for(int k=0;k<c[2];k++){const uint32_t*r=c+3+13*k;uint32_t pr=0x4800+0x40*k,mb=0x5000+0x80*k,ob=mb+0x40;'
              'if(r[1]!=1){store(pr+12,r[2]?mb:0);store(pr+8,r[3]?ob:0);}}')
    outs = ('if(load(0x6000))printf("%08x | ",load(0x6004));else printf("- | ");'
            'if(load(0x6010))printf("%08x %08x | ",load(0x6014),load(0x6018));else printf("- | ");'
            'printf("%08x %08x' + chr(92) + 'n",load(0x2000+96),load(0x2000+100));')
    stubs = {init: '',
             scrape: 'store(0x6010,1);store(0x6014,load(ctx.r6.u32));store(0x6018,load(ctx.r6.u32+4));store(ctx.r5.u32,0x8888);',
             impact: 'store(0x6000,1);store(0x6004,load(ctx.r5.u32));'}
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){if(a!=0x90000000)throw std::runtime_error("indirect");'
            'for(int i=0;i<4;i++)store(ctx.r4.u32+4*i,load(ctx.r3.u32+16+4*i));}')
    body = ('static const uint32_t C[][%d]={' % (3 + 13 * MAXK) + nl + (',' + nl).join(flat) + nl + '};' + nl +
            'for(auto&cc:C){const uint32_t*c=cc;' + setup +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1800;ctx.r5.u32=0x1900;ctx.r13.u32=0x5C00;ctx.r1.u32=0x7F00;' + fn + '(ctx,nullptr);' + outs + '}')
    helpers = ('static void store8s(uint32_t a,uint32_t v){for(int i=0;i<4;i++)*pointer(a+i)=uint8_t(v>>(24-8*i));}' + nl)
    main_src = (helpers + disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [fn], main_src, a.out_dir, 'piececontact', stubs=stubs, vector=True)
    out = [x.strip() for x in recomp_closure.run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = []
    for (loop, now, rows), o in zip(cases, out):
        w = ['%d' % loop, '%08x' % fb(now), '%d' % len(rows)]
        for r in rows:
            w += ['|'] + ['%d' % x for x in r[:4]] + ['%08x' % fb(float(x)) for x in r[4:]]
        lines.append(' '.join(w) + ' -> ' + o)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'piece_contact_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
