"""A sound's inherited parameters run offline: F-57176 (category, rolloff curve, curve scale, doppler, speaker mask and route
from the sound's expanded field block and its parent's context), the generated code unchanged with its callee (the
DEFAULT_ROLLOFF name hash), on flat guest memory: the sound at 0x1000 (+5 parent index, 255 = a root; +8 bit 0x01 doppler
not inherited; +9 bit 0x40 curve scale, 0x20 speaker mask not inherited), the context at 0x1100 (+0 category, +4 curve,
+8 curve scale, +12 doppler, +20 speaker mask, +21 route), the block at 0x1200 (+43 doppler, +45 category, +49 curve,
+53 curve scale, +55 speaker mask, +56 route); the constant 0.01 read from the image. The name hash callee F-58929 is stubbed and its once-only cache (0x82831D34 / bit 0 of
0x82831D38) preset to joaat("DEFAULT_ROLLOFF"), the value it stores (the harness shim lacks its byte loads).

Writes <out-dir>/inherit_ref.txt, one case a line:
"root s8 s9 b43 b45 b49 b53 b55 b56 p0 p4 p8 p12 p20 p21 -> c0 c4 c8 c12 c20 c21" (hex; floats as bits)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def joaat(s):
    h = 0
    for c in s.lower().encode():
        h = (h + c) & 0xffffffff
        h = (h + (h << 10)) & 0xffffffff
        h ^= h >> 6
    h = (h + (h << 3)) & 0xffffffff
    h ^= h >> 11
    return (h + (h << 15)) & 0xffffffff


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = addr(a.tool, 'F-57176')
    rnd = random.Random(57176)
    hashes = [0, 0, 0x1d2f8a43, 0xe0b4c2aa, rnd.getrandbits(32)]
    cases = []
    for _ in range(6000):
        root = rnd.random() < 0.25
        s8 = rnd.choice((0, 1, rnd.getrandbits(8)))
        s9 = rnd.choice((0, 0x40, 0x20, 0x60, rnd.getrandbits(8)))
        b43 = rnd.choice((100, 0, 50, 200, rnd.getrandbits(16)))
        b45 = rnd.choice(hashes)
        b49 = rnd.choice(hashes)
        b53 = rnd.choice((100, 0, 150, rnd.getrandbits(16)))
        b55 = rnd.choice((0, 3, rnd.getrandbits(8)))
        b56 = rnd.choice((0, 1, rnd.getrandbits(8)))
        p = [rnd.choice(hashes), rnd.choice(hashes), fb(rnd.choice((1.0, 0.5, 2.0, rnd.uniform(0, 5)))),
             fb(rnd.choice((1.0, 0.0, 1.5, rnd.uniform(0, 5)))), rnd.choice((0, 3, rnd.getrandbits(8))),
             rnd.choice((0, 0, 1, 3, 4, rnd.getrandbits(8)))]
        cases.append([int(root), s8, s9, b43, b45, b49, b53, b55, b56] + p)
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%x' % v for v in c) for c in cases)
    body = ('static const uint32_t C[][15]={' + nl + rows + nl + '};' + nl +
            ('store(0x82831D34,0x%08x);store(0x82831D38,load(0x82831D38)|1);' % joaat('DEFAULT_ROLLOFF')) +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            'objects[5]=c[0]?255:3;objects[8]=c[1];objects[9]=c[2];' +
            'store16(0x1200+43,c[3]);store(0x1200+45,c[4]);store(0x1200+49,c[5]);store16(0x1200+53,c[6]);' +
            'objects[0x200+55]=c[7];objects[0x200+56]=c[8];' +
            'store(0x1100,c[9]);store(0x1104,c[10]);store(0x1108,c[11]);store(0x110C,c[12]);objects[0x100+20]=c[13];objects[0x100+21]=c[14];' +
            ('Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;ctx.r5.u32=0x1200;sub_%s(ctx,nullptr);' % f) +
            'printf("%08x %08x %08x %08x %02x %02x' + NL + '",load(0x1100),load(0x1104),load(0x1108),load(0x110C),objects[0x100+20],objects[0x100+21]);}' + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + f], main_src, a.out_dir, 'inherit', stubs=('sub_' + addr(a.tool, 'F-58929'),))
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s -> %s' % (' '.join('%x' % v for v in c), o) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'inherit_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
