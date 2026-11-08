"""A Crossfade's fade, F-39612, run offline (the generated code whole and unchanged, vector mode): the record's Min (+72),
Max (+76), margin (+80) and mode (+114), no variables (+84 / +88 / +96 null); stubbed, as inputs of the case: the distance
to the nearest listener F-24107 (returns the case's distance) and the curve F-33490 (returns sqrtf of its input, the
port's test uses the same curve). Laid out as the original's: the pool global (+200 request size, +232 bucket table),
bucket 0 (+11444 request entries), request entry 0 (+199 = 0), the record's request index +112 = 0.

Cases: Min / Max from the two disc objects (40 .. 150, and others) and random ones, distances inside, outside, on the
ends and NaN-free; margins 0 .. 50; modes 0, 1, 2.

Writes <out-dir>/crossfade_ref.txt, one case a line: "min max margin mode d -> near_db far_db near_alive far_alive"
(hex float bits for the floats)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd
from recomp_closure import index, run

recomp_closure.EXTRA += 'static double DIST;' + chr(10)

BT = 0x40100000      # bucket 0
SND = 0x40200000     # the Crossfade sound (+64 = bucket 0)
REQ = 0x40300000     # its request entries
REC = 0x40400000     # the runtime record
STACK = 0x40F00000


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def gen(rnd):
    pick = rnd.choice
    mn, mx = pick(((40.0, 150.0), (40.0, 150.0), (10.0, 60.0), (0.0, 100.0)))
    if rnd.random() < 0.3:
        mn = rnd.uniform(0, 200)
        mx = mn + pick((rnd.uniform(1, 300), 0.5, 1e-3))
    d = pick((mn, mx, 0.0, mn - 0.001, mx + 0.001, rnd.uniform(0, mn + 1), rnd.uniform(mn, mx), rnd.uniform(mn, mx),
              rnd.uniform(mx, mx + 120), rnd.uniform(0, 500)))
    margin = pick((0.0, 10.0, 20.0, rnd.uniform(0, 50)))
    mode = pick((0, 1, 2, 2))
    return mn, mx, margin, mode, max(d, 0.0)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    fade, dist, curve = (addr(a.tool, f) for f in ('F-39612', 'F-24107', 'F-33490'))
    idx = index(a.recomp)
    # the pool global (lis -32125 / addi 7216 in the fade)
    pool = ((-32125 << 16) + 7216) & 0xffffffff
    rnd = random.Random(39612)
    cases = [gen(rnd) for _ in range(a.cases)]
    nl = chr(10)
    rows = nl.join('{0x%08xu,0x%08xu,0x%08xu,%du,0x%08xu},' % (fb(mn), fb(mx), fb(mg), mode, fb(d)) for mn, mx, mg, mode, d in cases)
    setup = 'store(%du+232,%du);store(%du+200,256);store(%du+11444,%du);' % (pool, BT, pool, BT, REQ)
    stubs = {
        'sub_' + dist: 'ctx.f1.f64=DIST;',
        'sub_' + curve: 'ctx.f1.f64=double(sqrtf(float(ctx.f1.f64)));',
    }
    main_src = (
        'static const uint32_t C[][5]={' + nl + rows + nl + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + setup + nl +
        'for(auto&c:C){memset(pointer(%du),0,256);memset(pointer(%du),0,256);memset(pointer(%du),0,128);' % (SND, REQ, REC) +
        'store(%du+72,c[0]);store(%du+76,c[1]);store(%du+80,c[2]);*pointer(%du+114)=uint8_t(c[3]);' % (REC, REC, REC, REC) +
        'float d;std::memcpy(&d,&c[4],4);DIST=d;'
        'Context ctx{};ctx.r1.u32=%du;ctx.r3.u32=%du;ctx.r4.u32=%du;sub_%s(ctx,nullptr);' % (STACK, SND, REC, fade) +
        'printf("%%08x %%08x %%u %%u\\n",load(%du+64),load(%du+68),(unsigned)*pointer(%du+115),(unsigned)*pointer(%du+116));}}' % (REC, REC, REC, REC) + nl)
    exe = rd.build(idx, ['sub_' + fade], main_src, a.out_dir, 'crossfade', stubs=stubs, image=rd.load_image(a.image))
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%08x %08x %08x %d %08x -> %s' % (fb(mn), fb(mx), fb(mg), mode, fb(d), o) for (mn, mx, mg, mode, d), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'crossfade_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
