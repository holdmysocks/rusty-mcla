"""Four more small vector routines run offline (generated code unchanged, vector mode; a register's lane k is the host's
f32[3 - k]):

- a part's matrix in the world F-35644 (v1..v4 the car's rows, v5..v8 the part's, r3 four rows out);
- a bound's centre of gravity in the world F-86352 (`cg`: r3 the bound, byte +6 the flag, +80 the centre; r4 the matrix;
  r5 out);
- a rope's local anchor carried to the world F-36793 (`anchor`: r4 the point, r5 the instance (+16 its matrix, +4 -> +12 the
  bound, byte +4 = 12 a composite with its parts' matrices at +132), r6 the part, r7 out);
- the tick's pending momentum change F-24169 (`pend`: r3 the collider, +336 the force, +368 the impulse; r4 the scale; r5 out).

Writes <out-dir>/vec_small2_ref.txt, one case a line: "kind words.. -> out.." (hex float bits)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def rot(rnd):
    # a random orthonormal-ish frame (rows), sometimes a little off
    import math
    a, b, c = (rnd.uniform(-math.pi, math.pi) for _ in range(3))
    ca, sa, cb, sb, cc, sc = math.cos(a), math.sin(a), math.cos(b), math.sin(b), math.cos(c), math.sin(c)
    r = [[cb * cc, cb * sc, -sb], [sa * sb * cc - ca * sc, sa * sb * sc + ca * cc, sa * cb], [ca * sb * cc + sa * sc, ca * sb * sc - sa * cc, ca * cb]]
    if rnd.random() < 0.2:
        r = [[x * rnd.uniform(0.999, 1.001) for x in row] for row in r]
    return r


def mat(rnd, far):
    return rot(rnd) + [[rnd.uniform(-far, far) for _ in range(3)]]


# the start-up vector constants (zero in the image): the xyz mask and (0, 0, 0, 1)
CONSTS = 'for(int k=0;k<3;k++){store(0x82860950+4*k,0xFFFFFFFF);store(0x82860930+4*k,0);}store(0x8286095C,0);store(0x8286093C,0x3F800000);'


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fw, fc, fa, fp = (addr(a.tool, f) for f in ('F-35644', 'F-86352', 'F-36793', 'F-24169'))
    rnd = random.Random(53)
    jobs = []
    for _ in range(2000):
        car, part = mat(rnd, 3000), mat(rnd, 3)
        jobs.append(('wm', [x for r in car + part for x in r]))
    for _ in range(2000):
        m = mat(rnd, 3000)
        cg = [rnd.uniform(-2, 2) for _ in range(3)]
        jobs.append(('cg', [float(rnd.random() < 0.7)] + [x for r in m for x in r] + cg))
    for _ in range(2000):
        inst, part = mat(rnd, 3000), mat(rnd, 3)
        p = [rnd.uniform(-2, 2) for _ in range(3)]
        jobs.append(('anchor', [float(rnd.random() < 0.7)] + [x for r in inst + part for x in r] + p))
    for _ in range(1000):
        f = [rnd.uniform(-1, 1) * 10 ** rnd.uniform(-2, 5) for _ in range(3)]
        j = [rnd.uniform(-1, 1) * 10 ** rnd.uniform(-2, 4) for _ in range(3)]
        s = rnd.choice((1 / 60, 1 / 30, 0.0, rnd.uniform(0, 0.1)))
        jobs.append(('pend', f + j + [s]))
    kind = {'wm': 0, 'cg': 1, 'anchor': 2, 'pend': 3}
    W = 28
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%d,%s},' % (kind[k], ','.join('0x%08x' % fb(x) for x in (c + [0.0] * W)[:W])) for k, c in jobs)
    setv = lambda n, at: 'for(int k=0;k<3;k++)ctx.v%d.u32[3-k]=c[%d+k];ctx.v%d.u32[0]=%s;' % (n, at, n, '0x3f800000' if n == 4 else '0')
    st3 = lambda base, at: 'for(int k=0;k<3;k++)store(%s+4*k,c[%d+k]);store(%s+12,0);' % (base, at, base)
    pr = lambda base, n: 'for(int k=0;k<%d;k++){if((k&3)!=3)printf(" %%08x",load(%s+4*k));}' % (n, base)
    body = ('static const uint32_t C[][%d]={' % (W + 1) + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));Context ctx{};ctx.r1.u32=0x2F00;' +
            'if(c[0]==0){' + ''.join(setv(1 + i, 1 + 3 * i) for i in range(8)) + 'ctx.v4.u32[0]=0x3f800000;ctx.v8.u32[0]=0x3f800000;' +
            ('ctx.r3.u32=0x1900;sub_%s(ctx,nullptr);' % fw) + pr('0x1900', 16) + '}' +
            'else if(c[0]==1){store(0x1004,(c[1]?1u:0u)<<8);' + st3('0x1000+80', 14) + 'store(0x1000+92,0x40400000);' +
            ''.join(st3('0x1200+%d' % (16 * i), 2 + 3 * i) for i in range(4)) +
            ('ctx.r3.u32=0x1000;ctx.r4.u32=0x1200;ctx.r5.u32=0x1900;sub_%s(ctx,nullptr);' % fc) + pr('0x1900', 3) + '}' +
            'else if(c[0]==2){store(0x1000+4,0x1100);store(0x1100+12,0x1200);store(0x1200+4,(c[1]?12u:3u)<<24);store(0x1200+132,0x1400);' +
            ''.join(st3('0x1000+16+%d' % (16 * i), 2 + 3 * i) for i in range(4)) +
            ''.join(st3('0x1400+64*2+%d' % (16 * i), 14 + 3 * i) for i in range(4)) +
            st3('0x1800', 26) + 'store(0x1800+12,0x40400000);' +
            ('ctx.r4.u32=0x1800;ctx.r5.u32=0x1000;ctx.r6.u32=2;ctx.r7.u32=0x1900;sub_%s(ctx,nullptr);' % fa) + pr('0x1900', 3) + '}' +
            'else{' + st3('0x1000+336', 1) + st3('0x1000+368', 4) + 'store(0x1800,c[7]);' +
            ('ctx.r3.u32=0x1000;ctx.r4.u32=0x1800;ctx.r5.u32=0x1900;sub_%s(ctx,nullptr);' % fp) + pr('0x1900', 3) + '}' +
            'printf("' + NL + '");}' + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + CONSTS + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + f for f in (fw, fc, fa, fp)], main_src, a.out_dir, 'vec_small2', vector=True, flags=('-msse4.1', '-mssse3'))
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(jobs), (len(out), len(jobs))
    lines = ['%s %s -> %s' % (k, ' '.join('%08x' % fb(x) for x in c), o) for (k, c), o in zip(jobs, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'vec_small2_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
