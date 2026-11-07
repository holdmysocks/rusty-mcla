"""The player car's per-tick wheel part matrices run offline on the original's code: F-24284 (called from the car's update) writes,
for every wheel i < car +24 whose composite part (car +392 + i) is set, the composite's matrix +132 + 64 * (+392 + i) = the wheel's
world matrix (wheel +16) carried into the chassis's frame (car +240 inverted as a rigid matrix), and clears the wheel's byte +530.

A case: the chassis's 16 words (rows, place, each with its w lane), the wheel count (1 .. 4), the first part index (+392: -1 .. 3),
a 4-bit mask of the parts that are set, then 4 wheels x 16 words. Most cases are the game's: rigid f32 rotations with w lanes 0 and
the place's w 1 (or 0), the wheels the chassis times a local turn (steer, camber, spin) and offset; some have random w lanes and
non-rigid rows, to show what the code reads. Out per wheel: the 16 words written (or '-' when the part is not set) and +530.
Writes <out-dir>/wheel_part_matrix_ref.txt. Test `gjk::tests::part_matrix_matches_the_original`."""
import argparse
import math
import random
import re
import struct
import subprocess
from pathlib import Path
import memcap  # (every harness process capped at 4 GB)
import recomp_dispatch as rd

_run = subprocess.run


def _gpp(cmd, *a, **k):
    if cmd and cmd[0] == 'g++':
        cmd = cmd[:1] + ['-msse4.1'] + cmd[1:]
    return _run(cmd, *a, **k)


def addr(tool, fid):
    return _run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


MAIN = r'''
static const uint32_t CAR=0x40D00000, A=0x40D00400, PARTS=0x40D00800, WARR=0x40D00900, COMP=0x40D00600, MATS=0x40D01000,
 WH=0x40D02000, STK=0x40DF0000;
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 FILE*f=fopen(argv[2],"r"); unsigned h[4];
 while(fscanf(f,"%x %x %x %x",&h[0],&h[1],&h[2],&h[3])==4){
  if(h[1]<1||h[1]>4||(h[2]!=0xFFFFFFFFu&&h[2]>3)||h[3]>15){fprintf(stderr,"bad case file: %x %x %x\n",h[1],h[2],h[3]); return 2;}
  unsigned w[80]; for(int i=0;i<80;i++) if(fscanf(f,"%x",&w[i])!=1) return 1;  // (16 chassis + 4 x 16 wheel words, as the writer puts them)
  rt_reset();
  for(uint32_t a=CAR;a<WH+0x1000;a+=4) store(a,0);
  for(int k=0;k<16;k++) store(CAR+240+4*k,w[k]);
  store(CAR+24,h[1]); store(CAR+28,WARR); store(CAR+392,h[2]); store(CAR+4,A); store(A+28,A+0x40); store(A+0x40+4,A+0x80); store(A+0x80+12,COMP);
  *pointer(COMP+4)=12; store(COMP+128,PARTS); store(COMP+132,MATS);
  for(uint32_t a=MATS;a<MATS+8*64;a+=4) store(a,0xDEADBEEF);
  for(int i=0;i<4;i++){ uint32_t wh=WH+0x400*i; store(WARR+4*i,wh); for(int k=0;k<16;k++) store(wh+16+4*k,w[16+16*i+k]); *pointer(wh+530)=0xAA;
   int p=int(h[2])+i; if(p>=0) store(PARTS+4*p,(h[3]>>i)&1 ? 0x40D0F000u+0x100*i : 0); }
  Context c=fresh(); c.r1.u64=STK; c.r3.u64=CAR;
  try{ sub_F_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); continue; }
  for(unsigned i=0;i<4;i++){ int p=int(h[2])+int(i);
   if(p<0||load(MATS+64*p)==0xDEADBEEFu&&load(MATS+64*p+4)==0xDEADBEEFu) printf(i?" | -":"-");
   else { printf(i?" |":""); for(int k=0;k<16;k++) printf(i||k?" %08x":"%08x",load(MATS+64*p+4*k)); }
   printf(" %02x",*pointer(WH+0x400*i+530)); }
  printf("\n"); } }
'''


