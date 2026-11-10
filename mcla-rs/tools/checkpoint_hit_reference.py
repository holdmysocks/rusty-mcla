"""Run the original's checkpoint hit test (F-78734) offline on generated cases (static recompilation, native, flat guest memory,
vector mode; the line-line intersection F-66886 and the points helper run unchanged, nothing is stubbed).

A 368-byte checkpoint object is built at 0x1000 (+64 position, +80 second position, +96 gate offset, +168 radius, +180
activation height, +184 depth, +330 line flag), the car's `ahead` point (r4) and last position `prev` (r5) are 16-byte
vectors. Output, one line per case (floats as big-endian f32 bit words, hex; the first field is the line flag):
  HIT <line 0/1> <pos 3> <second 3> <radius> <act> <depth> <gate offset 3> <ahead 3> <prev 3> <result 0/1>
Cases: round checkpoints with the nearest point of the way at, just inside and just outside radius and 1.2 x radius (integer
grids for exact equality and ulp neighbours), heights on the depth / activation edges, ways ending exactly at the
checkpoint; lines crossed at t / u = 0, 1, -0.2, 1.2 (and their ulp neighbours), parallel ways, the three gate lines,
heights on the window edges; random world-sized cases. Test: `checkpoints::tests::hit_matches_the_original`
(crates/game/tests/data/checkpoint_hit_reference.txt).
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


def step(x, n):
    """x moved by n float32 steps (ulps)"""
    y = np.float32(x)
    for _ in range(abs(n)):
        y = np.nextafter(y, np.float32(np.inf if n > 0 else -np.inf))
    return float(y)


def normal10(a, b):
    """the gate offset as the port makes it: (dz, 0, -dx) normalized, x 10 (only input data here)"""
    dx, dz = f32(b[0] - a[0]), f32(b[2] - a[2])
    l = math.sqrt(dx * dx + dz * dz)
    if l < 1e-9:
        return (0.0, 0.0, 0.0)
    return (f32(dz / l * 10), 0.0, f32(-dx / l * 10))


def cases():
    rnd = random.Random(20261109)
    out = []  # (line, pos, second, radius, act, depth, off, ahead, prev)
    ulps = (-2, -1, 0, 1, 2)
    Z = (0.0, 0.0, 0.0)

    # --- round checkpoints
    for _ in range(1800):
        c = (float(rnd.randint(-8, 8)), float(rnd.randint(-4, 4)), float(rnd.randint(-8, 8)))
        radius = float(rnd.choice([1, 2, 3, 5, 10, 12.5, 20]))
        act = float(rnd.choice([4, 10, 50, 100]))
        depth = float(rnd.choice([3, 3, 3, 1, 5]))
        mode = rnd.choice(['interior', 'end_prev', 'end_ahead', 'through', 'zero'])
        ang = rnd.choice([0, 90, 180, 270, 45, 30]) * math.pi / 180
        ux, uz = f32(math.cos(ang)), f32(math.sin(ang))
        reach = radius * rnd.choice([1.0, 1.2, 0.5, 0.0, 2.0])
        d = f32(reach) if rnd.random() < 0.5 else step(f32(reach), rnd.choice(ulps))
        wl = float(rnd.choice([0.5, 2, 5, 40]))
        wx, wz = -uz * wl, ux * wl
        qy = rnd.choice([c[1] - depth, c[1] + act, c[1], c[1] + act / 2, c[1] - depth - 1, c[1] + act + 1])
        qy = f32(qy) if rnd.random() < 0.6 else step(f32(qy), rnd.choice(ulps))
        q = (f32(c[0] + ux * d), qy, f32(c[2] + uz * d))
        if mode == 'interior':
            f = rnd.choice([0.25, 0.5, 0.75])
            prev = (f32(q[0] - wx * f), q[1], f32(q[2] - wz * f))
            ahead = (f32(q[0] + wx * (1 - f)), q[1], f32(q[2] + wz * (1 - f)))
        elif mode == 'end_prev':
            prev = q
            ahead = (f32(q[0] + ux * wl), q[1], f32(q[2] + uz * wl))
        elif mode == 'end_ahead':
            ahead = q
            prev = (f32(q[0] + ux * wl), q[1], f32(q[2] + uz * wl))
        elif mode == 'through':
            prev = (f32(c[0] - wx), q[1], f32(c[2] - wz))
            ahead = (f32(c[0] + wx), q[1], f32(c[2] + wz))
        else:
            prev = q
            ahead = q
        if rnd.random() < 0.4:  # height varies along the way
            ahead = (ahead[0], f32(ahead[1] + rnd.choice([-3.0, 2.0, 7.0, 0.5])), ahead[2])
        out.append((0, c, Z, f32(radius), f32(act), f32(depth), Z, ahead, prev))
    for _ in range(1200):  # random world floats
        c = tuple(f32(rnd.uniform(-300, 300)) for _ in range(3))
        radius = f32(rnd.uniform(2, 80))
        act = f32(rnd.uniform(4, 250))
        prev = tuple(f32(c[i] + rnd.uniform(-90, 90)) for i in range(3))
        ahead = tuple(f32(prev[i] + rnd.uniform(-2.5, 2.5)) for i in range(3))
        if rnd.random() < 0.3:
            ahead = prev  # a standing car
        out.append((0, c, Z, radius, act, 3.0, Z, ahead, prev))

    # --- line checkpoints
    for _ in range(3000):
        a = (float(rnd.randint(-8, 8)), float(rnd.randint(-3, 3)), float(rnd.randint(-8, 8)))
        b = (a[0] + rnd.choice([0, 4, -4, 8, 12, -12, 6]), float(rnd.randint(-3, 3)), a[2] + rnd.choice([0, 4, -4, 8, 12, -12, 6]))
        if rnd.random() < 0.15:
            b = (a[0], b[1], a[2])  # a degenerate line
        act = float(rnd.choice([4, 10, 50, 100]))
        depth = float(rnd.choice([3, 3, 3, 1, 5]))
        off = normal10(a, b)
        if rnd.random() < 0.1:
            off = tuple(f32(rnd.uniform(-10, 10)) for _ in range(3))
        which = rnd.choice([0, 1, 2, 3])  # the gate line the crossing is on: 0 the line, 1 +off, 2 -off, 3 none (random way)
        u = rnd.choice([0.0, 1.0, -0.2, 1.2, 0.5, 0.25, -0.25, 1.25, 0.75])
        u = f32(u) if rnd.random() < 0.5 else step(f32(u), rnd.choice(ulps))
        t = rnd.choice([0.0, 1.0, 0.5, 0.25, 0.75, -0.1, 1.1])
        t = f32(t) if rnd.random() < 0.5 else step(f32(t), rnd.choice(ulps))
        sh = {0: Z, 1: off, 2: (-off[0], -off[1], -off[2]), 3: Z}[which]
        P = tuple(f32(a[i] + (b[i] - a[i]) * u + sh[i]) for i in range(3))
        gy = f32(a[1] + (b[1] - a[1]) * u)
        w = rnd.choice([(2.0, 0.0), (0.0, 2.0), (-2.0, 1.0), (1.0, 1.0), (0.5, -2.0), (0.0, 0.0)])
        if which == 3:
            w = (rnd.uniform(-3, 3), rnd.uniform(-3, 3))
        w = (f32(w[0]), f32(w[1]))
        # the way: ahead + (prev - ahead) * t = P  ->  ahead = P - w*t, prev = ahead + w
        ay = rnd.choice([gy - depth, gy + act, gy, gy - depth - 1, gy + act + 1, gy + 1])
        py = ay + rnd.choice([0.0, 0.5, -1.0])
        ay = f32(ay) if rnd.random() < 0.6 else step(f32(ay), rnd.choice(ulps))
        ahead = (f32(P[0] - w[0] * t), ay, f32(P[2] - w[1] * t))
        prev = (f32(ahead[0] + w[0]), f32(py), f32(ahead[2] + w[1]))
        out.append((1, a, b, f32(rnd.choice([10.0, 12.5, 20.0])), f32(act), f32(depth), off, ahead, prev))
    for _ in range(1500):  # random lines and ways around them (car step sized and long)
        a = tuple(f32(rnd.uniform(-300, 300)) for _ in range(3))
        b = tuple(f32(a[i] + rnd.uniform(-60, 60)) for i in range(3))
        m = tuple((a[i] + b[i]) / 2 for i in range(3))
        prev = tuple(f32(m[i] + rnd.uniform(-25, 25)) for i in range(3))
        ahead = tuple(f32(prev[i] + rnd.uniform(-8, 8)) for i in range(3))
        out.append((1, a, b, 10.0, f32(rnd.uniform(4, 250)), 3.0, normal10(a, b), ahead, prev))
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
    hit = nm('F-78734')
    cs = cases()
    flat = lambda c: [c[0]] + [bits(v) for v in (*c[1], *c[2], c[3], c[4], c[5], *c[6], *c[7], *c[8])]
    arr = ','.join('{%s}' % ','.join('0x%08xu' % v for v in flat(c)) for c in cs)
    body = '''
static const uint32_t CASES[][19]={%s};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int i=0;i<4;i++)store(0x828609E0+4*i,0x7f800000); // the start-up vector constant +inf (zero in the static image; docs/reference/cameras/probe_rule.md)
 for(auto&c:CASES){
  std::memset(objects,0,sizeof(objects));
  *pointer(0x1000+330)=uint8_t(c[0]);
  for(int i=0;i<3;i++){store(0x1000+64+4*i,c[1+i]);store(0x1000+80+4*i,c[4+i]);store(0x1000+96+4*i,c[10+i]);}
  store(0x1000+168,c[7]);store(0x1000+180,c[8]);store(0x1000+184,c[9]);
  for(int i=0;i<3;i++){store(0x1200+4*i,c[13+i]);store(0x1210+4*i,c[16+i]);}
  Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1200;ctx.r5.u32=0x1210;ctx.r1.u32=0x2800;
  sub_%s(ctx,nullptr);
  std::printf("HIT %%u",c[0]);for(int i=1;i<19;i++)std::printf(" %%08x",c[i]);
  std::printf(" %%d\\n",int(ctx.r3.u32&0xff));}
}
''' % (arr, hit[4:])
    exe = build(index(a.recomp), [hit], body, a.out, 'cp_hit', vector=True)
    a.ref.write_text(run(exe, a.image))


if __name__ == '__main__':
    main()
