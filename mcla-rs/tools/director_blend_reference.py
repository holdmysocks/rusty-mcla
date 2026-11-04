"""Run the director's blend camera step of the original offline (static recompilation, native, flat guest memory) and
record its results for the port's test (crates/game/src/testdata/director_blend_ref.txt).

  DB <48 input words> -> <done> <weight> <left> <4 lens> <9 rows> <3 pos>
        F-18583 (camTransitionCS's step, every callee unchanged: the time source F-76427, the eases F-86323 / F-17888 /
        F-39265, the angles of a matrix F-33934 with F-49999, the matrix of angles F-62659, the game's own sin / cos /
        atan2 / asin) on random blends. The finish F-70013 is replaced by a stub that only records that it was called
        (`done` = 1; it hands the director to the target and is not compared here). Inputs: mode, T (the director's +16 /
        +20), dt (the time source's global), the blend's weight / goal / time left, then per camera (source, target): the
        corrected-matrix byte +156 (the matrix goes to +80 when set, +16 else; the other slot is filled with NaN so a
        wrong read shows), whether it follows an object, that object's place (+48), the matrix's three rows and place,
        the lens +168 / +172 / +176 / +180. All words are f32 bits except the flags.
"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

# (the step's vector unpack and the angle code's sign extensions: the register union's signed halves / bytes and the
# vector temporary the generated prologue declares)
_shim = recomp_closure.vector_shim
recomp_closure.vector_shim = lambda: _shim().replace('uint16_t u16; uint8_t u8;', 'uint16_t u16; uint8_t u8; int16_t s16; int8_t s8;', 1) \
    + 'typedef VReg PPCVRegister;\n'
# (compiled with -msse4.1 as the recomp's own x64 build is (rexglue_helpers.cmake): vmsum3fp is simde_mm_dp_ps, and only
# the native dpps sums as the game does, (0 + z z') + (y y' + x x'); simde's portable fallback sums the lanes in turn)
_run = recomp_closure.subprocess.run
recomp_closure.subprocess.run = lambda cmd, *a, **k: _run([cmd[0], '-msse4.1'] + cmd[1:] if cmd and str(cmd[0]).endswith('g++') else cmd, *a, **k)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def rot(rnd):
    # rows of a random rotation (from a random unit quaternion), sometimes skewed or scaled a little
    q = [rnd.gauss(0, 1) for _ in range(4)]
    n = math.sqrt(sum(v * v for v in q))
    w, x, y, z = (v / n for v in q)
    m = [[1 - 2 * (y * y + z * z), 2 * (x * y + w * z), 2 * (x * z - w * y)],
         [2 * (x * y - w * z), 1 - 2 * (x * x + z * z), 2 * (y * z + w * x)],
         [2 * (x * z + w * y), 2 * (y * z - w * x), 1 - 2 * (x * x + y * y)]]
    if rnd.random() < 0.3:
        m = [[v + rnd.uniform(-0.05, 0.05) for v in r] for r in m]
    if rnd.random() < 0.1:
        m = [[v * rnd.uniform(0.5, 2.0) for v in r] for r in m]
    return m


def cam(rnd, near):
    sel = rnd.choice([0, 1])
    has = rnd.choice([1, 1, 1, 0])
    t = [rnd.uniform(-500, 500), rnd.uniform(-20, 80), rnd.uniform(-500, 500)]
    m = rot(rnd)
    pos = [t[i] + rnd.uniform(-8, 8) for i in range(3)] if near else [rnd.uniform(-500, 500) for _ in range(3)]
    lens = [rnd.uniform(30, 90), rnd.choice([0.1, 0.2, rnd.uniform(0.05, 1)]), rnd.choice([600.0, 6000.0, rnd.uniform(100, 9000)]),
            rnd.choice([0.0, rnd.uniform(-5, 5)])]
    return [sel, has] + [fb(v) for v in t] + [fb(v) for r in m for v in r] + [fb(v) for v in pos] + [fb(v) for v in lens]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = index(a.recomp)
    step, fin = (addr(a.tool, f) for f in ('F-18583', 'F-70013'))
    rnd = random.Random(1104)
    rows = []
    for k in range(5000):
        mode = rnd.choice([0, 1, 2, 3, 3, 3, 4])
        t = rnd.choice([1.2, 0.5, 0.7, 1.0, rnd.uniform(0.05, 3)])
        dt = rnd.choice([1 / 30, 1 / 60, rnd.uniform(0, 0.1)])
        goal = rnd.choice([1.0, 1.0, 1.0, rnd.uniform(0.2, 1.0)])
        weight = rnd.choice([0.0, rnd.uniform(0, 1), goal, rnd.uniform(0, goal)])
        left = rnd.choice([t, rnd.uniform(-0.1, t), rnd.uniform(0, 0.05), rnd.uniform(t, 2 * t), dt])
        near = rnd.random() < 0.8
        rows.append([mode, fb(t), fb(dt), fb(weight), fb(goal), fb(left)] + cam(rnd, near) + cam(rnd, near))
    table = 'static const uint32_t CASES[][48]={' + ','.join('{' + ','.join('0x%08xu' % v for v in r) + '}' for r in rows) + '};\n'
    B, D, CA, CB, TA, TB, DONE, DT = 0x1000, 0x1100, 0x1200, 0x1300, 0x1400, 0x1480, 0x1600, 0x827D7508
    body = table + (
        'static void putcam(uint32_t c,uint32_t t,const uint32_t*w){'
        ' for(int i=0;i<16;i++){store(c+16+4*i,0x7fc00000u);store(c+80+4*i,0x7fc00000u);}'
        ' uint32_t m=c+(w[0]?80:16);'
        ' for(int r=0;r<3;r++){for(int i=0;i<3;i++)store(m+16*r+4*i,w[5+3*r+i]);store(m+16*r+12,0);}'
        ' for(int i=0;i<3;i++)store(m+48+4*i,w[14+i]);store(m+60,0x3f800000u);'
        ' *pointer(c+156)=uint8_t(w[0]);'
        ' for(int i=0;i<4;i++)store(c+168+4*i,w[17+i]);'
        ' if(w[1]){store(c+144,t);for(int i=0;i<3;i++)store(t+48+4*i,w[2+i]);store(t+60,0x3f800000u);}else store(c+144,0);}\n'
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});\n'
        'for(size_t k=0;k<sizeof(CASES)/sizeof(CASES[0]);k++){const uint32_t*c=CASES[k];memset(objects,0,sizeof(objects));'
        'store16(0x%x,uint16_t(c[0]));store(0x%x,c[1]);store(0x%x,c[2]);'
        'store(0x%x,0x%x);store(0x%x,0x%x);store(0x%x,0x%x);store(0x%x,0);*pointer(0x%x)=0;'
        'store(0x%x,c[3]);store(0x%x,c[4]);store(0x%x,c[5]);'
        'putcam(0x%x,0x%x,c+6);putcam(0x%x,0x%x,c+27);'
        'Context ctx{};ctx.r3.u32=0x%x;ctx.r1.u32=0x2f00;sub_%s(ctx,nullptr);'
        'printf("DB");for(int i=0;i<48;i++)printf(" %%08x",c[i]);printf(" -> %%u",load(0x%x));'
        'printf(" %%08x %%08x",load(0x%x),load(0x%x));for(int i=0;i<4;i++)printf(" %%08x",load(0x%x+4*i));'
        'for(int r=0;r<3;r++)for(int i=0;i<3;i++)printf(" %%08x",load(0x%x+16*r+4*i));'
        'for(int i=0;i<3;i++)printf(" %%08x",load(0x%x+4*i));printf("\\n");}}\n') % (
        D + 16, D + 20, DT,
        B + 148, D, B + 192, CA, B + 196, CB, B + 200, B + 184,
        B + 204, B + 208, B + 212,
        CA, TA, CB, TB,
        B, step[4:], DONE,
        B + 204, B + 212, B + 168,
        B + 16, B + 64)
    a.ref.write_text(run(build(idx, [step], body, a.out, 'dirblend', {fin: 'store(0x%x,1);' % DONE}, vector=True), a.image))


if __name__ == '__main__':
    main()