def rotation(rnd, ax=None, th=None):
    unit = lambda v: [x / (math.sqrt(sum(y * y for y in v)) or 1.0) for x in v]
    ax = unit([rnd.gauss(0, 1) for _ in range(3)]) if ax is None else ax
    th = rnd.uniform(-math.pi, math.pi) if th is None else th
    c, s = math.cos(th), math.sin(th)
    x, y, z = ax
    return [[c + x * x * (1 - c), x * y * (1 - c) + z * s, x * z * (1 - c) - y * s],
            [y * x * (1 - c) - z * s, c + y * y * (1 - c), y * z * (1 - c) + x * s],
            [z * x * (1 - c) + y * s, z * y * (1 - c) - x * s, c + z * z * (1 - c)]]


def mul(a, b):
    """rows: a's row through b's frame (row vectors)."""
    return [[sum(a[i][k] * b[k][j] for k in range(3)) for j in range(3)] for i in range(3)]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=600)
    ap.add_argument('--seed', type=int, default=24284)
    a = ap.parse_args()
    rnd = random.Random(a.seed)
    cases = []
    for n in range(a.cases):
        game = n % 5 != 4
        body = rotation(rnd, [0.0, 1.0, 0.0] if rnd.random() < 0.3 else None)
        place = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 80), rnd.uniform(-3000, 3000)]
        if game:
            ch = [[f32(v) for v in r] + [0.0] for r in body] + [[f32(v) for v in place] + [rnd.choice([1.0, 0.0])]]
        else:
            ch = [[f32(v + rnd.gauss(0, 0.05)) for v in r] + [f32(rnd.uniform(-2, 2))] for r in body] + \
                 [[f32(v) for v in place] + [f32(rnd.uniform(-2, 2))]]
        wheels = []
        for i in range(4):
            local = mul(mul(rotation(rnd, [1.0, 0.0, 0.0]), rotation(rnd, [0.0, 0.0, 1.0], rnd.uniform(-0.2, 0.2))),
                        rotation(rnd, [0.0, 1.0, 0.0], rnd.uniform(-0.6, 0.6) if i < 2 else 0.0))
            off = [(-1) ** i * rnd.uniform(0.7, 0.9), rnd.uniform(-0.3, 0.1), (1 if i < 2 else -1) * rnd.uniform(1.2, 1.6)]
            rows = mul(local, body)
            at = [place[j] + sum(off[k] * body[k][j] for k in range(3)) for j in range(3)]
            if game:
                wheels.append([[f32(v) for v in r] + [0.0] for r in rows] + [[f32(v) for v in at] + [rnd.choice([1.0, 0.0])]])
            else:
                wheels.append([[f32(v) for v in r] + [f32(rnd.uniform(-2, 2))] for r in rows] + [[f32(v) for v in at] + [f32(rnd.uniform(-2, 2))]])
        cnt = 4 if rnd.random() < 0.8 else rnd.randint(1, 4)
        first = rnd.choice([0, 0, 1, 2, 3, 0xFFFFFFFF])
        if first != 0xFFFFFFFF and first + cnt > 8:
            first = 0
        mask = 15 if rnd.random() < 0.8 else rnd.randrange(16)
        words = [fb(v) for r in ch for v in r] + [fb(v) for wh in wheels for r in wh for v in r]
        head = [int(not game), cnt, first, mask]
        assert len(words) == 80 and 1 <= cnt <= 4 and mask < 16  # (the reader's layout)
        cases.append(head + words)
    work = a.out_dir / 'build'
    work.mkdir(parents=True, exist_ok=True)
    inp = work / 'cases.txt'
    inp.write_text(''.join(' '.join('%x' % w for w in c) + '\n' for c in cases))
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    fn = addr(a.tool, 'F-24284')
    subprocess.run = _gpp
    exe = rd.build(idx, ['sub_' + fn], MAIN.replace('sub_F_', 'sub_' + fn), work, 'wheel_part_matrix', image=img)
    subprocess.run = _run
    r = _run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    out = [x for x in r.stdout.split('\n') if x]
    assert len(out) == len(cases), (len(out), len(cases), r.stderr[-800:])
    assert not [o for o in out if 'EXC' in o], [o for o in out if 'EXC' in o][:3]
    nl = chr(10)
    text = nl.join(' '.join('%x' % w for w in c) + ' -> ' + o.strip() for c, o in zip(cases, out)) + nl
    (a.out_dir / 'wheel_part_matrix_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', text))
    print(len(cases), 'cases')


if __name__ == '__main__':
    main()
