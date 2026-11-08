"""mcCarFluid's small pieces run offline (generated code unchanged, vector mode): the constructor F-52233 with the base's
F-72081 under it, the base's leave F-29560 and the current F-98483.

The constructor runs on memory filled with a pattern, so the words it leaves alone show: in, level, current, buoyancy,
damping and the timer it writes. The leave runs on random +12 words. The current runs as the fluid callback F-58580
calls it: the direction (1, 0, 0) from the callback's two constants (1.0, 0.0) with the stack's w lane, times the
tune's CurrentScale (+12) splatted - in the vector unit's flush mode, so a subnormal scale gives zeros. Scales: the
shipped 2, random, 0 / -0, negative, subnormal, huge, inf, NaN.

Writes <out-dir>/fluid_small_ref.txt, all words hex: "ctor -> +12 +16 +32 +36 +40 +44 +1072 +1076 +1088", then
"leave flags -> flags" and "flow scale w -> x y z w" lines (a word that reads as one of the original's addresses is
written 82xx_xxxx)."""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += '#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("indirect")' + chr(10)
_shim = recomp_closure.vector_shim


def flushing_shim():
    # (the vector unit's flush mode as the SDK sets it: MXCSR FTZ | DAZ while on, cleared by disableFlushMode)
    return _shim().replace('struct { void disableFlushMode() {} void enableFlushMode() {}',
                           'struct { void disableFlushMode() { _mm_setcsr(_mm_getcsr() & ~0x8040); }'
                           ' void enableFlushMode() { _mm_setcsr(_mm_getcsr() | 0x8040); }', 1).replace(
        '#include <simde/x86/sse4.1.h>', '#include <xmmintrin.h>' + chr(10) + '#include <simde/x86/sse4.1.h>', 1)


recomp_closure.vector_shim = flushing_shim


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
    fc, fl, ff = ('sub_' + addr(a.tool, x) for x in ('F-52233', 'F-29560', 'F-98483'))
    rnd = random.Random(98483)
    leaves = [rnd.choice((0, 1, 0xFFFFFFFF, rnd.getrandbits(32))) for _ in range(500)]
    flows = []
    for _ in range(1500):
        s = rnd.choice((2.0, rnd.uniform(-10, 10), 0.0, -0.0, -rnd.uniform(0, 5), 1e-40, -1e-40, 1e-39, 3e38, float('inf'),
                        float('-inf'), float('nan'), 1.2e-38))
        flows.append((fb(s), rnd.choice((0, 0x3F800000, rnd.getrandbits(32)))))
    nl = chr(10)
    NL = chr(92) + 'n'
    F, T, D = 0x1000, 0x1480, 0x1600
    body = ('for(uint32_t i=0;i<0x600;i+=4)store(' + str(F) + '+i,0xA5A50000u+i);' +
            f'{{Context ctx{{}};ctx.r1.u32=0x2F00;ctx.r3.u32={F};{fc}(ctx,nullptr);}}' +
            'printf("ctor -> %08x %08x %08x %08x %08x %08x %08x %08x %08x' + NL + '",' +
            ','.join(f'load({F}+{o})' for o in (12, 16, 32, 36, 40, 44, 1072, 1076, 1088)) + ');' + nl +
            'static const uint32_t L[]={' + ','.join('0x%08x' % x for x in leaves) + '};' + nl +
            f'for(auto l:L){{store({F}+12,l);Context ctx{{}};ctx.r1.u32=0x2F00;ctx.r3.u32={F};{fl}(ctx,nullptr);' +
            'printf("leave %08x -> %08x' + NL + f'",l,load({F}+12));}}' + nl +
            'static const uint32_t W[][2]={' + ','.join('{0x%08x,0x%08x}' % x for x in flows) + '};' + nl +
            f'for(auto&w:W){{store({F}+4,{T});store({T}+12,w[0]);store({D},0x3F800000);store({D}+4,0);store({D}+8,0);store({D}+12,w[1]);' +
            f'Context ctx{{}};ctx.r1.u32=0x2F00;ctx.r3.u32={F};ctx.r4.u32={D};{ff}(ctx,nullptr);' +
            'printf("flow %08x %08x -> %08x %08x %08x %08x' + NL + f'",w[0],w[1],load({F}+32),load({F}+36),load({F}+40),load({F}+44));}}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), [fc, fl, ff], main_src, a.out_dir, 'fluid_small', vector=True)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == 1 + len(leaves) + len(flows), len(out)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'fluid_small_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(out) + nl))
    print(len(out), 'lines')


if __name__ == '__main__':
    main()
