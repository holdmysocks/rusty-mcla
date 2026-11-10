"""The draw matrices of a skeleton F-96265 (the part of F-74033 that builds them) run offline (generated code unchanged,
vector mode; a register's lane k is the host's f32[3 - k]).  The object at r3: +4 the skeleton data D (+0 the bind
records, 224 bytes apart, the bind position at +96 of a record; +12 per-bone rotation matrices, 64 bytes apart; +20 u16
bone count; +28 flags, bits 3 and 2 set for this path), +24 a table of matrix-set pointers 8 bytes apart indexed by the
u16 at +34, +36 the second set (0 here).  r4 is the output (64 bytes per bone).  Per bone k the output is
`G_k x M_k` in rows 0-2 and row 3 = `G_k row 3 - dot3(bind_k, columns of the rotated rows)`.
Writes <out-dir>/skin_matrices_ref.txt: "n flags G(16 n) M(16 n) B(4 n) -> out(16 n)" (hex bits)."""
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
    f5340 = addr(a.tool, 'F-96265')
    rnd = random.Random(96265)
    cases = []
    for i in range(1500):
        n = rnd.choice((1, 1, 2, 3, 4, 5, 8))
        flags = 0xC
        g = [frame(rnd, 2000) if i % 4 else [rnd.uniform(-2, 2) for _ in range(16)] for _ in range(n)]
        ident = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
        m = [ident if i % 3 == 0 else frame(rnd, 0.0) for _ in range(n)]
        b = [[rnd.uniform(-1.5, 1.5) for _ in range(3)] + [rnd.uniform(-1, 1)] for _ in range(n)]
        cases.append((n, flags, g, m, b))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = []
    for n, flags, g, m, b in cases:
        w = [n, flags] + [fb(x) for k in range(8) for x in (g[k] if k < n else [0.0] * 16)] + [fb(x) for k in range(8) for x in (m[k] if k < n else [0.0] * 16)] + [fb(x) for k in range(8) for x in (b[k] if k < n else [0.0] * 4)]
        rows.append('{' + ','.join('0x%08x' % x for x in w) + '},')
    body = r'''
static const uint32_t C[][2+128+128+32]={
@ROWS@
};
uint32_t g=0x9E3779B9u;
auto garb=[&](uint32_t from,int n){for(int k=0;k<n;k++){g=g*1664525u+1013904223u;store(from+4*k,(g&0x3FFFFFFF)|0x20000000u);}};
for(auto&c:C){
  memset(objects,0,sizeof(objects));
  garb(0x1000,0x700);
  uint32_t n=c[0];
  // D at 0x1000: records at 0x1800 (224 apart), M array at 0x1400 (64 apart); count +20, flags +28
  store(0x1000+0,0x1800);store(0x1000+12,0x1400);store16(0x1000+20,n);store(0x1000+28,c[1]);
  for(int b=0;b<8;b++){for(int k=0;k<4;k++)store(0x1800+224*b+96+4*k,c[2+256+4*b+k]);}
  for(int b=0;b<8;b++)for(int k=0;k<16;k++)store(0x1400+64*b+4*k,c[2+128+16*b+k]);
  // object at 0x1100: +4 D, +24 table (0x1180: entry i*8 -> G set at 0x2000), +34 index 1, +36 0
  store(0x1100+4,0x1000);store(0x1100+24,0x1180);store16(0x1100+34,1);store(0x1100+36,0);
  store(0x1180+8,0x2000);
  for(int b=0;b<8;b++)for(int k=0;k<16;k++)store(0x2000+64*b+4*k,c[2+16*b+k]);
  garb(0x2400,0x80);
  Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1100;ctx.r4.u32=0x2400;
  sub_@FN@(ctx,nullptr);
  for(uint32_t b=0;b<n;b++)for(int k=0;k<16;k++)printf(" %08x",load(0x2400+64*b+4*k));
  printf("@NL@");
}
'''
    body = body.replace('@ROWS@', nl.join(rows)).replace('@NL@', NL).replace('@FN@', f5340)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + f5340], main_src, a.out_dir, 'skin_matrices', vector=True, scratch_end=0x3000)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = []
    for (n, flags, g, m, b), o in zip(cases, out):
        lines.append('%d %d %s %s %s -> %s' % (n, flags, ' '.join('%08x' % fb(x) for k in range(n) for x in g[k]), ' '.join('%08x' % fb(x) for k in range(n) for x in m[k]), ' '.join('%08x' % fb(x) for k in range(n) for x in b[k]), o))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'skin_matrices_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
