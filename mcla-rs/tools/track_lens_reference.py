"""Run the event camera's per-frame update of the original offline (static recompilation, native, flat guest memory) and record
its lens results for the port's test (crates/game/src/testdata/track_lens_ref.txt).

  TL <19 input words> -> <+168> <+180> <+300> <+304> <+768> <+200>
        F-92343 (camTrackCS's update slot): the car's speed (car +224, 0 when the transmission's gear +20 is 0), the speed
        filter (+768 = (1 - SSFilter) old + SSFilter speed, fused), the weight k = (1 - cos(pi x)) / 2 over SSSpeedMin..Max
        (F-39265), the lens and distance scale with the slipstream turbo (car +668: +20 added to the lens, +24 times the
        distance scale) and the nitrous (car +664: +24 added), DistMin / DistMax, and the mask bit 0x200 (F-81967, below
        22.35 m/s or the global flag). F-63744 (the track update itself, verified on 10,067 recorded updates:
        track_update_matches_the_original) is a stub here; F-53646 / F-52786 (the turns by +776 / +772, which no code writes:
        0 always) are stubs too. +224 / +228 (the base's lens offset and distance factor) are 0 / 1 as on every recorded camera.
  Inputs: speed, gear, turbo (0/1), turbo +20, turbo +24, nitro (0/1), nitro +24, SSFilter, old filtered speed, SSSpeedMin,
        SSSpeedMax, MinFOV, MaxFOV, DistScale, DistMin, DistMax, flag byte +4, flag byte +10, +200 before. f32 bits except flags.
"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


C, OWN, CAR, TR, TURBO, NITRO, FLAG = 0x1000, 0x1380, 0x1400, 0x1700, 0x1720, 0x1740, 0x1780
FLAGPTR = 0x828CCEA0   # lis -32115, -12640


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = index(a.recomp)
    upd, track, yaw, pitch = (addr(a.tool, f) for f in ('F-92343', 'F-63744', 'F-53646', 'F-52786'))
    rnd = random.Random(1111)
    rows = []
    for k in range(5000):
        lo = rnd.choice([10.0, 20.0, 0.0, rnd.uniform(0, 40)])
        hi = rnd.choice([lo + rnd.uniform(1, 60), lo, lo - 1.0, 60.0, 80.0])
        speed = rnd.choice([0.0, lo, hi, 22.35, 22.349998, 22.350002, rnd.uniform(0, 100), rnd.uniform(lo - 2, hi + 2)])
        old = rnd.choice([0.0, speed, rnd.uniform(0, 100)])
        rows.append([fb(speed), rnd.choice([0, 1, 1, 1, 2, 5]), rnd.choice([0, 1]), fb(rnd.choice([0.0, rnd.uniform(-5, 15)])),
                     fb(rnd.choice([1.0, rnd.uniform(0.7, 1.5)])), rnd.choice([0, 1]), fb(rnd.choice([0.0, rnd.uniform(0, 20)])),
                     fb(rnd.choice([0.1, 0.05, 1.0, 0.0, rnd.uniform(0, 1)])), fb(old), fb(lo), fb(hi),
                     fb(rnd.choice([40.0, 55.0, rnd.uniform(20, 80)])), fb(rnd.choice([60.0, 70.0, rnd.uniform(20, 100)])),
                     fb(rnd.choice([0.8, 1.0, rnd.uniform(0.3, 2)])), fb(rnd.choice([2.0, 3.0, 6.0, rnd.uniform(0.5, 10)])),
                     fb(rnd.choice([5.0, 7.0, 10.0, rnd.uniform(0.5, 20)])), rnd.choice([0, 1, 1]), rnd.choice([0, 0, 1]),
                     rnd.choice([0x100, 0x300, 0, 0x200, 0xffffffff])])
    table = 'static const uint32_t CASES[][19]={' + ','.join('{' + ','.join('0x%08xu' % v for v in r) + '}' for r in rows) + '};\n'
    body = table + (
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});\n'
        'for(size_t k=0;k<sizeof(CASES)/sizeof(CASES[0]);k++){const uint32_t*c=CASES[k];memset(objects,0,sizeof(objects));'
        # the camera: owner +192 -> +8 car; the record +736 ..; the base +224 / +228; the corrected byte 0
        'store(0x%x,0x%x);store(0x%x+8,0x%x);'
        'store(0x%x,c[9]);store(0x%x,c[10]);store(0x%x,c[11]);store(0x%x,c[12]);store(0x%x,c[13]);store(0x%x,c[14]);store(0x%x,c[15]);'
        'store(0x%x,c[7]);store(0x%x,c[8]);store(0x%x,0);store(0x%x,0);store(0x%x,0);store(0x%x,0x3f800000u);store(0x%x,c[18]);*pointer(0x%x)=0;'
        # the car: speed +224, transmission +20 -> gear +20, turbo +668, nitro +664
        'store(0x%x,c[0]);store(0x%x,0x%x);store(0x%x+20,c[1]);'
        'if(c[2]){store(0x%x,0x%x);store(0x%x+20,c[3]);store(0x%x+24,c[4]);}else store(0x%x,0);'
        'if(c[5]){store(0x%x,0x%x);store(0x%x+24,c[6]);}else store(0x%x,0);'
        # the global flag record
        'store(0x%x,0x%x);*pointer(0x%x+4)=uint8_t(c[16]);*pointer(0x%x+10)=uint8_t(c[17]);'
        'Context ctx{};ctx.r3.u32=0x%x;ctx.r1.u32=0x2f00;sub_%s(ctx,nullptr);'
        'printf("TL");for(int i=0;i<19;i++)printf(" %%08x",c[i]);'
        'printf(" -> %%08x %%08x %%08x %%08x %%08x %%08x\\n",load(0x%x),load(0x%x),load(0x%x),load(0x%x),load(0x%x),load(0x%x));}}\n') % (
        C + 192, OWN, OWN, CAR,
        C + 736, C + 740, C + 744, C + 748, C + 752, C + 756, C + 760,
        C + 764, C + 768, C + 772, C + 776, C + 224, C + 228, C + 200, C + 156,
        CAR + 224, CAR + 20, TR, TR,
        CAR + 668, TURBO, TURBO, TURBO, CAR + 668,
        CAR + 664, NITRO, NITRO, CAR + 664,
        FLAGPTR, FLAG, FLAG, FLAG,
        C, upd[4:],
        C + 168, C + 180, C + 300, C + 304, C + 768, C + 200)
    stubs = {track: '', yaw: '', pitch: ''}
    a.ref.write_text(run(build(idx, [upd], body, a.out, 'tracklens', stubs, vector=True), a.image))


if __name__ == '__main__':
    main()
