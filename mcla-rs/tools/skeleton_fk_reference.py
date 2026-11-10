"""The skeleton's global matrices from its local ones, F-97374, run offline (generated code unchanged, vector mode; a
register's lane k is the host's f32[3 - k]): the object at r3 holds the skeleton data at +4 (u16 bone count at +20,
parent index words at +4), the parent matrix pointer at +8 (0: bone 0's global is its local copied), the locals at +12
(80 bytes apart, the matrix first) and the globals out at +20 (64 bytes apart); bone k >= 1 has the global
`local_k x global[parent_k]` (a row-vector product, the vector unit's unfused multiply-add chain).
Writes <out-dir>/skeleton_fk_ref.txt: "n hasP parents(n) P(16) locals(16 n) -> globals(16 n)" (hex bits)."""
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


def rot(rnd):
    q = [rnd.gauss(0, 1) for _ in range(4)]
    n = math.sqrt(sum(c * c for c in q))
    x, y, z, w = (c / n for c in q)
    return [[1 - 2 * (y * y + z * z), 2 * (x * y + z * w), 2 * (x * z - y * w)],
            [2 * (x * y - z * w), 1 - 2 * (x * x + z * z), 2 * (y * z + x * w)],
            [2 * (x * z + y * w), 2 * (y * z - x * w), 1 - 2 * (x * x + y * y)]]


def frame(rnd, span):
    r = rot(rnd)
    return [r[0][0], r[0][1], r[0][2], 0.0, r[1][0], r[1][1], r[1][2], 0.0, r[2][0], r[2][1], r[2][2], 0.0] + [rnd.uniform(-span, span) for _ in range(3)] + [1.0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fk = addr(a.tool, 'F-97374')
    rnd = random.Random(97374)
    cases = []
    for i in range(1500):
        n = rnd.choice((1, 1, 2, 3, 5, 8, 12))
        has_p = 0 if i % 7 == 0 else 1
        parents = [0] + [rnd.randrange(k) if i % 3 else k - 1 for k in range(1, n)]    # chains and trees; parent index < k
        p = frame(rnd, 3000) if i % 5 else [rnd.uniform(-2, 2) for _ in range(16)]
        locs = []
        for k in range(n):
            if i % 4 == 3:
                locs.append([rnd.uniform(-2, 2) for _ in range(16)])        # general lanes, w lanes included
            else:
                locs.append(frame(rnd, 1.5))
        cases.append((n, has_p, parents, p, locs))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = []
    for n, has_p, parents, p, locs in cases:
        words = [n, has_p] + parents + [0] * (12 - n) + [fb(x) for x in p] + [fb(x) for k in range(12) for x in (locs[k] if k < n else [0.0] * 16)]
        rows.append('{' + ','.join('0x%08x' % w for w in words) + '},')
    body = r'''
static const uint32_t C[][2+12+16+192]={
@ROWS@
};
uint32_t g=0x9E3779B9u;
auto garb=[&](uint32_t from,int n){for(int k=0;k<n;k++){g=g*1664525u+1013904223u;store(from+4*k,(g&0x3FFFFFFF)|0x20000000u);}};
for(auto&c:C){
  memset(objects,0,sizeof(objects));
  garb(0x1000,0x400);
  uint32_t n=c[0];
  // skeleton data at 0x1000: parent array pointer at +4 (words at 0x1400), count at +20
  store(0x1000+4,0x1400);store16(0x1000+20,n);
  for(int k=0;k<12;k++)store(0x1400+4*k,c[2+k]);
  // object at 0x1100
  store(0x1100+4,0x1000);store(0x1100+8,c[1]?0x1500:0);store(0x1100+12,0x1600);store(0x1100+20,0x1A00);
  for(int k=0;k<16;k++)store(0x1500+4*k,c[14+k]);
  for(int b=0;b<12;b++)for(int k=0;k<16;k++)store(0x1600+80*b+4*k,c[30+16*b+k]);
  garb(0x1A00,0x100);
  Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1100;
  sub_@FK@(ctx,nullptr);
  for(uint32_t b=0;b<n;b++)for(int k=0;k<16;k++)printf(" %08x",load(0x1A00+64*b+4*k));
  printf("@NL@");
}
'''
    body = body.replace('@ROWS@', nl.join(rows)).replace('@NL@', NL).replace('@FK@', fk)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + fk], main_src, a.out_dir, 'skeleton_fk', vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = []
    for (n, has_p, parents, p, locs), o in zip(cases, out):
        c = [str(n), str(has_p), ','.join(map(str, parents)), ' '.join('%08x' % fb(x) for x in p), ' '.join('%08x' % fb(x) for l in locs for x in l)]
        lines.append(' '.join(c) + ' ->' + ' ' + o)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'skeleton_fk_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
