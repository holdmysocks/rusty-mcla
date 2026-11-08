"""The prop list visit F-79256 run offline (the generated code as it is, its object type vfunc answered by a dispatch
stub), for the port's test (crates/game/src/testdata/lamp_list_reference.txt).

The query object at 0x1000 (+20 its list array, +112 the view, +176 the camera, +192 / +196 / +200 the three size
factors), the list's nodes at 0x1200 (+16 the object, +24 the next, +30 the flags halfword), the objects at 0x1800 +
0x100 k (vtable at +0 whose slot 1 the dispatch stub answers with the case's type, +92 the drawable, +96 the sphere
xyzw, +156 a word), their drawables at 0x2000 + 0x140 k (+28 the radius, +308 a byte), the manager the function writes
(the global at 0x8286EC9C) at 0x1100, the far global (0x828D4CE4) at 0x1300 (+8 the case's value).
Stubs: the view's sphere test F-29884 answers the case's bit for the object and prints it was asked; the occlusion
F-38575 answers the case's word and prints the sphere and the mode it got; the list's callback F-92987 prints the
object and what the manager holds then (+112 the object, +116 the flags, +120 the value, +124 the distance).

  CASE <n> cam <x y z> k <k1 k2 k3> far <f> nodes <m>          (floats as hex bits)
  N <k> <flags hw> <type> <skip byte> <+156> <radius> <sphere x y z w> <vis> <occ word>
  VIS <k>        OCC <k> <x y z w> <mode>        CB <k> <flags> <value> <distance>

python lamp_list_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file> --tool <mcla-tool>
"""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += ('#define ctr r2' + chr(10) + 'static void dispatch(Context&,const uint8_t*,uint32_t);' + chr(10) +
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)' + chr(10) +
                         'static uint32_t VIS[8],OCC[8];' + chr(10))

