"""The walkers' turn clip choice (F-16288) run offline: the generated code whole and unchanged in vector mode, with the
quaternion rotate F-86566 and the xz angle F-32060 (and the C library's acos) in the closure, on flat guest memory. Stubbed,
as inputs of the case: the clip's root sample (F-21043 at the clip's end: the case's quaternion of each entry, written to
the out block's first four words, answer 1). The list is the manager's avoid list (+8 the entries, 20 bytes: +4
AvoidType, +16 the clip; +12 the count), here up to 8 entries of random types, clips present or not, and turns about y
with a little tilt; f1 the wanted turn.
Writes <out-dir>/turn_pick_ref.txt: the case's words, then '->', the picked entry (-1 none) and the rest angle written (hex;
ffffffff: not written)."""

import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

N = 8

_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    return s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                     'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)


recomp_closure.vector_shim = vector_shim
recomp_closure.EXTRA += ('#define ctr r2\nstatic const uint32_t *K;static int calls[16];\n'
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n')
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)


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
    pick, sample = (addr(a.tool, f) for f in ('F-16288', 'F-21043'))
    rnd = random.Random(16288)
    cases = []
    yaws = [math.pi / 2, -math.pi / 2, math.pi, -math.pi, 0.0]
    for _ in range(4000):
        n = rnd.choice((0, 1, 2, 4, 8, 8))
        want = rnd.choice((rnd.uniform(-math.pi, math.pi), rnd.uniform(-4, 4), rnd.choice(yaws)))
        c = [n, fb(want)]
        for i in range(N):
            ty = rnd.choice((3, 3, 3, 0, 4))
            pres = int(rnd.random() < 0.9)
            y = rnd.choice(yaws) + rnd.choice((0.0, 0.0, rnd.uniform(-0.2, 0.2), rnd.uniform(-1e-5, 1e-5)))
            if rnd.random() < 0.15:
                y = want + rnd.uniform(-3e-5, 3e-5)
            t = rnd.choice((0.0, 0.0, rnd.uniform(-0.05, 0.05)))
            # (a turn y about the up axis, then a small tilt t about x)
            qy = (0.0, math.sin(y / 2), 0.0, math.cos(y / 2))
            qt = (math.sin(t / 2), 0.0, 0.0, math.cos(t / 2))
            x1, y1, z1, w1 = qt
            x2, y2, z2, w2 = qy
            qq = (w1 * x2 + x1 * w2 + y1 * z2 - z1 * y2, w1 * y2 - x1 * z2 + y1 * w2 + z1 * x2,
                  w1 * z2 + x1 * y2 - y1 * x2 + z1 * w2, w1 * w2 - x1 * x2 - y1 * y2 - z1 * z2)
            c += [ty, pres] + [fb(v) for v in qq]
        cases.append(c)
    W = len(cases[0])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    # the root sample: r3 = the clip (0x1800 + 16 i), r7 = the out block (the quaternion first)
    stub = ('int i=(ctx.r3.u32-0x1800)/16;for(int j=0;j<4;j++)store(ctx.r7.u32+4*j,K[2+6*i+2+j]);ctx.r3.u32=1;')
    setup = ('memset(objects,0,sizeof(objects));K=c;'
             'store(0x1000,0x1080);store(0x1080+8,0x1100);objects[0x1080-0x1000+12]=0;objects[0x1080-0x1000+13]=c[0];'
             'for(int i=0;i<8;i++){store(0x1100+20*i+4,c[2+6*i]);store(0x1100+20*i+16,c[3+6*i]?0x1800+16*i:0);'
             'store(0x1800+16*i+12,0x3f800000);}')
    body = ('static const uint32_t C[][%d]={' % W + nl + rows + nl + '};' + nl +
            'for(auto&c:C){' + setup +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r6.u32=0x2FC0;store(0x2FC0,0xFFFFFFFF);'
            'float v;memcpy(&v,&c[1],4);ctx.f1.f64=v;'
            'sub_%s(ctx,nullptr);int e=ctx.r3.u32?int((ctx.r3.u32-0x1100)/20):-1;' % pick +
            ('printf("%%d %%08x%s",e,load(0x2FC0));}' % NL) + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + pick], main_src, a.out_dir, 'turn_pick', stubs={'sub_' + sample: stub}, vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%x' % x for x in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'turn_pick_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(o.split()[0] != '-1' for o in out), 'with a pick')


if __name__ == '__main__':
    main()
