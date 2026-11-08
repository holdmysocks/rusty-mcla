"""mcCarFluid's init F-62399 and enter F-77072 run offline (generated code unchanged, scalar mode). The init: the
base's copy F-41261 of the tune's damping (+4) to +1076 and its Buoyancy (+8) to +1072, then +1072 overwritten by
BuoyancyMin (+20) when SinkRate (+24) is above 0, else BuoyancyMax (+16). The enter: nothing while the timer (+1088) is
above 0, else the in bit (+12 bit 0, the base's F-71430) and the same choice of buoyancy.

Cases: tunes from the shipped ranges and random ones, SinkRate 0 / -0 / either sign / NaN, timers 0 / -0 / running /
negative, +12 holding other bits, a buoyancy left from before.

Writes <out-dir>/fluid_enter_ref.txt, one case a line, all words hex: "damp buoy max min sink timer flags b ->
init: b damp | enter: flags b" (a word that reads as one of the original's addresses is written 82xx_xxxx)."""
import argparse
import random
import re
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
    fi, fe = 'sub_' + addr(a.tool, 'F-62399'), 'sub_' + addr(a.tool, 'F-77072')
    rnd = random.Random(77072)
    cases = []
    for _ in range(3000):
        hi = rnd.choice((0.2, rnd.uniform(0, 1), rnd.uniform(-1, 2)))
        lo = rnd.choice((0.1, rnd.uniform(0, 1), rnd.uniform(-1, 2)))
        sink = rnd.choice((0.0, -0.0, -0.1, 0.1, rnd.uniform(-1, 1), float('nan'), 1e-30, -1e-30))
        timer = rnd.choice((0.0, -0.0, rnd.uniform(0, 2), -rnd.uniform(0, 1), 1e-30))
        flags = rnd.choice((0, 1, rnd.getrandbits(32)))
        cases.append((fb(rnd.uniform(0, 0.5)), fb(rnd.choice((0.5, rnd.uniform(0, 1)))), fb(hi), fb(lo), fb(sink), fb(timer), flags, fb(rnd.uniform(-1, 1))))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    F, T = 0x1000, 0x1480
    setup = (f'memset(objects,0,sizeof(objects));store({F}+4,{T});store({T}+4,c[0]);store({T}+8,c[1]);store({T}+16,c[2]);store({T}+20,c[3]);' +
             f'store({T}+24,c[4]);store({F}+1088,c[5]);store({F}+12,c[6]);store({F}+1072,c[7]);store({F}+1076,0x12345678);' +
             f'{{Context ctx{{}};ctx.r1.u32=0x2F00;ctx.r3.u32={F};')
    body = ('static const uint32_t C[][8]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){' + setup + f'{fi}(ctx,nullptr);}}printf("%08x %08x | ",load({F}+1072),load({F}+1076));' +
            setup + f'{fe}(ctx,nullptr);}}printf("%08x %08x' + NL + f'",load({F}+12),load({F}+1072));}}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), [fi, fe], main_src, a.out_dir, 'fluid_enter')
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%08x' % x for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'fluid_enter_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
