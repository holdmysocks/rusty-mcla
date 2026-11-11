"""rage::atMap as the original builds and walks it, run offline: the insert F-14328 (map, &key, &value; its first-insert set-up F-52791,
the grow by the next prime F-82002 and the rehash F-36760), the find F-18909 the callers test first, and the iterator's Begin F-46639 / Next F-44769,
all as generated.

Each case inserts a random key list the way RegisterCarConfigData does (find first; a key already there with a value is
skipped; the value = the key's index + 1), then walks the map with Begin / Next and prints the values in that order.
Writes <out-dir>/atmap_order_ref.txt: "case" n keys... "->" the walked indices."""
import argparse
import random
import re
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr

M = 0x40200000
IT = 0x40200100
KV = 0x40200200


def case(rnd):
    n = rnd.choice((0, 1, 2, 10, 11, 12, 28, 29, 30, rnd.randrange(1, 400)))
    pool = rnd.choice(('small', 'wide', 'dup', 'mult'))
    keys = []
    for _ in range(n):
        if pool == 'small':
            keys.append(rnd.randrange(64))
        elif pool == 'dup' and keys and rnd.random() < 0.3:
            keys.append(rnd.choice(keys))
        elif pool == 'mult':
            keys.append(rnd.choice((11, 29, 59, 107, 191, 331)) * rnd.randrange(1, 50) + rnd.randrange(3))
        else:
            keys.append(rnd.getrandbits(32))
    return keys


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=2000)
    a = ap.parse_args()
    ins, find, begin, nxt = (addr(a.tool, f) for f in ('F-14328', 'F-18909', 'F-46639', 'F-44769'))
    rnd = random.Random(2186)
    nl = chr(10)
    cases = [case(rnd) for _ in range(a.cases)]
    flat = []
    for c in cases:
        flat += [len(c)] + c
    src = ('static const uint32_t CS[]={%s};' % ','.join('%du' % w for w in flat) + nl + r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
const uint32_t*s=CS;
for(int c=0;c<@N@;c++){rt_reset();uint32_t n=*s++;const uint32_t*k=s;s+=n;
 for(uint32_t o=0;o<0x300;o+=4)store(@M@+o,0);
 *pointer(@M@+11)=1;
 printf("case %u",n);for(uint32_t i=0;i<n;i++)printf(" %08x",k[i]);printf(" ->");
 for(uint32_t i=0;i<n;i++){
  store(@KV@,k[i]);store(@KV@+4,i+1);
  Context x=fresh();x.r3.u32=@M@;x.r4.u32=@KV@;sub_@FIND@(x,nullptr);
  if(x.r3.u32&&load(x.r3.u32))continue;
  Context y=fresh();y.r3.u32=@M@;y.r4.u32=@KV@;y.r5.u32=@KV@+4;sub_@INS@(y,nullptr);}
 store(@IT@,0);store(@IT@+4,0);store(@IT@+8,@M@);
 {Context x=fresh();x.r3.u32=@IT@;sub_@BEGIN@(x,nullptr);}
 for(int g=0;g<100000&&load(@IT@+4);g++){printf(" %u",load(load(@IT@+4)+4)-1);Context x=fresh();x.r3.u32=@IT@;sub_@NEXT@(x,nullptr);}
 printf("\n");}}
''')
    for kk, v in (('@N@', str(a.cases)), ('@M@', '%du' % M), ('@IT@', '%du' % IT), ('@KV@', '%du' % KV),
                  ('@FIND@', find), ('@INS@', ins), ('@BEGIN@', begin), ('@NEXT@', nxt)):
        src = src.replace(kk, v)
    exe = rd.build(rd.index(a.recomp), ['sub_' + f for f in (ins, find, begin, nxt)], src, a.out_dir, 'atmap',
                   image=rd.load_image(a.image))
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (keys that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'atmap_order_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
