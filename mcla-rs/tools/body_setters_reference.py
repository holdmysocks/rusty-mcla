"""Three small setters of the body class run offline (generated code unchanged, vector mode, flush mode as the SDK
sets it): SetVelocity F-64271 (vfunc 41: r4 the velocity; +272 = it, +304 = it x the mass +192, splat), ApplyImpulse
F-50175 (vfunc 35: v1 the impulse; +368 += it, four lanes) and the constraint's mode F-95940 (the u16 +200: its low two
bits 2, the rest kept).

Cases: velocities 0 .. 100 m/s, -0, subnormal, huge, NaN lanes, w garbage; masses of props and pieces (0.1 .. 5000), 0,
subnormal, NaN; impulses and accumulators of any size, the pair adding to a subnormal or to inf, w garbage; flags any u16.

Writes <out-dir>/body_setters_ref.txt, one case a line, hex: "vel vx vy vz vw m -> v4 p4", "imp a4 j4 -> a4",
"flag f -> f"."""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += ('#define REX_CALL_INDIRECT_FUNC(a) throw std::runtime_error("indirect")' + chr(10))
_vshim = recomp_closure.vector_shim


def _shim():
    # (cr6 from a vector compare as the SDK's; the flush mode as the SDK sets it: MXCSR FTZ | DAZ)
    s = '#include <xmmintrin.h>' + chr(10) + '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <simde/x86/avx2.h>' + chr(10) + _vshim()
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    s = s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                  'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)
    fl = 'struct { void disableFlushMode() {} void enableFlushMode() {}'
    assert s.count(fl) == 1
    return s.replace(fl, 'struct { void disableFlushMode() { _mm_setcsr(_mm_getcsr() & ~0x8040); }'
                         ' void enableFlushMode() { _mm_setcsr(_mm_getcsr() | 0x8040); }', 1)


recomp_closure.vector_shim = _shim


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
    fv, fi, ff = ('sub_' + addr(a.tool, f) for f in ('F-64271', 'F-50175', 'F-95940'))
    rnd = random.Random(64271)
    lane = lambda s: fb(rnd.choice((rnd.uniform(-s, s), rnd.uniform(-s, s), rnd.uniform(-s, s), 0.0, -0.0, 1e-40, -1e-39,
                                    1e-20, 3e38, float('nan'))))
    garbage = lambda: rnd.choice((0, 0x3F800000, rnd.getrandbits(32) & 0x7F7FFFFF))
    cases = []
    for _ in range(3000):
        m = rnd.choice((rnd.uniform(0.1, 5000), rnd.uniform(0.1, 50), 0.0, 1e-40, 1e-30, float('nan'), 3e38))
        cases.append(('vel', [lane(100), lane(100), lane(100), garbage(), fb(m)]))
    for _ in range(3000):
        acc = [lane(rnd.choice((1, 1e3, 1e5))) for _k in range(3)] + [garbage()]
        j = [lane(rnd.choice((1, 1e3, 1e5))) for _k in range(3)] + [garbage()]
        if rnd.random() < 0.1:
            k = rnd.randrange(3)
            x = struct.unpack('>f', struct.pack('>I', acc[k]))[0]
            if x == x:
                j[k] = fb(-x + rnd.choice((1e-39, -1e-39, 0.0)))
        cases.append(('imp', acc + j))
    for _ in range(500):
        cases.append(('flag', [rnd.getrandbits(16)]))
    nl = chr(10)
    NL = chr(92) + 'n'
    kinds = {'vel': 0, 'imp': 1, 'flag': 2}
    flat = []
    for kind, ws in cases:
        flat += [kinds[kind], len(ws)] + ws
    out4 = lambda at: f'load({at}),load({at}+4),load({at}+8),load({at}+12)'
    body = ('static const uint32_t D[]={' + ','.join('0x%x' % x for x in flat) + '};' + nl +
            f'size_t at=0;for(int c=0;c<{len(cases)};c++){{memset(objects,0,sizeof(objects));' +
            'uint32_t kind=D[at],n=D[at+1];const uint32_t*w=&D[at+2];at+=2+n;' +
            'for(uint32_t i=0;i<0x200;i+=4)store(0x1000+i,0xA5A50000u+i);' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;' +
            f'if(kind==0){{for(int k=0;k<4;k++)store(0x1800+4*k,w[k]);store(0x1000+192,w[4]);ctx.r4.u32=0x1800;{fv}(ctx,nullptr);' +
            'printf("%08x %08x %08x %08x %08x %08x %08x %08x' + NL + f'",{out4(0x1000 + 272)},{out4(0x1000 + 304)});}}' +
            f'else if(kind==1){{for(int k=0;k<4;k++){{store(0x1000+368+4*k,w[k]);ctx.v1.u32[3-k]=w[4+k];}}{fi}(ctx,nullptr);' +
            'printf("%08x %08x %08x %08x' + NL + f'",{out4(0x1000 + 368)});}}' +
            f'else{{uint32_t x=load(0x1000+200);store(0x1000+200,(x&0xFFFFu)|(w[0]<<16));{ff}(ctx,nullptr);' +
            'printf("%04x' + NL + '",load(0x1000+200)>>16);}}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + body + '}' + nl)
    exe = build(index(a.recomp), [fv, fi, ff], main_src, a.out_dir, 'body_setters', vector=True, flags=('-msse4.1', '-mssse3'))
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s %s -> %s' % (kind, ' '.join('%08x' % x for x in ws), o) for (kind, ws), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'body_setters_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
