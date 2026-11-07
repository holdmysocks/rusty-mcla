"""The holder's draw (F-24071) with the base type draw (F-30631) behind the effect's and the second slot's vtable +40 run
offline: unchanged generated code on flat guest memory (recomp_closure, vector shim, -msse4.1).
Holder at 0x1000 (+4 its car 0x1200, +144 / +176 / +208 vectors, +192 / +224 floats, +228 shot, +232 flag, +236 effect,
+240 second slot, +164 / +196 icons), car 0x1200 (+8 the physical object 0x1300, whose +240 matrix is the car's),
type objects 0x1100 (effect) / 0x1500 (second slot) / 0x1600 (shot), each with byte +36 DrawIcon and +4 its id; their
vtable 0x1180: +40 the real F-30631, every other slot a marker (+4, the mega test, answers 0: its branch only writes
statics no other code reads - see the evidence file).
Stub F-12500 (the source draw: what it is given): logs the matrix rows (64 bytes at r3), r4 the look, f1, f2 and r8.
Each output line: the case, then the calls in order.
Usage: pu_icon_card_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --tool mcla-tool
       --work DIR --out FILE"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build

recomp_closure.EXTRA += '#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--tool', required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    a = ap.parse_args()
    hdraw, base, src = (addr(a.tool, f) for f in ('F-24071', 'F-30631', 'F-12500'))
    rnd = random.Random(20261107)
    cases = []
    for i in range(400):
        eff = rnd.random() < 0.8
        icon = rnd.random() < 0.6
        kind = rnd.randrange(24)
        second = rnd.random() < 0.3
        icon2 = rnd.random() < 0.6
        # holder +208: zero in the game (F-24971 is its only writer); random here to see where it goes
        v208 = [0.0, 0.0, 0.0, 0.0] if i % 2 == 0 else [rnd.uniform(-500, 500) for _ in range(3)] + [rnd.choice((0.0, 1.0))]
        mat = [rnd.uniform(-1, 1) for _ in range(12)] + [rnd.uniform(-900, 900) for _ in range(3)] + [1.0]
        cases.append([int(eff), int(icon), kind, int(second), int(icon2)] + [fb(x) for x in v208] + [fb(x) for x in mat])
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){if(a==0x%s){printf(" B:%%x:%%x:%%x",ctx.r3.u32,ctx.r4.u32,ctx.r5.u32);sub_%s(ctx,base);}'
            'else{printf(" V:%%x:%%x",a,ctx.r3.u32);ctx.r3.u32=0;}}' % (base, base))
    stubs = {'sub_' + src: 'printf(" S:%x:%u:%08x:%08x:%u",ctx.r3.u32,ctx.r4.u32,[](float v){uint32_t b;std::memcpy(&b,&v,4);return b;}(float(ctx.f1.f64)),'
                           '[](float v){uint32_t b;std::memcpy(&b,&v,4);return b;}(float(ctx.f2.f64)),ctx.r8.u32&0xFF);'
                           'for(int k=0;k<16;k++)printf(":%08x",load(ctx.r3.u32+4*k));'}
    body = ('static const uint32_t C[][25]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            'printf("%u %u %u %u %u",c[0],c[1],c[2],c[3],c[4]);for(int k=5;k<25;k++)printf(" %08x",c[k]);' +
            'store(0x1000+4,0x1200);store(0x1200+8,0x1300);for(int k=0;k<16;k++)store(0x1300+240+4*k,c[9+k]);' +
            'for(int k=0;k<4;k++)store(0x1000+208+4*k,c[5+k]);' +
            'for(int k=0;k<24;k++)store(0x1180+4*k,0x91000000+4*k);store(0x1180+40,0x%s);' % base +
            'store(0x1100,0x1180);store(0x1100+4,c[2]);pointer(0x1100+36)[0]=c[1];' +
            'store(0x1500,0x1180);store(0x1500+4,23);pointer(0x1500+36)[0]=c[4];' +
            'store(0x1000+236,c[0]?0x1100:0);store(0x1000+240,c[3]?0x1500:0);' +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r1.u32=0x2F00;sub_%s(ctx,nullptr);printf("' % hdraw + NL + '");}' + nl)
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + hdraw, 'sub_' + base], main_src, a.work, 'pu_icon_card', stubs=stubs, vector=True, flags=('-msse4.1',))
    out = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True, check=True).stdout.replace('\r', '')
    lines = out.splitlines()
    assert len(lines) == len(cases), len(lines)
    a.out.write_text(out)
    print(len(lines), 'lines')


if __name__ == '__main__':
    main()
