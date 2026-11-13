"""The traffic spawn's slot test against one lane (F-84149) run offline: the generated code whole and unchanged on flat
guest memory. The lanes' slots 12 / 13 (the first and the last point, +48 / +52) and 21 (the car list's count, +84), the
cars' obstacle objects' slots 4 / 6 (width, length: +16 / +24) are dispatched to the case's values; the list walk
F-25314 is stubbed (the case's i-th car); the car-object getter F-88657 runs in the closure (+500 bit 0x20 and +52: the
proxy). Memory: our lane 0x1000 (vtable 0x1100), the other lane 0x1200 (vtable 0x1300) or ours again, their points at
0x1800 / 0x1810 / 0x1820 / 0x1830, the spot P 0x1840, our size 0x1850 (+0 width, +8 length), the cars 0x2000 + 0x200 i
(+52 the proxy 0x6000 + 0x100 i -> +48 -> +28 the matrix 0x8000 + 0x100 i, place at +64; else the place at +176; +496
the obstacle object 0x4000 + 0x40 i, vtable 0x5000), r13's thread byte 0 (no ring buffer), the xyz mask 0x82860950 set as the running game has it.

A case: same lane, our first / last point, the other's first / last, P, our width and length, the count, our own car's
index (-1 none), then six cars: proxy, place, width, length. Writes <out-dir>/slot_test_ref.txt, a case a line: the
case's words '->' the answer (1 clear, 0 not)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\nstatic const uint32_t *K;\n')
NL = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def clean(c):
    return all(not (0x82000000 <= w <= 0x82FFFFFF) for w in c)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn, walk = addr(a.tool, 'F-84149'), addr(a.tool, 'F-25314')
    rnd = random.Random(84149)
    cs = []
    while len(cs) < 4000:
        p = [rnd.uniform(-3000, 3000), rnd.uniform(0, 60), rnd.uniform(-3000, 3000)]
        ow, ol = rnd.uniform(1.6, 2.6), rnd.uniform(3, 12)
        pts = [[rnd.uniform(-3000, 3000), rnd.uniform(0, 60), rnd.uniform(-3000, 3000)] for _ in range(4)]
        mode = rnd.randrange(6)
        if mode == 1:
            pts[2] = list(pts[1])  # the other's first = our last
        elif mode == 2:
            pts[3] = list(pts[0])  # the other's last = our first
        elif mode == 3:
            pts[2] = list(pts[1])
            pts[2][rnd.randrange(3)] += 1e-3  # nearly
        elif mode == 4:
            pts[2] = [0.0, pts[1][1], pts[1][2]]
            pts[1][0] = -0.0  # +0 against -0
        n = rnd.randrange(7)
        own = rnd.randrange(n) if n and rnd.random() < 0.3 else -1
        cars = []
        for _ in range(6):
            k = rnd.choice((2.0, 6.0, 12.0))
            q = [p[0] + rnd.uniform(-k, k), p[1] + rnd.choice((0.0, rnd.uniform(-7, 7), 5.0, -5.0, rnd.uniform(-5, 5))), p[2] + rnd.uniform(-k, k)]
            if rnd.random() < 0.02:
                q[1] = float('nan')
            cars.append([int(rnd.random() < 0.15), *map(fb, q), fb(rnd.uniform(1.6, 2.8)), fb(rnd.uniform(3, 14))])
        c = [int(mode == 0), *[fb(x) for pt in pts for x in pt], *map(fb, p), fb(ow), fb(ol), n, own & 0xFFFFFFFF] + [w for car in cars for w in car]
        if clean(c):
            cs.append(c)
    W = len(cs[0])
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){uint32_t o=ctx.r3.u32;'
            'if(a==0x9048)ctx.r3.u32=o==0x1000?0x1800:0x1820;'
            'else if(a==0x9052)ctx.r3.u32=o==0x1000?0x1810:0x1830;'
            'else if(a==0x9084)store(o,K[18]);'
            'else if(a==0x9016)ctx.f1.f64=getf(o+0x10);'
            'else if(a==0x9024)ctx.f1.f64=getf(o+0x14);'
            'else throw std::runtime_error("unexpected indirect call");}')
    stubs = {'sub_' + walk: 'if(ctx.r3.u32!=(K[0]?0x1000u:0x1200u))printf("badlane ");'
                            'ctx.r3.u64=ctx.r4.u32<K[18]?0x2000+0x200*ctx.r4.u32:0;'}
    rows = NL.join('{%s},' % ','.join('0x%08xu' % x for x in c) for c in cs)
    body = ('static const uint32_t C[][%d]={' % W + NL + rows + NL + '};' + NL + disp + NL +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + NL +
            'for(auto&c:C){memset(objects,0,sizeof(objects));K=c;'
            '/* the xyz mask as the running game has it (zero in the image file) */'
            'for(int k=0;k<3;k++)store(0x82860950u+4*k,0xFFFFFFFFu);store(0x8286095Cu,0);'
            'store(0x1000,0x1100);store(0x1200,0x1300);'
            'for(uint32_t v:{0x1100u,0x1300u}){store(v+48,0x9048);store(v+52,0x9052);store(v+84,0x9084);}'
            'store(0x5000+16,0x9016);store(0x5000+24,0x9024);'
            'for(int j=0;j<12;j++)store(0x1800+(j/3)*16+(j%3)*4,c[1+j]);'
            'for(int j=0;j<3;j++)store(0x1840+4*j,c[13+j]);store(0x1850,c[16]);store(0x1858,c[17]);'
            'for(int i=0;i<6;i++){const uint32_t*w=c+20+6*i;uint32_t car=0x2000+0x200*i,ob=0x4000+0x40*i;'
            'store(car+496,ob);store(ob,0x5000);store(ob+0x10,w[4]);store(ob+0x14,w[5]);'
            'if(w[0]){uint32_t px=0x6000+0x100*i,in=px+0x80,m=0x8000+0x100*i;*pointer(car+500)=0x20;store(car+52,px);store(px,0x5000);'
            'store(px+0x10,w[4]);store(px+0x14,w[5]);store(px+48,in);store(in+28,m);for(int j=0;j<3;j++)store(m+64+4*j,w[1+j]);}'
            'else for(int j=0;j<3;j++)store(car+176+4*j,w[1+j]);}'
            'store(0x1F00,0x1F40);'
            'Context ctx{};ctx.r1.u32=0xF000;ctx.r13.u32=0x1F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1840;ctx.r5.u32=0x1850;'
            'ctx.r6.u32=c[0]?0x1000:0x1200;ctx.r7.u32=c[19]==0xFFFFFFFFu?0x1F80:0x2000+0x200*c[19];'
            'sub_FN(ctx,nullptr);printf("A%xEOL",ctx.r3.u32);}}' + NL).replace('FN', fn).replace('EOL', chr(92) + 'n')
    exe = build(index(a.recomp), ['sub_' + fn], body, a.out_dir, 'slot_test', stubs=stubs, vector=True, scratch_end=0x10000)
    out = [x for x in run(exe, a.image).split(NL) if x]
    assert len(out) == len(cs), (len(out), len(cs))
    lines = []
    for c, o in zip(cs, out):
        assert 'badlane' not in o, o
        lines.append(' '.join('%x' % x for x in c) + ' -> ' + o.split()[-1][1:])
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'slot_test_ref.txt').write_text(NL.join(lines) + NL)
    print(len(lines), 'cases', sum(l.endswith(' 0') for l in lines), 'blocked')


if __name__ == '__main__':
    main()
