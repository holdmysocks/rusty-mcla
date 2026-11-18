"""The engine pool's sort compare F-34621 run offline (the generated code whole and unchanged): the qsort F-79639 of the
15 opponent-engine entries (8 bytes: entity, the float distance squared at +4) calls it with two entry pointers; it
returns 1 when a's float is greater, -1 when it is less, else 0 (equal, +0 / -0, and a NaN on either side).
Inputs: every pair of 20 special floats (zeros, denormals, 1, FLT_MAX, the empty slot's constant read from the image,
infinities, NaNs) and 30,000 random pairs, a third of them equal (no word with top byte 0x82).
Writes <out-dir>/pool_compare_ref.txt: 'C <a bits> <b bits> -> <r3 as signed>' (hex bits)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
from recomp_closure import index, run

A = 0x40100000
B = 0x40100010
STACK = 0x40F00000
EMPTY = 0x8200856C   # the empty slot's distance constant (lfs -31380 off lis -32255 in F-25575)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def ok(w):
    return w >> 24 != 0x82


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = addr(a.tool, 'F-34621')
    sp = [0x00000000, 0x80000000, 0x00000001, 0x80000001, 0x007fffff, 0x00800000, 0x3f800000, 0xbf800000, 0x3f800001,
          0x7f7fffff, 0xff7fffff, 0x7f800000, 0xff800000, 0x7fc00000, 0xffc00000, 0x7f800001, 0x41200000, 0x49742400,
          0x447a0000, 0xFFFFFFFF]   # (the last is replaced by the image's constant)
    r = random.Random(34621)
    pairs = [(x, y) for x in sp for y in sp]
    while len(pairs) < len(sp) ** 2 + 30000:
        x = r.getrandbits(32)
        if r.random() < 0.6:
            x = struct.unpack('<I', struct.pack('<f', r.uniform(0, 1e5) ** (1 if r.random() < 0.5 else 2)))[0]
        y = x if r.random() < 1 / 3 else (x + r.choice((1, -1))) & 0xffffffff if r.random() < 0.3 else r.getrandbits(32)
        if ok(x) and ok(y):
            pairs.append((x, y))
    data = ','.join('%du,%du' % p for p in pairs)
    c = r"""
static const uint32_t P[]={@DATA@};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
uint32_t empty=load(@EMPTY@u);printf("E %08x\n",empty);
for(size_t i=0;i<sizeof(P)/4;i+=2){uint32_t x=P[i]==0xFFFFFFFFu?empty:P[i],y=P[i+1]==0xFFFFFFFFu?empty:P[i+1];
 store(@A@u,0x11111111u);store(@A@u+4,x);store(@B@u,0x22222222u);store(@B@u+4,y);
 Context c{};c.r1.u32=@STACK@u;c.r3.u32=@A@u;c.r4.u32=@B@u;sub_@FN@(c,nullptr);
 printf("C %08x %08x -> %d\n",x,y,(int32_t)c.r3.u32);}}
"""
    for k, v in (('DATA', data), ('A', A), ('B', B), ('STACK', STACK), ('FN', fn), ('EMPTY', EMPTY)):
        c = c.replace('@%s@' % k, str(v))
    img = rd.load_image(a.image)
    exe = rd.build(index(a.recomp), ['sub_' + fn], c, a.out_dir, 'pool_compare', image=img)
    out = [x for x in run(exe, a.image).split(chr(10)) if x]
    assert len(out) == len(pairs) + 1, (len(out), len(pairs))
    print(out[0], '(the empty slot constant)')
    lines = out[1:]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'pool_compare_ref.txt').write_text(chr(10).join(lines) + chr(10))
    print(len(lines), 'pairs;', {v: sum(1 for x in lines if x.endswith(' ' + v)) for v in ('1', '-1', '0')})


if __name__ == '__main__':
    main()
