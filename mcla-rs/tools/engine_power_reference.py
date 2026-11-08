"""The engine's torque at a speed F-71482 and power at a speed F-46563 run offline (generated code unchanged, scalar
mode), their two torque sources stubbed - the control-value curve F-36517 and the built-in curve F-91323, both
checked on their own - each stub logging that it ran and the speed it got, and returning the case's torque.

The choice: the curve when the engine's curve count (+120, u16) is not 0 and the global use-curve byte is set (the
boot routine F-20115 sets it to 1, its only writer), else the built-in curve. Power = (torque x speed) x 1/550.

Cases: counts 0 / 12 / random, the byte 0 / 1 / random, speeds 0 / negative / 0 .. 1200 rad/s / huge / NaN, torques
-500 .. 1500 / 0 / huge / NaN.

Writes <out-dir>/engine_power_ref.txt, one case a line, hex: "count byte w torque -> which_t t | which_p p" (which: 1
the curve ran, 2 the built-in, with the speed it was given checked equal to w)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += ('#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("indirect")' + chr(10) +
                         'static uint32_t TQ, W, WHICH;' + chr(10))
# (the use-curve byte, as an offset from BASE so the file cites no address)
BASE = 0x828D << 16
FLAG = 0xA40C


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
    ft, fp = 'sub_' + addr(a.tool, 'F-71482'), 'sub_' + addr(a.tool, 'F-46563')
    curve, builtin = 'sub_' + addr(a.tool, 'F-36517'), 'sub_' + addr(a.tool, 'F-91323')
    rnd = random.Random(46563)
    cases = []
    for _ in range(4000):
        count = rnd.choice((0, 12, 12, rnd.getrandbits(16)))
        flag = rnd.choice((0, 1, 1, rnd.getrandbits(8)))
        w = rnd.choice((0.0, -rnd.uniform(0, 100), rnd.uniform(0, 1200), rnd.uniform(50, 900), 1e30, float('nan')))
        t = rnd.choice((rnd.uniform(-500, 1500), 0.0, rnd.uniform(0, 800), 3e38, float('nan')))
        cases.append((count, flag, fb(w), fb(t)))
    nl = chr(10)
    NL = chr(92) + 'n'
    stub = lambda k: (f'{{float g=float(ctx.f1.f64);uint32_t u;memcpy(&u,&g,4);if(u!=W)throw std::runtime_error("speed");}}' +
                      f'WHICH={k};float t;memcpy(&t,&TQ,4);ctx.f1.f64=t;')
    stubs = {curve: stub(1), builtin: stub(2)}
    flag_at = '0x%08x' % (BASE + FLAG)
    rows = nl.join('{0x%x,0x%x,0x%08x,0x%08x},' % c for c in cases)
    run_one = lambda fn: (f'{{WHICH=0;Context ctx{{}};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;float w;memcpy(&w,&W,4);ctx.f1.f64=w;{fn}(ctx,nullptr);' +
                          'float r=float(ctx.f1.f64);uint32_t u;memcpy(&u,&r,4);printf("%u %08x",WHICH,u);}')
    body = ('static const uint32_t C[][4]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            f'store(0x1000+120,c[0]<<16);{{uint32_t f=load({flag_at}&~3u);f=(f&0x00FFFFFFu)|(c[1]<<24);store({flag_at}&~3u,f);}}' +
            'W=c[2];TQ=c[3];' + run_one(ft) + 'printf(" | ");' + run_one(fp) + 'printf("' + NL + '");}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), [ft, fp], main_src, a.out_dir, 'engine_power', stubs=stubs)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%04x %02x %08x %08x -> %s' % (*c, o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'engine_power_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
