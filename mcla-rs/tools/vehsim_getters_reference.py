"""Five small getters of the car's sim run offline (generated code unchanged, scalar mode):

- the gearbox's direction F-80284 (r3 +20 the gearbox or 0; its +20 the gear index, 0 reverse, 1 neutral, 2.. forward):
  r3 = 0 with no gearbox, 1 from index 2 up, -1 at 0, else 0;
- "driving" F-65255 (r3 +8 the chassis; its +20 the gearbox or 0, the gearbox's +20 the index, the chassis's +136 the
  throttle against the image's 0.0): a gearbox, not neutral, the throttle above 0;
- the wheels with a contact record F-33415 (mcCarSim vfunc 11: r3 +24 the count, +28 the list; each wheel's byte +529);
- the nitrous view at rest F-91794 (+24 == +20; +20 is 0.0 in the running game, its constructor's, other values too here);
- the turbo lens at rest F-50750 (+20 == the image's 0.0).

Cases: no gearbox, the indices 0 .. 12 and odd ones (negative, huge), throttles 0 / -0 / tiny / 0 .. 1 / negative / NaN,
counts 0 .. 4 and odd ones (0 or negative: the loop does not run), any mix of the bytes 0 / 1 / 255; views and lenses
0 / -0 / tiny / subnormal / random / NaN.

Writes <out-dir>/vehsim_getters_ref.txt, one case a line, hex: "dir has gear -> r3", "drive has gear thr -> r3",
"count n b0 b1 .. -> r3", "nitro view rest -> r3", "lens v -> r3"."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += '#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("indirect")' + chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fd, fv, fc, fn, fl = ('sub_' + addr(a.tool, f) for f in ('F-80284', 'F-65255', 'F-33415', 'F-91794', 'F-50750'))
    rnd = random.Random(80284)
    gear = lambda: rnd.choice((0, 1, 2, 3, rnd.randrange(0, 13), rnd.randrange(0, 13), 0xFFFFFFFF, 0x80000000, 0x7FFFFFFF))
    real = lambda: fb(rnd.choice((0.0, -0.0, 1e-40, 1e-30, rnd.uniform(0, 1), rnd.uniform(0, 1), 1.0, -rnd.uniform(0, 1),
                                  float('nan'), float('inf'), -1e-40)))
    cases = []
    for _ in range(1500):
        cases.append(('dir', [rnd.random() < 0.9, gear()]))
    for _ in range(3000):
        cases.append(('drive', [rnd.random() < 0.9, gear(), real()]))
    for _ in range(2000):
        n = rnd.choice((0, 1, 2, 2, 4, 4, 4, 3, 0xFFFFFFFF, 0x80000000))
        k = n if n <= 4 else 0
        cases.append(('count', [n] + [rnd.choice((0, 1, 1, 255, rnd.getrandbits(8))) for _i in range(k)]))
    for _ in range(1500):
        rest = rnd.choice((fb(0.0), fb(0.0), fb(0.0), fb(-0.0), real()))
        cases.append(('nitro', [real() if rnd.random() < 0.7 else rest, rest]))
    for _ in range(1000):
        cases.append(('lens', [real()]))
    nl = chr(10)
    NL = chr(92) + 'n'
    kinds = {'dir': 0, 'drive': 1, 'count': 2, 'nitro': 3, 'lens': 4}
    flat = []
    for kind, ws in cases:
        flat += [kinds[kind], len(ws)] + [int(x) for x in ws]
    body = ('static const uint32_t D[]={' + ','.join('0x%x' % x for x in flat) + '};' + nl +
            f'size_t at=0;for(int c=0;c<{len(cases)};c++){{memset(objects,0,sizeof(objects));' +
            'uint32_t kind=D[at],n=D[at+1];const uint32_t*w=&D[at+2];at+=2+n;' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;' +
            f'if(kind==0){{store(0x1000+20,w[0]?0x1400:0);store(0x1400+20,w[1]);{fd}(ctx,nullptr);}}' +
            f'else if(kind==1){{store(0x1000+8,0x1200);store(0x1200+20,w[0]?0x1400:0);store(0x1400+20,w[1]);store(0x1200+136,w[2]);{fv}(ctx,nullptr);}}' +
            f'else if(kind==2){{store(0x1000+24,w[0]);store(0x1000+28,0x1600);for(uint32_t k=0;k+1<n;k++){{store(0x1600+4*k,0x1800+0x300*k);' +
            f'*pointer(0x1800+0x300*k+529)=uint8_t(w[1+k]);}}{fc}(ctx,nullptr);}}' +
            f'else if(kind==3){{store(0x1000+24,w[0]);store(0x1000+20,w[1]);{fn}(ctx,nullptr);}}' +
            f'else{{store(0x1000+20,w[0]);{fl}(ctx,nullptr);}}' +
            'printf("%d' + NL + '",ctx.r3.s32);}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), [fd, fv, fc, fn, fl], main_src, a.out_dir, 'vehsim_getters')
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s %s -> %s' % (kind, ' '.join('%x' % int(x) for x in ws), o) for (kind, ws), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'vehsim_getters_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
