"""Run the sun glow's query step F-63954 offline (the generated code as it is) with chosen query answers, and record what it
does, for the port's test (crates/game/src/testdata/sun_visible_reference.txt).

The glow object at 0x1000 (+0 the count, +4 visible, +8 the device's reset count, +12 the query index, +16 / +20 the two
queries, +24 the reference total, +28 the probe's radius, +32 no probe, +33 creation failed, +36 the distance, +44 the
intensity), the sun direction at 0x1100, the main viewport at 0x1200 (its +112 the camera), the viewport global the
function reads set to it. Stubs (each prints what it gets): the device's reset count F-50821 (answers the case's), the query
release F-12299 / create F-63097 (answers the case's object or 0), the read F-52930 (answers the case's ready flag and
count), the state get F-38438 (answers 100 + id) / set F-33110, the library event F-20853, begin F-91575 / end F-84063,
the sphere F-81209 (its matrix, radius, slices, flags), the flush F-26310, the state pop F-68992 (its saved entries).

  CASE <n> count0 <old +0> idx <+12> total <+24> ready <0/1> answer <count> noprobe <+32> reset <device count> <+8> create <q>
  CAMDIR <sun direction x y z bits> <camera x y z bits>
  (the stubs' lines in call order)
  OUT <+0> <+4 bits> <+8> <+12> <+16> <+20> <+33> <+44 bits>

python sun_visible_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file> --tool <mcla-tool>
"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

# the case's answers, read by the stubs (declared ahead of them)
recomp_closure.EXTRA += 'static uint32_t DEV,CREATE,READY,ANSWER;' + chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def cases(rnd):
    counts = [0, 1, 2, 624, 625, 999, 1000, 1249, 1250, 1251, 2500, 5000, 1843200, 16777217, 0x7FFFFFFF, 0x80000000, 0xFFFFFFFF]
    totals = [1250, 1250, 1250, 1, 1000, 0, -1, -1250, 0x7FFFFFFF, 3]
    out = []
    for k in range(600):
        c = rnd.choice(counts) if rnd.random() < 0.6 else rnd.randrange(0, 4000)
        old = rnd.choice(counts) if rnd.random() < 0.5 else rnd.randrange(0, 4000)
        t = rnd.choice(totals)
        ready = int(rnd.random() < 0.7)
        noprobe = int(rnd.random() < 0.15)
        idx = rnd.randrange(2)
        rc = 7
        dev = 7 if rnd.random() < 0.9 else rnd.choice((8, -1 & 0xFFFFFFFF))
        create = 0x1E00 if rnd.random() < 0.9 else 0
        d = [rnd.uniform(-1, 1) for _ in range(3)]
        n = sum(x * x for x in d) ** 0.5 or 1
        d = [x / n for x in d]
        cam = [rnd.uniform(-3000, 3000), rnd.uniform(-50, 300), rnd.uniform(-3000, 3000)]
        out.append((c, old, t & 0xFFFFFFFF, ready, noprobe, idx, rc, dev, create, d, cam))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    A = lambda f: 'sub_' + addr(a.tool, f)
    fn = A('F-63954')
    nl, NL = chr(10), chr(92) + 'n'
    # the viewport global the function reads (lis -32124, -26876) and the library's device global (lis -32124, -28076)
    vpg = (0x82840000 - 26876) & 0xFFFFFFFF
    stubs = {
        A('F-50821'): 'store(ctx.r3.u32,DEV);printf("DEVCOUNT' + NL + '");',
        A('F-12299'): 'printf("RELEASE %08x' + NL + '",load(ctx.r3.u32));',
        A('F-63097'): 'printf("CREATE %d' + NL + '",ctx.r3.s32);ctx.r3.u64=CREATE;',
        A('F-52930'): 'printf("READ %08x' + NL + '",ctx.r3.u32);if(READY)store(ctx.r4.u32,ANSWER);ctx.r3.u64=READY;',
        A('F-38438'): 'ctx.r3.u64=100+ctx.r3.u32;',
        A('F-33110'): 'printf("SET %u %u' + NL + '",ctx.r3.u32,ctx.r4.u32);',
        A('F-20853'): 'printf("EVENT %u' + NL + '",ctx.r4.u32);',
        A('F-91575'): 'printf("BEGIN %08x' + NL + '",ctx.r3.u32);',
        A('F-84063'): 'printf("END %08x' + NL + '",ctx.r3.u32);',
        A('F-26310'): 'printf("FLUSH' + NL + '");',
        A('F-68992'): ('{uint32_t n=load(ctx.r3.u32);printf("POP %u",n);for(uint32_t i=0;i<n&&i<8;i++)printf(" %u=%u",load(ctx.r3.u32+4+8*i),load(ctx.r3.u32+8+8*i));printf("' + NL + '");}'),
    }
    # the sphere: its radius, slices, flags, the matrix it gets (r3) and where r4 points against r3
    stubs[A('F-81209')] = ('{float r=(float)ctx.f1.f64;uint32_t rb;memcpy(&rb,&r,4);printf("SPHERE %08x %u %u %u",rb,ctx.r5.u32,ctx.r6.u32,ctx.r7.u32);'
                           'for(int i=0;i<16;i++)printf(" %08x",load(ctx.r3.u32+4*i));printf(" r4 %d' + NL + '",int(ctx.r4.u32-ctx.r3.u32));}')
    cs = cases(random.Random(63954))
    rows = []
    for (c, old, t, ready, noprobe, idx, rc, dev, create, d, cam) in cs:
        rows.append([c, old, t, ready, noprobe, idx, rc, dev, create] + [fbits(x) for x in d] + [fbits(x) for x in cam])
    body = ('static const uint32_t C[][15]={' + ','.join('{' + ','.join('0x%08xu' % v for v in r) + '}' for r in rows) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'int k=0;for(auto&c:C){memset(objects,0,sizeof(objects));' +
            'printf("CASE %d count0 %u idx %u total %d ready %u answer %u noprobe %u reset %d %d create %08x' + NL + '",k++,c[1],c[5],int(c[2]),c[3],c[0],c[4],int(c[7]),int(c[6]),c[8]);' +
            'printf("CAMDIR %08x %08x %08x %08x %08x %08x' + NL + '",c[9],c[10],c[11],c[12],c[13],c[14]);' +
            'DEV=c[7];CREATE=c[8];READY=c[3];ANSWER=c[0];' +
            'store(0x1000,c[1]);store(0x1004,0x7FC00001u);store(0x1008,c[6]);store(0x100C,c[5]);store(0x1010,0x1E40);store(0x1014,0x1E80);store(0x1018,c[2]);'
            'store(0x101C,0x41A00000u);pointer(0x1020)[0]=uint8_t(c[4]);pointer(0x1021)[0]=0;store(0x1024,0x442F0000u);store(0x1028,0x43480000u);store(0x102C,0x42340000u);' +
            'for(int i=0;i<3;i++){store(0x1100+4*i,c[9+i]);store(0x1270+4*i,c[12+i]);}store(0x110C,0);store(0x127C,0x3F800000u);' +
            ('store(0x%08xu,0x1200);' % vpg) +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;' + ('%s(ctx,nullptr);' % fn) +
            'printf("OUT %u %08x %d %u %08x %08x %u %08x' + NL + '",load(0x1000),load(0x1004),int(load(0x1008)),load(0x100C),load(0x1010),load(0x1014),unsigned(pointer(0x1021)[0]),load(0x102C));}}' + nl)
    out = run(build(index(a.recomp), [fn], body, a.out, 'sun_visible', stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    assert out.count('CASE') == len(cs) and out.count('OUT') == len(cs), out[:600]
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(out)
    print(len(cs), 'cases', out.count('SPHERE'), 'probes', out.count('READ'), 'reads')


if __name__ == '__main__':
    main()
