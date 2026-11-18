"""Run the event / wipeout / ability cameras' pull-in of the original offline (static recompilation, native, flat guest
memory) over sequences of frames and record its results for the port's test (crates/game/src/testdata/track_pull_ref.txt).

  PC <10 case words>             one camera: CollideType, near +704, tan up +712, tan across +716, +468, CollideMinDist
                                 +476, MinAppXZPos +492, dt (G+88), +668 and +672 before the first frame
  PF <19 frame words> -> <+128 3> <+156> <+668> <+672> <ball: n, centre 3, radius, include, states>
                                 <sweep: n, start 3, end 3, radius, include, states>
        F-91096 (TYPE13 with CollideType 1) on one frame: the smoothed output's place C (+64, copied to +128), the point
        tracked T (+320), the ball's answer (hit, normal 3), the sweep's answer (hit, normal 3, depth +68). The ball
        F-87651 and the sweep F-58512 are stubs: they log their arguments and answer the frame's words; the approach
        F-90102 (verified) and every other callee run unchanged. +668 / +672 / +156 carry over from frame to frame.

Function names come from `mcla-tool registry addr`. Usage:
  python mcla-rs/tools/track_pull_reference.py --recomp D:\\MCLA-static --image work/2026-10-10/original-image.bin \\
      --out work/<session>/pull --ref mcla-rs/crates/game/src/testdata/track_pull_ref.txt --tool <mcla-tool.exe>
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
recomp_closure.vector_shim = lambda *a: _shim(*a).replace('uint16_t u16; uint8_t u8;', 'uint16_t u16; uint8_t u8; int16_t s16; int8_t s8;', 1)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


CAM, TGT, VP, FR, LOG = 0x1000, 0x1300, 0x1400, 0x1700, 0x1800
G88 = 0x827D7558   # (the game's clock block 0x827D7500, +88: the frame's dt)
FRAMES = 12


def unit(rnd):
    while True:
        v = [rnd.uniform(-1, 1) for _ in range(3)]
        n = math.sqrt(sum(x * x for x in v))
        if 0.1 < n <= 1:
            return [x / n for x in v]


def case(rnd):
    near = rnd.choice([0.2, 0.1, rnd.uniform(0.05, 0.5)])
    ty = math.tan(math.radians(rnd.choice([40.0, 60.0, 25.0, rnd.uniform(20, 70)])) * 0.5)
    tx = ty * rnd.choice([16 / 9, 4 / 3, 1.0])
    head = [1, fb(near), fb(ty), fb(tx), fb(0.1), fb(rnd.choice([2.0, 2.0, 0.0, rnd.uniform(0, 5)])),
            fb(rnd.choice([0.0, 0.0, 0.1, rnd.uniform(0.05, 20)])), fb(rnd.choice([1 / 30, 1 / 30, 1 / 60, rnd.uniform(0.005, 0.1)])),
            rnd.choice([0, 0, 0, 1, 0x8, 0x12, 0x3C01, 0x20]), fb(rnd.choice([0.0, 0.0, rnd.uniform(0.5, 9)]))]
    t = [rnd.uniform(-300, 300), rnd.uniform(0, 40), rnd.uniform(-300, 300)]
    off = unit(rnd)
    dist = rnd.uniform(2, 9)
    frames = []
    # a run of touches, then clear frames (the release), touches again
    pattern = rnd.choice(['touch-release', 'random', 'ball', 'clear', 'release-long'])
    for k in range(FRAMES):
        t = [t[i] + rnd.uniform(-0.3, 0.3) for i in range(3)]
        if rnd.random() < 0.3:
            off = [off[i] + rnd.uniform(-0.1, 0.1) for i in range(3)]
            n = math.sqrt(sum(x * x for x in off)) or 1
            off = [x / n for x in off]
        if rnd.random() < 0.2:
            dist = max(0.5, dist + rnd.uniform(-1, 1))
        c = [t[i] + off[i] * dist for i in range(3)]
        if rnd.random() < 0.03:
            c = list(t)   # the camera on the point (L = 0)
        if pattern == 'touch-release':
            hit = k < FRAMES // 2
        elif pattern == 'release-long':
            hit = k < 2
        elif pattern == 'clear':
            hit = False
        else:
            hit = rnd.random() < 0.5
        ball = hit and (pattern == 'ball' or rnd.random() < 0.25)
        sweep = hit and not ball or (hit and rnd.random() < 0.3)
        # normals that face T -> C (n.u < 0) mostly, some not, some square to it
        u = off
        def facing():
            r = rnd.random()
            if r < 0.7:
                n = [-u[i] + rnd.uniform(-0.8, 0.8) for i in range(3)]
            elif r < 0.85:
                n = unit(rnd)
            else:
                s = [-u[2], 0.0, u[0]]
                n = s if any(s) else [1.0, 0.0, 0.0]
            m = math.sqrt(sum(x * x for x in n)) or 1
            return [x / m for x in n]
        bn, sn = facing(), facing()
        depth = rnd.choice([rnd.uniform(0, dist), rnd.uniform(0, dist), dist * 1.2, -0.5, 0.0, dist])
        frames.append([fb(x) for x in c] + [fb(x) for x in t] + [int(ball)] + [fb(x) for x in bn] + [int(sweep)]
                      + [fb(x) for x in sn] + [fb(depth)])
    return head, frames


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=1500)
    a = ap.parse_args()
    idx = index(a.recomp)
    pull, ball, sweep = (addr(a.tool, f) for f in ('F-91096', 'F-87651', 'F-58512'))
    rnd = random.Random(1117)
    cases = [case(rnd) for _ in range(a.cases)]
    table = ('static const uint32_t HEAD[][10]={' + ','.join('{' + ','.join('0x%08xu' % v for v in h) + '}' for h, _ in cases) + '};\n'
             'static const uint32_t FR[][%d][19]={' % FRAMES
             + ','.join('{' + ','.join('{' + ','.join('0x%08xu' % v for v in f) + '}' for f in fr) + '}' for _, fr in cases) + '};\n')
    body = table + (
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});\n'
        'for(size_t k=0;k<sizeof(HEAD)/sizeof(HEAD[0]);k++){const uint32_t*h=HEAD[k];memset(objects,0,sizeof(objects));\n'
        ' printf("PC");for(int i=0;i<10;i++)printf(" %%08x",h[i]);printf("\\n");\n'
        # the camera: +192 the target (its +28 the instance the probes skip), +152 the viewport, +200 include 0x100, byte +196
        # states 7; +464 CollideType, +468, +476, +492; +668 / +672 before; rows +16 .. +48 identity, +64 the place
        ' store(%d,%d);store(%d,0x5a5a);store(%d,%d);store(%d,0x100);*pointer(%d)=7;store(%d,h[0]);store(%d,h[4]);store(%d,h[5]);'
        'store(%d,h[6]);store(%d,h[8]);store(%d,h[9]);store(%d,h[1]);store(%d,h[2]);store(%d,h[3]);store(0x%x,h[7]);\n'
        ' for(int r=0;r<3;r++)for(int i=0;i<4;i++)store(%d+16*r+4*i,i==r?0x3f800000u:0);\n'
        ' for(int j=0;j<%d;j++){const uint32_t*f=FR[k][j];for(int i=0;i<3;i++){store(%d+4*i,f[i]);store(%d+4*i,f[3+i]);}'
        'store(%d,0x3f800000u);store(%d,0x3f800000u);for(int i=0;i<19;i++)store(%d+4*i,f[i]);memset(pointer(%d),0,256);\n'
        '  Context ctx{};ctx.r3.u32=%d;ctx.r1.u32=0x3f00;sub_%s(ctx,nullptr);\n'
        '  printf("PF");for(int i=0;i<19;i++)printf(" %%08x",f[i]);printf(" ->");for(int i=0;i<3;i++)printf(" %%08x",load(%d+4*i));'
        'printf(" %%u %%08x %%08x",unsigned(*pointer(%d)),load(%d),load(%d));for(int i=0;i<20;i++)printf(" %%08x",load(%d+4*i));printf("\\n");}}}\n') % (
        CAM + 192, TGT, TGT + 28, CAM + 152, VP, CAM + 200, CAM + 196, CAM + 464, CAM + 468, CAM + 476,
        CAM + 492, CAM + 668, CAM + 672, VP + 704, VP + 712, VP + 716, G88,
        CAM + 16,
        FRAMES, CAM + 64, CAM + 320, CAM + 76, CAM + 332, FR, LOG,
        CAM, pull[4:],
        CAM + 128, CAM + 156, CAM + 668, CAM + 672, LOG)
    # the frame's words at FR: C 0..2, T 3..5, ball hit 6, normal 7..9, sweep hit 10, normal 11..13, depth 14; the log at LOG:
    # ball (calls, centre 3, radius, include, states) 0..6, sweep (calls, start 3, end 3, radius, include, states) 7..16
    rec = lambda r: ('store(%s+32,load(0x%x));store(%s+36,load(0x%x));store(%s+40,load(0x%x));store(%s+44,0);'
                     % (r, FR + 4 * 7, r, FR + 4 * 8, r, FR + 4 * 9, r))
    stub_ball = ('store(0x%x,load(0x%x)+1);for(int i=0;i<3;i++)store(0x%x+4*i,load(ctx.r4.u32+4*i));{float x=float(ctx.f1.f64);'
                 'uint32_t w;memcpy(&w,&x,4);store(0x%x,w);}store(0x%x,ctx.r8.u32);store(0x%x,ctx.r10.u32);'
                 'if(load(0x%x)){%sctx.r3.u64=1;}else ctx.r3.u64=0;') % (
        LOG, LOG, LOG + 4, LOG + 16, LOG + 20, LOG + 24, FR + 24, rec('ctx.r6.u32'))
    stub_sweep = ('store(0x%x,load(0x%x)+1);for(int i=0;i<3;i++){store(0x%x+4*i,load(ctx.r4.u32+4*i));store(0x%x+4*i,load(ctx.r5.u32+4*i));}'
                  '{float x=float(ctx.f1.f64);uint32_t w;memcpy(&w,&x,4);store(0x%x,w);}store(0x%x,ctx.r9.u32);'
                  'store(0x%x,*pointer(ctx.r1.u32+87));'
                  'if(load(0x%x)){%sstore(ctx.r7.u32+68,load(0x%x));ctx.r3.u64=1;}else ctx.r3.u64=0;') % (
        LOG + 28, LOG + 28, LOG + 32, LOG + 44, LOG + 56, LOG + 60, LOG + 64, FR + 40,
        rec('ctx.r7.u32').replace('0x%x' % (FR + 28), '0x%x' % (FR + 44)).replace('0x%x' % (FR + 32), '0x%x' % (FR + 48))
        .replace('0x%x' % (FR + 36), '0x%x' % (FR + 52)), FR + 56)
    exe = build(idx, [pull], body, a.out, 'trackpull', {ball: stub_ball, sweep: stub_sweep}, vector=True, scratch_end=0x4000)
    a.ref.write_text(run(exe, a.image))


if __name__ == '__main__':
    main()
