"""Run the original's wrong-way condition F-29334 offline and record it, for the port's `police::wrong_way_pre`
(crates/game/src/testdata/wrong_way_ref.txt; test police::tests::wrong_way_matches_the_original).

F-29334(racer r4): over an ai_shortcut polygon (F-66554 on the matrix's origin; stubbed: the case's flag) -> no. Else the racer's
virtual slot 40 (stubbed: the case's flag), its car +48, the car's +8 and the lane +640 must all be there. Then the speed (+8's
+224) under fMovingSpeed x 0.447: the matrix's row 2 (F-35658 stubbed: the matrix at racer + 0x1000) against the kept tangent
+624..+632 (y first, then z and x fused) over 0 -> yes; else the velocity (+8's +128 object, +272..+280) against the tangent,
same order with the x term's operands swapped, under 0 -> yes.

  <shortcut> <vcall> <car> <obj> <lane> <speed> <moving> <row2 xyz> <tan xyz> <vel xyz> (floats as bits) -> <result>
"""
import argparse
import math
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

TUNE = 0x8282CAC8 + 312      # fMovingSpeed in the police tune globals (the function's lis / addi / lfs)

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t R=0x40800000u, VT=0x40880000u, CH=0x40810000u, O=0x40820000u, V=0x40830000u, G=0x40A00000u;
 while(in>>tok){ if(tok!="case") break; uint32_t sc,vc,car,obj,lane,w[11]; in>>sc>>vc>>car>>obj>>lane; for(int k=0;k<11;k++) in>>std::hex>>w[k]; in>>std::dec;
  rt_reset(); store(G,sc); store(G+4,vc);
  store(R,VT); store(VT+40,0x7F000040u); store(R+48,car?CH:0); store(CH+8,obj?O:0); store(R+640,lane?0x40840000u:0);
  store(O+224,w[0]); store(TUNEADDR,w[1]); store(O+128,V);
  for(int k=0;k<3;k++){ store(R+0x1000+32+4*k,w[2+k]); store(R+624+4*k,w[5+k]); store(V+272+4*k,w[8+k]); }
  Context c=fresh(); c.r3.u32=0x40870000u; c.r4.u32=R;
  WW(c,nullptr);
  printf("%u\n",(unsigned)(c.r3.u32&0xff)); }
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


def perp(g, t):
    """A unit vector at right angles to t, or nearly: the sign of the dot product decided by rounding."""
    k = unit(g)
    d = sum(a * c for a, c in zip(k, t))
    p = [a - d * c for a, c in zip(k, t)]
    n = math.sqrt(sum(a * a for a in p)) or 1
    return [a / n for a in p]


def cases():
    g = random.Random(29334)
    out = []
    for i in range(8000):
        sc = 1 if g.random() < 0.08 else 0
        vc = 1 if g.random() < 0.94 else 0
        car = 1 if g.random() < 0.97 else 0
        obj = 1 if g.random() < 0.97 else 0
        lane = 1 if g.random() < 0.94 else 0
        moving = g.choice([1.0, 1.0, 1.0, 0.0, 2.5, g.uniform(0, 5)])
        thr = struct.unpack('<f', struct.pack('<f', moving * 0.447))[0]
        speed = g.choice([0.0, g.uniform(0, 1), thr, thr * 1.0000001, g.uniform(0, 60), g.uniform(0.3, 0.6)])
        tan = unit(g)
        if g.random() < 0.4:      # a level lane
            a = g.uniform(-math.pi, math.pi)
            tan = [math.sin(a), 0.0, math.cos(a)]
        row2 = unit(g) if g.random() < 0.7 else perp(g, tan)
        vel = [t * g.uniform(0, 40) for t in unit(g)]
        if g.random() < 0.25:
            vel = [t * g.uniform(0, 40) for t in perp(g, tan)]
        if g.random() < 0.05:
            vel = [0.0, 0.0, 0.0]
        out.append((sc, vc, car, obj, lane, [b(x) for x in [speed, moving] + row2 + tan + vel]))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-29334', 'F-35658', 'F-66554']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    assert len(addr) == len(ids), addr
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    stubs = {name('F-35658'): 'ctx.r3.u64=ctx.r3.u32+0x1000u;',
             name('F-66554'): 'ctx.r3.u64=load(0x40A00000u);'}
    fakes = {0x7F000040: 'ctx.r3.u64=load(0x40A00004u);'}
    src = MAIN.replace('WW', name('F-29334')).replace('TUNEADDR', '0x%08Xu' % TUNE)
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-29334')], src, a.out, 'wrong_way', image=img, stubs=stubs, fakes=fakes)
    cs = cases()
    cf = a.out / 'wrong_way_cases.txt'
    line = lambda c: '%d %d %d %d %d %s' % (*c[:5], ' '.join('%08x' % w for w in c[5]))
    cf.write_text(''.join('case %s\n' % line(c) for c in cs) + 'stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True, timeout=120)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    res = r.stdout.split('\n')[:-2]
    assert len(res) == len(cs)
    a.ref.write_text(''.join('%s -> %s\n' % (line(c), o) for c, o in zip(cs, res)))
    print('cases', len(res), 'wrong way', sum(o == '1' for o in res), '->', a.ref)


if __name__ == '__main__':
    main()
