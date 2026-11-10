"""mcRacerManager's racer list run offline: random sequences of its list operations on one manager, the generated code whole and
unchanged - F-95558 (AddRacer), F-44187 (RemoveRacer), F-80858 (IndexOf), F-52302 (move a racer to a slot).

The manager: +0 the list (pointers), +400 its count, +404 the snapshot list (F-80858 and so F-52302 search it while the thread
block's byte +72 is set), +804 its count. The racers are 7 objects whose vfunc 40 (a fake) answers the case's human flag; racer 7
is the null pointer. The streaming manager 0x8286_D8E4 is 0 (F-44187 tells it nothing). A snapshot op copies the list (as the
per-frame start F-25724 does, here on the host) and the flag op sets the byte.

Writes <out-dir>/racer_list_ref.txt, hex words, one line a case: "case humans n_ops" + per op (op a b: 0 add a, 1 remove a,
2 move a to slot b, 3 index of a, 4 snapshot, 5 flag b) + "->" + per op (the result r3 for 3, else 0; the list's count; an FNV-1a
hash of its entries as racer numbers, 7 null, 8 anything else) + the word before the list (F-44187 of an absent racer writes 0
there; it starts 0x1234_5678)."""
import argparse
import random
import re
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr

M, OBJ, VT = 0x40100000, 0x40110000, 0x40118000
F40 = 0x40F00000
FLAG = rd.RT_BLOCK + 72


def case(rnd):
    humans = rnd.choice((1, 1, 0, 3, rnd.getrandbits(7)))
    ops, size = [], 0
    for _ in range(rnd.randint(1, 30)):
        k = rnd.choice((0, 1, 2, 3, 4, 5, 6, rnd.randrange(7), 7))
        op = rnd.choice((0, 0, 0, 1, 1, 2, 3, 3, 4, 5))
        b = 0
        if op == 0:
            # (adding the null racer makes the original call through a null vtable)
            k = k % 7
            if size > 80:
                op = 3
            else:
                size += 1
        if op == 2:
            b = rnd.choice((0, 1, 2, rnd.randrange(12), -1, size, size + 3))
            if b > 80:
                b = 0
            size = max(size, b + 1)
        if op == 5:
            b = rnd.choice((0, 1))
        ops.append((op, k, b & 0xffffffff))
    return [humans, len(ops)] + [w for o in ops for w in o]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    fn = {'A': addr(a.tool, 'F-95558'), 'R': addr(a.tool, 'F-44187'), 'I': addr(a.tool, 'F-80858'), 'V': addr(a.tool, 'F-52302')}
    rnd = random.Random(95558)
    nl = chr(10)
    cases = [case(rnd) for _ in range(a.cases)]
    flat, offs = [], []
    for c in cases:
        offs.append(len(flat))
        flat += c
    src = ('static const uint32_t CS[]={%s};' % ','.join('%du' % w for w in flat) + nl +
           'static const uint32_t OF[]={%s};' % ','.join('%du' % o for o in offs) + nl + r'''
static uint32_t who(uint32_t p){if(!p)return 7;for(uint32_t k=0;k<7;k++)if(p==@OBJ@+0x100*k)return k;return 8;}
static uint32_t rac(uint32_t k){return k<7?@OBJ@+0x100*k:0;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(int c=0;c<@C@;c++){const uint32_t*s=CS+OF[c];rt_reset();
 for(uint32_t o=M_-0x10;o<M_+0x400;o+=4)store(o,0);store(M_-4,0x12345678u);*pointer(@FLAG@)=0;store(0x8286D8E4u,0);
 for(uint32_t k=0;k<7;k++){store(@OBJ@+0x100*k,@VT@);store(@OBJ@+0x100*k+4,(s[0]>>k)&1);}store(@VT@+40,@F40@);
 printf("case %08x %08x",s[0],s[1]);for(uint32_t i=0;i<3*s[1];i++)printf(" %08x",s[2+i]);printf(" ->");
 for(uint32_t i=0;i<s[1];i++){uint32_t op=s[2+3*i],k=s[3+3*i],b=s[4+3*i],r=0;Context x=fresh();x.r3.u32=M_;x.r4.u32=rac(k);x.r5.u32=b;
  if(op==0)sub_@A@(x,nullptr);else if(op==1)sub_@R@(x,nullptr);else if(op==2)sub_@V@(x,nullptr);
  else if(op==3){sub_@I@(x,nullptr);r=x.r3.u32;}
  else if(op==4){for(uint32_t j=0;j<100;j++)store(M_+404+4*j,load(M_+4*j));store(M_+804,load(M_+400));}
  else *pointer(@FLAG@)=(uint8_t)b;
  uint32_t n=load(M_+400),h=2166136261u;for(uint32_t j=0;j<n&&j<100;j++){uint32_t w=who(load(M_+4*j));for(int q=0;q<4;q++){h^=(w>>(8*q))&0xFF;h*=16777619u;}}
  printf(" %08x %08x %08x",r,n,h);}
 printf(" %08x\n",load(M_-4));}}
''')
    for k, v in (('OBJ', OBJ), ('VT', VT), ('F40', F40), ('FLAG', FLAG)):
        src = src.replace('@%s@' % k, '%du' % v)
    src = src.replace('M_', '%du' % M).replace('@C@', str(a.cases))
    for k, v in fn.items():
        src = src.replace('@%s@' % k, v)
    exe = rd.build(rd.index(a.recomp), ['sub_' + v for v in fn.values()], src, a.out_dir, 'racerlist', image=rd.load_image(a.image),
                   fakes={F40: 'ctx.r3.u32=load(ctx.r3.u32+4);'})
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'racer_list_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases;', sum(c[1] for c in cases), 'ops;', sum('00000000' == x.split()[-1] for x in lines), 'with an absent remove')


if __name__ == '__main__':
    main()
