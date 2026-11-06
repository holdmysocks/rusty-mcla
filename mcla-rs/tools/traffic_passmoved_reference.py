"""The pass tracker's test F-73744 (the rider update F-87488 calls it after the switch with the tracker behaviour +480:
+0 the obstacle being passed, +16 its place when the pass began) run offline: the generated code whole on flat guest
memory, the obstacle's slot 1 (its matrix, the place at +48) from the case.

Writes <out-dir>/traffic_passmoved_ref.txt, a case a line: has an obstacle, the stored place xyz, the obstacle's place xyz
(hex bits); ' -> ' 1 when the obstacle is returned (it moved more than 0.333 m: squared 0x3DE319C6), 0 for null (test
`pass_moved_matches_the_original`)."""
import argparse
import math
import random
import subprocess
from pathlib import Path
import traffic_select_reference as tsr
from traffic_select_reference import addr, fb, bf, f32, NL
from recomp_closure import index, build


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-73744')
    rnd = random.Random(73744)
    lim = math.sqrt(bf(0x3DE319C6))
    cs = []
    for _ in range(a.cases):
        at = [rnd.choice((0.0, rnd.uniform(-900, 900))), rnd.uniform(-20, 60), rnd.choice((0.0, rnd.uniform(-900, 900)))]
        d = rnd.choice((lim, lim, rnd.uniform(0, 1), 0.0, rnd.uniform(0.3, 0.37)))
        u = [rnd.gauss(0, 1) for _ in range(3)]
        if rnd.random() < 0.3:
            u[1] = 0.0
        n = math.sqrt(sum(x * x for x in u)) or 1.0
        p = [at[i] + u[i] / n * d for i in range(3)]
        if rnd.random() < 0.02:
            p[rnd.randrange(3)] = float('nan')
        w = [fb(f32(x)) for x in at + p]
        if rnd.random() < 0.5:
            k = rnd.randrange(3, 6)
            w[k] = (w[k] + rnd.choice((-2, -1, 1, 2))) & 0xFFFFFFFF
        cs.append([int(rnd.random() < 0.95)] + w)
    setup = r'''
static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){uint32_t t=ctx.r3.u32;switch(a){
case 0x9001: ctx.r3.u64=t+0x40;return;}
printf("bad call %08x\n",a);throw std::runtime_error("call");}
'''
    body = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
std::ifstream in(argv[2]);std::string line;
while(std::getline(in,line)){std::vector<uint32_t> w;{const char*s=line.c_str();char*e;while(*s){uint32_t v=strtoul(s,&e,16);if(e==s)break;w.push_back(v);s=e;}}
memset(objects,0,sizeof(objects));uint32_t O=0x10000,T=0x1000;store(0x2000+4,0x9001);store(O,0x2000);
store(T,w[0]?O:0);for(int j=0;j<3;j++)store(T+16+4*j,w[1+j]);for(int j=0;j<3;j++)store(O+0x70+4*j,w[4+j]);
Context ctx{};ctx.r1.u32=0x7F000;ctx.r3.u32=T;sub_%s(ctx,nullptr);printf("%%d\n",ctx.r3.u32==O?1:ctx.r3.u32==0?0:2);}
}
''' % fn
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fn], setup + body, a.out_dir, 'traffic_passmoved', vector=True, scratch_end=0x80000)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'traffic_passmoved_in.txt'
    inp.write_text(NL.join(' '.join('%x' % x for x in c) for c in cs) + NL)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True, check=True)
    out = [x for x in r.stdout.split(NL) if x]
    assert len(out) == len(cs) and all(o in ('0', '1') for o in out), (len(out), r.stdout[-200:])
    lines = []
    for c, o in zip(cs, out):
        line = ' '.join('%x' % x for x in c) + ' -> ' + o
        assert not any(t.startswith('82') and len(t) == 8 for t in line.split()), line
        lines.append(line)
    (a.out_dir / 'traffic_passmoved_ref.txt').write_text(NL.join(lines) + NL)
    print(len(lines), 'cases;', sum(o == '1' for o in out), 'moved')


if __name__ == '__main__':
    main()
