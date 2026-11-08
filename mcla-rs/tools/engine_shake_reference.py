"""The engine's shake angle (vehEngine +40), the tail of the engine update F-55648, run offline (generated code
unchanged, scalar mode) on a set-up engine object with the update's callees stubbed: the torque F-48349 returns the
case's torque, the others (the gear-change and nitro helpers, the auxiliary integrator, the transmission's virtual call)
are never reached or do nothing to +40. Engine +4 the tune (+0 inertia, +48 0: no boost), +46 the engine word, +56 /
+60 the transmission objects (gear 3, ratio 0, no change pending), the step at the global time step, the game's
generator (lo, carry) at its global.

Cases: torques -500 .. 800, the torque at best power 0 / negative / 100 .. 800, inertia 0.03 .. 0.3, start angles
round the target and equal to it, steps 1/60, 1/30 and random, the engine word 0 and 1, random generator states.

Writes <out-dir>/engine_shake_ref.txt, one case a line: "torque sync inertia angle dt running lo carry -> angle lo
carry" (hex; a word that reads as one of the
original's addresses is written 82xx_xxxx)."""
import argparse
import re
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
import recomp_closure

recomp_closure.EXTRA += '#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("indirect")' + chr(10) + 'static uint32_t TQ;' + chr(10)


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
    fe = addr(a.tool, 'F-55648')
    torque_fn = 'sub_' + addr(a.tool, 'F-48349')
    rnd = random.Random(55648)
    cases = []
    for _ in range(8000):
        run_ = rnd.random() < 0.8
        tq = rnd.uniform(-500, 800) if run_ else 0.0
        sy = rnd.choice((0.0, -rnd.uniform(1, 100), rnd.uniform(100, 800), rnd.uniform(100, 800)))
        inr = rnd.uniform(0.03, 0.3)
        share = (tq / sy) * inr if sy > 0 else 0.0
        tgt = struct.unpack('>f', struct.pack('>f', share * 0.05))[0]
        ang = rnd.choice((tgt, tgt + rnd.uniform(-0.02, 0.02), tgt + rnd.uniform(-0.001, 0.001), rnd.uniform(-0.1, 0.1)))
        dt = rnd.choice((1 / 60, 1 / 30, rnd.uniform(0.001, 0.05)))
        cases.append((fb(tq), fb(sy), fb(inr), fb(ang), fb(dt), int(run_), rnd.getrandbits(32), rnd.getrandbits(32)))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    stubs = {torque_fn: 'float t;memcpy(&t,&TQ,4);ctx.f1.f64=t;'}
    for f in ('F-74724', 'F-83469', 'F-55540', 'F-58081', 'F-90102'):
        s = 'sub_' + addr(a.tool, f)
        stubs[s] = ''
    E, T, S, X, Y = 0x1000, 0x1200, 0x1400, 0x1600, 0x1800
    body = ('static const uint32_t C[][8]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));TQ=c[0];' +
            f'store({E}+4,{T});store({E}+56,{S});store({E}+60,{X});store({E}+64,0);' +
            f'store({E}+20,c[1]);store({T}+0,c[2]);store({E}+40,c[3]);store(0x827D7508,c[4]);' +
            f'store({E}+44,c[5]);' +  # (+44 the latch 0, +46 the engine word: halfwords of one word)
            f'store({E}+68,0x447A0000);store({E}+8,0x42C80000);store({E}+76,0x42C80000);' +
            f'store({X}+4,{Y});store({X}+20,3);store({X}+24,0);' +
            'store(0x82860890,c[6]);store(0x82860894,c[7]);' +
            f'Context ctx{{}};ctx.r1.u32=0x2F00;ctx.r3.u32={E};' +
            ('sub_%s(ctx,nullptr);printf("%%08x %%08x %%08x' % fe) + NL + f'",load({E}+40),load(0x82860890),load(0x82860894));}}' + nl)
    main_src = (
                'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + fe], main_src, a.out_dir, 'engine_shake', stubs=stubs)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % x for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'engine_shake_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
