"""The sound object's Positional tristate getter F-55012 run offline: the generated code whole and unchanged.
It reads tristate pair 1 of the object's +112 (the header's flag word at +5, copied by the Init F-89731 from the parsed header)
and, while that is 2 (inherit), walks to the parent: the parent's pool index is +5 (255 = none), the pool the bucket
+64 of the object it is on (base = the pool table +232 + 11,488 x bucket, word +11,440; stride = the pool's +196).
Inputs: 3,000 random chains of 1 - 9 objects (random flag words, pair 1 = 2 half of the time, the others 0 / 1 / 3),
random distinct pool indices, a random bucket 0 - 3 for each object.
Writes <out-dir>/positional_walk_ref.txt: 'P <words, root first> -> <r3>' (hex), r3 the getter's value from the
leaf (the last word)."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_dispatch as rd
from recomp_closure import index, run

POOLS = 0x82831C30   # the sound pool globals (+196 object size, +232 the bucket table)
TABLE = 0x40200000
BASE = 0x40300000    # bucket b's objects at BASE + b x 0x100000
SIZE = 0x100
STACK = 0x40F00000


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = addr(a.tool, 'F-55012')
    r = random.Random(55012)
    chains = []
    for _ in range(3000):
        n = r.randrange(1, 10)
        idx = r.sample(range(0, 255), n)
        objs = []
        for i in range(n):
            w = r.getrandbits(32) & ~0xC
            while w >> 24 == 0x82:   # (no word that reads as an address in the fixture)
                w = r.getrandbits(32) & ~0xC
            w |= (2 if r.random() < 0.5 else r.choice((0, 1, 3))) << 2
            objs.append((w, idx[i], r.randrange(4)))
        chains.append(objs)
    # flat: n, then (word, index, bucket) root first
    flat = []
    for ch in chains:
        flat.append(len(ch))
        for o in ch:
            flat.extend(o)
    data = ','.join('%du' % x for x in flat)
    c = r"""
static const uint32_t C[]={@DATA@};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
store(@POOLS@u+196,@SIZE@u);store(@POOLS@u+232,@TABLE@u);
for(uint32_t b=0;b<4;b++)store(@TABLE@u+b*11488u+11440u,@BASE@u+b*0x100000u);
size_t i=0;while(i<sizeof(C)/4){uint32_t n=C[i++];const uint32_t*o=&C[i];i+=3*n;
 uint32_t leaf=0;
 // object k lives in the pool its child's +64 names (the walk looks the parent up through the child's bucket), the leaf in its own
 uint32_t addr[16];
 for(uint32_t k=0;k<n;k++){uint32_t bucket=(k+1<n)?o[3*(k+1)+2]:o[3*k+2];
  addr[k]=@BASE@u+bucket*0x100000u+o[3*k+1]*@SIZE@u;}
 for(uint32_t k=0;k<n;k++){uint32_t p=addr[k];for(uint32_t j=0;j<@SIZE@u;j+=4)store(p+j,0xA5A5A5A5u);
  store(p+112,o[3*k]);pointer(p+5)[0]=(uint8_t)(k==0?255:o[3*(k-1)+1]);pointer(p+64)[0]=(uint8_t)o[3*k+2];}
 leaf=addr[n-1];
 Context c{};c.r1.u32=@STACK@u;c.r3.u32=leaf;sub_@FN@(c,nullptr);
 printf("P");for(uint32_t k=0;k<n;k++)printf(" %08x",o[3*k]);printf(" -> %x\n",c.r3.u32);}}
"""
    for k, v in (('DATA', data), ('POOLS', POOLS), ('TABLE', TABLE), ('BASE', BASE), ('SIZE', SIZE), ('STACK', STACK),
                 ('FN', fn)):
        c = c.replace('@%s@' % k, str(v))
    img = rd.load_image(a.image)
    exe = rd.build(index(a.recomp), ['sub_' + fn], c, a.out_dir, 'positional_walk', image=img)
    out = [x for x in run(exe, a.image).split(chr(10)) if x]
    assert len(out) == len(chains), (len(out), len(chains))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'positional_walk_ref.txt').write_text(chr(10).join(out) + chr(10))
    print(len(chains), 'chains')


if __name__ == '__main__':
    main()
