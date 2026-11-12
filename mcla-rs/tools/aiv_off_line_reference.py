"""The race line's off-the-line test F-13872 run offline, the generated code whole and unchanged with its callees as
generated: the cell walk and clamp F-15125, the surface height F-84036, the reach test F-80559 (with F-91226 / F-32060 /
F-57591) and the vector constants from the static initialisers (as aiv_reach_reference.py).

The aiv (zeroed) holds: the car position +25184; the portal ring (start +22548, count +22552, base +22560; cell k at
aiv + 16 + 176 k: +0 / +16 raw ends a / b, +32 / +48 shrunk ends, +64 normal, +80 forward plane, +96 back plane, +112 / +128 /
+144 / +160 the side planes, w 0); the control-point ring (points at +25440 + 16 slot, start +26468, count +26472, base
+26480, 64 slots); the points' portal words (+26496 + 4 slot, start +26756, count +26760, base +26764: cell << 1 | flag);
the progress +30784. Slot of index i = (i - base + start + 1) mod N.

Each case: a corridor of 2 .. 10 portals (bends, widths, jitter; planes from the corridor's directions), 1 .. 10 control
points near the corridor with their cells, the progress (10 % at the ring's end: the line used up), the car near the line,
sometimes off it in plan by about 0.25 m, sometimes 5 m below or anywhere, the out pointer null in 10 %. Writes
<out-dir>/aiv_off_line_ref.txt, one line a case: "case" and the inputs (ints decimal, floats hex) "-> result out3 used next3" (out as
written, or the zeros it was cleared to; used = the line-used-up getter F-65527, next = the point the next-point getter
F-28749 points at)."""
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
OUT = 0x40410000


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def unit(x, z):
    n = math.hypot(x, z) or 1.0
    return (x / n, 0.0, z / n)


