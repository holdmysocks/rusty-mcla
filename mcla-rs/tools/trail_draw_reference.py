"""The light trails' draw F-89089 run offline (the generated code as it is), for the port's test
(crates/game/src/testdata/trail_draw_reference.txt).

Its callees are stubs: the camera cut F-40536 and the pause test F-50459 answer the case's flags (both are run offline
elsewhere: trail_small_reference.py); the ribbon F-80988 (run offline by trail_ribbon_reference.py) writes the case's
outputs for the record it gets (count, centres, across, alphas, the two caps); the effect's pass count F-94845 answers the
case's; the immediate-mode begin F-33202 / vertex F-97666 / end F-15766 and the shader variable set F-17339 (the record's newest point) print what
they get; the matrix and effect calls print nothing. Globals: the 96 records at 0x82883970 (336 bytes: +48 colour, +112
the newest point, +304 count, +308 frame of last use, +316 drawn), the frame 0x827D2DCC, the gate byte 0x8287E368 = 0
(0 in the image; no code writes it by name), the viewport and effect pointers pointed at scratch objects.

  CASE <n> frame <frame> cut <0/1> paused <0/1> passes <p>
  IN <record> <count> <stamp> <drawn> <colour, decimal>                (each record before)
  RIB <record> <m> ...                                           (the ribbon outputs the stub hands back: m, then
                                                                  start xyz, end xyz, m x (centre xyz, across xyz, alpha))
  CP <x y z w>        B <prim> <count>        V <x y z> <nx ny nz> <u v> <colour, decimal>        E
  OUT <record> <count> <drawn>                                   (each record after)

python trail_draw_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file> --tool <mcla-tool>
"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += 'static const uint32_t*cur;static uint32_t CUT,PAUSED,PASSES;' + chr(10)

REC, N, FRAME, GATE = 0x82883970, 96, 0x827D2DCC, 0x8287E368
VIEWPORT, EFFECT, RENDERER = 0x82839704, 0x8287E318, 0x829054A0


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def cases(rnd):
    out = []
    for k in range(30):
        frame = rnd.randint(50, 100000)
        cut = int(rnd.random() < 0.08)
        paused = int(rnd.random() < 0.08)
        passes = rnd.choice((1, 1, 1, 2, 0))
        recs = []
        for i in range(N):
            count = rnd.choice((0, 0, 1, 2, 3, 3, 4, 5, 6, 7, 8, 8, 8))
            stamp = rnd.choice((frame, frame, frame - 1, frame - rnd.randint(2, 50))) & 0xFFFFFFFF
            drawn = rnd.randrange(2)
            colour = rnd.getrandbits(32)
            h0 = [rnd.uniform(-50, 50), rnd.uniform(-20, 20), -rnd.uniform(1, 300)]
            m = rnd.choice((2, 2, 3, 4, 5, 6, 7))
            rib = [m] + [fb(rnd.uniform(-3, 3)) for _ in range(6)]
            for j in range(m):
                rib += [fb(h0[c] + rnd.uniform(-5, 5)) for c in range(3)] + [fb(rnd.uniform(-2, 2)) for _ in range(3)]
                rib += [fb(rnd.choice((0.0, rnd.uniform(0, 0.55), 0.5, 0.55, 0.421875, 1.0 / 255, 0.999, 1.0, rnd.uniform(1, 3), -rnd.uniform(0, 2), 1e10, -1e10)))]
            recs.append((count, stamp, drawn, colour, [fb(x) for x in h0], rib))
        out.append((frame, cut, paused, passes, recs))
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
    fn = A('F-89089')
    nl, NL = chr(10), chr(92) + 'n'
    fbits = 'auto bits=[](double d){float f=(float)d;uint32_t u;memcpy(&u,&f,4);return u;};'
    stubs = {
        A('F-40536'): 'ctx.r3.u64=CUT;',
        A('F-50459'): 'ctx.r3.u64=PAUSED;',
        A('F-94845'): 'ctx.r3.u64=PASSES;',
        A('F-17339'): 'printf("CP %08x %08x %08x %08x' + NL + '",load(ctx.r6.u32),load(ctx.r6.u32+4),load(ctx.r6.u32+8),load(ctx.r6.u32+12));',
        A('F-33202'): 'printf("B %u %u' + NL + '",ctx.r3.u32,ctx.r4.u32);',
        A('F-97666'): fbits + 'printf("V %08x %08x %08x %08x %08x %08x %08x %08x %u' + NL + '",bits(ctx.f1.f64),bits(ctx.f2.f64),bits(ctx.f3.f64),'
                      'bits(ctx.f4.f64),bits(ctx.f5.f64),bits(ctx.f6.f64),bits(ctx.f7.f64),bits(ctx.f8.f64),ctx.r9.u32);',
        A('F-15766'): 'printf("E' + NL + '");',
        # the ribbon: the case's outputs for this record (centres r4, across r5, alphas r6, start cap r10, end cap at 84(r1))
        A('F-80988'): ('{uint32_t i=(ctx.r3.u32-%du)/336;const uint32_t*q=cur;' % REC +
                       # (walk the case's records to record i's ribbon block)
                       'q+=5;for(uint32_t k=0;k<i;k++){q+=8;q+=1+6+7*q[0];}q+=8;uint32_t m=q[0];'
                       'for(int c=0;c<3;c++){store(ctx.r10.u32+4*c,q[1+c]);store(load(ctx.r1.u32+84)+4*c,q[4+c]);}'
                       'for(uint32_t j=0;j<m;j++){const uint32_t*p=q+7+7*j;for(int c=0;c<3;c++){store(ctx.r4.u32+16*j+4*c,p[c]);store(ctx.r5.u32+16*j+4*c,p[3+c]);}'
                       'store(ctx.r4.u32+16*j+12,0x3f800000u);store(ctx.r5.u32+16*j+12,0);store(ctx.r6.u32+4*j,p[6]);}'
                       'printf("RIB %u %u",i,m);for(uint32_t w=1;w<7+7*m;w++)printf(" %08x",q[w]);printf("' + NL + '");ctx.r3.u64=m;}'),
    }
    for f in ('F-60960', 'F-37236', 'F-56814', 'F-89493', 'F-90259', 'F-46773'):
        stubs[A(f)] = ''
    cs = cases(random.Random(89089))
    flat = []
    for frame, cut, paused, passes, recs in cs:
        flat += [frame, cut, paused, passes, len(recs)]
        for count, stamp, drawn, colour, h0, rib in recs:
            flat += [count, stamp, drawn, colour] + h0 + [0]
            flat += rib
    body = ('static const uint32_t C[]={' + ','.join('0x%08xu' % v for v in flat) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};' + nl +
            'size_t k=0;int n=0;while(k<sizeof(C)/4){cur=&C[k];memset(objects,0,sizeof(objects));' +
            'uint32_t frame=C[k],np=C[k+4];CUT=C[k+1];PAUSED=C[k+2];PASSES=C[k+3];' +
            'printf("CASE %d frame %u cut %u paused %u passes %u' + NL + '",n++,frame,CUT,PAUSED,PASSES);' +
            ('put(0x%08xu,frame);image[0x%xu]=0;put(0x%08xu,0x1800);put(0x%08xu,0x1A00);put(0x%08xu,0x1B00);' % (FRAME, GATE - 0x82000000, VIEWPORT, EFFECT, RENDERER)) +
            'size_t q=k+5;for(uint32_t i=0;i<np;i++){uint32_t b=%du+336*i;for(uint32_t w=0;w<84;w++)put(b+4*w,0);' % REC +
            'put(b+304,C[q]);put(b+308,C[q+1]);image[b+316-0x82000000u]=uint8_t(C[q+2]);put(b+48,C[q+3]);'
            'for(int c=0;c<3;c++)put(b+112+4*c,C[q+4+c]);put(b+124,0x3f800000u);'
            'printf("IN %u %u %u %u %u' + NL + '",i,C[q],C[q+1],C[q+2],C[q+3]);q+=8;q+=1+6+7*C[q];}' +
            'Context ctx{};ctx.r1.u32=0x2F00;' + ('%s(ctx,nullptr);' % fn) +
            'for(uint32_t i=0;i<np;i++){uint32_t b=%du+336*i;' % REC +
            'uint32_t c=uint32_t(image[b+304-0x82000000u])<<24|uint32_t(image[b+305-0x82000000u])<<16|uint32_t(image[b+306-0x82000000u])<<8|image[b+307-0x82000000u];'
            'printf("OUT %u %u %u' + NL + '",i,c,unsigned(image[b+316-0x82000000u]));}k=q;}}' + nl)
    out = run(build(index(a.recomp), [fn], body, a.out, 'trail_draw', stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    assert out.count('CASE') == len(cs), out[:600]
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(out)
    print(len(cs), 'frames', out.count('RIB'), 'ribbons', out.count('V '), 'vertices')


if __name__ == '__main__':
    main()
