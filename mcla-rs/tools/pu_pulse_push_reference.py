"""Pulse's push F-19990 run offline (the original's generated code unchanged, vector mode). r3 the type object (+112 Force,
+128 Torque, +144 ForceDir), r4 the car (+28 the vehicle: its matrix rows at +16 .. +64, +4 the body whose vtable slot 7 gives the
mass in f1 and slot 9 a pointer to the inertia vector). Stubbed (said so): the two simulator hand-overs F-33609 (force: r5 points at
the world force x mass) and F-77639 (torque: v1 = world torque x inertia) - recording stubs; the body's two virtual calls are host
functions answering the case's mass and inertia vector.

Line (hex float patterns): "<row0 3> <row1 3> <row2 3> <force 3> <torque 3> <force_dir 3> <mass> <inertia 3> -> <force x mass 3> <torque x inertia 3>"
(the unused fourth lane of every vector is a filler: the code ignores it).
Usage: pu_pulse_push_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import numpy as np
import recomp_dispatch as rd
from cite_safe import cite_safe

MAIN = r'''
uint32_t rec[8];
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int k=0;k<3;k++){store(0x82860950+4*k,0xFFFFFFFFu);store(0x82860930+4*k,0);} store(0x8286095C,0); store(0x8286093C,0x3F800000u);
 const uint32_t T=0x1000,CAR=0x1200,V=0x1300,B=0x1500,VT=0x1600,MASS=0x1700,INERTIA=0x1710;
 uint32_t w[22];
 while(true){bool ok=true;for(int k=0;k<22;k++)ok=ok&&std::scanf("%x",&w[k])==1;if(!ok)break;
  rt_reset();std::memset(rec,0,sizeof rec);
  for(int r=0;r<3;r++){for(int k=0;k<3;k++)store(V+16+16*r+4*k,w[3*r+k]);store(V+16+16*r+12,0x3f000000u);}
  for(int v=0;v<3;v++){for(int k=0;k<3;k++)store(T+112+16*v+4*k,w[9+3*v+k]);store(T+112+16*v+12,0x40a00000u);}
  store(CAR+28,V);store(V+4,B);store(B,VT);store(VT+28,0x7F001100u);store(VT+36,0x7F001200u);store(V+8,0x00050000u);
  store(MASS,w[18]);for(int k=0;k<3;k++)store(INERTIA+4*k,w[19+k]);store(INERTIA+12,0x3f800000u);
  Context c=fresh();c.r3.u32=T;c.r4.u32=CAR;
  sub_PUSH(c,nullptr);
  std::printf("->");for(int k=0;k<6;k++)std::printf(" %x",rec[k]);std::printf("\n");}
}
'''

STUB_FORCE = 'extern uint32_t rec[];for(int k=0;k<3;k++)rec[k]=load(ctx.r5.u32+4*k);'
STUB_TORQUE = 'extern uint32_t rec[];for(int k=0;k<3;k++)rec[3+k]=ctx.v1.u32[3-k];'


def bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return float(np.float32(x))


def rows(rnd):
    kind = rnd.choice(['id', 'yaw', 'free', 'free', 'axis', 'scaled'])
    if kind == 'id':
        r = [(1, 0, 0), (0, 1, 0), (0, 0, 1)]
    elif kind == 'axis':
        r = rnd.choice([[(0, 0, 1), (0, 1, 0), (-1, 0, 0)], [(-1, 0, 0), (0, 1, 0), (0, 0, -1)], [(0, 0, -1), (0, 1, 0), (1, 0, 0)]])
    elif kind == 'yaw':
        a = rnd.uniform(0, 2 * math.pi)
        c, s = math.cos(a), math.sin(a)
        r = [(c, 0, -s), (0, 1, 0), (s, 0, c)]
    else:
        v = np.array([rnd.gauss(0, 1) for _ in range(4)])
        w, x, y, z = v / np.linalg.norm(v)
        r = [(1 - 2 * (y * y + z * z), 2 * (x * y + w * z), 2 * (x * z - w * y)),
             (2 * (x * y - w * z), 1 - 2 * (x * x + z * z), 2 * (y * z + w * x)),
             (2 * (x * z + w * y), 2 * (y * z - w * x), 1 - 2 * (x * x + y * y))]
        if kind == 'scaled':
            k = rnd.choice([0.5, 2.0, 100.0])
            r = [tuple(c * k for c in row) for row in r]
    return [f32(c) for row in r for c in row]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    addr = lambda f: 'sub_' + subprocess.run([a.tool, 'registry', 'addr', f], capture_output=True, text=True, check=True).stdout.split()[1]
    push, fh, th = addr('F-19990'), addr('F-33609'), addr('F-77639')
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    exe = rd.build(idx, [push], MAIN.replace('sub_PUSH', push), a.work, 'pu_pulse', image=img, stubs={fh: STUB_FORCE, th: STUB_TORQUE},
                   fakes={0x7F001100: 'ctx.f1.f64=getf(0x1700);', 0x7F001200: 'ctx.r3.u32=0x1710;'})
    rnd = random.Random(19990)
    cases = []
    # the real Pulse tune: Force, Torque, ForceDir are file values; cases around them and random ones
    for _ in range(4000):
        r = rows(rnd)
        vec = lambda: [f32(rnd.choice([0.0, -0.0, 1.0, -1.0, 2.5, 5.0, 100.0, rnd.uniform(-10, 10), rnd.uniform(-1, 1)])) for _ in range(3)]
        force, torque, fdir = vec(), vec(), vec()
        if rnd.random() < 0.3:  # ForceDir on a row: the dot products near zero
            fdir = [f32(x) for x in rnd.choice([(0.0, 0.0, 1.0), (1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (-1.0, 0.0, 0.0), (1.0, 0.0, 1.0)])]
        mass = f32(rnd.choice([1.0, 1.0, rnd.uniform(500, 3000), rnd.uniform(0.1, 10)]))
        inertia = [f32(rnd.choice([1.0, rnd.uniform(100, 5000), rnd.uniform(0.1, 3)])) for _ in range(3)]
        cases.append([bits(x) for x in r + force + torque + fdir] + [bits(mass)] + [bits(x) for x in inertia])
    inp = ''.join(' '.join('%x' % x for x in c) + chr(10) for c in cases)
    p = subprocess.run([str(exe), str(a.image)], input=inp, capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    out = p.stdout.splitlines()
    assert len(out) == len(cases), (len(out), len(cases))
    a.out.write_text(cite_safe('\n'.join(' '.join('%x' % x for x in c) + ' ' + o for c, o in zip(cases, out)) + '\n'))
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
