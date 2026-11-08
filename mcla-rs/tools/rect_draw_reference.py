"""Run the draw of the signal lamps' reflection rectangles F-50970 offline (the generated code as it is) and record what
it hands its quad draw for each rectangle, for the port's test (crates/game/src/testdata/rect_draw_reference.txt).

The picture object's vfuncs 24 / 28 (the target's width / height) go through a dispatch that returns the case's
size; the quad draw F-30024 is stubbed to print the 48 bytes it is given. The renderer pointer, the gate byte, the
count and the 112-byte records sit at the globals the code reads (cited by the evidence file); the divisor is set to 4
(its recorded value: StreakParams.y = I / 4).

  CASE <width> <height> <count>
  REC <+24 z> <+32 x0> <+36 y0> <+40 x1> <+44 y1> <+48 colour> <+96 I>        (one per record)
  DRAW <x0 y0 x1 y1> <z> <u0 v0 u1 v1> <colour> <texture> <strength>           (one per draw, in order)
  END <count after>

python rect_draw_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file> --tool <mcla-tool>
"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

# the two vtable calls go through `dispatch` (defined with main); r2 stands in for the count register
recomp_closure.EXTRA += '#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


# the globals F-50970 reads: the renderer pointer, the gate byte (0 = draw), the record count, the records, the divisor
REND, GATE, COUNT, RECS, DIV = 0x829054A0, 0x8287E368, 0x8287E360, 0x8287E3B0, 0x828393D4


def cases(rnd):
    out = []
    S = [0.0, -0.0, 1.0, -1.0, float('nan'), float('inf'), 1e-7, 1e-30, -1e-30]
    for k in range(600):
        w, h = rnd.choice(((320, 180), (1280, 720), (640, 360), (1, 1), (3, 7), (rnd.randrange(1, 2000), rnd.randrange(1, 2000))))
        n = rnd.choice((0, 1, 2, 5, 96, rnd.randrange(1, 97)))
        recs = []
        for _ in range(n):
            f = lambda lo, hi: rnd.choice(S) if rnd.random() < 0.05 else rnd.uniform(lo, hi)
            z = rnd.choice((f(0, 1), f(-1, 1), 0.0, 0.5))
            x0 = f(-1.2, 1.2)
            x1 = rnd.choice((f(-1.2, 1.2), x0, x0 + rnd.uniform(-1, 1) / w, x0 + 1.0 / w, x0 - 1.0 / w))
            y0 = f(-1.2, 1.2)
            y1 = rnd.choice((f(-1.2, 1.2), y0, y0 + rnd.uniform(-1, 1) / h, y0 + 1.0 / h, y0 - 1.0 / h))
            col = rnd.getrandbits(32)
            while col >> 24 == 0x82:  # (no word that reads as a function address of the original in the tracked data)
                col = rnd.getrandbits(32)
            i = rnd.choice((f(0, 400), 80.0, 0.0, f(-10, 10)))
            recs.append([fbits(z), fbits(x0), fbits(y0), fbits(x1), fbits(y1), col, fbits(i)])
        out.append((w, h, recs))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn, quad = addr(a.tool, 'F-50970'), addr(a.tool, 'F-30024')
    cs = cases(random.Random(50970))
    nl, NL = chr(10), chr(92) + 'n'
    stubs = {'sub_' + quad: 'printf("DRAW");for(int i=0;i<48;i+=4)printf(" %08x",load(ctx.r4.u32+i));printf("' + NL + '");'}
    # the case table: w, h, n, then n x 7 words
    flat = []
    for w, h, recs in cs:
        flat += [w, h, len(recs)] + sum(recs, [])
    body = ('static const uint32_t C[]={' + ','.join('0x%08xu' % v for v in flat) + '};' + nl +
            'static uint32_t W,H;' + nl +
            # vfunc 24 / 28 of the picture: the target's width / height (fake addresses 0xA0000018 / 0xA000001C)
            'static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){if(a==0xA0000018u)ctx.r3.u64=W;else if(a==0xA000001Cu)ctx.r3.u64=H;'
            'else{printf("bad dispatch %08x' + NL + '",a);exit(1);}}' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'size_t k=0;while(k<sizeof(C)/4){memset(objects,0,sizeof(objects));W=C[k];H=C[k+1];uint32_t n=C[k+2];k+=3;' +
            'printf("CASE %u %u %u' + NL + '",W,H,n);' +
            # the renderer at 0x1000: +44 the picture (0x1100), +3272 the texture word; the picture's vtable at 0x1200
            'store(0x%08X,0x1000);store(0x1000+44,0x1100);store(0x1000+3272,0x7E57u);store(0x1100,0x1200);store(0x1200+24,0xA0000018u);store(0x1200+28,0xA000001Cu);' % REND +
            'pointer(0x%08X)[0]=0;store(0x%08X,n);store(0x%08X,4);' % (GATE, COUNT, DIV) +
            'for(uint32_t r=0;r<n;r++){uint32_t b=0x%08X+112*r;const uint32_t*q=&C[k+7*r];printf("REC");for(int i=0;i<7;i++)printf(" %%08x",q[i]);printf("' % RECS + NL + '");'
            'for(int i=0;i<112;i+=4)store(b+i,0);store(b+24,q[0]);store(b+32,q[1]);store(b+36,q[2]);store(b+40,q[3]);store(b+44,q[4]);store(b+48,q[5]);store(b+96,q[6]);}' +
            'k+=7*n;Context ctx{};ctx.r1.u32=0x2F00;' + ('sub_%s(ctx,nullptr);' % fn) +
            'printf("END %%u' % () + NL + '",load(0x%08X));}}' % COUNT + nl)
    out = run(build(index(a.recomp), ['sub_' + fn], body, a.out, 'rect_draw', stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    assert out.count('CASE') == len(cs) and out.count('END') == len(cs), out[:400]
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(out)
    print(len(cs), 'cases', out.count('REC'), 'records', out.count('DRAW'), 'draws')


if __name__ == '__main__':
    main()
