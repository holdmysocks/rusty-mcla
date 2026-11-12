"""The speed controller's car-ahead entry (F-10623) and stop-line entry (F-83573), as F-33409 calls them, run offline: the
generated code whole and unchanged on flat guest memory. Stubbed, as inputs of the case: the obstacle search F-47175
(writes the case's nearest to the context +0), the light test F-16850 (the case's go / stop), the car's place F-58860 (the
case's place) and the car's obstacle object F-88657 (the case's). The obstacles' slots 1 (matrix: row 2 at +32, place at
+48), 2 (velocity) and 6 (length) and the light lane's slots 15 / 2 (ignored) and 12 (the stop point) are dispatched to the
case's values; F-96472 (the obstacle rides a lane) runs in the closure on the case's car / behaviour / rail words.
Memory: the context 0x1000 (+0 nearest, +12 mark, +16 car 0x1100, +20 the stop's junction, +24 the obstacle, +28 gap),
the car 0x1100 (+412 speed, +448 0x1500, +496 0x1600, +500 0), the params 0x1080 (+0 behaviour 0x1400), the own obstacle
object 0x1600 and the obstacle 0x1800 (vtable 0x1700; +6 kind, +16 car 0x2400 -> +452 behaviour 0x2500 -> +188 rail
0x2600 -> +4 flags), the record 0x1E40, the light lane 0x2800 (vtable 0x2900; slot 15 -> 0x2A00, vtable 0x2B00).
Writes <out-dir>/ahead_stop_ref.txt: 'A' or 'S', the case's words, then '->' and the record's +0 +4 +8 +12 words, its
+16 byte and the context's identity word (+24 / +20), hex."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\nstatic const uint32_t *K;\n')


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def unit(rnd):
    t = rnd.uniform(0, 6.283185)
    y = rnd.choice((0.0, 0.0, rnd.uniform(-0.1, 0.1)))
    return (math.cos(t), y, math.sin(t))


def clean(c):
    # (no word that reads as an image address in the test data)
    return all(not (0x82000000 <= w <= 0x82FFFFFF) for w in c)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ahead, stop, search, light, place, obj = (addr(a.tool, f) for f in ('F-10623', 'F-83573', 'F-47175', 'F-16850', 'F-58860', 'F-88657'))
    rnd = random.Random(10623)
    A, S = [], []
    while len(A) < 4000:
        near = int(rnd.random() < 0.9)
        mark = int(near and rnd.random() < 0.08)
        prev = rnd.choice((0, 1, 1, 2))
        sp = rnd.choice((0.0, 13.4112, 31.29, 31.2928, 31.293, rnd.uniform(0, 40), -rnd.uniform(0, 2)))
        gap = rnd.choice((3.5, rnd.uniform(2, 5)))
        me = (rnd.uniform(-3000, 3000), rnd.uniform(0, 60), rnd.uniform(-3000, 3000))
        r2 = unit(rnd)
        mylen = rnd.uniform(3, 12)
        if rnd.random() < 0.04:
            op = me
        else:
            k = rnd.uniform(0.5, 80)
            op = (me[0] + rnd.uniform(-1, 1) * k, me[1] + rnd.uniform(-2, 2), me[2] + rnd.uniform(-1, 1) * k)
        o2 = unit(rnd)
        vs = rnd.choice((0.0, rnd.uniform(0, 30)))
        ov = (o2[0] * vs * rnd.choice((1, -1)), rnd.uniform(-0.5, 0.5), o2[2] * vs) if rnd.random() < 0.8 else unit(rnd)
        olen = rnd.uniform(3, 12)
        kind = rnd.choice((0, 1, 2, 2, 2, 9))
        beh = int(rnd.random() < 0.85)
        rail = int(beh and rnd.random() < 0.9)
        flags = rnd.choice((0, 0, 0x800, 0x1000, 0x1800, 0x2000)) if rail else 0
        byte = rnd.randrange(256)
        c = [near, mark, prev, fb(sp), fb(gap), *map(fb, me), *map(fb, r2), fb(mylen), *map(fb, op), *map(fb, o2), *map(fb, ov),
             fb(olen), kind, beh, rail, flags, byte]
        if clean(c):
            A.append(c)
    while len(S) < 4000:
        lane = int(rnd.random() < 0.9)
        go = int(rnd.random() < 0.3)
        prev = rnd.choice((0, 1, 1, 2))
        me = (rnd.uniform(-3000, 3000), rnd.uniform(0, 60), rnd.uniform(-3000, 3000))
        if rnd.random() < 0.03:
            me = (float('nan'), me[1], me[2])
        if rnd.random() < 0.03:
            pt = me
        else:
            k = rnd.uniform(0, 80)
            pt = (me[0] + rnd.uniform(-1, 1) * k, me[1] + rnd.uniform(-1, 1), me[2] + rnd.uniform(-1, 1) * k)
        mylen = rnd.uniform(3, 12)
        byte = rnd.randrange(256)
        c = [lane, go, prev, 0x3000, *map(fb, me), *map(fb, pt), fb(mylen), byte]
        if clean(c):
            S.append(c)
    nl = chr(10)
    NL = chr(92) + 'n'
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){'
            'uint32_t o=ctx.r3.u32;'
            'if(a==0x9104)ctx.r3.u32=o==0x1600?0x1A00:0x1B00;'
            'else if(a==0x9108)ctx.r3.u32=0x1BC0;'
            'else if(a==0x9118)ctx.f1.f64=getf(o==0x1600?0x1D00:0x1D04);'
            'else if(a==0x9300)ctx.r3.u32=0x2A00;else if(a==0x9308){}'
            'else if(a==0x9330)ctx.r3.u32=0x1C40;'
            'else throw std::runtime_error("unexpected indirect call");}')
    common = ('memset(objects,0,sizeof(objects));K=c;'
              'store(0x1010,0x1100);store(0x1100+448,0x1500);store(0x1100+496,0x1600);store(0x1080,0x1400);store(0x1084,0x2000);'
              'store(0x1600,0x1700);store(0x1800,0x1700);store(0x1700+4,0x9104);store(0x1700+8,0x9108);store(0x1700+24,0x9118);')
    body_a = ('for(auto&c:CA){' + common +
              'store(0x1100+412,c[3]);store(0x101C,c[4]);store(0x1C00,c[5]);store(0x1C04,c[6]);store(0x1C08,c[7]);'
              'store(0x1A20,c[8]);store(0x1A24,c[9]);store(0x1A28,c[10]);store(0x1D00,c[11]);'
              'store(0x1B30,c[12]);store(0x1B34,c[13]);store(0x1B38,c[14]);store(0x1B20,c[15]);store(0x1B24,c[16]);store(0x1B28,c[17]);'
              'store(0x1BC0,c[18]);store(0x1BC4,c[19]);store(0x1BC8,c[20]);store(0x1D04,c[21]);'
              'objects[0x1806-0x1000]=c[22];store(0x1810,0x2400);store(0x2400+452,c[23]?0x2500:0);store(0x2500+188,c[24]?0x2600:0);'
              'store(0x2604,c[25]);objects[0x1E50-0x1000]=c[26];'
              'store(0x100C,c[1]?0x1800:0x1F00);store(0x1018,c[2]==0?0:c[2]==1?0x1800:0x1E00);'
              'Context ctx{};ctx.r1.u32=0x7F000;ctx.r3.u32=0x1000;ctx.r4.u32=0x1080;ctx.r5.u32=0x1E40;'
              'sub_%s(ctx,nullptr);' % ahead +
              ('printf("%%08x %%08x %%08x %%08x %%02x %%08x%s",load(0x1E40),load(0x1E44),load(0x1E48),load(0x1E4C),'
               'objects[0x1E50-0x1000],load(0x1018));}' % NL))
    body_s = ('for(auto&c:CS){' + common +
              'store(0x1C00,c[4]);store(0x1C04,c[5]);store(0x1C08,c[6]);store(0x1C40,c[7]);store(0x1C44,c[8]);store(0x1C48,c[9]);'
              'store(0x1D00,c[10]);objects[0x1E50-0x1000]=c[11];store(0x1E40,0x1234);store(0x1E44,0x40400000);'
              'store(0x2800,0x2900);store(0x2900+60,0x9300);store(0x2900+48,0x9330);store(0x2A00,0x2B00);store(0x2B00+8,0x9308);'
              'store(0x1014,c[2]==0?0:c[2]==1?0x3000:0x3100);'
              'Context ctx{};ctx.r1.u32=0x7F000;ctx.r3.u32=0x1000;ctx.r4.u32=0x1400;ctx.r5.u32=0x1E40;ctx.r6.u32=c[0]?0x2800:0;'
              'ctx.r7.u32=c[3];sub_%s(ctx,nullptr);' % stop +
              ('printf("%%08x %%08x %%08x %%08x %%02x %%08x%s",load(0x1E40),load(0x1E44),load(0x1E48),load(0x1E4C),'
               'objects[0x1E50-0x1000],load(0x1014));}' % NL))
    rows = lambda cs: nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cs)
    src = (disp + nl + 'static const uint32_t CA[][%d]={' % len(A[0]) + nl + rows(A) + nl + '};' + nl +
           'static const uint32_t CS[][%d]={' % len(S[0]) + nl + rows(S) + nl + '};' + nl +
           'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
           'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body_a + nl + body_s + nl + '}' + nl)
    stubs = {
        'sub_' + search: 'store(ctx.r3.u32,K[0]?0x1800:0);',
        'sub_' + light: 'ctx.r3.u32=K[1];',
        'sub_' + place: 'ctx.r3.u32=0x1C00;',
        'sub_' + obj: 'ctx.r3.u32=0x1600;',
    }
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + ahead, 'sub_' + stop], src, a.out_dir, 'ahead_stop', stubs=stubs, vector=True, scratch_end=0x80000)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(A) + len(S), (len(out), len(A), len(S))
    lines = (['A ' + ' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(A, out[:len(A)])] +
             ['S ' + ' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(S, out[len(A):])])
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'ahead_stop_ref.txt').write_text(nl.join(lines) + nl)
    oa, os_ = out[:len(A)], out[len(A):]
    print(len(A), 'ahead cases,', sum(int(o.split()[4], 16) & 0x80 != 0 for o in oa), 'with the entry,',
          sum(int(o.split()[4], 16) & 0x40 != 0 for o in oa), 'new;', len(S), 'stop cases,',
          sum(int(o.split()[4], 16) & 0x80 != 0 for o in os_), 'with the entry,', sum(int(o.split()[4], 16) & 0x40 != 0 for o in os_), 'new')


if __name__ == '__main__':
    main()
