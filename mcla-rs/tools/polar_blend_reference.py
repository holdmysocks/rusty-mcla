"""Run the pause cameras' polar blend and its path test of the original offline (static recompilation, native, flat guest
memory) and record their results for the port's test (crates/game/src/testdata/polar_blend_ref.txt).

  PB <15 input words> -> <pos 3> <look 3> <up 3> <fov>
        F-22967 (SetPolarCam's blend, every callee unchanged: the look-at rows F-97951, the angles of a
        matrix F-33934 with order "zxy", the matrix of angles F-62659) on random camera pairs. Inputs: t, the flag (go the long way round
        on y), fov A, fov B, then A's eye and look-at point, B's eye and look-at point (f32 bits). The up vector is the
        image's (0, 1, 0) at 0x827D8480, as F-32916 passes it.
  PP <15 input words> <answers> -> <ok> <n> <n x (from 3, to 3)>
        The path test (the blend at t = 0.25, 0.5, 0.75 and the line probe of the four pieces eye A -> P25 -> P50 -> P75 ->
        eye B): the probe is a stub that logs its two points and answers bit k of `answers` for its k-th call. `t` is unused.

Function names come from `mcla-tool registry addr`. Usage:
  python mcla-rs/tools/polar_blend_reference.py --recomp D:\\MCLA-static --image work/2026-10-10/original-image.bin \\
      --out work/<session>/pbref --ref mcla-rs/crates/game/src/testdata/polar_blend_ref.txt --tool <mcla-tool.exe>
"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

_shim = recomp_closure.vector_shim
recomp_closure.vector_shim = lambda *a: _shim(*a).replace('uint16_t u16; uint8_t u8;', 'uint16_t u16; uint8_t u8; int16_t s16; int8_t s8;', 1) \
    + 'typedef VReg PPCVRegister;\n'
# (-msse4.1 as the recomp's own x64 build: the dot products by the CPU's dpps)
_run = recomp_closure.subprocess.run
recomp_closure.subprocess.run = lambda cmd, *a, **k: _run([cmd[0], '-msse4.1'] + cmd[1:] if cmd and str(cmd[0]).endswith('g++') else cmd, *a, **k)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def point(rnd, near=None):
    if near is not None and rnd.random() < 0.7:
        return [near[i] + rnd.uniform(-12, 12) for i in range(3)]
    return [rnd.uniform(-60, 60), rnd.uniform(-5, 30), rnd.uniform(-60, 60)]


def case(rnd):
    t = rnd.choice([0.0, 1.0, 0.5, rnd.uniform(0, 1), rnd.uniform(0, 1), rnd.uniform(-0.2, 1.2)])
    flag = rnd.choice([0, 0, 1])
    fa, fb_ = rnd.uniform(20, 70), rnd.uniform(20, 70)
    la = point(rnd)
    ea = point(rnd, la)
    lb = point(rnd, la) if rnd.random() < 0.6 else list(la)
    eb = point(rnd, lb)
    if rnd.random() < 0.05:
        eb = [lb[0], lb[1] + rnd.uniform(1, 9), lb[2]]  # straight above: the look-at's degenerate frame
    if rnd.random() < 0.05:
        ea, la = list(eb), list(lb)  # the same camera
    return [fb(t), flag, fb(fa), fb(fb_)] + [fb(v) for v in ea + la + eb + lb]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = index(a.recomp)
    blend, path, probe = (addr(a.tool, f) for f in ('F-22967', 'F-56569', 'F-96274'))
    rnd = random.Random(1117)
    rows = [case(rnd) for _ in range(4000)]
    answers = [rnd.choice([0, 0, 0, rnd.randrange(16), 1 << rnd.randrange(4)]) for _ in rows]
    table = 'static const uint32_t CASES[][17]={' + ','.join(
        '{' + ','.join('0x%08xu' % v for v in r + [ans]) + '}' for r, ans in zip(rows, answers)) + '};\n'
    # guest memory: A eye 0x1000, A look 0x1010, up 0x1020, B eye 0x1030, B look 0x1040; out pos / look / up / fov
    # 0x1100 / 0x1110 / 0x1120 / 0x1130; the probe's log: answers 0x17F8, count 0x17FC, records 0x1800 (32 bytes)
    S = 0x2f00
    body = table + (
        'static void put(uint32_t at,const uint32_t*w){for(int i=0;i<3;i++)store(at+4*i,w[i]);store(at+12,0);}\n'
        'static void setup(const uint32_t*c){memset(objects,0,sizeof(objects));put(0x1000,c+4);put(0x1010,c+7);'
        'store(0x1020,0);store(0x1024,0x3f800000u);store(0x1028,0);store(0x102c,0);put(0x1030,c+10);put(0x1040,c+13);'
        'store(0x17F8,c[16]);store(0x17FC,0);}\n'
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});\n'
        'for(size_t k=0;k<sizeof(CASES)/sizeof(CASES[0]);k++){const uint32_t*c=CASES[k];\n'
        ' setup(c);{Context ctx{};ctx.r1.u32=0x%x;store(0x%x,0x1100);store(0x%x,0x1110);store(0x%x,0x1120);store(0x%x,0x1130);'
        '*pointer(0x%x)=uint8_t(c[1]);ctx.r4.u32=0x1000;ctx.r5.u32=0x1010;ctx.r6.u32=0x1020;ctx.r8.u32=0x1030;ctx.r9.u32=0x1040;'
        'ctx.r10.u32=0x1020;uint32_t w=c[0];float x;memcpy(&x,&w,4);ctx.f1.f64=x;w=c[2];memcpy(&x,&w,4);ctx.f2.f64=x;'
        'w=c[3];memcpy(&x,&w,4);ctx.f3.f64=x;sub_%s(ctx,nullptr);\n'
        ' printf("PB");for(int i=0;i<16;i++)printf(" %%08x",c[i]);printf(" ->");for(int i=0;i<3;i++)printf(" %%08x",load(0x1100+4*i));'
        'for(int i=0;i<3;i++)printf(" %%08x",load(0x1110+4*i));for(int i=0;i<3;i++)printf(" %%08x",load(0x1120+4*i));'
        'printf(" %%08x\\n",load(0x1130));}\n'
        ' setup(c);{Context ctx{};ctx.r1.u32=0x%x;*pointer(0x%x)=uint8_t(c[1]);ctx.r3.u32=0x1000;ctx.r4.u32=0x1010;ctx.r5.u32=0x1020;'
        'ctx.r7.u32=0x1030;ctx.r8.u32=0x1040;ctx.r9.u32=0x1020;uint32_t w=c[2];float x;memcpy(&x,&w,4);ctx.f1.f64=x;w=c[3];memcpy(&x,&w,4);ctx.f2.f64=x;'
        'sub_%s(ctx,nullptr);uint32_t n=load(0x17FC);\n'
        ' printf("PP");for(int i=0;i<17;i++)printf(" %%08x",c[i]);printf(" -> %%u %%u",unsigned(ctx.r3.u32&0xff),n);'
        'for(uint32_t j=0;j<n;j++){for(int i=0;i<3;i++)printf(" %%08x",load(0x1800+32*j+4*i));for(int i=0;i<3;i++)printf(" %%08x",load(0x1810+32*j+4*i));}'
        'printf("\\n");}}}\n') % (
        S, S + 92, S + 100, S + 108, S + 116, S + 127, blend[4:],
        S, S + 87, path[4:])
    stub = ('uint32_t n=load(0x17FC);for(int k=0;k<4;k++){store(0x1800+32*n+4*k,ctx.v1.u32[3-k]);store(0x1810+32*n+4*k,ctx.v2.u32[3-k]);}'
            'store(0x17FC,n+1);ctx.r3.u64=(load(0x17F8)>>n)&1;')
    a.ref.write_text(run(build(idx, [blend, path], body, a.out, 'polarblend', {probe: stub}, vector=True), a.image))


if __name__ == '__main__':
    main()
