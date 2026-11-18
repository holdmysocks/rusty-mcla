"""Run the garage camera's move of the original offline (static recompilation, native, flat guest memory) and record its
results for the port's test (crates/game/src/testdata/garage_move_ref.txt).

  GS <26 input words> -> <view> <saved> <+39229> <+39236> <+39244> <+39252> <A 6> <B 6> <target A 3> <target B 3> <panel calls>
        F-34768 (a view other than Rotate). Inputs: the current view (the global view index), the saved view (+39240), the
        view asked, forced, camTransDuration (+39248), the camera's +192 height, +196 dist, +200 azimuth, +204 incline,
        +208 roll, +168 FOV, the camera's target row (the matrix at camera +144, row 3), the car's place (its matrix row
        3), the asked view's table row (dist, azimuth, incline, height, roll, FOV, offset x, y, z). Stubs: the camera
        manager F-65049 (+880 -> +124 the camera, +48 the car), the movie's panel F-93426 (counted), the movie lookup
        F-52614 (none). Words a skipped call does not write stay 0x7F7F7F7F (bytes 0x7F).
  GE <24 input words> -> <camera 192 196 200 204 208 168> <target 3> <timer> <moving> <calls> <last arg> <camera +144>
        F-53958 on one game frame of a move. Inputs: the view, +21676, the duration +39252, the timer +39236, the frame
        time (the clock block +8), A (height, dist, azimuth, incline, roll, FOV), B, the target's start (0x82908DE0) and
        end (+39328), the view's table azimuth. The pad index is -1 (no free look); the UI broadcast F-42428 is a stub
        that logs its argument; the game's cosine (boomcam::cos_pi) runs unchanged.

Function names come from `mcla-tool registry addr`. Usage:
  python mcla-rs/tools/garage_move_reference.py --recomp D:\\MCLA-static --image work/2026-10-10/original-image.bin \\
      --out work/<session>/move --ref mcla-rs/crates/game/src/testdata/garage_move_ref.txt --tool <mcla-tool.exe>
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
# (-msse4.1 as the recomp's own x64 build)
_run = recomp_closure.subprocess.run
recomp_closure.subprocess.run = lambda cmd, *a, **k: _run([cmd[0], '-msse4.1'] + cmd[1:] if cmd and str(cmd[0]).endswith('g++') else cmd, *a, **k)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


PADX, MGR, MID, CAM, CARI, TLS, TLSB, LOG, GAR = 0x1000, 0x1100, 0x1200, 0x1300, 0x1600, 0x1800, 0x1900, 0x1A00, 0x2000
VIEW = 0x82828DE4    # the global view index
MAT = 0x82908DA0     # the matrix F-53958 hangs on camera +144 (row 3 the target)
TSTART = 0x82908DE0  # the target at the move's start
CLOCK8 = 0x827D7508  # the clock block +8: the game frame's time
PI = math.pi


def figs(rnd):
    return [rnd.uniform(-0.5, 1.5), rnd.choice([0.5, rnd.uniform(0.5, 8)]), rnd.choice([rnd.uniform(-4, 4), rnd.uniform(-8, 8), 0.0]),
            rnd.uniform(-0.1, 0.9), rnd.choice([0.0, 0.0, rnd.uniform(-0.3, 0.3)]), rnd.uniform(25, 70)]


def pt(rnd):
    return [rnd.uniform(-900, 900), rnd.uniform(0, 40), rnd.uniform(-900, 900)]


def start_case(rnd):
    cur = rnd.choice([rnd.randrange(25), 3, 4, 4, 10, 12, 20, 21])
    new = rnd.choice([rnd.randrange(1, 25), 3, 4, 10, 13, cur if cur else 3])
    a = figs(rnd)
    row = figs(rnd)
    if rnd.random() < 0.3:   # the short-way edges
        row[1 + 1] = a[2] + rnd.choice([-1, 1]) * (PI + rnd.choice([0.0, 1e-6, -1e-6, rnd.uniform(0, 2)]))
    tr = rnd.choice([0.7, 0.5, rnd.uniform(0.1, 3)])
    car = pt(rnd)
    ta = [car[i] + rnd.uniform(-2, 2) for i in range(3)]
    off = [rnd.uniform(-2, 2) for _ in range(3)]
    return [cur, rnd.choice([3, 10, 11, 12, 13]), new, rnd.choice([0, 0, 1]), fb(tr)] + [fb(x) for x in a] + [fb(x) for x in ta] \
        + [fb(x) for x in car] + [fb(x) for x in [row[1], row[2], row[3], row[0], row[4], row[5]] + off]


def ease_case(rnd):
    view = rnd.choice([rnd.randrange(25), 21, 3, 4, 0])
    dur = rnd.choice([0.7, 0.4, rnd.uniform(0.1, 3)])
    timer = rnd.choice([rnd.uniform(0, dur), rnd.uniform(0, dur * 1.2), 0.0, dur - 1 / 30, dur, dur * 2])
    g8 = rnd.choice([1 / 30, 1 / 30, 1 / 60, rnd.uniform(0.005, 0.1)])
    a, b = figs(rnd), figs(rnd)
    t0 = pt(rnd)
    t1 = [t0[i] + rnd.uniform(-5, 5) for i in range(3)]
    return [view, rnd.choice([0, 2, 2, 1]), fb(dur), fb(timer), fb(g8)] + [fb(x) for x in a + b + t0 + t1] + [fb(rnd.uniform(-4, 4))]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=8000)
    a = ap.parse_args()
    idx = index(a.recomp)
    start, step, mgr, panel, movie, padidx, ui = (addr(a.tool, f) for f in ('F-34768', 'F-53958', 'F-65049', 'F-93426', 'F-52614', 'F-33636', 'F-42428'))
    rnd = random.Random(1117)
    starts = [start_case(rnd) for _ in range(a.cases)]
    eases = [ease_case(rnd) for _ in range(a.cases)]
    table = ('static const uint32_t GS[][26]={' + ','.join('{' + ','.join('0x%08xu' % v for v in r) + '}' for r in starts) + '};\n'
             'static const uint32_t GE[][24]={' + ','.join('{' + ','.join('0x%08xu' % v for v in r) + '}' for r in eases) + '};\n')
    body = table + (
        'static void common(){memset(objects,0,sizeof(objects));store(0x%x,0x%x);store(0x%x,0x%x);store(0x%x,0x%x);store(0x%x,0x%x);'
        'store(0x%x,0x%x);store(0x%x,0xffffffffu);}\n'
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});\n'
        # F-34768
        'for(size_t k=0;k<sizeof(GS)/sizeof(GS[0]);k++){const uint32_t*c=GS[k];common();\n'
        ' store(0x%x,c[0]);store(0x%x,c[1]);store(0x%x,c[4]);'
        'for(int i=0;i<5;i++)store(0x%x+4*i,c[5+i]);store(0x%x,c[10]);store(0x%x,0x%x);'
        'for(int i=0;i<3;i++){store(0x%x+4*i,c[11+i]);store(0x%x+4*i,c[14+i]);store(0x%x+4*i,0x7f7f7f7fu);}store(0x%x,0x3f800000u);store(0x%x,0x3f800000u);'
        'uint32_t row=0x%x+48*c[2];for(int i=0;i<6;i++)store(row+4*i,c[17+i]);for(int i=0;i<3;i++)store(row+32+4*i,c[23+i]);\n'
        ' for(uint32_t o:{39236u,39252u,39268u,39272u,39276u,39280u,39284u,39288u,39292u,39296u,39300u,39304u,39308u,39312u,39328u,39332u,39336u})store(0x%x+o,0x7f7f7f7fu);'
        '*pointer(0x%x+39229)=0x7f;*pointer(0x%x+39244)=0x7f;\n'
        ' Context ctx{};ctx.r3.u32=0x%x;ctx.r4.u32=c[2];ctx.r5.u32=c[3];ctx.r1.u32=0x1ff00;ctx.r13.u32=0x%x;sub_%s(ctx,nullptr);\n'
        ' printf("GS");for(int i=0;i<26;i++)printf(" %%08x",c[i]);printf(" -> %%08x %%08x %%02x %%08x %%02x %%08x",load(0x%x),load(0x%x+39240),'
        'unsigned(*pointer(0x%x+39229)),load(0x%x+39236),unsigned(*pointer(0x%x+39244)),load(0x%x+39252));'
        'for(int i=0;i<12;i++)printf(" %%08x",load(0x%x+39268+4*i));for(int i=0;i<3;i++)printf(" %%08x",load(0x%x+4*i));'
        'for(int i=0;i<3;i++)printf(" %%08x",load(0x%x+39328+4*i));printf(" %%u %%u\\n",load(0x%x),load(0x%x+8));}\n'
        # F-53958
        'for(size_t k=0;k<sizeof(GE)/sizeof(GE[0]);k++){const uint32_t*c=GE[k];common();\n'
        ' store(0x%x,c[0]);store(0x%x+21676,c[1]);store(0x%x+39252,c[2]);store(0x%x+39236,c[3]);store(0x%x,c[4]);*pointer(0x%x+39244)=1;'
        'for(int i=0;i<12;i++)store(0x%x+39268+4*i,c[5+i]);for(int i=0;i<3;i++){store(0x%x+4*i,c[17+i]);store(0x%x+39328+4*i,c[20+i]);'
        'store(0x%x+4*i,0x7f7f7f7fu);}store(0x%x,0x3f800000u);store(0x%x,0x3f800000u);store(0x%x+39588+48*c[0],c[23]);'
        'for(uint32_t o:{168u,192u,196u,200u,204u,208u})store(0x%x+o,0x7f7f7f7fu);\n'
        ' Context ctx{};ctx.r3.u32=0x%x;ctx.r1.u32=0x1ff00;ctx.r13.u32=0x%x;sub_%s(ctx,nullptr);\n'
        ' printf("GE");for(int i=0;i<24;i++)printf(" %%08x",c[i]);printf(" ->");'
        'for(uint32_t o:{192u,196u,200u,204u,208u,168u})printf(" %%08x",load(0x%x+o));for(int i=0;i<3;i++)printf(" %%08x",load(0x%x+4*i));'
        'printf(" %%08x %%02x %%u %%08x %%08x %%u\\n",load(0x%x+39236),unsigned(*pointer(0x%x+39244)),load(0x%x),load(0x%x+4),load(0x%x+144),load(0x%x+8));}}\n') % (
        # common: the manager's chain, the car, TLS (byte +72 = 0: the car matrix at +144), the pad index -1
        MGR + 880, MID, MID + 124, CAM, MGR + 48, CARI, TLS, TLSB, CAM + 144, MAT, PADX + 4,
        # GS
        VIEW, GAR + 39240, GAR + 39248,
        CAM + 192, CAM + 168, CAM + 144, MAT,
        MAT + 48, CARI + 192, TSTART, MAT + 60, CARI + 204,
        GAR + 39584,
        GAR, GAR, GAR,
        GAR, TLS, start[4:],
        VIEW, GAR, GAR, GAR, GAR, GAR,
        GAR, TSTART, GAR, LOG, LOG,
        # GE
        VIEW, GAR, GAR, GAR, CLOCK8, GAR,
        GAR, TSTART, GAR, MAT + 48, TSTART + 12, MAT + 60, GAR,
        CAM,
        GAR, TLS, step[4:],
        CAM, MAT + 48, GAR, GAR, LOG, LOG, CAM, LOG)
    stubs = {
        mgr: 'ctx.r3.u64=0x%x;' % MGR,
        padidx: 'ctx.r3.u64=0x%x;' % PADX,
        panel: 'store(0x%x,load(0x%x)+1);' % (LOG, LOG),
        movie: 'ctx.r3.u64=0;',
        ui: 'store(0x%x,load(0x%x)+1);store(0x%x,ctx.r4.u32);' % (LOG, LOG, LOG + 4),
    }
    # (not reached by the cases: the outside views' free look, the interior one, the idle test, Rotate's panel and the
    # movie's setter - each counts at LOG + 8, which the test asks to stay 0)
    for f in ('F-76476', 'F-28551', 'F-30747', 'F-94992', 'F-59146'):
        stubs[addr(a.tool, f)] = 'store(0x%x,load(0x%x)+1);' % (LOG + 8, LOG + 8)
    exe = build(idx, [start, step], body, a.out, 'garagemove', stubs, vector=True, scratch_end=0x20000)
    a.ref.write_text(run(exe, a.image))


if __name__ == '__main__':
    main()
