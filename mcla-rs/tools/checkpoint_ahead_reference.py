"""Run CheckpointManager_HasHitCheckpoint (F-51496) offline and record the two points it hands the hit test (F-78734): the car's
last position (car +400, or its position when car +48 is 0) and the point 2 m ahead along its velocity (the code's
normalisation: dot product, 1 / sqrt, one Newton step; the zero-length case as the code does it).

Stubbed, and said so in the evidence: the hit test F-78734 (a recording stub: it prints its two vector arguments and returns 1;
the hit test itself is checked by `checkpoint_hit_reference.py`), and the car's position and velocity getters (two small pointer functions that pick a row of the car record by its +48
pointer; the stubs return the harness's position and velocity vectors).
Output, one line per case (f32 bit words, big-endian hex):
  AHEAD <vel 3> <pos 3> <car+48 flag> <prev 3> | [<ahead 3> <prev 3> as handed to the hit test, only when it was called] <state> <ret>
Cases: zero, tiny (squares underflow), huge (squares overflow), axis-aligned, car speeds 0.001 .. 400 in random directions,
negative zero lanes, exact power-of-two lengths. Test: `checkpoints::tests::ahead_point_matches_the_original`
(crates/game/tests/data/checkpoint_ahead_reference.txt).
"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import numpy as np
from recomp_closure import index, build, run


def bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return float(np.float32(x))


def cases():
    rnd = random.Random(20261109)
    out = []
    # velocities
    vels = [(0.0, 0.0, 0.0), (-0.0, -0.0, -0.0), (0.0, 0.0, -30.0), (30.0, 0.0, 0.0), (0.0, 5.0, 0.0), (1.0, 1.0, 1.0), (2.0, 0.0, 0.0),
            (1e-20, 0.0, 0.0), (1e-19, 1e-19, 0.0), (1e-30, 1e-30, 1e-30), (1e-38, 0.0, 0.0), (1e-45, 0.0, 0.0), (1e-10, 2e-10, -1e-10),
            (1e19, 0.0, 0.0), (1e20, 1e20, 1e20), (3e38, 0.0, 0.0), (1.0, 0.0, 0.0), (0.5, 0.0, 0.0), (4.0, 0.0, 0.0), (0.25, 0.25, 0.25),
            (3.0, 4.0, 0.0), (1e-3, 0.0, 0.0), (-1e-3, 2e-3, 0.0), (0.0, 0.0, 400.0)]
    for v in vels:
        for _ in range(8):
            pos = tuple(f32(rnd.uniform(-1000, 1000)) for _ in range(3))
            out.append((v, pos))
    for _ in range(2200):
        mag = 10 ** rnd.uniform(-3, 2.7)
        d = [rnd.gauss(0, 1) for _ in range(3)]
        n = math.sqrt(sum(x * x for x in d)) or 1.0
        v = tuple(f32(x / n * mag) for x in d)
        pos = tuple(f32(rnd.uniform(-1000, 1000)) for _ in range(3))
        out.append((v, pos))
    for _ in range(300):  # random float bit patterns of moderate exponent
        v = tuple(f32(rnd.choice([-1, 1]) * 2.0 ** rnd.randint(-20, 10) * rnd.uniform(1, 2)) for _ in range(3))
        pos = tuple(f32(rnd.uniform(-1000, 1000)) for _ in range(3))
        out.append((v, pos))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    nm = lambda f: subprocess.run([a.tool, 'registry', 'addr', f], capture_output=True, text=True, check=True).stdout.split()[2]
    fn = nm('F-51496')
    hit = nm('F-78734')
    getpos, getvel = nm('F-35658'), nm('F-98427')  # the car's position and velocity getters
    rnd = random.Random(5)
    cs = []
    for v, pos in cases():
        flag = rnd.choice([0, 1, 1, 1])
        prev = tuple(f32(pos[i] + rnd.uniform(-3, 3)) for i in range(3))
        state = rnd.choice([0, 0, 0, 2, 1, 3])
        cs.append((v, pos, flag, prev, state))
    arr = ','.join('{%s}' % ','.join('0x%08xu' % x for x in (*[bits(t) for t in (*c[0], *c[1])], c[2], *[bits(t) for t in c[3]], c[4])) for c in cs)
    body = '''
static const uint32_t CASES[][11]={%s};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int i=0;i<4;i++)store(0x828609E0+4*i,0x7f800000); // the start-up vector constant +inf (zero in the static image; docs/reference/cameras/probe_rule.md)
 for(auto&c:CASES){
  std::memset(objects,0,sizeof(objects));
  // the manager (r3 = 0x1000): +32 -> list header 0x1100 (+4 -> cp array 0x1800, +8 u16 count = 3)
  store(0x1000+32,0x1100);store(0x1100+4,0x1800);store16(0x1100+8,3);
  // the developer-flag block the function tests first: pointer at 0x828ccea0 -> a zeroed block (byte +4 = 0)
  store(0x828ccea0,0x1700);
  // racer 2's state word of checkpoint index 1 is +200+4*2 of cp 1 (368 bytes each)
  store(0x1800+368*1+200+4*2,c[10]);
  // the car record (r4 = 0x1400): +48 the physics pointer (nonzero: simulated), +400 last position; the position row at 0x1500
  // and the velocity at 0x1600 are what the two getter stubs return
  store(0x1400+48,c[6]);
  for(int i=0;i<3;i++){store(0x1400+400+4*i,c[7+i]);store(0x1500+4*i,c[3+i]);store(0x1600+4*i,c[0+i]);}
  Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1400;ctx.r5.u32=2;ctx.r6.u32=1;ctx.r1.u32=0x2800;
  std::printf("AHEAD");for(int i=0;i<10;i++)std::printf(" %%08x",c[i]);
  std::printf(" |");
  sub_%s(ctx,nullptr);
  std::printf(" %%u %%d\\n",c[10],int(ctx.r3.u32&0xff));}
}
''' % (arr, fn[4:])
    stubs = {getpos: 'ctx.r3.u32=0x14D0;',   # the position matrix: its row +48 (0x1500) is the car's position
             getvel: 'ctx.r3.u32=0x1600;',   # the velocity vector
             hit: 'for(int i=0;i<3;i++)std::printf(" %08x",load(ctx.r4.u32+4*i));for(int i=0;i<3;i++)std::printf(" %08x",load(ctx.r5.u32+4*i));ctx.r3.u32=1;'}
    exe = build(index(a.recomp), [fn], body, a.out, 'cp_ahead', stubs, vector=True)
    a.ref.write_text(run(exe, a.image))


if __name__ == '__main__':
    main()
