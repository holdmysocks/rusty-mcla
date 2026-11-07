"""Run two small light-trail routines of the original offline (static recompilation, native, flat guest memory) and
record their results for the port's test (crates/game/src/render/testdata/trail_small_reference.txt).

  CUT  <saved z x y z> <saved pos x y z> <cam z x y z> <cam pos x y z> <cut> <moved>   F-40536, the camera-cut test
  PUSH <view 16 floats, rows> <place x y z> <trail> <moved> <h0 x y z> <value>          F-25303, the history push

The guest's view pointer, saved camera, its flag, the moved factor and the record table are image globals; the
harness writes them directly (addresses resolved from the code, see the doc).

python trail_small_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file>
"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2]


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def unit(rnd):
    while True:
        v = [rnd.uniform(-1, 1) for _ in range(3)]
        n = math.sqrt(sum(x * x for x in v))
        if n > 1e-3:
            return [x / n for x in v]


# guest globals (read from F-40536 / F-25303): the viewport pointer, the saved camera (z at +32, place at +48), its
# flag word, the moved factor, the first trail record's count (+304)
VIEWPTR, SAVED, FLAG, MOVED, COUNT0 = 0x82839704, 0x8288B770, 0x8288B7B0, 0x8287E364, 0x82883970 + 304
HEAD = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});\n'
MAX_CASES = 20000


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = index(a.recomp)
    cut, push = (addr(a.tool, f)[4:] for f in ('F-40536', 'F-25303'))
    rnd = random.Random(40)
    out = []
    # --- camera cut
    rows = []
    for k in range(4000):
        sz = unit(rnd)
        m = k % 6
        if m == 0:
            cz = [sz[i] + rnd.uniform(-0.05, 0.05) for i in range(3)]
        elif m == 1:  # about the 0.2 cut
            t = unit(rnd)
            d = sum(t[i] * sz[i] for i in range(3))
            p = [t[i] - d * sz[i] for i in range(3)]
            n = math.sqrt(sum(x * x for x in p)) or 1.0
            c = 0.2 + rnd.uniform(-1e-6, 1e-6) * rnd.choice([1, 0])
            cz = [c * sz[i] + math.sqrt(1 - c * c) * p[i] / n for i in range(3)]
        else:
            cz = unit(rnd)
        sp = [rnd.uniform(-3000, 3000), rnd.uniform(-30, 100), rnd.uniform(-2500, 2500)]
        step = rnd.choice([rnd.uniform(0, 0.1), rnd.uniform(0, 1.2), rnd.uniform(3.9, 4.1), 4.0 / math.sqrt(3), 1.0 / math.sqrt(3), 0.0, rnd.uniform(0, 30)])
        dv = unit(rnd)
        cp = [sp[i] + dv[i] * step for i in range(3)]
        rows.append([fbits(x) for x in sz + sp + cz + cp])
    assert 0 < len(rows) <= MAX_CASES
    body = 'static const uint32_t C[][12]={' + ','.join('{' + ','.join('0x%08xu' % v for v in r) + '}' for r in rows) + '};\n' + HEAD + (
        'for(auto&c:C){memset(objects,0,sizeof(objects));store(%du,0x1000u);store(%du,1u);'
        'for(int i=0;i<3;i++){store(%du+4*i,c[i]);store(%du+4*i,c[3+i]);store(0x1000+96+4*i,c[6+i]);store(0x1000+112+4*i,c[9+i]);}'
        'store(%du,5u);Context ctx{};ctx.r1.u32=0x2800;sub_%s(ctx,nullptr);'
        'printf("CUT");for(int i=0;i<12;i++)printf(" %%08x",c[i]);printf(" %%u %%08x\\n",load(%du)==0?1u:0u,load(%du));}}\n') % (
        VIEWPTR, FLAG, SAVED + 32, SAVED + 48, COUNT0, cut, COUNT0, MOVED)
    out.append(run(build(idx, ['sub_' + cut], body, a.out, 'cut', vector=True, flags=('-msse4.1', '-mssse3')), a.image))
    # --- history push
    rows = []
    for k in range(4000):
        z = unit(rnd)
        x = unit(rnd)
        d = sum(x[i] * z[i] for i in range(3))
        x = [x[i] - d * z[i] for i in range(3)]
        n = math.sqrt(sum(v * v for v in x)) or 1.0
        x = [v / n for v in x]
        y = [z[1] * x[2] - z[2] * x[1], z[2] * x[0] - z[0] * x[2], z[0] * x[1] - z[1] * x[0]]
        cam = [rnd.uniform(-3000, 3000), rnd.uniform(-30, 100), rnd.uniform(-2500, 2500)]
        t = [-sum(cam[j] * r[j] for j in range(3)) for r in (x, y, z)]
        view = [x[0], y[0], z[0], 0, x[1], y[1], z[1], 0, x[2], y[2], z[2], 0, t[0], t[1], t[2], 1]
        place = [cam[i] + rnd.uniform(-300, 300) for i in range(3)]
        trail = rnd.choice([0.0, 1.0, 0.5, rnd.uniform(0, 2)])
        moved = rnd.choice([0.0, 1.0, rnd.uniform(0, 1)])
        rows.append([fbits(v) for v in view + place + [trail, moved]])
    assert 0 < len(rows) <= MAX_CASES
    body = 'static const uint32_t C[][21]={' + ','.join('{' + ','.join('0x%08xu' % v for v in r) + '}' for r in rows) + '};\n' + HEAD + (
        'for(auto&c:C){memset(objects,0,sizeof(objects));store(%du,0x1000u);for(int i=0;i<16;i++)store(0x1000+384+4*i,c[i]);'
        'for(int i=0;i<3;i++)store(0x1400+4*i,c[16+i]);store(%du,c[20]);float tr;memcpy(&tr,&c[19],4);'
        'Context ctx{};ctx.r3.u32=0x1800;ctx.r4.u32=0x1400;ctx.f1.f64=tr;ctx.r1.u32=0x2c00;sub_%s(ctx,nullptr);'
        'printf("PUSH");for(int i=0;i<21;i++)printf(" %%08x",c[i]);for(int i=0;i<3;i++)printf(" %%08x",load(0x1800+112+4*i));'
        'printf(" %%08x\\n",load(0x1800+272));}}\n') % (VIEWPTR, MOVED, push)
    out.append(run(build(idx, ['sub_' + push], body, a.out, 'push', vector=True, flags=('-msse4.1', '-mssse3')), a.image))
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(''.join(out))


if __name__ == '__main__':
    main()
