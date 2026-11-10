"""A shot's target ranking F-47137 and target search F-33277 run offline (the original's generated code unchanged, vector mode, with
the matrix inverse F-30106 / F-43153 it calls). Nothing is stubbed except the debug-log call the search makes for each accepted
target (an empty stub) and the racers' team getter (vtable slot 17 of the object at racer +128: a host body that answers 0 - no
teams, as offline play has none; the original skips a candidate whose team equals the shooter's non-zero one, the port has no teams).

The start-up vector constants (zero in the static image) are set as the start-up code does: the xyz lane mask and the (0,0,0,1) vector (docs/reference/ai/obstacle_cell.md).
Layout built per case (arena addresses): the thread block's flag byte +72 = 0 (rt_reset), the racer manager G (the global cell
0x828745E4 points at it; +400 the count, +4*i the entry), entry i (+48 the racer), racer i (+28 the vehicle whose matrix is the
four rows at +16, +68 the holder, +128 the team object), holder i (+4 the racer, +112 the back flag, +236 the effect, whose +4 is
its type id; +44.. the found list, +104 its count), the shooter being racer 0 / holder 0 / entry 0.

Lines (hex; floats as the 32-bit pattern):
  R <width> <effect type or ff for none> <row0 3> <row1 3> <row2 3> <place 3> <target place 3> -> <result>       F-47137
  T <mega 0/1> <back 0/1> <range> <width> <n> {<effect or ff> <row0..2 9> <place 3>} x n -> <found 0/1> <count> <car indexes ...>   F-33277
(for T the first car is the shooter; the original appends racers, the indexes are those of the cars in the line)
Usage: pu_targets_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
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
static const uint32_t A=0x40100000u, G_CELL=0x828745E4u, TEAM_VT=0x40100080u, FAKE_TEAM=0x7F001000u;
static uint32_t cellx(int i){return A+0x1000+0x400*i;}
static void build_car(int i,uint32_t eff,const uint32_t*m){          // racer i of 0x400 bytes: +0 racer, +0x100 vehicle, +0x200 holder, +0x300 effect
 uint32_t R=cellx(i),V=R+0x100,H=R+0x200,E=R+0x300;
 for(uint32_t o=0;o<0x400;o+=4)store(R+o,0);
 store(R+28,V);store(R+68,H);store(R+128,TEAM_VT-0x0);store(H+4,R);
 for(int k=0;k<12;k++)store(V+16+16*(k/3)+4*(k%3),m[k]);     // rows of 16 bytes
 if(eff!=0xff){store(H+236,E);store(E+4,eff);}
}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int k=0;k<3;k++){store(0x82860950+4*k,0xFFFFFFFFu);store(0x82860930+4*k,0);} store(0x8286095C,0); store(0x8286093C,0x3F800000u);   // start-up vector constants: xyz mask, (0,0,0,1)
 char kind;
 while(std::scanf(" %c",&kind)==1){
  rt_reset();std::memset(arena+(A-0x40000000u),0,0x20000);
  store(TEAM_VT,TEAM_VT+16);store(TEAM_VT+16+68,FAKE_TEAM);       // the team object's vtable: slot 17
  if(kind=='R'){
   uint32_t w,eff,s[12],t[3];std::scanf("%x %x",&w,&eff);for(int k=0;k<12;k++)std::scanf("%x",&s[k]);for(int k=0;k<3;k++)std::scanf("%x",&t[k]);
   uint32_t m1[12]={0,0,0,0,0,0,0,0,0,0,0,0};uint32_t mt[12]={0,0,0,0,0,0,0,0,0,0,0,0};
   for(int k=0;k<3;k++)mt[9+k]=t[k];
   build_car(0,0xff,s);build_car(1,eff,mt);
   Context c=fresh();c.r3.u32=cellx(0)+0x200;c.r4.u32=cellx(1);c.f1.f64=0;{uint32_t b=w;float f;std::memcpy(&f,&b,4);c.f1.f64=f;}
   sub_RANK(c,nullptr);
   float f=float(c.f1.f64);uint32_t b;std::memcpy(&b,&f,4);
   std::printf("-> %08x\n",b);
  } else {
   uint32_t mega,back,range,width,n;std::scanf("%x %x %x %x %x",&mega,&back,&range,&width,&n);
   uint32_t G=A+0x800;store(G_CELL,G);store(G+400,n);
   for(uint32_t i=0;i<n;i++){uint32_t eff,m[12];std::scanf("%x",&eff);for(int k=0;k<12;k++)std::scanf("%x",&m[k]);
    build_car(i,eff,m);uint32_t ent=A+0x400+0x40*i;store(G+4*i,ent);store(ent+48,cellx(i));}
   *pointer(cellx(0)+0x200+112)=uint8_t(back);     // a byte
   Context c=fresh();c.r3.u32=cellx(0)+0x200;{float f;std::memcpy(&f,&range,4);c.f1.f64=f;std::memcpy(&f,&width,4);c.f2.f64=f;}
   c.r6.u32=A+0x600;c.r7.u32=mega;
   sub_TARGETS(c,nullptr);
   uint32_t cnt=load(A+0x600+60);
   std::printf("-> %u %u",c.r3.u32&0xff,cnt);
   for(uint32_t k=0;k<cnt;k++){uint32_t r=load(A+0x600+4*k);std::printf(" %u",(r-cellx(0))/0x400);}
   std::printf("\n");}
 }
}
'''


def bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return float(np.float32(x))


def step(x, n):
    y = np.float32(x)
    for _ in range(abs(n)):
        y = np.nextafter(y, np.float32(np.inf if n > 0 else -np.inf))
    return float(y)


def rows_of(rnd, kind=None):
    """a car frame: three rows (the car's axes in the world) and its place"""
    kind = kind or rnd.choice(['id', 'id', 'yaw', 'yaw', 'yaw', 'free', 'free', 'free', 'scaled', 'degenerate', 'axis'])
    if kind == 'id':
        r = [(1, 0, 0), (0, 1, 0), (0, 0, 1)]
    elif kind == 'axis':
        r = rnd.choice([[(0, 0, 1), (0, 1, 0), (-1, 0, 0)], [(-1, 0, 0), (0, 1, 0), (0, 0, -1)], [(0, 0, -1), (0, 1, 0), (1, 0, 0)]])
    elif kind == 'yaw':
        a = rnd.uniform(0, 2 * math.pi)
        c, s = math.cos(a), math.sin(a)
        r = [(c, 0, -s), (0, 1, 0), (s, 0, c)]
    elif kind in ('free', 'scaled'):
        v = np.array([rnd.gauss(0, 1) for _ in range(4)])
        w, x, y, z = v / np.linalg.norm(v)
        r = [(1 - 2 * (y * y + z * z), 2 * (x * y + w * z), 2 * (x * z - w * y)),
             (2 * (x * y - w * z), 1 - 2 * (x * x + z * z), 2 * (y * z + w * x)),
             (2 * (x * z + w * y), 2 * (y * z - w * x), 1 - 2 * (x * x + y * y))]
        if kind == 'scaled':
            k = rnd.choice([0.5, 2.0, 1e-3, 100.0])
            r = [tuple(c * k for c in row) for row in r]
    else:  # degenerate: the inverse refuses, the original leaves the rows as they are
        r = rnd.choice([[(1, 0, 0), (1, 0, 0), (0, 0, 1)], [(0, 0, 0), (0, 1, 0), (0, 0, 1)], [(1, 0, 0), (0, 1, 0), (1e-8, 0, 0)]])
    return [tuple(f32(c) for c in row) for row in r]


def place(rnd):
    return tuple(f32(rnd.choice([0.0, float(rnd.randint(-40, 40)), rnd.uniform(-400, 400), rnd.uniform(-5, 5)])) for _ in range(3))


def effect(rnd):
    return rnd.choice([0xff, 0xff, 0xff, 3, 3, 0, 1, 2, 4, 5, 8, 12, 16, 24])


def hexs(vals):
    return ' '.join('%x' % v for v in vals)


def flat_bits(rows, pos):
    return [bits(c) for row in rows for c in row] + [bits(c) for c in pos]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    addr = lambda f: 'sub_' + subprocess.run([a.tool, 'registry', 'addr', f], capture_output=True, text=True, check=True).stdout.split()[1]
    rank, targets, log = addr('F-47137'), addr('F-33277'), addr('F-47121')
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    main_src = MAIN.replace('sub_RANK', rank).replace('sub_TARGETS', targets)
    exe = rd.build(idx, [rank, targets], main_src, a.work, 'pu_targets', image=img, stubs={log: ''},
                   fakes={0x7F001000: 'ctx.r3.u64=0;'})
    rnd = random.Random(47137)
    ulps = (-2, -1, 0, 1, 2)
    lines = []
    # --- rank
    for _ in range(2200):
        rows = rows_of(rnd)
        pos = place(rnd)
        width = f32(rnd.choice([-1.0, -1.0, 3.0, 5.0, 8.0, 12.0, 20.0, 40.0, 0.0, rnd.uniform(0.5, 60), -0.5]))
        t = tuple(f32(pos[i] + rnd.choice([0.0, 1.0, -1.0, 12.0, -12.0, 30.0, rnd.uniform(-60, 60)])) for i in range(3))
        lines.append(('R', bits(width), effect(rnd), flat_bits(rows, pos), [bits(c) for c in t]))
    for _ in range(1500):  # identity frames with the target on the width's edge (exact local values)
        pos = tuple(f32(float(rnd.randint(-10, 10))) for _ in range(3))
        width = f32(float(rnd.choice([3, 5, 8, 12])))
        edge = [step(width, rnd.choice(ulps)), -step(width, rnd.choice(ulps)), 0.0, 1.0, width - 0.5]
        d = (rnd.choice(edge), rnd.choice(edge), f32(rnd.choice([-1.0, -5.0, -40.0, 0.0, 5.0, rnd.uniform(-60, 10)])))
        t = tuple(f32(pos[i] + d[i]) for i in range(3))
        rows = [(1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)]
        lines.append(('R', bits(width), effect(rnd), flat_bits(rows, pos), [bits(c) for c in t]))
    out_lines = []
    for ln in lines:
        out_lines.append('R %x %x %s %s' % (ln[1], ln[2], hexs(ln[3]), hexs(ln[4])))
    # --- targets
    tcases = []
    for _ in range(1500):
        n = rnd.randint(1, 7)
        cars = []
        for i in range(n):
            rows = rows_of(rnd, rnd.choice(['id', 'yaw', 'free', 'axis'])) if i else rows_of(rnd, rnd.choice(['id', 'yaw', 'yaw', 'free', 'axis']))
            cars.append((effect(rnd) if i else rnd.choice([0xff, 5]), rows, place(rnd) if rnd.random() < 0.5 else tuple(f32(rnd.uniform(-60, 60)) for _ in range(3))))
        s = cars[0][2]
        cars = [cars[0]] + [(e, r, tuple(f32(s[k] + p[k] / (1 + (j % 3) * 3)) for k in range(3))) for j, (e, r, p) in enumerate(cars[1:])]
        mega = rnd.choice([0, 0, 0, 1])
        back = rnd.randint(0, 1)
        rng = f32(rnd.choice([10.0, 20.0, 40.0, 100.0, 500.0, rnd.uniform(5, 150)]))
        width = f32(rnd.choice([-1.0, 5.0, 8.0, 12.0, 20.0])) if mega else f32(rnd.choice([5.0, 8.0, 12.0, 20.0, 3.0, -1.0, rnd.uniform(1, 30)]))
        tcases.append((mega, back, bits(rng), bits(width), cars))
    # ties: two targets at the same distance ahead, a target exactly at the range
    for _ in range(200):
        rows = [(1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)]
        s = (0.0, 0.0, 0.0)
        d = f32(rnd.choice([5.0, 12.0, 20.0]))
        cars = [(0xff, rows, s)] + [(0xff, rows, (f32(rnd.choice([0.0, 1.0, -1.0])), 0.0, -d if rnd.random() < 0.7 else d)) for _ in range(rnd.randint(1, 5))]
        tcases.append((0, rnd.randint(0, 1), bits(f32(rnd.choice([d, step(d, 1), step(d, -1), 100.0]))), bits(f32(rnd.choice([5.0, 12.0, -1.0]))), cars))
    for mega, back, rng, width, cars in tcases:
        parts = ['T %x %x %x %x %x' % (mega, back, rng, width, len(cars))]
        for e, rows, pos in cars:
            parts.append('%x %s' % (e, hexs(flat_bits(rows, pos))))
        out_lines.append(' '.join(parts))
    inp = '\n'.join(out_lines) + '\n'
    p = subprocess.run([str(exe), str(a.image)], input=inp, capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    # the program prints "-> result" per case, in order
    res = p.stdout.splitlines()
    assert len(res) == len(out_lines), (len(res), len(out_lines))
    a.out.write_text(cite_safe('\n'.join(f'{c} {r}' for c, r in zip(out_lines, res)) + '\n'))
    print(len(out_lines), 'cases')


if __name__ == '__main__':
    main()