def case(rnd):
    n = rnd.randrange(2, 11)
    pstart, pbase = rnd.randrange(128), rnd.randrange(0, 500)
    heading, cx, cz = rnd.uniform(-math.pi, math.pi), rnd.uniform(-500, 500), rnd.uniform(-500, 500)
    y0 = rnd.uniform(-5, 40)
    step = rnd.uniform(2.0, 15.0)
    mids, cells = [], []
    for j in range(n):
        heading += rnd.gauss(0, 0.2)
        cx += math.sin(heading) * step
        cz += math.cos(heading) * step
        y = y0 + rnd.uniform(-0.5, 0.5)
        w = rnd.uniform(1.0, 8.0)
        rx, rz = math.cos(heading), -math.sin(heading)
        a = (cx - rx * w, y + rnd.uniform(-0.3, 0.3), cz - rz * w)
        b = (cx + rx * w, y + rnd.uniform(-0.3, 0.3), cz + rz * w)
        k = rnd.uniform(0.6, 0.95)
        sa = (cx - rx * w * k, a[1], cz - rz * w * k)
        sb = (cx + rx * w * k, b[1], cz + rz * w * k)
        nrm = (rnd.gauss(0, 0.05), 1.0, rnd.gauss(0, 0.05))
        mids.append((cx, y, cz, heading))
        cells.append([a, b, sa, sb, nrm])
    for j in range(n):
        h = mids[min(j + 1, n - 1)][3] if j + 1 < n else mids[j][3]
        fwd = unit(math.sin(h) + rnd.gauss(0, 0.02), math.cos(h) + rnd.gauss(0, 0.02))
        back = unit(-math.sin(mids[j][3]), -math.cos(mids[j][3]))
        ra = unit(-math.cos(mids[j][3]), math.sin(mids[j][3]))
        rb = (-ra[0], 0.0, -ra[2])
        cells[j] += [fwd, back, ra, rb, ra, rb]
    # (the control points: their cells along the corridor)
    m = rnd.randrange(1, 11)
    qstart, qbase = rnd.randrange(64), rnd.randrange(0, 300)
    wstart = rnd.randrange(64)
    pts, words = [], []
    cell = 0
    for i in range(m):
        cell = min(n - 2, cell + rnd.choice((0, 0, 1)))
        cx, y, cz, _ = mids[cell]
        pts.append((cx + rnd.gauss(0, 2), y + rnd.uniform(-0.5, 0.5), cz + rnd.gauss(0, 2)))
        words.append(((pbase + cell) << 1) | rnd.randrange(2))
    prog = qbase + m if rnd.random() < 0.1 else qbase + rnd.randrange(1, m + 1) if m > 1 else qbase + 1
    prog = min(prog, qbase + m)
    # (the car: near the point before the progress, sometimes just off in plan, 5 m below, or anywhere)
    ref = pts[max(0, min(m - 1, prog - qbase - 1))]
    r = rnd.random()
    if r < 0.1:
        pos = tuple(rnd.uniform(-600, 600) for _ in range(3))
    elif r < 0.3:
        ang = rnd.uniform(0, 2 * math.pi)
        d = rnd.uniform(0.0, 0.5)
        pos = (ref[0] + math.cos(ang) * d, ref[1] + rnd.uniform(-1, 1), ref[2] + math.sin(ang) * d)
    elif r < 0.4:
        pos = (ref[0], ref[1] - rnd.uniform(4.5, 6.0), ref[2])
    else:
        pos = (ref[0] + rnd.gauss(0, 3), ref[1] + rnd.uniform(-1, 2), ref[2] + rnd.gauss(0, 3))
    return dict(pstart=pstart, pbase=pbase, n=n, cells=cells, qstart=qstart, qbase=qbase, m=m, wstart=wstart,
                pts=pts, words=words, prog=prog, pos=pos, out=rnd.random() >= 0.1)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    off, used, nxt = addr(a.tool, 'F-13872'), addr(a.tool, 'F-65527'), addr(a.tool, 'F-28749')
    inits = [addr(a.tool, f) for f in INITS]
    rnd = random.Random(13872)
    cases = [case(rnd) for _ in range(a.cases)]
    nl = chr(10)
    flat = []
    for c in cases:
        flat += [c['pstart'], c['pbase'], c['n'], c['qstart'], c['qbase'], c['m'], c['wstart'], c['prog'], int(c['out'])]
        flat += [fb(v) for v in c['pos']]
        for cl in c['cells']:
            for vec in cl:
                flat += [fb(v) for v in vec]
        for p in c['pts']:
            flat += [fb(v) for v in p]
        flat += c['words']
    src = ('static const uint32_t CS[]={%s};' % ','.join('%du' % w for w in flat) + nl + r'''
static void put3(uint32_t at,const uint32_t*v){store(at,v[0]);store(at+4,v[1]);store(at+8,v[2]);store(at+12,0);}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
rt_reset();
@INITS@
const uint32_t*s=CS;
const uint32_t A=@AIV@;
for(int c=0;c<@N@;c++){
 uint32_t ps=*s++,pb=*s++,n=*s++,qs=*s++,qb=*s++,m=*s++,ws=*s++,prog=*s++,out=*s++;const uint32_t*pos=s;s+=3;
 for(uint32_t o=0;o<31232;o+=4)store(A+o,0);
 store(A+22548,ps);store(A+22552,n);store(A+22560,pb);
 store(A+26468,qs);store(A+26472,m);store(A+26480,qb);
 store(A+26756,ws);store(A+26760,m);store(A+26764,qb);
 store(A+30784,prog);put3(A+25184,pos);
 printf("case %u %u %u %u %u %u %u %u %u %08x %08x %08x",ps,pb,n,qs,qb,m,ws,prog,out,pos[0],pos[1],pos[2]);
 for(uint32_t j=0;j<n;j++){uint32_t at=A+16+176*((j+ps+1)%128);
  for(int f=0;f<11;f++){put3(at+16*f,s);for(int i=0;i<3;i++)printf(" %08x",s[i]);s+=3;}}
 for(uint32_t i=0;i<m;i++){put3(A+25440+16*((i+qs+1)%64),s);for(int k=0;k<3;k++)printf(" %08x",s[k]);s+=3;}
 for(uint32_t i=0;i<m;i++){store(A+26496+4*((i+ws+1)%64),*s);printf(" %u",*s);s++;}
 for(uint32_t o=0;o<16;o+=4)store(@OUT@+o,0);
 Context k=fresh();k.r3.u32=A;k.r4.u32=out?@OUT@:0;k.r5.u32=0;
 sub_@OFF@(k,nullptr);
 printf(" -> %u",k.r3.u32&0xff);for(int i=0;i<3;i++)printf(" %08x",load(@OUT@+4*i));
 {Context g=fresh();g.r3.u32=A;sub_@USED@(g,nullptr);printf(" %u",g.r3.u32&0xff);}
 {Context g=fresh();g.r3.u32=A;sub_@NEXT@(g,nullptr);for(int i=0;i<3;i++)printf(" %08x",load(g.r3.u32+4*i));}
 printf("\n");}}
''')
    for kk, v in (('@N@', str(a.cases)), ('@AIV@', '%du' % AIV), ('@OUT@', '%du' % OUT), ('@OFF@', off), ('@USED@', used), ('@NEXT@', nxt),
                  ('@INITS@', ''.join('{Context k=fresh();sub_%s(k,nullptr);}' % f for f in inits))):
        src = src.replace(kk, v)
    exe = rd.build(rd.index(a.recomp), ['sub_' + off, 'sub_' + used, 'sub_' + nxt] + ['sub_' + f for f in inits], src, a.out_dir, 'offline',
                   image=rd.load_image(a.image))
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'aiv_off_line_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases', sum(' -> 1 ' in l for l in lines), 'off the line')


if __name__ == '__main__':
    main()
