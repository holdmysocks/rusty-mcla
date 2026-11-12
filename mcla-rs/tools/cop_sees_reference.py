"""Run the original's cop sight test F-94676 offline and record it, for the port's `police::sees_pre`
(crates/game/src/testdata/cop_sees_ref.txt; test police::tests::cop_sees_matches_the_original).

F-94676(cop racer r3, target racer r4, radius f1, half angle f2): a cop with +465 (a police racer) and +880 (underground)
both set sees nothing. Else with r > 0 the squared distance of the two matrices' origins (F-35658 gives the matrix: row 2
+32, origin +48), dy dy then dz and dx fused, over r r -> no. With half > 0 the 3D angle F-88594 of (target - cop) and
(0 - row 2) of the cop, not under half x 0.0174533 -> no. Then the line test F-69576 (its own row; stubbed here: the case's
answer, the two points recorded) from the target's origin 2 m up to the cop's origin 2 m up decides. The matrix getter is
stubbed (the matrix at racer + 0x1000); F-88594 and the C runtime's acos run as generated.

  <b465> <b880> <cop x y z> <row2 x y z> <tgt x y z> <r> <half> <line> (floats as bits) ->
  <result> <line called> [<a x y z> <b x y z>]
"""
import argparse
import math
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t C=0x40800000u, T=0x40900000u, G=0x40A00000u;
 while(in>>tok){ if(tok!="case") break; uint32_t b465,b880,w[11],line; in>>b465>>b880; for(int k=0;k<11;k++) in>>std::hex>>w[k]; in>>std::dec>>line;
  rt_reset(); for(uint32_t a=G;a<G+64;a+=4) store(a,0);
  store(C+464,b465<<16|0u); store(C+880,b880<<24);
  for(int k=0;k<3;k++){ store(C+0x1000+32+4*k,w[3+k]); store(C+0x1000+48+4*k,w[k]); store(T+0x1000+48+4*k,w[6+k]); }
  store(C+0x1000+44,0); store(C+0x1000+60,0x3f800000u); store(T+0x1000+60,0x3f800000u); store(T+0x1000+32,0); store(T+0x1000+36,0); store(T+0x1000+40,0); store(T+0x1000+44,0);
  store(G,line);
  Context c=fresh(); c.r3.u32=C; c.r4.u32=T; c.r5.u32=1; uint32_t r=w[9],h=w[10]; float rf,hf; memcpy(&rf,&r,4); memcpy(&hf,&h,4); c.f1.f64=rf; c.f2.f64=hf;
  SEES(c,nullptr);
  printf("%u %u",(unsigned)(c.r3.u32&0xff),load(G+4));
  if(load(G+4)) for(int k=0;k<6;k++) printf(" %08x",load(G+8+4*k));
  printf("\n"); }
 printf("end\n");
}
'''


def b(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


def unit(g):
    while True:
        v = [g.uniform(-1, 1) for _ in range(3)]
        n = math.sqrt(sum(t * t for t in v))
        if n > 0.1:
            return [t / n for t in v]


def cases():
    g = random.Random(94676)
    out = []
    for i in range(6000):
        cop = [g.uniform(-3000, 3000), g.uniform(-20, 120), g.uniform(-3000, 3000)]
        row2 = unit(g)
        if g.random() < 0.3:     # a car on flat ground: row 2 level
            a = g.uniform(-math.pi, math.pi)
            row2 = [math.sin(a), 0.0, math.cos(a)]
        r = g.choice([0.0, 15.0, 40.0, 100.0, 150.0, 300.0, g.uniform(1, 400), -5.0])
        half = g.choice([0.0, 45.0, 90.0, 180.0, g.uniform(1, 179), -1.0])
        d = g.uniform(0, 1.3 * max(r, 50))
        dirv = unit(g)
        if g.random() < 0.15:    # on the radius
            d = r
        if g.random() < 0.2 and half > 0:     # on the cone's edge
            f = [-t for t in row2]
            k = unit(g)
            ax = [f[1] * k[2] - f[2] * k[1], f[2] * k[0] - f[0] * k[2], f[0] * k[1] - f[1] * k[0]]
            n = math.sqrt(sum(t * t for t in ax)) or 1
            ax = [t / n for t in ax]
            th = math.radians(half)
            dirv = [f[j] * math.cos(th) + (ax[(j + 1) % 3] * f[(j + 2) % 3] - ax[(j + 2) % 3] * f[(j + 1) % 3]) * math.sin(th) for j in range(3)]
        tgt = [cop[j] + dirv[j] * d for j in range(3)]
        if g.random() < 0.02:
            tgt = list(cop)
        b465 = 1 if g.random() < 0.9 else 0
        b880 = 1 if g.random() < 0.1 else 0
        out.append((b465, b880, [b(x) for x in cop + row2 + tgt + [r, half]], g.randrange(2)))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-94676', 'F-35658', 'F-69576']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    assert len(addr) == len(ids), addr
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    stubs = {name('F-35658'): 'ctx.r3.u64=ctx.r3.u32+0x1000u;',
             name('F-69576'): ('store(0x40A00004u,1u); for(int k=0;k<3;k++){store(0x40A00008u+4*k,ctx.v2.u32[3-k]); '
                               'store(0x40A00014u+4*k,ctx.v1.u32[3-k]);} ctx.r3.u64=load(0x40A00000u);')}
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-94676')], MAIN.replace('SEES', name('F-94676')), a.out, 'cop_sees',
                      image=img, stubs=stubs)
    cs = cases()
    cf = a.out / 'cop_sees_cases.txt'
    cf.write_text(''.join('case %d %d %s %d\n' % (c[0], c[1], ' '.join('%08x' % w for w in c[2]), c[3]) for c in cs) + 'stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True, timeout=120)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    res = r.stdout.split('\n')[:-2]
    assert len(res) == len(cs)
    rows = ['%d %d %s %d -> %s' % (c[0], c[1], ' '.join('%08x' % w for w in c[2]), c[3], o) for c, o in zip(cs, res)]
    a.ref.write_text('\n'.join(rows) + '\n')
    seen = sum(o.startswith('1') for o in res)
    called = sum(o.split()[1] == '1' for o in res)
    print('cases', len(rows), 'seen', seen, 'line tested', called, '->', a.ref)


if __name__ == '__main__':
    main()
