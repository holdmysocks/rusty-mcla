"""The turns the car-parts object's update F-98654 gives the bones it holds, run offline on the generated code
unchanged: the bone turns F-52786 (about x) and F-83846 (about z) are stubbed to print the bone and the angle they are
handed, the steps around them (the flaps' step F-16752 is not run: its result, the flap's +180, is an input here; the
bumpers F-44446, the skeleton reset, the eased steering F-69494 and the drawn wheel at the end) stubbed out, and the
valve's slew F-38786 runs as it is.

Car-parts object D at 0x1000 (the original's offsets): +0 the car (its +12 - 16 the model, +122 the byte that keeps the
trunk still), +4 the chassis (+16 the engine: +40 its shake angle, +46 the running word; +136 the throttle), +1104 the
skeleton to reset (0: not reset), +1124 / +1128 / +1132 / +1136 / +1164 the steering values, +1200 / +1204 / +1208 /
+1212 / +1216 the bones (steer_whl's, the hood's, the blower's, its valve's, steer), +1220 / +1224 the bone lists (0),
+1456 / +1664 the trunk / hood flaps (+180 angle, +188 reversed, +189 still), +1872 / +1876 the bumpers, +1880 the valve
{rate, target, value, start, start}.

Writes <out-dir>/part_turns_ref.txt: one case a line, "in: <words> -> out: <calls> = <valve words>": the calls in order,
'x' / 'z', the bone's address (hex) and the angle (hex float), then the valve's target and value.
Test `steer_parts::tests::part_turns_match_the_original`."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure  # (imports memcap: the harness runs capped)
import recomp_leaf

recomp_closure.EXTRA += '#define ctr r2\n#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("unexpected indirect call")\n'
recomp_closure.shim = lambda: recomp_leaf.shim(0x8000)
_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + _vshim()
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    return s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                     'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)


recomp_closure.vector_shim = vector_shim
_run = subprocess.run
# (the recomp's own x86 flags, as the generated code is built)
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


# (stubbed: the flaps' step, the bumpers' step, the skeleton reset, the eased steering, the drawn wheel)
IDLE = ['F-16752', 'F-44446', 'F-89389', 'F-69494', 'F-47822']


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    fn = 'sub_' + addr(a.tool, 'F-98654')
    rnd = random.Random(98654)

    def val():
        return rnd.choice((0.0, -0.0, 0.5, -0.5, 1.0, rnd.uniform(-3, 3), rnd.uniform(-0.6, 0.6), rnd.uniform(-1e-3, 1e-3), math.nan,
                           0.5 + 2 ** -24, -0.5 - 2 ** -24))

    cases = []
    for k in range(a.cases):
        f = [val() for _ in range(5)]                                        # +1124 +1128 +1132 +1136 +1164
        f += [rnd.uniform(-2, 2), rnd.uniform(-2, 2)]                        # hood, trunk flap angles
        f += [rnd.choice((0.0, rnd.uniform(-0.05, 0.05), 1.0)), rnd.choice((0.0, 1.0, rnd.uniform(0, 1), 0.3333))]  # shake, throttle
        f += [rnd.choice((9.0, rnd.uniform(0, 20))), rnd.uniform(-1, 2), rnd.choice((0.0, 1.0, rnd.uniform(-1, 2), 0.005))]  # rate target value
        f += [rnd.choice((1 / 30, 1 / 60, rnd.uniform(0, 0.2)))]             # dt
        ints = [rnd.random() < 0.8 for _ in range(5)]                        # whl hood blower valve steer present
        ints += [rnd.random() < 0.3, rnd.random() < 0.3, rnd.random() < 0.3, rnd.random() < 0.3]  # hood rev/still, trunk rev/still
        ints += [rnd.random() < 0.2, rnd.choice((0, 1, 1, 1))]               # model +122, engine running
        cases.append((f, [int(x) for x in ints]))
    rows = ',\n'.join('{%s}' % ','.join(['0x%08x' % fb(x) for x in c] + ['%d' % x for x in ints]) for c, ints in cases)
    n = len(cases[0][0]) + len(cases[0][1])
    # (the image's statics a static initialiser fills: the identity, dt)
    setup = ('memset(objects,0,0x7000);'
             'for(int k=0;k<4;k++)store(0x827D85D0+16*k+4*k,0x3f800000);store(0x827D7508,c[12]);'
             'store(0x1000,0x3000);store(0x3000+12,0x3110);*pointer(0x3100+122)=uint8_t(c[22]);'
             'store(0x1000+4,0x4000);store(0x4000+16,0x4100);store(0x4100+40,c[7]);*pointer(0x4100+46)=0;*pointer(0x4100+47)=uint8_t(c[23]);'
             'store(0x4000+136,c[8]);'
             'store(0x1000+1124,c[0]);store(0x1000+1128,c[1]);store(0x1000+1132,c[2]);store(0x1000+1136,c[3]);store(0x1000+1164,c[4]);'
             'store(0x1000+1200,c[13]?0x6000:0);store(0x1000+1204,c[14]?0x6100:0);store(0x1000+1208,c[15]?0x6200:0);'
             'store(0x1000+1212,c[16]?0x6300:0);store(0x1000+1216,c[17]?0x6400:0);'
             'store(0x1000+1664+180,c[5]);*pointer(0x1000+1664+188)=uint8_t(c[18]);*pointer(0x1000+1664+189)=uint8_t(c[19]);'
             'store(0x1000+1456+180,c[6]);*pointer(0x1000+1456+188)=uint8_t(c[20]);*pointer(0x1000+1456+189)=uint8_t(c[21]);'
             'store(0x1000+1872,0x5000);store(0x1000+1876,0x5100);store(0x1000+1880,0x5200);'
             'store(0x5200,c[9]);store(0x5204,c[10]);store(0x5208,c[11]);')
    # (the angle comes in f1: stored through a float word to print it)
    stubs = {'sub_' + addr(a.tool, 'F-52786'): '{float v=float(ctx.f1.f64);uint32_t u;memcpy(&u,&v,4);printf(" x %04x %08x",ctx.r3.u32,u);}',
             'sub_' + addr(a.tool, 'F-83846'): '{float v=float(ctx.f1.f64);uint32_t u;memcpy(&u,&v,4);printf(" z %04x %08x",ctx.r3.u32,u);}',
             **{'sub_' + addr(a.tool, f): '' for f in IDLE}}
    nl = chr(10)
    body = ('static const uint32_t C[][%d]={' % n + nl + rows + nl + '};' + nl +
            'for(auto&cc:C){const uint32_t*c=cc;' + setup +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r13.u32=0x5800;ctx.r1.u32=0x7F00;' + fn + '(ctx,nullptr);'
            'printf(" = %08x %08x' + chr(92) + 'n",load(0x5204),load(0x5208));}')
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = recomp_closure.index(a.recomp)
    exe = recomp_closure.build(idx, [fn], main_src, a.out_dir, 'partturns', stubs=stubs, vector=True)
    out = [x.strip() for x in recomp_closure.run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = [' '.join('%08x' % fb(x) for x in c) + ' ' + ' '.join(str(x) for x in ints) + ' ->' + (' ' if not o.startswith('=') else ' ') + o
             for (c, ints), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'part_turns_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
