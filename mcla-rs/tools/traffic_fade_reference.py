"""The ambient car's fade (F-82632) run offline: the generated code whole and unchanged on flat guest memory. The removal
F-51512 is stubbed and prints its car and reason. Memory: the car 0x2000 (+484 the fade, +488 the reason, +500 the flag
byte, +456 the draw holder 0x3000 or 0); the view index word (0x8288B9F0, zero in the image) is left as it is, so the
holder's slot +44 gets the fade.

Writes <out-dir>/traffic_fade_ref.txt, a case a line: flags fade dt holder (hex words; floats as bits) '->' the flag
byte, the fade, the removal's reason (or -), the holder's slot +44."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build, run

recomp_closure.shim = lambda *_: recomp_leaf.shim(0x10000)
NL = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def bf(u):
    return struct.unpack('>f', struct.pack('>I', max(u, 0)))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn, rem = addr(a.tool, 'F-82632'), addr(a.tool, 'F-51512')
    rnd = random.Random(82632)
    cs = []
    for _ in range(5000):
        flags = rnd.randint(0, 255) & ~0xC
        flags |= rnd.choice((0, 4, 8, 12, 4, 8))
        dt = rnd.choice((1 / 30, 1 / 60, 0.0, rnd.uniform(0, 0.1), rnd.uniform(0, 1), bf(1)))
        fade = rnd.choice((0.0, 1.0, rnd.uniform(0, 1), rnd.uniform(-0.1, 1.1), f32(1 - 2 * f32(dt)), f32(2 * f32(dt)),
                           bf(fb(f32(2 * f32(dt))) + rnd.randint(-2, 2)), bf(fb(f32(1 - 2 * f32(dt))) + rnd.randint(-2, 2))))
        holder = rnd.choice((0, 0x3000))
        cs.append([flags, fb(f32(fade)), fb(f32(dt)), holder])
    stubs = {'sub_' + rem: 'printf("R%x ",load(ctx.r4.u32+0)==0?ctx.r5.u32:ctx.r5.u32);'}
    rows = NL.join('{%s},' % ','.join('0x%08xu' % x for x in c) for c in cs)
    body = ('static const uint32_t C[][4]={' + NL + rows + NL + '};' + NL +
            'for(auto&c:C){memset(objects,0,sizeof(objects));'
            '*pointer(0x2000+500)=uint8_t(c[0]);store(0x2000+484,c[1]);store(0x2000+488,0x2A);store(0x2000+456,c[3]);'
            'Context ctx{};ctx.r1.u32=0xF000;ctx.r3.u32=0x2000;uint32_t d=c[2];float df;memcpy(&df,&d,4);ctx.f1.f64=df;'
            'sub_%s(ctx,nullptr);printf("%%x %%x %%x\\n",*pointer(0x2000+500),load(0x2000+484),load(0x3000+44));}' % fn) + NL
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + NL + body + '}' + NL)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fn], main_src, a.out_dir, 'traffic_fade', stubs=stubs, scratch_end=0x10000)
    out = [x for x in run(exe, a.image).split(NL) if x]
    assert len(out) == len(cs), (len(out), len(cs))
    lines = []
    for c, o in zip(cs, out):
        w = o.split()
        reason = w[0][1:] if w[0].startswith('R') else '-'
        rest = w[1:] if w[0].startswith('R') else w
        lines.append(' '.join('%x' % x for x in c) + ' -> ' + ' '.join([rest[0], rest[1], reason, rest[2]]))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'traffic_fade_ref.txt').write_text(NL.join(lines) + NL)
    print(len(lines), 'cases,', sum(' 2a ' in l for l in lines), 'removed')


if __name__ == '__main__':
    main()
