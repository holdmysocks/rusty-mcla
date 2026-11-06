"""The mutual-nearest mark F-37525 (called by the rider's per-frame driver F-33409 through the thunk F-29255 right after
F-10623, r3 = the obstacle result + 8: +0 our car, +4 the mark out; r4 = the nearest obstacle) run offline: the generated
code whole in vector mode (the VMX flush mode as the host's DAZ / FTZ) on flat guest memory. Stubbed with the case's
answers: the cast to the car wrapper (__RTDynamicCast, F-75805: the obstacle itself or null) and F-88657 (our car's
obstacle). The objects' slot 1 (their matrix: rows 2 / 3 at +32 / +48) and the behaviour's slot 0 answer from the case.

Writes <out-dir>/traffic_mark_ref.txt, a case a line: flags (has the nearest, castable, has a car, has a behaviour,
its slot 0 answer, its +432 our obstacle), our behaviour (0 none, 1 no rail, 2 a run-time rail, 3 a lane), the other's
rail (1 none, 2 run-time, 3 lane), then rows 2 and places (other, ours: xyz each); ' -> ' 1 when the mark is the other,
0 when it is null (test `mutual_mark_matches_the_original`)."""
import argparse
import math
import random
import subprocess
from pathlib import Path
import traffic_select_reference as tsr
from traffic_select_reference import addr, fb, bf, f32, NL
import recomp_closure
from recomp_closure import index, build


def gen(rnd):
    flags = [int(rnd.random() < 0.97), int(rnd.random() < 0.95), int(rnd.random() < 0.95), int(rnd.random() < 0.95),
             0 if rnd.random() < 0.95 else rnd.choice((1, 2)), int(rnd.random() < 0.93)]
    ours = rnd.choice((0, 1, 2, 3, 3, 3, 3))
    other = rnd.choice((1, 2, 3, 3, 3))
    base = [rnd.choice((0.0, rnd.uniform(-900, 900))), rnd.uniform(-20, 60), rnd.choice((0.0, rnd.uniform(-900, 900)))]
    y0 = rnd.uniform(-math.pi, math.pi)
    k = rnd.random()
    if k < 0.3:
        y1 = y0 + rnd.choice((0.0, math.pi, rnd.uniform(-0.3, 0.3), math.pi + rnd.uniform(-0.3, 0.3)))
    else:
        y1 = rnd.uniform(-math.pi, math.pi)
    tilt = lambda: rnd.choice((0.0, 0.0, rnd.uniform(-0.05, 0.05)))
    r2o = [math.sin(y1), tilt(), math.cos(y1)]
    r2u = [math.sin(y0), tilt(), math.cos(y0)]
    if rnd.random() < 0.03:
        r2o = [-r2u[0], r2u[1], -r2u[2]]  # opposite: the sum's flat part zero
    d = rnd.choice((rnd.uniform(0, 30), rnd.uniform(0, 3), 0.0))
    a = rnd.uniform(-math.pi, math.pi)
    po = [base[0] + d * math.cos(a), base[1] + rnd.choice((0.0, rnd.uniform(-2, 2))), base[2] + d * math.sin(a)]
    pu = list(base)
    m = rnd.random()
    if m < 0.1:
        po = list(pu)
    elif m < 0.25:
        # side by side along the mean heading's normal: a and b nearly equal
        n = [-(r2o[2] + r2u[2]), 0.0, r2o[0] + r2u[0]]
        po = [pu[0] + n[0] * rnd.uniform(-3, 3) + rnd.choice((0.0, 1e-4, -1e-4)), pu[1], pu[2] + n[2] * rnd.uniform(-3, 3)]
    elif m < 0.27:
        po[0] = float('nan')
    vals = [fb(f32(x)) for x in r2o + po + r2u + pu]
    return flags + [ours, other] + vals


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    fn, cast, obst = (addr(a.tool, f) for f in ('F-37525', 'F-75805', 'F-88657'))
    rnd = random.Random(37525)
    cs = [gen(rnd) for _ in range(a.cases)]
    # memory: S 0x1000 (+0 our car, +4 out), the other's obstacle 0x10000 (vtable 0x2000, matrix +0x40: row 2 +0x60, row 3
    # +0x70; +16 its car), our obstacle 0x10400, our car 0x50000, its car 0x50400 (+452 behaviours 0x70000 / 0x70100:
    # vtable 0x2200 slot 0, +188 the rail, +432 its nearest), rails 0x20000 (run-time, +4 = 0x1800) / 0x20100 (a lane)
    recomp_closure.EXTRA += 'static uint32_t SLOT0;static int CASTOK;' + NL
    setup = r'''
static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){uint32_t t=ctx.r3.u32;switch(a){
case 0x9001: ctx.r3.u64=t+0x40;return;
case 0x9100: ctx.r3.u64=SLOT0;return;}
printf("bad call %08x\n",a);throw std::runtime_error("call");}
'''
    body = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
