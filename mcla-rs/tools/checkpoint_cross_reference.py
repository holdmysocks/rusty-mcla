"""Run the original's line-line intersection (F-66886) and its segment test (F-95591) offline on generated cases.

Cases: axis-aligned and parallel lines (exact zeros), values near the 1e-5 threshold, random floats, both flag values.
Output (one line per case): "<fn> <flag> <8 input f32 bits: a0x a0z a1x a1z b0x b0z b1x b1z> <ret> <t bits> <u bits>".
fn 0 = F-66886 (intersection), 1 = F-95591 (segment test). The test is `checkpoints::tests::cross_matches_the_original`.
"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def cases():
    rnd = random.Random(20261027)
    out = []
    vals = [0.0, 1.0, -1.0, 2.0, -2.0, 4.0, 0.5, 1e-5, -1e-5, 9e-6, 1.1e-5, 3.0, 100.0, -37.25]
    for _ in range(800):  # small grid values: hits the axis-aligned and parallel branches
        out.append([rnd.choice(vals) for _ in range(8)])
    for _ in range(800):  # integers on a small grid
        out.append([float(rnd.randint(-4, 4)) for _ in range(8)])
    for _ in range(1000):  # random world-sized floats
        out.append([f32(rnd.uniform(-500, 500)) for _ in range(8)])
    for _ in range(600):  # random with a tiny move (the per-tick car step)
        a = [rnd.uniform(-500, 500), rnd.uniform(-500, 500)]
        b = [a[0] + rnd.uniform(-1, 1), a[1] + rnd.uniform(-1, 1)]
        g = [rnd.uniform(-500, 500), rnd.uniform(-500, 500)]
        h = [g[0] + rnd.uniform(-12, 12), g[1] + rnd.uniform(-12, 12)]
        # gate through the move's neighbourhood so crossings happen
        m = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2]
        g = [m[0] + rnd.uniform(-6, 6), m[1] + rnd.uniform(-6, 6)]
        h = [m[0] + rnd.uniform(-6, 6), m[1] + rnd.uniform(-6, 6)]
        out.append([f32(v) for v in (a[0], a[1], b[0], b[1], g[0], g[1], h[0], h[1])])
    return [[f32(v) for v in c] for c in out]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    nm = lambda f: subprocess.run([a.tool, 'registry', 'addr', f], capture_output=True, text=True, check=True).stdout.split()[2]
    cross, seg = nm('F-66886'), nm('F-95591')
    cs = cases()
    arr = ','.join('{%s}' % ','.join('%sf' % repr(v) if v == v else '0' for v in c) for c in cs)
    # float literals: use exact bit patterns to avoid parse rounding
    arr = ','.join('{%s}' % ','.join('0x%08xu' % bits(v) for v in c) for c in cs)
    body = '''
static const uint32_t CASES[][8]={%s};
static void setf(uint32_t a,uint32_t b){store(a,b);}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int fn=0;fn<2;fn++)for(int flag=0;flag<2;flag++)for(auto&c:CASES){
  std::memset(objects,0,sizeof(objects));
  // a0 0x1010, a1 0x1020, b0 0x1030, b1 0x1040 (x at +0, z at +4)
  setf(0x1010,c[0]);setf(0x1014,c[1]);setf(0x1020,c[2]);setf(0x1024,c[3]);
  setf(0x1030,c[4]);setf(0x1034,c[5]);setf(0x1040,c[6]);setf(0x1044,c[7]);
  Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1004;ctx.r5.u32=0x1010;ctx.r6.u32=0x1020;ctx.r7.u32=0x1030;ctx.r8.u32=0x1040;ctx.r9.u32=flag;ctx.r1.u32=0x2800;
  if(fn==0)sub_%s(ctx,nullptr);else sub_%s(ctx,nullptr);
  std::printf("%%d %%d",fn,flag);for(int i=0;i<8;i++)std::printf(" %%08x",c[i]);
  std::printf(" %%d %%08x %%08x\\n",int(ctx.r3.u32&0xff),load(0x1000),load(0x1004));}
}
''' % (arr, cross[4:], seg[4:])
    exe = build(index(a.recomp), [cross, seg], body, a.out, 'cross')
    a.ref.write_text(run(exe, a.image))


if __name__ == '__main__':
    main()
