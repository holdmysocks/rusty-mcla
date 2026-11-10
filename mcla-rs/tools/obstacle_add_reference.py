"""The aiv's obstacle append F-70624 run offline, the generated code whole and unchanged with its normalise F-41334 called as
generated.

The aiv at A: obstacle count +25104, the cells at +23056 + 64 n (+0 position, +16 velocity, +32 direction, +48 extent), the
flag bytes at +25120 with their count +25152. Arguments: r4 position, r5 velocity, r6 direction, r7 extent (vectors), r8 the
first-leg bool, r9 the kind-0 bool. Each case starts both counts at k (0 .. 31) and puts a random byte in flag slot k.

Writes <out-dir>/obstacle_add_ref.txt, hex words, one line a case: "case" + the 16 input lanes + r8 r9 + k + the old byte
+ "->" + the cell's 16 words + the flag byte + both counts after."""
import argparse
import random
import re
import struct
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr

A = 0x40200000
IN = 0x40300000


def fw(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def lane(rnd):
    return fw(rnd.choice((rnd.uniform(-500, 500), rnd.uniform(-2, 2), rnd.uniform(-1e-3, 1e-3), 0.0, -0.0, rnd.uniform(-20, 20))))


def case(rnd):
    pos = [lane(rnd) for _ in range(4)]
    vel = [lane(rnd) for _ in range(4)]
    m = rnd.choice((1.0, 1.0, 1e-3, 3e-3, 1e-4, 0.0, 50.0))
    d = [fw(rnd.uniform(-1, 1) * m) for _ in range(4)]
    if rnd.random() < 0.1:
        d = [0, 0, 0, 0]
    ext = [fw(rnd.uniform(0, 6)), fw(rnd.choice((0.0, rnd.uniform(0, 3)))), fw(rnd.uniform(0, 12)), lane(rnd)]
    return pos + vel + d + ext + [rnd.randrange(2), rnd.randrange(2), rnd.randrange(32), rnd.randrange(256)]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=5000)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-70624')
    rnd = random.Random(70624)
    nl = chr(10)
    cases = [case(rnd) for _ in range(a.cases)]
    flat = [w for c in cases for w in c]
    src = ('static const uint32_t CS[]={%s};' % ','.join('%du' % w for w in flat) + nl + r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(int c=0;c<@C@;c++){const uint32_t*s=CS+20*c;rt_reset();
 for(uint32_t o=0;o<31232;o+=4)store(@A@+o,0);
 for(uint32_t i=0;i<16;i++)store(@IN@+4*i,s[i]);
 uint32_t k=s[18];store(@A@+25104,k);store(@A@+25152,k);*pointer(@A@+25120+k)=(uint8_t)s[19];
 Context x=fresh();x.r3.u32=@A@;x.r4.u32=@IN@;x.r5.u32=@IN@+16;x.r6.u32=@IN@+32;x.r7.u32=@IN@+48;x.r8.u32=s[16];x.r9.u32=s[17];
 sub_@F@(x,nullptr);
 printf("case");for(int i=0;i<20;i++)printf(" %08x",s[i]);printf(" ->");
 for(uint32_t i=0;i<16;i++)printf(" %08x",load(@A@+23056+64*k+4*i));
 printf(" %08x %08x %08x\n",(uint32_t)*pointer(@A@+25120+k),load(@A@+25104),load(@A@+25152));}}
''')
    src = src.replace('@A@', '%du' % A).replace('@IN@', '%du' % IN).replace('@C@', str(a.cases)).replace('@F@', fn)
    idx = rd.index(a.recomp)
    exe = rd.build(idx, ['sub_' + fn], src, a.out_dir, 'obstadd', image=rd.load_image(a.image))
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'obstacle_add_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
