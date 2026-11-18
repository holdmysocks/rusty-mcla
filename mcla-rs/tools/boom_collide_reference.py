"""Run the boom camera's collision of the original offline (static recompilation, native, flat guest memory) and record
its results for the port's test (crates/game/src/testdata/boom_collide_ref.txt).

  BC <case words> -> <log 40> <+128 4> <+156> <return>
        One call of F-20517 (chase / reverse views: the ball's contacts at P, else the sweep P -> C) or F-77832 (POV /
        bumper: the sweep only) on a camera object: the boom frame's matrix +256 (4 rows), the tune's Center (+16) and
        Collide (+64) through +240, the camera's place +128, the viewport's near +704 and lens tangents +712 / +716
        through +152, the focus +432 (in the boom frame), roll +448, include +200, states byte +196, the target's
        instance [+192] +28. The ball F-87651's wrapper and the sweep's wrapper are stubs: they log their arguments
        (centre / start, delta, length, radius, instance, include, states, the stack words) and answer the case's
        records (contact point +16, normal +32, 96 bytes apart). The sum F-87913 runs unchanged; the look F-97951 and
        the roll F-83846 (both verified elsewhere) are stubs that log their arguments. Out: the log (40 words), the
        camera's place +128 after the call, byte +156 and the return.

Case words: fn (0 F-20517, 1 F-77832), M 16, Center 4, Collide 4, C 4, near, tan up, tan across, focus 4, roll,
include, states, ball count, 10 x (point 4, normal 4), sweep count, point 4, normal 4  = 129 words.

Function names come from `mcla-tool registry addr`. Usage:
  python mcla-rs/tools/boom_collide_reference.py --recomp D:\\MCLA-static --image work/2026-10-10/original-image.bin \\
      --out work/<session>/boomcol --ref mcla-rs/crates/game/src/testdata/boom_collide_ref.txt --tool <mcla-tool.exe>
"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


CAM, TUNE, VP, TGT, FR, LOG = 0x1000, 0x1400, 0x1600, 0x1900, 0x1A00, 0x1E00
NW = 129


def unit(rnd):
    while True:
        v = [rnd.uniform(-1, 1) for _ in range(3)]
        n = math.sqrt(sum(x * x for x in v))
        if 0.1 < n <= 1:
            return [x / n for x in v]


def case(rnd, fn):
    yaw = rnd.uniform(-math.pi, math.pi)
    pitch = rnd.uniform(-0.4, 0.4)
    # a boom frame: right, up, back rows (near orthonormal, sometimes a little off), the origin
    cy, sy, cp, sp = math.cos(yaw), math.sin(yaw), math.cos(pitch), math.sin(pitch)
    r0 = [cy, 0.0, -sy]
    r1 = [sy * sp, cp, cy * sp]
    r2 = [sy * cp, -sp, cy * cp]
    if rnd.random() < 0.2:
        r1 = [x + rnd.uniform(-0.01, 0.01) for x in r1]
    o = [rnd.uniform(-2000, 2000), rnd.uniform(-5, 80), rnd.uniform(-2000, 2000)]
    w = lambda: rnd.choice([0.0, 0.0, 1.0, rnd.uniform(-2, 2)])
    m = r0 + [w()] + r1 + [w()] + r2 + [w()] + o + [1.0]
    center = [rnd.uniform(-0.3, 0.3), rnd.uniform(0.3, 1.2), rnd.uniform(-0.5, 0.5), w()]
    collide = [center[0] + rnd.uniform(-0.3, 0.3), center[1] + rnd.uniform(-0.2, 0.6), center[2] + rnd.uniform(-0.5, 2), w()]
    back = rnd.uniform(0.5, 9) if fn == 0 else rnd.uniform(0.0, 0.8)
    loc = [rnd.uniform(-0.5, 0.5), rnd.uniform(0.5, 2.5), back]
    c = [o[i] + loc[0] * r0[i] + loc[1] * r1[i] + loc[2] * r2[i] for i in range(3)] + [w()]
    if rnd.random() < 0.03:
        c = [o[i] + (collide[0] - center[0]) * r0[i] + (collide[1] - center[1]) * r1[i] + (collide[2] - center[2]) * r2[i]
             for i in range(3)] + [w()]
    near = rnd.choice([0.2, 0.2, 0.1, rnd.uniform(0.05, 0.5)])
    ty = math.tan(math.radians(rnd.choice([40.0, 55.0, 35.0, rnd.uniform(20, 75)])) * 0.5)
    tx = ty * rnd.choice([16 / 9, 4 / 3, 16 / 10, rnd.uniform(0.8, 2.4)])
    focus = [rnd.uniform(-0.3, 0.3), rnd.uniform(0.3, 1.5), rnd.uniform(-3, 0.5), w()]
    roll = rnd.choice([0.0, 0.0, rnd.uniform(-0.3, 0.3)])
    inc = rnd.choice([0x100, 0x300, 0x100, 0xFFFFFFFF])
    states = rnd.choice([7, 7, 4, 3])
    ball = rnd.choice([0, 0, 1, 1, 2, 3, rnd.randint(4, 10)]) if fn == 0 else 0
    pattern = rnd.choice(['random', 'parallel', 'ortho', 'same', 'opposite'])
    recs = []
    base_n = unit(rnd)
    for k in range(10):
        if pattern == 'parallel':
            n = base_n
        elif pattern == 'ortho' and k % 2:
            s = [-base_n[2], 0.0, base_n[0]]
            l = math.sqrt(sum(x * x for x in s)) or 1
            n = [x / l for x in s] if any(s) else [1.0, 0.0, 0.0]
        elif pattern == 'opposite' and k % 2:
            n = [-x for x in base_n]
        elif pattern == 'same' and k:
            n = recs[0][4:7]
        else:
            n = unit(rnd)
        p = [c[i] + rnd.uniform(-1.5, 1.5) for i in range(3)]
        if pattern == 'same' and k:
            p = recs[0][:3]
        recs.append(p + [w()] + n + [rnd.choice([0.0, rnd.uniform(-1, 1)])])
    sweep = rnd.choice([0, 1, 1])
    sp_ = [c[i] + rnd.uniform(-2, 2) for i in range(3)] + [w()]
    sn = unit(rnd) + [rnd.choice([0.0, rnd.uniform(-1, 1)])]
    words = ([fn] + [fb(x) for x in m + center + collide + c] + [fb(near), fb(ty), fb(tx)] + [fb(x) for x in focus]
             + [fb(roll), inc, states, ball] + [fb(x) for r in recs for x in r] + [sweep] + [fb(x) for x in sp_ + sn])
    assert len(words) == NW, len(words)
    return words


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    idx = index(a.recomp)
    chase, pov, ball, sweep, look, roll = (addr(a.tool, f) for f in ('F-20517', 'F-77832', 'F-87651', 'F-52008', 'F-97951', 'F-83846'))
    rnd = random.Random(1118)
    cases = [case(rnd, k % 2) for k in range(a.cases)]
    table = 'static const uint32_t CS[][%d]={' % NW + ','.join('{' + ','.join('0x%08xu' % v for v in c) + '}' for c in cases) + '};\n'
    # word places in a case
    M_, CEN, COL, C_, NEAR, FOC, ROLL, INC, ST, NB, RECS, NS, SREC = 1, 17, 21, 25, 29, 32, 36, 37, 38, 39, 40, 120, 121
    body = table + (
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});\n'
        # the vector unit's xyz mask and w = 1 (set at the game's start, zero in the image)
        ' for(int k=0;k<3;k++){store(0x82860950+4*k,0xFFFFFFFFu);store(0x82860930+4*k,0);}store(0x8286095C,0);store(0x8286093C,0x3F800000u);\n'
        'for(size_t k=0;k<sizeof(CS)/sizeof(CS[0]);k++){const uint32_t*h=CS[k];memset(pointer(0x1000),0,0x1000);\n'
        ' for(int i=0;i<%d;i++)store(%d+4*i,h[i]);\n'
        ' for(int i=0;i<16;i++)store(%d+4*i,h[%d+i]);for(int i=0;i<4;i++){store(%d+4*i,h[%d+i]);store(%d+4*i,h[%d+i]);'
        'store(%d+4*i,h[%d+i]);store(%d+4*i,h[%d+i]);}\n'
        ' store(%d,%d);store(%d,%d);store(%d,%d);store(%d,0x5a5a);store(%d,h[%d]);store(%d,h[%d]);store(%d,h[%d]);'
        'store(%d,h[%d]);store(%d,h[%d]);*pointer(%d)=uint8_t(h[%d]);\n'
        ' for(int r=0;r<3;r++)for(int i=0;i<4;i++)store(%d+16*r+4*i,i==r?0x3f800000u:0);\n'
        '  Context ctx{};ctx.r3.u32=%d;ctx.r1.u32=0x7f00;if(h[0])sub_%s(ctx,nullptr);else sub_%s(ctx,nullptr);uint32_t ret=ctx.r3.u32&0xff;\n'
        '  printf("BC");for(int i=0;i<%d;i++)printf(" %%08x",h[i]);printf(" ->");for(int i=0;i<40;i++)printf(" %%08x",load(%d+4*i));'
        'for(int i=0;i<4;i++)printf(" %%08x",load(%d+4*i));'
        'printf(" %%u %%u\\n",unsigned(*pointer(%d)),ret);}}\n') % (
        NW, FR,
        CAM + 256, M_, TUNE + 16, CEN, TUNE + 64, COL, CAM + 128, C_, CAM + 432, FOC,
        CAM + 152, VP, CAM + 192, TGT, CAM + 240, TUNE, TGT + 28, CAM + 200, INC, VP + 704, NEAR, VP + 712, NEAR + 1,
        VP + 716, NEAR + 2, CAM + 448, ROLL, CAM + 196, ST,
        CAM + 80,
        CAM, pov[4:], chase[4:],
        NW, LOG, CAM + 128, CAM + 156)
    # the log (words): ball 0 calls, 1..4 P, 5 radius, 6 instance, 7 include, 8 r9, 9 states, 10 [sp+84], 11 [sp+92];
    # sweep 12 calls, 13..16 start, 17..20 delta, 21 length, 22 radius, 23 instance, 24 include, 25 r9, 26 states,
    # 27 byte [sp+127], 28 [sp+132], 29 [sp+140]; look 30 calls, 31..34 the direction, 35 the rows (r3), 36 the up (r5);
    # roll 37 calls, 38 the angle, 39 the rows (r3).
    L = lambda w: LOG + 4 * w
    recs = ('for(int j=0;j<int(load(0x%x));j++)for(int i=0;i<4;i++){store(ctx.r6.u32+96*j+16+4*i,load(0x%x+32*j+4*i));'
            'store(ctx.r6.u32+96*j+32+4*i,load(0x%x+32*j+16+4*i));}' % (FR + 4 * NB, FR + 4 * RECS, FR + 4 * RECS))
    fw = lambda reg, at: '{float x=float(ctx.%s.f64);uint32_t w;memcpy(&w,&x,4);store(0x%x,w);}' % (reg, at)
    stub_ball = ('store(0x%x,load(0x%x)+1);for(int i=0;i<4;i++)store(0x%x+4*i,load(ctx.r4.u32+4*i));%s'
                 'store(0x%x,ctx.r7.u32);store(0x%x,ctx.r8.u32);store(0x%x,ctx.r9.u32);store(0x%x,ctx.r10.u32);'
                 'store(0x%x,load(ctx.r1.u32+84));store(0x%x,load(ctx.r1.u32+92));%sctx.r3.u64=load(0x%x);') % (
        L(0), L(0), L(1), fw('f1', L(5)), L(6), L(7), L(8), L(9), L(10), L(11), recs, FR + 4 * NB)
    srec = ('if(load(0x%x)){for(int i=0;i<4;i++){store(ctx.r6.u32+16+4*i,load(0x%x+4*i));store(ctx.r6.u32+32+4*i,load(0x%x+4*i));}}'
            % (FR + 4 * NS, FR + 4 * SREC, FR + 4 * (SREC + 4)))
    stub_sweep = ('store(0x%x,load(0x%x)+1);for(int i=0;i<4;i++){store(0x%x+4*i,ctx.v1.u32[3-i]);store(0x%x+4*i,ctx.v2.u32[3-i]);}'
                  '%s%sstore(0x%x,ctx.r7.u32);store(0x%x,ctx.r8.u32);store(0x%x,ctx.r9.u32);store(0x%x,ctx.r10.u32);'
                  'store(0x%x,*pointer(ctx.r1.u32+127));store(0x%x,load(ctx.r1.u32+132));store(0x%x,load(ctx.r1.u32+140));'
                  '%sctx.r3.u64=load(0x%x);') % (
        L(12), L(12), L(13), L(17), fw('f1', L(21)), fw('f2', L(22)), L(23), L(24), L(25), L(26), L(27), L(28), L(29),
        srec, FR + 4 * NS)
    stub_look = ('store(0x%x,load(0x%x)+1);for(int i=0;i<4;i++)store(0x%x+4*i,load(ctx.r4.u32+4*i));store(0x%x,ctx.r3.u32);'
                 'store(0x%x,ctx.r5.u32);') % (L(30), L(30), L(31), L(35), L(36))
    stub_roll = 'store(0x%x,load(0x%x)+1);%sstore(0x%x,ctx.r3.u32);' % (L(37), L(37), fw('f1', L(38)), L(39))
    exe = build(idx, [chase, pov], body, a.out, 'boomcol', {ball: stub_ball, sweep: stub_sweep, look: stub_look, roll: stub_roll}, vector=True, scratch_end=0x8000)
    a.ref.write_text(run(exe, a.image))


if __name__ == '__main__':
    main()
