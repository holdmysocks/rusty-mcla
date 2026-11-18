"""Run the garage's interior free look of the original offline (static recompilation, native, flat guest memory) and record
its results for the port's test (crates/game/src/testdata/garage_look_ref.txt).

  GL <9 input words> -> <fov> <az> <inc> <view> <saved> <+39232> <+39300> <+39252> <calls> <view asked> <forced>
                        <az at the call> <inc at the call> <fov at the call>
        F-28551 on one game frame. Inputs: the view (the global view index), the saved view (+39240), the pad record's
        buttons word (+8), its right stick's bytes (+18, +19), the online room's flag (the answer of F-78526), and the
        camera's FOV (+168), azimuth (+200) and incline (+204) (f32 bits). The flip F-57991 and the dead zone F-93983 run
        unchanged; the stubs: F-33636 (the pad index: 0), F-65049 (the camera manager: +880 -> +124 the camera),
        F-78526 (answers the case's flag) and F-34768 (logs its arguments and the camera at the call, then sets the
        global view to its argument and saves an interior view entered from outside them, as its asm does). +39232,
        +39300 and +39252 start at 0x7F7F7F7F (to see a write).

Function names come from `mcla-tool registry addr`. Usage:
  python mcla-rs/tools/garage_look_reference.py --recomp D:\\MCLA-static --image work/2026-10-10/original-image.bin \\
      --out work/<session>/look --ref mcla-rs/crates/game/src/testdata/garage_look_ref.txt --tool <mcla-tool.exe>
"""
import argparse
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


PADX, MGR, MID, CAM, LOG, GAR = 0x1000, 0x1100, 0x1200, 0x1300, 0x1800, 0x2000
VIEW = 0x82828DE4   # the global view index
PAD = 0x82905388    # the pad records (64 bytes each), pad 0
Q = 1.5707963705062866


def stick(rnd):
    r = rnd.random()
    if r < 0.25:
        return rnd.randrange(256)
    if r < 0.5:
        return rnd.choice([0, 255, 127, 128, 95, 96, 159, 160, 158, 97])
    return rnd.choice([rnd.randrange(80, 176), 128, 128])


def case(rnd):
    view = rnd.choice([3, 4, 4, 10, 11, 12, 13, 3])
    saved = rnd.choice([3, 10, 11, 12, 13])
    buttons = rnd.choice([0, 0, 1, 2, 3, rnd.randrange(1 << 16)])
    fov = rnd.choice([35.2, 69.8, 35.0, 70.0, rnd.uniform(30, 75), rnd.uniform(35, 70), 55.0])
    if rnd.random() < 0.1:
        fov = struct.unpack('>f', struct.pack('>I', fb(rnd.choice([35.2, 69.8])) + rnd.choice([-1, 1])))[0]
    az = rnd.choice([rnd.uniform(-2, 2), rnd.uniform(-Q - 0.05, -Q + 0.05), rnd.uniform(Q - 0.05, Q + 0.05), Q, -Q,
                     rnd.uniform(-7, 7), 0.0])
    inc = rnd.choice([rnd.uniform(-0.2, 1.1), 0.015, 0.275, 0.05, 0.9, rnd.uniform(0, 0.3)])
    return [view, saved, buttons, stick(rnd), stick(rnd), rnd.choice([0, 0, 0, 1]), fb(fov), fb(az), fb(inc)]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=20000)
    a = ap.parse_args()
    idx = index(a.recomp)
    look, padidx, mgr, rmr, start = (addr(a.tool, f) for f in ('F-28551', 'F-33636', 'F-65049', 'F-78526', 'F-34768'))
    rnd = random.Random(1117)
    rows = [case(rnd) for _ in range(a.cases)]
    table = 'static const uint32_t CASES[][9]={' + ','.join('{' + ','.join('0x%08xu' % v for v in r) + '}' for r in rows) + '};\n'
    body = table + (
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});\n'
        'for(size_t k=0;k<sizeof(CASES)/sizeof(CASES[0]);k++){const uint32_t*c=CASES[k];memset(objects,0,sizeof(objects));\n'
        ' store(0x%x,0);store(0x%x,0x%x);store(0x%x,0x%x);store(0x%x,c[0]);store(0x%x,c[1]);store(0x%x,c[2]);'
        '*pointer(0x%x)=uint8_t(c[3]);*pointer(0x%x)=uint8_t(c[4]);store(0x%x,c[5]);store(0x%x,c[6]);store(0x%x,c[7]);store(0x%x,c[8]);'
        'store(0x%x,0x7f7f7f7fu);store(0x%x,0x7f7f7f7fu);store(0x%x,0x7f7f7f7fu);\n'
        ' Context ctx{};ctx.r3.u32=0x%x;ctx.r1.u32=0xff00;sub_%s(ctx,nullptr);\n'
        ' printf("GL");for(int i=0;i<9;i++)printf(" %%08x",c[i]);printf(" ->");'
        'for(uint32_t at:{0x%xu,0x%xu,0x%xu,0x%xu,0x%xu,0x%xu,0x%xu,0x%xu})printf(" %%08x",load(at));'
        'for(int i=0;i<6;i++)printf(" %%08x",load(0x%x+4*i));printf("\\n");}}\n') % (
        PADX + 4, MGR + 880, MID, MID + 124, CAM, VIEW, GAR + 39240, PAD + 8,
        PAD + 18, PAD + 19, LOG + 64, CAM + 168, CAM + 200, CAM + 204,
        GAR + 39232, GAR + 39300, GAR + 39252,
        GAR, look[4:],
        CAM + 168, CAM + 200, CAM + 204, VIEW, GAR + 39240, GAR + 39232, GAR + 39300, GAR + 39252,
        LOG)
    # the log at LOG: calls, view asked, forced, the camera's azimuth, incline and FOV at the call; LOG+64 the room's flag
    stubs = {
        padidx: 'ctx.r3.u64=0x%x;' % PADX,
        mgr: 'ctx.r3.u64=0x%x;' % MGR,
        rmr: 'ctx.r3.u64=load(0x%x)&0xff;' % (LOG + 64),
        start: ('store(0x%x,load(0x%x)+1);store(0x%x,ctx.r4.u32);store(0x%x,ctx.r5.u32&0xff);store(0x%x,load(0x%x));'
                'store(0x%x,load(0x%x));store(0x%x,load(0x%x));'
                '{uint32_t v=ctx.r4.u32,cur=load(0x%x);auto in=[](uint32_t x){return x==3||(x>=10&&x<=13);};'
                'if(in(v)&&!in(cur))store(ctx.r3.u32+39240,v);store(0x%x,v);}') % (
            LOG, LOG, LOG + 4, LOG + 8, LOG + 12, CAM + 200, LOG + 16, CAM + 204, LOG + 20, CAM + 168, VIEW, VIEW),
    }
    exe = build(idx, [look], body, a.out, 'garagelook', stubs, vector=True, scratch_end=0x10000)
    a.ref.write_text(run(exe, a.image))


if __name__ == '__main__':
    main()
