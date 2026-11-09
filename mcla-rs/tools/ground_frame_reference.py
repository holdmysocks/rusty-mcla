"""The car's ground frame F-88547 run offline (the generated code as it is), for the port's test
(crates/game/src/testdata/ground_frame_reference.txt).

Memory: the car at 0x2000 (+20 its vehicle, +4512 / +4576 the two frames it writes, +4624 the position row); the vehicle at
0x4000 (+144 / +208 the two matrices); TLS r13 at 0x6000 (its block at 0x6100, +72 the matrix choice byte); the renderer
global (0x8287E064) 0x7000 (+320 / +336 the rows it writes); the camera-car global (0x8288DCF8) the car or not.
Stub: the probe F-20855 answers the case's hit and, on a hit, writes its point at r5+16 and its normal at r5+32.

  CASE <n> <inputs hex>      OUT <+4512 .. +4640, 32 words> <renderer +320 .. +352, 8 words>

python ground_frame_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file> --tool <mcla-tool>
"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build, run

recomp_closure.shim = lambda *_: recomp_leaf.shim(0x10000)
recomp_closure.EXTRA += 'static uint32_t HIT,HP[4],HN[4];' + chr(10)

CAMCAR, REND, PROBE_G = 0x8288DCF8, 0x8287E064, 0x828DA464


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def rot(rnd):
    yaw, pitch, roll = rnd.uniform(-math.pi, math.pi), rnd.gauss(0, 0.15), rnd.gauss(0, 0.15)
    if rnd.random() < 0.05:
        pitch, roll = rnd.uniform(-3, 3), rnd.uniform(-3, 3)
    cy, sy, cp, sp, cr, sr = math.cos(yaw), math.sin(yaw), math.cos(pitch), math.sin(pitch), math.cos(roll), math.sin(roll)
    x = [cy * cr + sy * sp * sr, cp * sr, -sy * cr + cy * sp * sr]
    y = [-cy * sr + sy * sp * cr, cp * cr, sy * sr + cy * sp * cr]
    z = [sy * cp, -sp, cy * cp]
    return x, y, z


def mat(rnd, pos):
    x, y, z = rot(rnd)
    return x + [0.0] + y + [0.0] + z + [0.0] + pos + [1.0]


def case(rnd):
    pos = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 80), rnd.uniform(-3000, 3000)]
    m144 = mat(rnd, pos)
    m208 = mat(rnd, [p + rnd.uniform(-1, 1) for p in pos])
    k = rnd.random()
    if k < 0.6:
        n = [rnd.gauss(0, 0.1), 1.0, rnd.gauss(0, 0.1)]
    elif k < 0.9:
        n = [rnd.uniform(-1, 1) for _ in range(3)]
    else:
        # (degenerate: the normal along the matrix's z row, either matrix)
        src = rnd.choice((m144, m208))
        n = [src[8] * rnd.choice((1.0, -1.0, 2.0)), src[9], src[10]]
    if k < 0.95:
        ln = math.sqrt(sum(v * v for v in n)) or 1.0
        n = [v / ln for v in n]
    nw = rnd.choice((0.0, 0.0, 1.0, rnd.uniform(-5, 5)))
    p = [pos[0] + rnd.uniform(-0.5, 0.5), pos[1] - rnd.uniform(0, 3), pos[2] + rnd.uniform(-0.5, 0.5), rnd.choice((0.0, 1.0, rnd.uniform(-2, 2)))]
    words = dict(tls=int(rnd.random() < 0.4), camcar=int(rnd.random() < 0.5), hit=int(rnd.random() < 0.85))
    return words, dict(m144=m144, m208=m208, p=p, n=n + [nw])


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    A = lambda f: 'sub_' + addr(a.tool, f)
    fn = A('F-88547')
    nl, NL = chr(10), chr(92) + 'n'
    stubs = {
        A('F-20855'): 'if(HIT){for(int i=0;i<4;i++){store(ctx.r5.u32+16+4*i,HP[i]);store(ctx.r5.u32+32+4*i,HN[i]);}}ctx.r3.u64=HIT;',
    }
    rnd = random.Random(0x356158)
    cs = [case(rnd) for _ in range(a.cases)]
    flat = []
    for w, v in cs:
        flat += [w['tls'], w['camcar'], w['hit']]
        flat += [fb(x) for x in v['m144'] + v['m208'] + v['p'] + v['n']]
    per = 3 + 32 + 8
    body = ('static const uint32_t C[]={' + ','.join('0x%08xu' % x for x in flat) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};' + nl +
            'for(size_t n=0;n<sizeof(C)/4/%d;n++){const uint32_t*c=&C[n*%d];memset(objects,0,sizeof(objects));' % (per, per) +
            'printf("CASE %d",int(n));for(int i=0;i<' + str(per) + ';i++)printf(" %08x",c[i]);printf("' + NL + '");' +
            'const uint32_t CAR=0x2000,V=0x4000;store(CAR+20,V);'
            'const uint32_t*m=c+3;for(int i=0;i<16;i++){store(V+144+4*i,m[i]);store(V+208+4*i,m[16+i]);}'
            'for(int i=0;i<4;i++){HP[i]=m[32+i];HN[i]=m[36+i];}HIT=c[2];'
            'store(0x6000,0x6100);pointer(0x6100+72)[0]=uint8_t(c[0]);' +
            ('put(0x%08xu,0x7000);put(0x%08xu,c[1]?CAR:0x2F00u);put(0x%08xu,0x7100);' % (REND, CAMCAR, PROBE_G)) +
            'Context ctx{};ctx.r1.u32=0xF000;ctx.r13.u32=0x6000;ctx.r3.u32=CAR;' + ('%s(ctx,nullptr);' % fn) +
            'printf("OUT");for(int i=0;i<32;i++)printf(" %08x",load(CAR+4512+4*i));for(int i=0;i<8;i++)printf(" %08x",load(0x7000+320+4*i));printf("' + NL + '");}}' + nl)
    out = run(build(index(a.recomp), [fn], body, a.out, 'ground_frame', stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    assert out.count('CASE') == len(cs) and out.count('OUT') == len(cs), out[:600]
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(out)
    print(len(cs), 'cases')


if __name__ == '__main__':
    main()
