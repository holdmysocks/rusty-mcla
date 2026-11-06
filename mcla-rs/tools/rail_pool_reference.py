"""The ambient rail pools (F-76414: index 0 = 256 dynamic curve rails, index 1 = 256 arc-line-arc rails, each a free
list at pool +196 + 8 i - head, tail - and a used list at +180 + 8 i; a node: +0 the rail, +4 next, +8 prev) and the
reverse rail's refusal, run offline: the generated code whole and unchanged on flat guest memory.

Two kinds of case (first word):
  0  F-88792 (r3 the pool, r4 the index) called `n + 1` times on a free list of `n` nodes (n 0 .. 300; the rail's
     slot 23, the reset, answered with nothing). Out: how many takes returned a rail before the first null, and
     whether every later take is null.
  1  F-56052 (r3 the behaviour) with the backing rail's builder F-54867 answering 0 (its pool take null) or 1. Out:
     its answer and the calls it made (the holder's destructor 'd', the rail unload 'u', the rail set 's').

Writes <out-dir>/rail_pool_ref.txt, a case a line: the case's words ' -> ' the answer (test
`rail_pool_matches_the_original` in `traffic.rs`)."""
import argparse
import random
import subprocess
from pathlib import Path
import traffic_select_reference as tsr  # noqa: F401 (vector shim, indirect-call dispatch)
from traffic_select_reference import addr, NL
import recomp_closure
from recomp_closure import index, build


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    take, rev, build_back = addr(a.tool, 'F-88792'), addr(a.tool, 'F-56052'), addr(a.tool, 'F-54867')
    rnd = random.Random(76414)
    cs = [[0, n, rnd.randrange(2)] for n in list(range(0, 9)) + [255, 256, 257] + [rnd.randrange(0, 300) for _ in range(40)]]
    cs += [[1, b, 0] for b in (0, 1) for _ in range(20)]
    # (the reverse rail's other callees: the holder's destructor, the unload, the rail set)
    from recomp_leaf import extract
    import re
    idx = index(a.recomp)
    s = extract(idx['sub_' + rev], 'sub_' + rev)
    cl = []
    for m in re.finditer(r'\b(sub_[0-9A-F]{8})\(ctx, base\)', s):
        if m.group(1) not in cl:
            cl.append(m.group(1))
    assert cl[0] == 'sub_' + build_back and len(cl) == 4, cl
    _, unload, setr, dtor = cl
    recomp_closure.EXTRA += 'static uint32_t ANS;static std::string LOG;' + NL
    stubs = {'sub_' + build_back: 'ctx.r3.u64=ANS;', unload: 'LOG+="u";', setr: 'LOG+="s";', dtor: 'LOG+="d";'}
    setup = r'''
static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){if(a==0x9001){return;}if(a==0x9002){LOG+="r";return;}
printf("bad call %08x\n",a);throw std::runtime_error("call");}
'''
    body = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
std::ifstream in(argv[2]);std::string line;
while(std::getline(in,line)){std::vector<uint32_t> w;{const char*s=line.c_str();char*e;while(*s){uint32_t v=strtoul(s,&e,16);if(e==s)break;w.push_back(v);s=e;}}
memset(objects,0,sizeof(objects));LOG.clear();
if(w[0]==0){uint32_t P=0x10000,n=w[1],i=w[2],N=0x20000,R=0x40000;store(0x2000+92,0x9001);
 uint32_t prev=0;for(uint32_t k=0;k<n;k++){uint32_t nd=N+16*k,rl=R+16*k;store(rl,0x2000);store(nd,rl);store(nd+8,prev);if(prev)store(prev+4,nd);else store(P+196+8*i,nd);prev=nd;}
 if(n)store(P+200+8*i,prev);
 int got=0,late=0;for(uint32_t k=0;k<=n+2;k++){Context ctx{};ctx.r1.u32=0x7F000;ctx.r3.u32=P;ctx.r4.u32=i;%s(ctx,nullptr);
  if(ctx.r3.u32){if(late)late=2;else got++;}else if(!late)late=1;}
 printf("%%d %%d\n",got,late==1);}
else{uint32_t Bh=0x10000;ANS=w[1];for(int k=0;k<4;k++)store(Bh+(k==0?92:k==1?4:k==2?36:124),0x2100);store(0x2100+4,0x9002);
 Context ctx{};ctx.r1.u32=0x7F000;ctx.r3.u32=Bh;%s(ctx,nullptr);printf("%%u %%s\n",ctx.r3.u32&0xFF,LOG.empty()?"-":LOG.c_str());}
}}
''' % ('sub_' + take, 'sub_' + rev)
    exe = build(idx, ['sub_' + take, 'sub_' + rev], setup + body, a.out_dir, 'rail_pool', stubs=stubs, vector=True)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'rail_pool_in.txt'
    inp.write_text(NL.join(' '.join('%x' % x for x in c) for c in cs) + NL)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True, check=True)
    out = [x for x in r.stdout.split(NL) if x]
    assert len(out) == len(cs), r.stdout[-300:]
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cs, out)]
    (a.out_dir / 'rail_pool_ref.txt').write_text(NL.join(lines) + NL)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
