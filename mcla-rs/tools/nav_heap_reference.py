"""The nav-mesh search's open list (a 1-based binary min-heap of 12-byte entries: key f32, node, record pointer; the record's
+0 points back at its entry) run offline, generated code whole and unchanged:

- F-37875 push: count + 1, the entry written at the new end (record pointer the heap's own +12 when none is given), back
  pointer, then the sift-up.
- F-24680 the sift-up: moves parents down while parent > item (`ble` branches on unordered too: a NaN stays), the item into the hole, back pointers.
- F-53789 the sift-down: picks the left child when left < cur, then the right when right < chosen (`bge` branches on
  unordered: a NaN child is not taken), moves it up.
- F-82094 the pop: empty -> 0; out key / node from entry 1, entry 1 and the last swapped, count - 1, the sift-down from 1.
- the decrease (as the record update F-27773 does it): a new key written in place, then the sift-up from that index.

As the search uses them: a node is pushed only when its record is not in the heap (record +0 == 0; else the update F-27773
lowers the key in place), and after a pop the record's +0 is cleared (the pop wrapper F-51218 does that); a push of a node
in the heap is printed as "skip".

Sequences of random ops on keys drawn with ties (a small set), -0 / +0, NaN and random floats. After every op the heap
(count, every entry's key bits and node) and every record's back pointer as an index are printed.

Writes <ref>: lines "op <kind> <a> <b> -> <ret> | <count> <key node>... | <node:index>..."."""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

H, DATA, REC = 0x40100000, 0x40110000, 0x40120000
NODES = 40


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def key(rnd):
    r = rnd.random()
    if r < 0.4:
        return fb(rnd.choice((1.0, 2.0, 2.5, 3.0, 7.0)))
    if r < 0.45:
        return rnd.choice((0x00000000, 0x80000000))
    if r < 0.48:
        return 0x7FC00000
    return fb(rnd.uniform(0, 5000))


def sequence(rnd):
    ops, live = [], set()
    for _ in range(rnd.randint(5, 60)):
        r = rnd.random()
        free = [n for n in range(NODES) if n not in live]
        if (r < 0.5 or not live) and free:
            n = rnd.choice(free)
            live.add(n)
            ops.append((0, n, key(rnd)))
        elif r < 0.75:
            ops.append((1, 0, 0))
            live = set()                 # (the harness re-reads the live set from the heap: only the count is tracked here)
        else:
            ops.append((2, rnd.randint(1, 64), key(rnd)))
    return ops


MAIN = r'''
static uint32_t cnt(){return load(H+4);}
static void dump(){uint32_t n=cnt();printf(" | %u",n);for(uint32_t i=1;i<=n;i++)printf(" %08x %u",load(DATA+12*i),load(DATA+12*i+4));
 printf(" |");for(uint32_t i=1;i<=n;i++){uint32_t nd=load(DATA+12*i+4);uint32_t r=load(DATA+12*i+8);printf(" %u:%d",nd,(int)(load(r)-DATA)/12);}printf("\n");}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});
int si=0;for(auto&sq:SEQS){rt_reset();for(uint32_t a=H;a<REC+0x2000;a+=4)store(a,0);store(H+8,DATA);printf("seq %d\n",si++);
 for(int k=0;k<sq.n;k++){uint32_t t=sq.op[k][0],a=sq.op[k][1],b=sq.op[k][2];
  if(t==0){if(load(REC+32*a)){printf("op 0 %u %08x -> skip",a,b);}else{
   Context c=fresh();c.r3.u32=H;c.r5.u32=a;c.r6.u32=REC+32*a;uint32_t kb=b;float f;memcpy(&f,&kb,4);c.f1.f64=f;PUSH(c,nullptr);printf("op 0 %u %08x -> 0",a,b);}}
  else if(t==1){Context c=fresh();c.r3.u32=H;c.r4.u32=H+0x100;c.r5.u32=H+0x104;store(H+0x100,0);store(H+0x104,0xFFFFFFFFu);POP(c,nullptr);
   if(c.r3.u32&0xFF)store(REC+32*load(H+0x104),0);
   printf("op 1 0 0 -> %u %08x %u",c.r3.u32&0xFF,load(H+0x100),load(H+0x104));}
  else{uint32_t n=cnt();if(n==0){printf("op 2 0 0 -> -");}else{uint32_t i=1+(a-1)%n;store(DATA+12*i,b);Context c=fresh();c.r3.u32=H;c.r4.u32=i;UP(c,nullptr);
   printf("op 2 %u %08x -> %u",i,b,c.r3.u32);}}
  dump();}}}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--seqs', type=int, default=3000)
    a = ap.parse_args()
    r = subprocess.run([a.tool, 'registry', 'addr', *FIDS], capture_output=True, text=True, check=True).stdout
    addr = [l.split()[1].upper().replace('0X', '') for l in r.splitlines() if l.startswith('F-')]
    assert len(addr) == 4, r
    rnd = random.Random(82771)
    seqs = [sequence(rnd) for _ in range(a.seqs)]
    nl = chr(10)
    body = ','.join('{%d,{%s}}' % (len(s), ','.join('{%du,%du,%du}' % o for o in s)) for s in seqs)
    src = ('struct Seq{int n;uint32_t op[64][3];};static const Seq SEQS[]={' + body + '};' + nl +
           'static const uint32_t H=%du,DATA=%du,REC=%du;' % (H, DATA, REC) + nl + MAIN)
    for k, v in zip(('PUSH', 'POP', 'UP'), addr[:3]):
        src = src.replace(k + '(c', 'sub_%s(c' % v)
    exe = rd.build(rd.index(a.recomp), ['sub_' + v for v in addr], src, a.out, 'nav_heap', image=rd.load_image(a.image), limit=20)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True, timeout=600)
    if r.returncode:
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    a.ref.write_text(r.stdout)
    print(r.stdout.count('op '), 'ops in', len(seqs), 'sequences ->', a.ref)


# push, pop, sift-up, sift-down
FIDS = ['F-37875', 'F-82094', 'F-24680', 'F-53789']

if __name__ == '__main__':
    main()
