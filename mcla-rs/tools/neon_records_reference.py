"""The player car's shadow / neon / head-lamp records F-17121 run offline (the generated code as it is), for the port's test
(crates/game/src/testdata/neon_records_reference.txt).

Memory: the car at 0x2000 (+20 its vehicle, +4512 its body matrix, +123 a byte, +168 the head lamps, +268 the lamp scale, +404 the neon flash,
+556 / +560 / +564 / +568 its record pointers, +1088 + 64 k the four wheels' matrices, +6477 a byte); the vehicle at
0x4000 (+8 the object the getter F-98453 reads, +80 the bounds owner, +132 the config, +144 / +208 the two matrices);
the config at 0x4200 (+80 ExtNeonColor, +368 ExtNeonIntensity); the records at 0x5000 (4 x 96 wheels) / 0x5200 (body) /
0x5300 (cone) / 0x5400 (4 x 160 neon); the time manager (0x8287E26C) at 0x8000; TLS r13 at 0x6000 (its block at 0x6100,
+72 the matrix choice byte); the renderer global (0x8287E124) 0x7000; the camera-car global (0x8288DCF8) the car or not.
Stubs: the stealth / mega test F-54047 answers 0; the getter F-98453 answers 0x4400 (+16 / +32 the case's two
vectors); the bounds getter F-63160 answers 0x4600 (+16 -> 0x4700: +16 / +32 the case's half extents); the ground probe
F-20855 answers the case's hit and writes its t at r5+64 on a hit; the hour F-22302 answers the case's; the renderer's add
F-62149 prints the body and neon records it gets (40 words; the wheels' and the cone's are not printed).

  CASE <n> ... (inputs)      ADD <record offset from 0x5000> <40 words hex>

python neon_records_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file> --tool <mcla-tool>
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
recomp_closure.EXTRA += 'static uint32_t HIT,TBITS,HOUR;' + chr(10)

CAMCAR, MGR, REND, PROBE_G = 0x8288DCF8, 0x8287E26C, 0x8287E124, 0x828DA464


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
    wheels = []
    for k in range(4):
        w = mat(rnd, [pos[0] + rnd.uniform(-2, 2), pos[1] + rnd.uniform(-1, 1), pos[2] + rnd.uniform(-2, 2)])
        s = rnd.choice((0.3, 0.33, 0.35, 0.4, rnd.uniform(0.2, 0.6)))
        wheels.append([v * (s if i < 12 and i % 4 != 3 else 1) for i, v in enumerate(w)])
    half = [rnd.uniform(0.6, 1.3), rnd.uniform(0.4, 0.9), rnd.uniform(1.5, 3.0)]
    ctr = [rnd.uniform(-0.1, 0.1), rnd.uniform(0.3, 0.8), rnd.uniform(-0.3, 0.3)]
    bmax = [ctr[i] + half[i] for i in range(3)] + [0.0]
    bmin = [ctr[i] - half[i] for i in range(3)] + [0.0]
    g16 = [rnd.uniform(-2, 2) for _ in range(3)] + [0.0]
    g32 = [rnd.uniform(-2, 2) for _ in range(3)] + [0.0]
    col = [rnd.choice((0.0, 1.0, rnd.random())) for _ in range(3)] + [rnd.choice((0.0, 1.0))]
    inten = rnd.choice((0.0, 0.0005, 0.001, 0.0011, 1.0, 1.0, rnd.random()))
    words = dict(
        b123=int(rnd.random() < 0.05), head=rnd.choice((0.0, 0.001, 0.0011, 1.0, 1.0, rnd.random())),
        scale=rnd.choice((1.0, 1.0, 0.0, rnd.random())), flash=rnd.choice((1.0, 1.0, 0.0, rnd.random())),
        b6477=int(rnd.random() < 0.15), tls=int(rnd.random() < 0.3), camcar=int(rnd.random() < 0.5),
        night_byte=int(rnd.random() < 0.15), hour=rnd.choice((6.0, 18.0, 5.999, 18.001, 12.0, 0.0, 23.5, rnd.uniform(0, 24))),
        hit=int(rnd.random() < 0.7), t=rnd.choice((0.0, 0.5, 1.0, rnd.random(), rnd.uniform(1, 2))),
        inten=inten)
    m4512 = mat(rnd, [p + rnd.uniform(-0.5, 0.5) for p in pos])
    return words, dict(m4512=m4512, m144=m144, m208=m208, wheels=wheels, bmax=bmax, bmin=bmin, g16=g16, g32=g32, col=col)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=2000)
    a = ap.parse_args()
    A = lambda f: 'sub_' + addr(a.tool, f)
    fn = A('F-17121')
    nl, NL = chr(10), chr(92) + 'n'
    stubs = {
        A('F-54047'): 'ctx.r3.u64=0;',
        A('F-98453'): 'ctx.r3.u64=0x4400;',
        A('F-63160'): 'ctx.r3.u64=0x4600;',
        A('F-20855'): 'if(HIT)store(ctx.r5.u32+64,TBITS);ctx.r3.u64=HIT;',
        A('F-22302'): '{uint32_t u=HOUR;float f;memcpy(&f,&u,4);ctx.f1.f64=f;}',
        A('F-62149'): '{if(ctx.r4.u32<0x5200||ctx.r4.u32==0x5300)return;printf("ADD %d",int(ctx.r4.u32)-0x5000);for(int i=0;i<40;i++)printf(" %08x",load(ctx.r4.u32+4*i));printf("' + NL + '");}',
    }
    rnd = random.Random(0x5553F8)
    cs = [case(rnd) for _ in range(a.cases)]
    flat = []
    for w, v in cs:
        flat += [w['b123'], fb(w['head']), fb(w['scale']), fb(w['flash']), w['b6477'], w['tls'], w['camcar'], w['night_byte'],
                 fb(w['hour']), w['hit'], fb(w['t']), fb(w['inten'])]
        flat += [fb(x) for x in v['m144'] + v['m208'] + sum(v['wheels'], []) + v['bmax'] + v['bmin'] + v['g16'] + v['g32'] + v['col'] + v['m4512']]
    per = 12 + 16 * 6 + 4 * 5 + 16
    body = ('static const uint32_t C[]={' + ','.join('0x%08xu' % x for x in flat) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};' + nl +
            'for(size_t n=0;n<sizeof(C)/4/%d;n++){const uint32_t*c=&C[n*%d];memset(objects,0,sizeof(objects));' % (per, per) +
            'printf("CASE %d",int(n));for(int i=0;i<' + str(per) + ';i++)printf(" %08x",c[i]);printf("' + NL + '");' +
            'const uint32_t CAR=0x2000,V=0x4000,R=0x4200;'
            'store(CAR+20,V);pointer(CAR+123)[0]=uint8_t(c[0]);store(CAR+168,c[1]);store(CAR+268,c[2]);store(CAR+404,c[3]);'
            'pointer(CAR+6477)[0]=uint8_t(c[4]);store(CAR+556,0x5000);store(CAR+560,0x5200);store(CAR+564,0x5300);store(CAR+568,0x5400);'
            'store(V+8,0x4300);store(V+80,0x4500);store(V+128,0x4580);store(V+132,R);'
            'const uint32_t*m=c+12;for(int i=0;i<16;i++){store(V+144+4*i,m[i]);store(V+208+4*i,m[16+i]);}'
            'for(int k=0;k<4;k++)for(int i=0;i<16;i++)store(CAR+1088+64*k+4*i,m[32+16*k+i]);'
            'const uint32_t*b=m+96;store(0x4600+16,0x4700);for(int i=0;i<4;i++){store(0x4700+16+4*i,b[i]);store(0x4700+32+4*i,b[4+i]);'
            'store(0x4400+16+4*i,b[8+i]);store(0x4400+32+4*i,b[12+i]);store(R+80+4*i,b[16+i]);}store(R+368,c[11]);for(int i=0;i<16;i++)store(CAR+4512+4*i,b[20+i]);'
            'store(0x6000,0x6100);pointer(0x6100+72)[0]=uint8_t(c[5]);'
            'pointer(0x8000+14546)[0]=uint8_t(c[7]);store(0x8000+14564,0x40c00000u);store(0x8000+14568,0x41900000u);'
            'HOUR=c[8];HIT=c[9];TBITS=c[10];' +
            ('put(0x%08xu,0x8000);put(0x%08xu,0x7000);put(0x%08xu,c[6]?CAR:0x2F00u);put(0x%08xu,0x7100);' % (MGR, REND, CAMCAR, PROBE_G)) +
            'Context ctx{};ctx.r1.u32=0xF000;ctx.r13.u32=0x6000;ctx.r3.u32=CAR;' + ('%s(ctx,nullptr);' % fn) + '}}' + nl)
    out = run(build(index(a.recomp), [fn], body, a.out, 'neon_records', stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    assert out.count('CASE') == len(cs), out[:600]
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(out)
    print(len(cs), 'cases', out.count('ADD'), 'records')


if __name__ == '__main__':
    main()
