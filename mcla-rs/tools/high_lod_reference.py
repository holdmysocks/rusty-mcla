"""UITransition_GPSJump's apply F-25207 run offline as generated: the camera height (+52 of the matrix it gets) against
750 m with the jump's latch (+385) and the city's 'high LOD off' byte (0x827E0DC8 + 28, City_SetHighLodEnable's).
Cases: every latch / byte pair at the heights round 750 (one ulp under and over, 0, -0, negative, large, infinite, NaN)
and 3,000 random heights. Writes <out-dir>/high_lod_ref.txt: "<latch> <byte> <y bits> -> <latch> <byte>"."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run

BYTE = (0x827E << 16) + 0x0DC8 + 28  # the city block's +28 (the jump's lis -32130 / addi 3528 / stb 28)
JUMP = 0x4000  # the transition object (its +385)
MAT = 0x5000  # the matrix the apply gets (its +52)


def fbits(f):
    return struct.unpack('>I', struct.pack('>f', f))[0]


def heights():
    lo = fbits(750.0)
    ys = [lo - 1, lo, lo + 1, 0, 0x80000000, fbits(-750.0), fbits(9000.0), fbits(749.5), fbits(850.0), 0x7F800000,
          0xFF800000, 0x7FC00000, fbits(1e-30)]
    r = random.Random(25207)
    ys += [fbits(r.uniform(-100.0, 10000.0)) for _ in range(3000)]
    return ys


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = subprocess.run([a.tool, 'registry', 'addr', 'F-25207'], capture_output=True, text=True, check=True).stdout.split()[2][4:]
    nl = chr(10)
    cases = [(l, b, y) for y in heights() for l in (0, 1) for b in (0, 1)]
    body = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'unsigned l,b,y;while(scanf("%u %u %x",&l,&b,&y)==3){memset(objects,0,sizeof(objects));' +
            '*pointer(0x%X)=l;*pointer(0x%X)=b;store(0x%X,y);' % (JUMP + 385, BYTE, MAT + 52) + nl +
            'Context ctx{};ctx.r1.u32=0x7F00;ctx.r3.u32=0x%X;ctx.r4.u32=0x%X;sub_' % (JUMP, MAT) + f + '(ctx,nullptr);' +
            'printf("%u %u' + chr(92) + 'n",(unsigned)*pointer(' + hex(JUMP + 385) + '),(unsigned)*pointer(' + hex(BYTE) + '));}' + nl + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + f], body, a.out_dir, 'high_lod', scratch_end=0x8000)
    inp = ''.join('%d %d %08x' % c + nl for c in cases)
    out = subprocess.run([str(exe), str(a.image)], input=inp, capture_output=True, text=True, check=True).stdout.split(nl)[:len(cases)]
    assert len(out) == len(cases)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'high_lod_ref.txt').write_text(''.join('%d %d %08x -> ' % c + o + nl for c, o in zip(cases, out)))
    print(len(cases), 'cases;', sum(o.split()[1] == '1' for o in out), 'with the byte set after')


if __name__ == '__main__':
    main()
