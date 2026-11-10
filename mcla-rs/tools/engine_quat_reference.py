"""Four small rotation routines run offline (generated code unchanged, vector mode; a register's lane k is the host's
f32[3 - k]), 2026-11-09:
 - F-86566: a vector turned by a quaternion (r3 q x y z w, r4 the vector (16 bytes read, w lane garbage), r5 out; the
   out's own first three words are overwritten with q's xyz first, its w lane is garbage in and out);
 - F-22579 (and its callee F-40856): a quaternion at r3 +0 and a position at r3 +16 to rows (r4 out: three rows at 0 /
   16 / 32 whose w lanes are not written, and the position copied to +48);
 - F-16377 (and its callee F-50812): a matrix (r4: three rows, the position row at +48) to a quaternion at r3 +0 and
   the position copied to r3 +16;
 - F-44424: a quaternion at r4 to four rows at r3 (1 - 2 x x, -2 x y ... and the row -2 w (x y z)).
Writes <out-dir>/engine_quat_ref.txt: "rot q4 v3 -> o3", "pose q4 p4 -> r0(3) r1(3) r2(3) r3(4)", "quat m16 -> q4 p4",
"refl q4 -> r0(3) r1(3) r2(3) r3(3)" (hex bits). Words whose lane is not written by the original are not printed."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def quat(rnd):
    q = [rnd.gauss(0, 1) for _ in range(4)]
    n = math.sqrt(sum(c * c for c in q))
    return [c / n for c in q]


def rows_of(q):
    x, y, z, w = q
    return [[1 - 2 * (y * y + z * z), 2 * (x * y + z * w), 2 * (x * z - y * w)],
            [2 * (x * y - z * w), 1 - 2 * (x * x + z * z), 2 * (y * z + x * w)],
            [2 * (x * z + y * w), 2 * (y * z - x * w), 1 - 2 * (x * x + y * y)]]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f_rot, f_pose, f_quat, f_refl = (addr(a.tool, i) for i in ('F-86566', 'F-22579', 'F-16377', 'F-44424'))
    rnd = random.Random(2026)
    jobs = []
    nan, inf = float('nan'), float('inf')
    for i in range(3000):
        q = quat(rnd)
        if i % 5 == 1:
            q = [c * rnd.uniform(0.5, 2) for c in q]      # not unit
        if i % 50 == 2:
            q = [0.0, 0.0, 0.0, 1.0]
        if i % 50 == 3:
            q = [0.0, 0.0, 0.0, 0.0]
        if i % 100 == 4:
            q[rnd.randrange(4)] = rnd.choice((nan, inf, -inf))
        v = [rnd.uniform(-100, 100) for _ in range(3)]
        if i % 60 == 5:
            v = [0.0, 0.0, 0.0]
        if i % 100 == 6:
            v[rnd.randrange(3)] = rnd.choice((nan, inf))
        jobs.append(('rot', q + v))
    for i in range(3000):
        q = quat(rnd)
        if i % 5 == 1:
            q = [c * rnd.uniform(0.5, 2) for c in q]
        if i % 50 == 2:
            q = [0.0, 0.0, 0.0, 1.0]
        if i % 100 == 3:
            q[rnd.randrange(4)] = rnd.choice((nan, inf))
        p = [rnd.uniform(-3000, 3000) for _ in range(3)] + [1.0 if i % 2 else rnd.uniform(-2, 2)]
        jobs.append(('pose', q + p))
    for i in range(3000):
        q = quat(rnd)
        if i % 5 == 1:
            q = [c * rnd.uniform(0.5, 2) for c in q]
        if i % 100 == 3:
            q[rnd.randrange(4)] = rnd.choice((nan, inf))
        jobs.append(('refl', q))
    quats = []
    for i in range(6000):
        q = quat(rnd)
        if i % 4 == 1:
            q = [0, 0, 0, 1] if i % 8 == 1 else [1, 0, 0, 0] if i % 8 == 5 else [0, 1, 0, 0] if i % 16 == 9 else [0, 0, 1, 0]
        m = rows_of(q)
        if i % 3 == 2:
            m = [[c + rnd.uniform(-0.02, 0.02) for c in r] for r in m]   # a little off square
        if i % 41 == 0:
            m = [[float(i % 3 - 1) if r == c else 0.0 for c in range(3)] for r in range(3)]   # diagonals, ties
        if i % 97 == 0:
            m = [[rnd.choice((-1.0, 0.0, 1.0)) for _ in range(3)] for _ in range(3)]
        if i % 150 == 7:
            m[rnd.randrange(3)][rnd.randrange(3)] = rnd.choice((nan, inf))
        p = [rnd.uniform(-3000, 3000) for _ in range(3)] + [1.0]
        flat = [m[0][0], m[0][1], m[0][2], 0.0, m[1][0], m[1][1], m[1][2], 0.0, m[2][0], m[2][1], m[2][2], 0.0] + p
        quats.append(flat)
    for flat in quats:
        jobs.append(('quat', flat))
    nl = chr(10)
    NL = chr(92) + 'n'
    kind = {'rot': 0, 'pose': 1, 'quat': 2, 'refl': 3}
    rows = nl.join('{%d,%s},' % (kind[k], ','.join('0x%08x' % fb(x) for x in (c + [0.0] * 16)[:16])) for k, c in jobs)
    body = r'''
static const uint32_t C[][17]={
@ROWS@
};
uint32_t g=0x9E3779B9u;
auto garb=[&](uint32_t from,int n){for(int k=0;k<n;k++){g=g*1664525u+1013904223u;store(from+4*k,(g&0x3FFFFFFF)|0x20000000u);}};
auto pr=[&](uint32_t a,int n){for(int k=0;k<n;k++)printf(" %08x",load(a+4*k));};
for(auto&c:C){
  memset(objects,0,sizeof(objects));
  garb(0x1000,0x100);
  Context ctx{};ctx.r1.u32=0x2F00;
  if(c[0]==0){ // rot: r3 q, r4 v, r5 out
    for(int k=0;k<4;k++)store(0x1000+4*k,c[1+k]);
    for(int k=0;k<3;k++)store(0x1100+4*k,c[5+k]);
    ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;ctx.r5.u32=0x1200;
    sub_@ROT@(ctx,nullptr);pr(0x1200,3);
  } else if(c[0]==1){ // pose: r3 {q, p}, r4 out
    for(int k=0;k<8;k++)store(0x1000+4*k,c[1+k]);
    ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;
    sub_@POSE@(ctx,nullptr);
    for(int r=0;r<3;r++)pr(0x1100+16*r,3);pr(0x1100+48,4);
  } else if(c[0]==2){ // quat: r3 out, r4 matrix
    for(int k=0;k<16;k++)store(0x1100+4*k,c[1+k]);
    ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;
    sub_@QUAT@(ctx,nullptr);pr(0x1000,4);pr(0x1010,4);
  } else { // refl: r3 out, r4 q
    for(int k=0;k<4;k++)store(0x1000+4*k,c[1+k]);
    ctx.r3.u32=0x1100;ctx.r4.u32=0x1000;
    sub_@REFL@(ctx,nullptr);
    for(int r=0;r<4;r++)pr(0x1100+16*r,3);
  }
  printf("@NL@");
}
'''
    body = body.replace('@ROWS@', rows).replace('@NL@', NL).replace('@ROT@', f_rot).replace('@POSE@', f_pose).replace('@QUAT@', f_quat).replace('@REFL@', f_refl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + f_rot, 'sub_' + f_pose, 'sub_' + f_quat, 'sub_' + f_refl], main_src, a.out_dir, 'engine_quat', vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(jobs), (len(out), len(jobs))
    lens = {'rot': 7, 'pose': 8, 'quat': 16, 'refl': 4}
    lines = ['%s %s -> %s' % (k, ' '.join('%08x' % fb(x) for x in c[:{'rot': 7, 'pose': 8, 'quat': 16, 'refl': 4}[k]]), o) for (k, c), o in zip(jobs, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'engine_quat_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
