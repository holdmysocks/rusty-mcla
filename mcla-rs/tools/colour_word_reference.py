"""Run the colour packer F-44647 (a Vec4 clamped 0 .. 1, x 255, truncated, into the word A R G B with A = w) offline (static
recompilation, native, flat guest memory) on 6,000 colours, for the port's `hookicons::colour_word` (the RGB the callers
keep, word & 0xFFFFFF) to be compared with (test colour_word_matches_the_original).

Colours: each lane from a fixed list of edge values (0, -0, 1, just under / over 0 and 1, k / 255 and k / 255 +- 1 ulp,
-1, 2, +inf, -inf, NaN) or a seeded random float in [-0.5, 1.5].
Output, one line a case, in decimal (hex bits would read as addresses): c <x bits> <y bits> <z bits> <w bits> <word>
"""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); const uint32_t V=0x40A00000u, O=0x40A00100u; rt_reset(); uint32_t x,y,z,w;
 while(in>>std::hex>>x>>y>>z>>w){ store(V,x); store(V+4,y); store(V+8,z); store(V+12,w); store(O,0xA5A5A5A5u);
  Context c=fresh(); c.r3.u32=O; c.r4.u32=V; PACK(c,nullptr);
  printf("c %u %u %u %u %u\n",x,y,z,w,load(O)); }
 printf("end\n");
}
'''


def bits(f):
    return struct.unpack('<I', struct.pack('<f', f))[0]


def cases(n=6000, seed=44647):
    rnd = random.Random(seed)
    edge = [0x00000000, 0x80000000, 0x3f800000, 0x3f7fffff, 0x3f800001, 0x00000001, 0x80000001, 0xbf800000, 0x40000000,
            0x7f800000, 0xff800000, 0x7fc00000, 0xffc00000]
    for k in range(256):
        b = bits(k / 255.0)
        edge += [b, b + 1, max(b - 1, 0)]
    out = []
    for _ in range(n):
        lane = lambda: rnd.choice(edge) if rnd.random() < 0.5 else bits(rnd.uniform(-0.5, 1.5))
        out.append((lane(), lane(), lane(), lane()))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, help='write the output here')
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    r = subprocess.run([a.tool, 'registry', 'addr', 'F-44647'], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = 'sub_%08X' % addr['F-44647']
    img = rdisp.load_image(a.image)
    exe = rdisp.build(rdisp.index(a.recomp), [name], MAIN.replace('PACK', name), a.out, 'colour_word', image=img, limit=5)
    a.out.mkdir(parents=True, exist_ok=True)
    inp = a.out / 'colours.txt'
    inp.write_text(''.join('%08x %08x %08x %08x\n' % c for c in cases()))
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    if a.ref:
        a.ref.write_text(r.stdout)
    print(r.stdout[:400], end='')


if __name__ == '__main__':
    main()