MGR_G, FAR_G = 0x8286EC9C, 0x828D4CE4


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def cases(rnd, count):
    out = []
    for _ in range(count):
        cam = [rnd.uniform(-3000, 3000), rnd.uniform(-20, 80), rnd.uniform(-3000, 3000)]
        tan = math.tan(math.radians(rnd.uniform(30, 95)) / 2)
        k = [0.0125 * tan, 0.1 * tan, 0.0301776695]
        if rnd.random() < 0.1:
            k = [rnd.uniform(0, 0.2) for _ in range(3)]
        far = rnd.choice((266.6667, 300.0, 150.0, 1000.0, 400.0, 0.0))
        nodes = []
        for _k in range(rnd.choice((0, 1, 2, 3, 5, 8, 8))):
            r = rnd.choice((0.05, 0.1, 0.149, 0.15, 0.15000001, 0.3, 1.0, 2.5, 5.0, 10.0, 30.0, -1.0, 0.0))
            # (distances round the thresholds: 200, 250, R / k, the far cap)
            d = rnd.choice((rnd.uniform(0, 400), rnd.uniform(195, 205), rnd.uniform(245, 255), rnd.uniform(0, 30),
                            abs(r) / k[rnd.randrange(3)] * rnd.uniform(0.98, 1.02) if k[0] > 0 else 10.0))
            u = [rnd.gauss(0, 1) for _ in range(3)]
            n = math.sqrt(sum(x * x for x in u)) or 1.0
            c = [cam[i] + u[i] / n * (d + max(r, 0)) for i in range(3)]
            nodes.append(dict(hw=rnd.choice((0x8000, 0x8000, 0x8000, 0xFFFF, 0x8001, 0, 0x7FFF)),
                              ty=rnd.choice((4, 4, 4, 5, 5, 3, 0, 6)), skip=rnd.choice((0, 0, 0, 1, 2)), w156=rnd.choice((0, 0, 1, 0xFFFFFFFF)),
                              r=r, c=c + [rnd.uniform(0.1, 20)], vis=rnd.choice((0, 1, 1)),
                              occ=rnd.choice((0, 0x20000000, 0x20000000, 0x10000000, 0x04000000, 0x14000000, 0x24000000))))
        out.append((cam, k, far, nodes))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    A = lambda f: 'sub_' + addr(a.tool, f)
    fn = A('F-79256')
    nl, NL = chr(10), chr(92) + 'n'
    stubs = {
        A('F-29884'): '{int i=int(load(0x1F84));printf("VIS %d %08x' + NL + '",i,ctx.r3.u32);ctx.r3.u64=VIS[i];}',
        A('F-38575'): '{int i=int(load(0x1F84));printf("OCC %d %08x %08x %08x %08x %u' + NL + '",i,load(ctx.r5.u32),load(ctx.r5.u32+4),load(ctx.r5.u32+8),load(ctx.r5.u32+12),ctx.r6.u32);ctx.r3.u64=OCC[i];}',
        A('F-92987'): '{int i=int((ctx.r3.u32-0x1800)/0x100);printf("CB %d %08x %08x %08x %08x' + NL + '",i,load(0x1174),load(0x1178),load(0x117C),load(0x1170)==ctx.r3.u32?0u:1u);}',
    }
    cs = cases(random.Random(79256), a.cases)
    flat = []
    for cam, k, far, nodes in cs:
        flat += [fb(x) for x in cam] + [fb(x) for x in k] + [fb(far), len(nodes)]
        for n in nodes:
            flat += [n['hw'], n['ty'], n['skip'], n['w156'], fb(n['r'])] + [fb(x) for x in n['c']] + [n['vis'], n['occ']]
    # (the object the visit is on: the dispatch stub and the two stubs find it by the node the list walk is at -
    # the type vfunc is the first thing asked of each object, so it records the object's index at 0x1F84)
    body = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t t){(void)base;uint32_t i=(ctx.r3.u32-0x1800)/0x100;store(0x1F84,i);ctx.r3.u64=t-0x7000;}' + nl +
            'static const uint32_t C[]={' + ','.join('0x%08xu' % v for v in flat) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};' + nl +
            'size_t q=0;int n=0;while(q<sizeof(C)/4){memset(objects,0,sizeof(objects));const uint32_t*c=&C[q];uint32_t m=c[7];' +
            'printf("CASE %d cam %08x %08x %08x k %08x %08x %08x far %08x nodes %u' + NL + '",n++,c[0],c[1],c[2],c[3],c[4],c[5],c[6],m);' +
            'for(int i=0;i<3;i++){store(0x1000+176+4*i,c[i]);store(0x1000+192+4*i,c[3+i]);}store(0x1000+20,0x11A0);store(0x1000+112,0x1400);' +
            'store(0x11A0,m?0x1200:0);store(0x1308,c[6]);' + ('put(0x%08xu,0x1100);put(0x%08xu,0x1300);' % (MGR_G, FAR_G)) +
            'q+=8;for(uint32_t k=0;k<m;k++,q+=11){const uint32_t*d=&C[q];uint32_t N=0x1200+32*k,O=0x1800+0x100*k,D=0x2000+0x140*k;' +
            'store(N+16,O);store(N+24,k+1<m?N+32:0);store(N+28,d[0]&0xFFFF);' +   # (+30: the halfword's big-endian place)
            'store(O,O+0xC0);store(O+0xC4,0x7000+d[1]);store(O+92,D);for(int i=0;i<4;i++)store(O+96+4*i,d[5+i]);store(O+156,d[3]);' +
            'store(D+28,d[4]);pointer(D+308)[0]=uint8_t(d[2]);VIS[k]=d[9];OCC[k]=d[10];' +
            'printf("N %u %u %u %u %08x %08x %08x %08x %08x %08x %u %08x' + NL + '",k,d[0],d[1],d[2],d[3],d[4],d[5],d[6],d[7],d[8],d[9],d[10]);}' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r5.u32=0;' + ('%s(ctx,nullptr);' % fn) + '}}' + nl)
    out = run(build(index(a.recomp), [fn], body, a.out, 'lamp_list', stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    assert out.count('CASE') == len(cs), out[:600]
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(out)
    print(len(cs), 'cases', out.count('N '), 'objects', out.count('CB'), 'listed', out.count('VIS'), 'view tests', out.count('OCC'), 'occlusion tests')


if __name__ == '__main__':
    main()
