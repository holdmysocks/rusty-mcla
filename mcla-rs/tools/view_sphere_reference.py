"""The viewport's two sphere tests run offline (the generated code as it is), for the port's test
(crates/game/src/testdata/view_sphere_reference.txt): F-29884 (every plane's d >= 0 - r) and F-91781 (every plane's
d + r >= 0, the first plane's d + r written out), d = x X + (y Y + (z Z + W)) over the 8 plane rows at viewport
+864 .. +976 as F-53768 lays them: rows X / Y / Z / W, lanes (P0, P1, P2, P2) at +864 and (P3, P4, P5, P5) at +928.

  CASE <n> <P0..P5: 24 floats> <x y z r>          (hex bits)
  OUT <F-29884> <F-91781> <F-91781's written value>

python view_sphere_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file> --tool <mcla-tool>
"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

# (vcmpequw. sets cr6 from the lanes: all true -> lt (bit 0), none -> eq; as the other vector harnesses)
_vshim = recomp_closure.vector_shim


def _shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + _vshim()
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    return s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                     'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)


recomp_closure.vector_shim = _shim


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def ff(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def cases(rnd, count):
    out = []
    for _ in range(count):
        planes = []
        for _p in range(6):
            n = [rnd.gauss(0, 1) for _ in range(3)]
            l = math.sqrt(sum(v * v for v in n)) or 1.0
            planes.append([v / l for v in n] + [rnd.uniform(-3000, 3000)])
        c = [rnd.uniform(-3000, 3000) for _ in range(3)]
        r = rnd.choice((0.0, 0.5, 1.0, 2.0, rnd.uniform(0, 50), 1e-3))
        if rnd.random() < 0.6:
            # (on a plane's edge: d = +-r, 0 or a hair off)
            p = rnd.choice(planes)
            d = sum(p[i] * c[i] for i in range(3)) + p[3]
            want = rnd.choice((-r, r, 0.0, -r * 1.0000001, -r - 0.001, -r + 0.001))
            c = [c[i] + p[i] * (want - d) for i in range(3)]
        if rnd.random() < 0.3:
            # (inside all: the planes moved to hold the centre)
            for p in planes:
                d = sum(p[i] * c[i] for i in range(3)) + p[3]
                if d < -r:
                    p[3] += -r - d + rnd.choice((0.0, 0.001, 5.0))
        out.append(([fb(ff(v)) for p in planes for v in p], [fb(ff(v)) for v in c + [r]]))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f1, f2 = 'sub_' + addr(a.tool, 'F-29884'), 'sub_' + addr(a.tool, 'F-91781')
    cs = cases(random.Random(29884), 6000)
    nl, NL = chr(10), chr(92) + 'n'
    flat = [w for p, s in cs for w in p + s]
    body = ('static const uint32_t C[]={' + ','.join('0x%08xu' % v for v in flat) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(size_t k=0;k<sizeof(C)/4/28;k++){const uint32_t*c=&C[28*k];memset(objects,0,sizeof(objects));printf("CASE %zu",k);for(int i=0;i<28;i++)printf(" %08x",c[i]);printf("' + NL + '");' +
            # (P_p component j at row j of its block, lane l)
            'const int B[8]={0,1,2,2,3,4,5,5};for(int q=0;q<8;q++){uint32_t base=0x1000+(q<4?864:928);int l=q%4;for(int j=0;j<4;j++)store(base+16*j+4*l,c[4*B[q]+j]);}' +
            'store(0x1400,c[24]);store(0x1404,c[25]);store(0x1408,c[26]);store(0x140C,c[27]);' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;memcpy(&ctx.v1,pointer(0x1400),16);' +
            # (v1 as lvx128 leaves it: the bytes reversed)
            '{uint8_t t[16];memcpy(t,pointer(0x1400),16);for(int i=0;i<16;i++)ctx.v1.u8[i]=t[15-i];}' +
            ('%s(ctx,nullptr);uint32_t a1=ctx.r3.u32;' % f1) +
            'Context c2{};c2.r1.u32=0x2F00;c2.r3.u32=0x1000;c2.r4.u32=0x1800;{uint8_t t[16];memcpy(t,pointer(0x1400),16);for(int i=0;i<16;i++)c2.v1.u8[i]=t[15-i];}' +
            ('%s(c2,nullptr);' % f2) + 'printf("OUT %u %u %08x' + NL + '",a1,c2.r3.u32,load(0x1800));}}' + nl)
    out = run(build(index(a.recomp), [f1, f2], body, a.out, 'view_sphere', vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    assert out.count('OUT') == len(cs), out[:400]
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(out)
    o = [l.split() for l in out.splitlines() if l.startswith('OUT')]
    print(len(cs), 'cases; F-29884 in', sum(x[1] == '1' for x in o), 'F-91781 in', sum(x[2] == '1' for x in o))


if __name__ == '__main__':
    main()