std::ifstream in(argv[2]);std::string line;
auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};
for(int j=0;j<4;j++)put(0x828609E0+4*j,0x7F800000u);
while(std::getline(in,line)){std::vector<uint32_t> w;{const char*s=line.c_str();char*e;while(*s){uint32_t v=strtoul(s,&e,16);if(e==s)break;w.push_back(v);s=e;}}
memset(objects,0,sizeof(objects));size_t k=0;
uint32_t has=w[k++],cast=w[k++],car=w[k++],beh=w[k++],slot0=w[k++],near=w[k++],ours=w[k++],other=w[k++];
uint32_t O=0x10000,U=0x10400,C=0x50000,OC=0x50400,UB=0x70000,OB=0x70100;
store(0x2000+4,0x9001);store(0x2200,0x9100);store(0x20000+4,0x1800);store(0x20100+4,0);
store(O,0x2000);store(U,0x2000);
for(int j=0;j<3;j++)store(O+0x60+4*j,w[k++]);for(int j=0;j<3;j++)store(O+0x70+4*j,w[k++]);
for(int j=0;j<3;j++)store(U+0x60+4*j,w[k++]);for(int j=0;j<3;j++)store(U+0x70+4*j,w[k++]);
if(car)store(O+16,OC);
if(beh){store(OC+452,OB);store(OB,0x2200);store(OB+432,near?U:0x10800);store(OB+188,other==1?0:other==2?0x20000:0x20100);}
if(ours){store(C+452,UB);store(UB+188,ours==1?0:ours==2?0x20000:0x20100);}
SLOT0=slot0;CASTOK=cast;
store(0x1000,C);store(0x1004,0xDEAD);
Context ctx{};ctx.r1.u32=0x7F000;ctx.r3.u32=0x1000;ctx.r4.u32=has?O:0;sub_%s(ctx,nullptr);
uint32_t m=load(0x1004);printf("%%d\n",m==O?1:m==0?0:2);}
}
''' % fn
    stubs = {'sub_' + cast: 'ctx.r3.u64=(CASTOK&&ctx.r3.u32)?ctx.r3.u32:0;',
             'sub_' + obst: 'ctx.r3.u64=ctx.r3.u32==0x50000?0x10400:0;'}
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fn], setup + body, a.out_dir, 'traffic_mark', stubs=stubs, vector=True, scratch_end=0x80000)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'traffic_mark_in.txt'
    inp.write_text(NL.join(' '.join('%x' % x for x in c) for c in cs) + NL)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True, check=True)
    out = [x for x in r.stdout.split(NL) if x]
    assert len(out) == len(cs), (len(out), len(cs), r.stdout[-300:])
    assert all(o in ('0', '1') for o in out)
    lines = []
    for c, o in zip(cs, out):
        line = ' '.join('%x' % x for x in c) + ' -> ' + o
        assert not any(t.startswith('82') and len(t) == 8 for t in line.split()), line
        lines.append(line)
    (a.out_dir / 'traffic_mark_ref.txt').write_text(NL.join(lines) + NL)
    print(len(lines), 'cases;', sum(o == '1' for o in out), 'marked')


if __name__ == '__main__':
    main()
