"""The race line's reach test F-80559 (can a point in one portal cell be reached from a point in another, bending the line at
most once round each shrunk portal end) run offline, the generated code whole and unchanged: its segment-portal test F-91226,
the angle F-32060 and the rotation F-57591 as generated, the vector constants from the static initialisers (as
ball_curved_reference.py).

The aiv (zeroed) holds only the portal ring: start at +22548, base at +22560 (the reach reads them from the follower core,
aiv + 16, at +22532 / +22544), the shrunk ends of cell k at aiv + 16 + 176 k + 32 / + 48 (w 0); slot of index i =
(i - base + start + 1) mod 128.

Each case: a corridor of 2 .. 12 portals (mostly across a road heading along z with random bends, widths and jitter, some
reversed or degenerate, some random), a random ring start (0 .. 127, so the ring wraps) and base, the cells a and b of the
corridor (b above, below or equal to a), x near cell a and p near cell b (or anywhere). Writes
<out-dir>/aiv_reach_ref.txt: "case start base n a b x3 p3 {sa3 sb3 a3 b3}... -> reach corridor3" (floats as hex words).
Also the corridor point ahead F-43876 on the same ring (the cell ends a / b at +0 / +16 of the core's cell)."""
import argparse
import math
import random
import re
import struct
from pathlib import Path
import recomp_dispatch as rd
import recomp_closure
from stuck_reference import addr

# (vrlimi128 -> simde_mm_permute_ps, an AVX intrinsic: the SSE shuffle, as tools/junction_force_reference.py)
recomp_closure.EXTRA += '#define simde_mm_permute_ps(a,imm) simde_mm_shuffle_ps(a,a,imm)' + chr(10)
INITS = ['F-50173', 'F-33402', 'F-26199', 'F-55749', 'F-97349', 'F-70580', 'F-79721', 'F-64228', 'F-24192', 'F-92824']
AIV = 0x40400000
X, PT, OUT = 0x40410000, 0x40410010, 0x40410020


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def case(rnd):
    n = rnd.randrange(2, 13)
    start, base = rnd.randrange(128), rnd.randrange(0, 2000)
    if rnd.random() < 0.15:
        n, base = 2, 0                                                # (the corridor point's two-portal mean)
    cells = []
    heading, cx, cz = rnd.uniform(-0.3, 0.3), rnd.uniform(-500, 500), rnd.uniform(-500, 500)
    step = rnd.uniform(1.0, 15.0)
    for j in range(n):
        if rnd.random() < 0.08:
            cells.append(tuple(rnd.uniform(-30, 30) for _ in range(12)))
            continue
        heading += rnd.gauss(0, 0.25)
        cx += math.sin(heading) * step
        cz += math.cos(heading) * step
        w = rnd.uniform(0.0, 8.0) if rnd.random() < 0.95 else 0.0
        off = rnd.uniform(-2, 2)
        ax, az = cx + math.cos(heading) * (-w + off), cz - math.sin(heading) * (-w + off)
        bx, bz = cx + math.cos(heading) * (w + off), cz - math.sin(heading) * (w + off)
        y = rnd.uniform(-5, 40)
        sa, sb = (ax, y + rnd.uniform(-1, 1), az), (bx, y + rnd.uniform(-1, 1), bz)
        if rnd.random() < 0.1:
            sa, sb = sb, sa
        ea = (ax + rnd.gauss(0, 0.5), y, az + rnd.gauss(0, 0.5))
        eb = (bx + rnd.gauss(0, 0.5), y, bz + rnd.gauss(0, 0.5))
        cells.append(sa + sb + ea + eb)
    a, b = rnd.randrange(n), rnd.randrange(n)
    if rnd.random() < 0.1:
        b = a

    def near(k):
        c = cells[k]
        if rnd.random() < 0.15:
            return tuple(rnd.uniform(-600, 600) for _ in range(3))
        t = rnd.uniform(-0.3, 1.3)
        return (c[0] + (c[3] - c[0]) * t + rnd.gauss(0, 2), c[1] + rnd.uniform(-1, 1), c[2] + (c[5] - c[2]) * t + rnd.gauss(0, 2))
    return start, base, n, a, b, near(a), near(b), cells


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    reach, corr = addr(a.tool, 'F-80559'), addr(a.tool, 'F-43876')
    inits = [addr(a.tool, f) for f in INITS]
    rnd = random.Random(80559)
    cases = [case(rnd) for _ in range(a.cases)]
    nl = chr(10)
    flat = []
    for start, base, n, ca, cb, x, p, cells in cases:
        flat += [start, base, n, ca, cb] + [fb(v) for v in x + p]
        for c in cells:
            flat += [fb(v) for v in c]
    src = ('static const uint32_t CS[]={%s};' % ','.join('%du' % w for w in flat) + nl + r'''
static void put3(uint32_t at,const uint32_t*v){store(at,v[0]);store(at+4,v[1]);store(at+8,v[2]);store(at+12,0);}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
rt_reset();
@INITS@
const uint32_t*s=CS;
for(int c=0;c<@N@;c++){
 uint32_t start=*s++,base=*s++,n=*s++,ca=*s++,cb=*s++;const uint32_t*x=s;s+=3;const uint32_t*p=s;s+=3;
 for(uint32_t o=0;o<31232;o+=4)store(@AIV@+o,0);
 store(@AIV@+22548,start);store(@AIV@+22560,base);store(@AIV@+22552,n);
 printf("case %u %u %u %u %u",start,base,n,ca,cb);for(int i=0;i<3;i++)printf(" %08x",x[i]);for(int i=0;i<3;i++)printf(" %08x",p[i]);
 for(uint32_t j=0;j<n;j++){const uint32_t*cl=s;s+=12;for(int i=0;i<12;i++)printf(" %08x",cl[i]);
  uint32_t slot=(j+start+1)%128;uint32_t at=@AIV@+16+176*slot;put3(at+32,cl);put3(at+48,cl+3);put3(at,cl+6);put3(at+16,cl+9);}
 put3(@X@,x);put3(@PT@,p);
 Context k=fresh();k.r3.u32=@AIV@+16;k.r4.u32=base+ca;k.r5.u32=@X@;k.r6.u32=base+cb;k.r7.u32=@PT@;
 sub_@REACH@(k,nullptr);
 printf(" -> %u",k.r3.u32&0xff);
 for(uint32_t o=0;o<16;o+=4)store(@OUT@+o,0);
 {Context q=fresh();q.r3.u32=@OUT@;q.r4.u32=@AIV@;sub_@CORR@(q,nullptr);}
 for(int i=0;i<3;i++)printf(" %08x",load(@OUT@+4*i));printf("\n");}}
''')
    for kk, v in (('@N@', str(a.cases)), ('@AIV@', '%du' % AIV), ('@X@', '%du' % X), ('@PT@', '%du' % PT), ('@REACH@', reach), ('@CORR@', corr), ('@OUT@', '%du' % OUT),
                  ('@INITS@', ''.join('{Context k=fresh();sub_%s(k,nullptr);}' % f for f in inits))):
        src = src.replace(kk, v)
    exe = rd.build(rd.index(a.recomp), ['sub_' + reach, 'sub_' + corr] + ['sub_' + f for f in inits], src, a.out_dir, 'reach',
                   image=rd.load_image(a.image))
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'aiv_reach_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases', sum(' -> 1 ' in l for l in lines), 'true')


if __name__ == '__main__':
    main()
