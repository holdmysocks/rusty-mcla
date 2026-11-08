"""Run the keyframe curve's slope pass F-18740 offline (the generated code as it is; it calls nothing) and record the
slopes it writes, for the port's test (crates/game/src/testdata/key_slope_reference.txt).

The curve object at 0x1000 (+24 the keys' pointer, +28 the u16 count), the 48-byte keys at 0x1100 (+0 the time,
+16 the four channel values, +32 the slope the pass writes - set to 0x7F800001 before, to see what it clears).

  CASE <n>
  KEY <t> <v0 v1 v2 v3>                 (one per key, as given)
  SLOPE <s0 s1 s2 s3> <+12 of the key>  (one per key, after the call; the words at +32 .. +44, and +12 to see it untouched)

python key_slope_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file> --tool <mcla-tool>
"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def cases(rnd):
    out = []
    S = [0.0, -0.0, 1.0, float('nan'), float('inf'), -float('inf'), 1e-30, 3e38]
    for k in range(1500):
        n = rnd.choice((0, 1, 2, 3, 5, rnd.randrange(1, 40)))
        t = 0.0
        keys = []
        for i in range(n):
            mode = rnd.random()
            if mode < 0.6:
                t += rnd.choice((rnd.uniform(0, 1), rnd.uniform(0, 0.01), rnd.uniform(0, 10)))
            elif mode < 0.75:
                pass  # the same time again
            elif mode < 0.85:
                t -= rnd.uniform(0, 1)  # out of order
            elif mode < 0.9:
                t = rnd.choice(S)
            else:
                t = rnd.uniform(-5, 5)
            v = [rnd.choice(S) if rnd.random() < 0.03 else rnd.choice((rnd.uniform(-10, 10), rnd.uniform(0, 1), 0.0, 1.0)) for _ in range(4)]
            keys.append([fbits(t)] + [fbits(x) for x in v])
        out.append(keys)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = addr(a.tool, 'F-18740')
    cs = cases(random.Random(18740))
    nl, NL = chr(10), chr(92) + 'n'
    flat = []
    for keys in cs:
        flat += [len(keys)] + [w for kk in keys for w in kk]
    body = ('static const uint32_t C[]={' + ','.join('0x%08xu' % v for v in flat) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'size_t k=0;while(k<sizeof(C)/4){memset(objects,0,sizeof(objects));uint32_t n=C[k++];printf("CASE %u' + NL + '",n);' +
            'store(0x1000+24,0x1100);pointer(0x1000+28)[0]=uint8_t(n>>8);pointer(0x1000+28)[1]=uint8_t(n);' +
            'for(uint32_t i=0;i<n;i++){uint32_t b=0x1100+48*i;const uint32_t*q=&C[k+5*i];store(b,q[0]);store(b+12,0x5A5A5A5Au);for(int c=0;c<4;c++){store(b+16+4*c,q[1+c]);store(b+32+4*c,0x7F800001u);}'
            'printf("KEY %08x %08x %08x %08x %08x' + NL + '",q[0],q[1],q[2],q[3],q[4]);}k+=5*n;' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;' + ('sub_%s(ctx,nullptr);' % fn) +
            'for(uint32_t i=0;i<n;i++){uint32_t b=0x1100+48*i;printf("SLOPE %08x %08x %08x %08x %08x' + NL + '",load(b+32),load(b+36),load(b+40),load(b+44),load(b+12));}}}' + nl)
    out = run(build(index(a.recomp), ['sub_' + fn], body, a.out, 'key_slope', vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    assert out.count('CASE') == len(cs), out[:400]
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(out)
    print(len(cs), 'cases', out.count('KEY'), 'keys')


if __name__ == '__main__':
    main()
