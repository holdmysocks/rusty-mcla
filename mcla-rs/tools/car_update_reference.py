"""An ambient car's slot 6 after the tick (F-43567) with the car update it runs first (F-50624), run offline: the generated
code whole and unchanged on flat guest memory, with the state check after the behaviour frame (F-35182), the body-active
getter (F-17857) and the behaviour context's init (F-39469) in the closure. Every other callee is a recorder that appends
its letter to the case's log (and returns the case's answer where it has one):
  a  +456's slot 9 (dt)                    F  the filing F-27899            T  the +208 tracker F-53150
  b  +472's slot 11 (behaviour / 0x10)    c  car slot 10 (F-93055)         D  the fade F-82632
  R  the render update F-37359             e  behaviour slot 3 (F-87488)   g / h  behaviour slots 8 / 9 (answer)
  i  behaviour slot 4 (the pose out)       N  the peer test F-23063 (answer)  X<n> the removal F-51512 with code n
  j  car slot 30 (the asked acceleration, answer)                            I  the integrator F-58614
  k  car slot 29 (F-99638, the trailer's cab)  l  car slot 28 (F-16044, the tow)  m  car slot 20 (F-22467, the seat)
  P  the lock's ring push F-60856 (with car +408)                            Z  the rest rule F-72067 (on car +2688)
Memory: the update context 0x1000 (+36 the manager 0x2000, +40 dt), the car 0x4000 (vtable 0x5000; +52 the proxy's
instance 0x6000 -> +48 0x6100 -> +28 0x6200; +112 its own instance, +120 its level index; +128 its matrix; +208 the
velocity; +452 behaviour 0x5C00 (vtable 0x5D00); +456 0x5800; +472 0x5A00; +476 the id; +500 the flags), the level at
0x828DA464 -> 0x7000 (+112 the state table 0x7100, 8 bytes an index, +4 the state), the update index at 0x8288B9F0.
Writes <out-dir>/car_update_ref.txt: the case's words, then '->', the log, car +412 / +408 / +460 and the first word of
the matrix slots +256 / +320 (0x1111xxxx the car's own matrix, 0x2222xxxx the proxy's), hex."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += ('#define ctr r2\n#include <string>\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\nstatic const uint32_t *K;static std::string L;\n')

CAR, SCR = 0x4000, 0x100000


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    S = {f: 'sub_' + addr(a.tool, f) for f in ['F-23063', 'F-27899', 'F-37359', 'F-43567', 'F-51512', 'F-53150', 'F-58614', 'F-60856', 'F-72067', 'F-82632']}
    rnd = random.Random(43567)
    C = []
    for _ in range(6000):
        flags = rnd.choice((0, 0, 0, 0x10, 0x20, 0x30, rnd.randrange(256)))
        has456, has472, beh = int(rnd.random() < 0.5), int(rnd.random() < 0.5), int(rnd.random() < 0.75)
        chain = rnd.choice((0, 1, 2, 2))
        own = rnd.choice((0, 1, 2, 3, 0xFFFF))
        prox = rnd.choice((0, 1, 2, 3, 0xFFFF))
        ownst, proxst = (rnd.choice((0, 0, 1, 2, 3, 4, 7)) for _ in range(2))
        s8, s9 = int(rnd.random() < 0.1), int(rnd.random() < 0.1)
        peer = int(rnd.random() < 0.3)
        acc = rnd.choice((0.0, -3.5, rnd.uniform(-20, 5)))
        cab = rnd.choice((0, 0, 0x4800))
        tow = rnd.choice((0, 1, 2, 3, 0x101))
        seat = int(rnd.random() < 0.85)
        v = [rnd.choice((0.0, rnd.uniform(-30, 30))) for _ in range(3)]
        u = rnd.randrange(2)
        dt = rnd.choice((1 / 30, 0.0, rnd.uniform(0.01, 0.05)))
        C.append([flags, has456, has472, beh, chain, own, ownst, prox, proxst, s8, s9, peer, fb(acc), cab, tow, seat,
                  *map(fb, v), u, fb(dt), rnd.randrange(1 << 16)])
    nl = chr(10)
    NL = chr(92) + 'n'
    rec = lambda ch: 'L+="%s";' % ch
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){'
            'if(a==0x9100)' + rec('a') + 'else if(a==0x9200)' + rec('b') + 'else if(a==0x9300)' + rec('c') +
            'else if(a==0x9400)' + rec('e') + 'else if(a==0x9408){' + rec('g') + 'ctx.r3.u32=K[9];}'
            'else if(a==0x940C){' + rec('h') + 'ctx.r3.u32=K[10];}else if(a==0x9410)' + rec('i') +
            'else if(a==0x9500){' + rec('j') + 'ctx.f1.f64=getf_bits(K[12]);}'
            'else if(a==0x9504){' + rec('k') + 'ctx.r3.u32=K[13];}else if(a==0x9508){' + rec('l') + 'ctx.r3.u32=K[14];}'
            'else if(a==0x950C){' + rec('m') + 'ctx.r3.u32=K[15];}'
            'else throw std::runtime_error("unexpected indirect call");}')
    body = ('for(auto&c:CC){memset(objects,0,sizeof(objects));K=c;L.clear();'
            'store(0x1024,0x2000);store(0x1028,c[20]);'
            'store(0x%X,0x5000);' % CAR +
            'store(0x5000+40,0x9300);store(0x5000+120,0x9500);store(0x5000+116,0x9504);store(0x5000+112,0x9508);store(0x5000+80,0x950C);'
            'store(0x5D00+12,0x9400);store(0x5D00+32,0x9408);store(0x5D00+36,0x940C);store(0x5D00+16,0x9410);'
            'store(0x5C00,0x5D00);store(0x5800,0x5900);store(0x5900+36,0x9100);store(0x5A00,0x5B00);store(0x5B00+44,0x9200);'
            'store(0x%X+452,c[3]?0x5C00:0);store(0x%X+456,c[1]?0x5800:0);store(0x%X+472,c[2]?0x5A00:0);' % (CAR, CAR, CAR) +
            'store(0x%X+476,c[21]);*pointer(0x%X+500)=c[0];' % (CAR, CAR) +
            'store(0x%X+52,c[4]?0x6000:0);store(0x6000+48,c[4]==2?0x6100:0);store(0x6100+28,0x6200);' % CAR +
            '*pointer(0x%X+120)=c[5]>>8;*pointer(0x%X+121)=c[5];*pointer(0x6208)=c[7]>>8;*pointer(0x6209)=c[7];' % (CAR, CAR) +
            'store(0x828DA464,0x7000);store(0x7000+112,0x7100);store(0x7100+c[5]*8+4,c[6]);store(0x7100+c[7]*8+4,c[8]);'
            'for(int k=0;k<16;k++){store(0x%X+128+4*k,0x11110000+k);store(0x6210+4*k,0x22220000+k);}' % CAR +
            'store(0x%X+208,c[16]);store(0x%X+212,c[17]);store(0x%X+216,c[18]);store(0x8288B9F0,c[19]);' % (CAR, CAR, CAR) +
            'store(0x%X+408,0xDEAD0408);store(0x%X+412,0xDEAD0412);' % (CAR, CAR) +
            'Context ctx{};ctx.r1.u32=0xFF000;ctx.r3.u32=0x%X;ctx.r4.u32=0x1000;%s(ctx,nullptr);' % (CAR, S['F-43567']) +
            ('printf("%%s %%08x %%08x %%08x %%08x %%08x%s",L.c_str(),load(0x%X+412),load(0x%X+408),load(0x%X+460),'
             'load(0x%X+256),load(0x%X+320));}' % (NL, CAR, CAR, CAR, CAR, CAR)))
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in C)
    src = (disp + nl + 'static const uint32_t CC[][%d]={' % len(C[0]) + nl + rows + nl + '};' + nl +
           'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
           'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + nl + '}' + nl)
    src = 'static double getf_bits(uint32_t b){float f;memcpy(&f,&b,4);return f;}' + nl + src
    stubs = {
        S['F-27899']: rec('F'), S['F-53150']: rec('T'), S['F-82632']: rec('D'), S['F-37359']: rec('R'),
        S['F-23063']: rec('N') + 'ctx.r3.u32=K[11];',
        S['F-51512']: 'L+="X"+std::to_string(ctx.r5.u32);',
        S['F-58614']: rec('I'),
        S['F-60856']: 'L+=ctx.r4.u32==0x%X+408?"P":"P?";' % CAR,
        S['F-72067']: 'L+=ctx.r3.u32==0x%X+2688?"Z":"Z?";' % CAR,
    }
    idx = index(a.recomp)
    exe = build(idx, [S['F-43567']], src, a.out_dir, 'car_update', stubs=stubs, vector=True, scratch_end=SCR)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(C), (len(out), len(C))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'car_update_ref.txt').write_text(nl.join(' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(C, out)) + nl)
    from collections import Counter
    print(len(C), 'cases;', Counter(o.split()[0] for o in out).most_common(25))


if __name__ == '__main__':
    main()
