"""Run the original's mcRaceArray_Fill (F-50395) offline and record it, for the port's `mc4::race_passes` and
`mc4::fill_fallback` (crates/game/src/testdata/fill_ref.txt; test mc4::tests::race_fill_matches_the_original).

F-50395 (r3 the array, r4 the district, r5 the type mask, r6 the sub type mask): the array emptied (F-80885), then every record of
the race table (F-24591's object: count +32, names +36, records +44 stride 160 - +0 type, +4 sub type, +8 district, +96 the
test-only byte) in order: a test-only one only when F-20186 (the dev switch) says so; its district the one asked or 6 asked; the
bit of its type in the mask or the mask -1 (a shift of 32 or more: no bit); the same for the sub type; unlocked (F-64698 with the
career and its name) - its index pushed (F-97496). None: 'No races were found' (F-47121) and the index (lo & 0x7FFFFFFF) mod the
table's count (signed) of one draw of the career's generator (career +0x16C78, a = 0x5CDCFAA7) pushed. Stubbed: the table getter,
the dev switch, the unlock test (from the case), the array's erase / push and the log.

  <district> m<types> m<subs> <dev> <records: district:type:sub:test:unlocked ...> g<lo> g<carry> -> <indices> g<lo> g<carry>
"""
import argparse
import random
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

IMG = 0x82 << 24
CAREERG = IMG + 0x86D8DC        # the career manager (-10020 off 0x82870000)

MAIN = r'''
static void sb(uint32_t a,uint8_t b){uint32_t w=load(a&~3u);int sh=(3-(a&3))*8;w=(w&~(0xffu<<sh))|((uint32_t)b<<sh);store(a&~3u,w);}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t A=0x40800000u, T=0x40810000u, C=0x40900000u, G0=0x40A00000u;
 while(in>>tok){
  if(tok!="case") break; uint32_t d,ty,su,dev,n,lo,ca; in>>d>>ty>>su>>dev>>n;
  rt_reset(); for(uint32_t a=A;a<A+0x400;a+=4) store(a,0); for(uint32_t a=G0;a<G0+0x200;a+=4) store(a,0);
  store(A,A+0x100); store(G0,dev); store(T+32,n); store(T+36,T+0x100); store(T+44,0x40820000u);
  for(uint32_t k=0;k<n;k++){ uint32_t rd,rt,rs,te,un; in>>rd>>rt>>rs>>te>>un; uint32_t r=0x40820000u+k*160u;
   for(uint32_t a=r;a<r+160;a+=4) store(a,0); store(r,rt); store(r+4,rs); store(r+8,rd); sb(r+96,te);
   store(T+0x100+4*k,0x40C00000u+k); store(G0+0x100+4*k,un); }
  in>>lo>>ca; store(CAREERG,C); store(C+0x16C78,lo); store(C+0x16C7C,ca);
  Context c=fresh(); c.r3.u32=A; c.r4.u32=d; c.r5.u32=ty; c.r6.u32=su; FILL(c,nullptr);
  uint32_t m=load(A+4)>>16; printf("%u",m); for(uint32_t i=0;i<m;i++) printf(" %u",load(A+0x100+8*i+4));
  printf(" | %u %u\n",load(C+0x16C78),load(C+0x16C7C)); }
 printf("end\n");
}
'''


def cases():
    g = random.Random(50395)
    out = []
    for _ in range(3000):
        d = g.choice((0, 1, 2, 3, 4, 6, 6, 5, -1))
        ty = g.choice((2, 2, 1, 4, 6, -1, 0, 0x7FFFFFFF, 0x80000000, g.getrandbits(32)))
        su = g.choice((6, 6, 2, 4, 1, -1, 0, g.getrandbits(32)))
        dev = g.random() < 0.2
        n = g.choice((1, 1, 2, 3, 5, 8, 20, 40, 200))
        none = g.random() < 0.45                      # (most of these find nothing: the fallback)
        recs = []
        for _ in range(n):
            rd = g.choice((0, 1, 2, 3, 4, 5, 6, -1)) if not none else g.choice((7, 9, -2))
            rt = g.choice((0, 1, 1, 2, 3, 5, 31, 32, 33, -1, 40))
            rs = g.choice((0, 1, 2, 2, 3, 6, 13, 31, 32, -1))
            recs.append((rd, rt, rs, int(g.random() < 0.1), int(g.random() < 0.8)))
        out.append((d & 0xFFFFFFFF, ty & 0xFFFFFFFF, su & 0xFFFFFFFF, int(dev), recs, g.getrandbits(32), g.getrandbits(32) % 0x5CDCFAA7))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-50395', 'F-80885', 'F-24591', 'F-20186', 'F-64698', 'F-97496', 'F-47121']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    assert len(addr) == len(ids), addr
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    stubs = {
        name('F-80885'): 'store(ctx.r3.u32+4u,load(ctx.r3.u32+4u)&0xFFFFu);',
        name('F-24591'): 'ctx.r3.u64=0x40810000u;',
        name('F-20186'): 'ctx.r3.u64=load(0x40A00000u);',
        name('F-64698'): 'ctx.r3.u64=load(0x40A00100u+4u*(ctx.r4.u32-0x40C00000u));',
        name('F-97496'): 'uint32_t n_=load(ctx.r3.u32+4u)>>16; store(ctx.r3.u32+4u,((n_+1u)<<16)|(load(ctx.r3.u32+4u)&0xFFFFu)); ctx.r3.u64=load(ctx.r3.u32)+8u*n_;',
        name('F-47121'): '',
    }
    src = MAIN.replace('FILL', name('F-50395')).replace('CAREERG', '0x%08Xu' % CAREERG)
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-50395')], src, a.out, 'fill', image=img, stubs=stubs)
    cs = cases()
    cf = a.out / 'fill_cases.txt'
    s32 = lambda v: v - (1 << 32) if v >= 0x80000000 else v
    with cf.open('w') as f:
        for d, ty, su, dev, recs, lo, ca in cs:
            f.write('case %d %d %d %d %d %s %d %d\n' % (d, ty, su, dev, len(recs), ' '.join('%d %d %d %d %d' % tuple(x & 0xFFFFFFFF for x in rc) for rc in recs), lo, ca))
        f.write('stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True, timeout=120)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    lines = r.stdout.split('\n')[:-2]
    assert len(lines) == len(cs)
    rows = []
    for (d, ty, su, dev, recs, lo, ca), l in zip(cs, lines):
        got, gen = l.split(' | ')
        lo2, ca2 = gen.split()
        idx = got.split()[1:]
        rows.append('%d m%d m%d %d %s g%08x g%08x -> %s g%08x g%08x' % (s32(d), s32(ty), s32(su), dev, ' '.join(':'.join(str(x) for x in rc) for rc in recs),
                                                                    lo, ca, ' '.join(idx) or '.', int(lo2), int(ca2)))
    a.ref.write_text('\n'.join(rows) + '\n')
    print('cases', len(rows), '->', a.ref)


if __name__ == '__main__':
    main()
