"""The rider's choice of the car ahead (F-47175's selection loop) with an arc-line-arc rail as the asker's current rail
(context +12, a run-time rail: word +4 & 0x1800) run offline: the selection's slot 14 on that rail is the real one of
mcRailAmbientTCP (vtable 0x820CD224 in the image: the thunk F-36154 -> F-16977, the record at rail +128) and its slot 12
the real F-55960 -> F-33742 (the record's P), the rail built by F-85722 from the case's P H0 T H1 R s0 s1. Everything
else as tools/traffic_select_reference.py (mode 0): the case's objects through their vtables, the gather on an empty
world with the candidates written in by the asker's slot 3, F-17857 / F-31543 stubbed with the case's answers; the
other rails (lanes) answer slot 12 / 14 from the case. The next rail +16 is none, a lane, or the TCP rail itself.

Writes <out-dir>/traffic_select_tcp_ref.txt, a case a line: traffic_select_reference's mode-0 words (mode word 3,
rail 0 the TCP rail, its row's start / nearest words unused), then P H0 T H1 (xyz each), R, s0, s1; ' -> ' the result's
two words (obstacle index, ffffffff none) and the rail's valid flag (test `select_tcp_matches_the_original`)."""
import argparse
import math
import random
import subprocess
from pathlib import Path
import traffic_select_reference as tsr
from traffic_select_reference import addr, fb, bf, f32, NONE, NL
import recomp_closure
from recomp_closure import index, build

_vs = recomp_closure.vector_shim


def vector_shim():
    s = _vs()
    un = 'void disableFlushModeUnconditional() {} void enableFlushModeUnconditional() {}'
    assert s.count(un) == 1
    return s.replace(un, 'void disableFlushModeUnconditional() {_mm_setcsr(_mm_getcsr()&~0x8040u);}'
                     ' void enableFlushModeUnconditional() {_mm_setcsr(_mm_getcsr()|0x8040u);}', 1)


recomp_closure.vector_shim = vector_shim


def gen(rnd):
    w = tsr.gen(rnd, 0)
    w[0] = 3
    nobj, nr = w[1], w[3]
    w[4] = 0
    if rnd.random() < 0.25:
        w[5] = 0
    k = 8 + w[7]
    # the asker's rows: its object block (26 words each)
    ob = k + 26 * w[2]
    r2 = [bf(x) for x in w[ob + 4:ob + 7]]
    p = [bf(x) for x in w[ob + 7:ob + 10]]
    rails = k + 26 * nobj
    w[rails] = 0x1800
    fwd = [-r2[0], 0.0, -r2[2]]
    side = [-fwd[2], 0.0, fwd[0]]
    d = rnd.choice((rnd.uniform(5, 60), rnd.uniform(20, 40), rnd.uniform(0, 8)))
    s = rnd.choice((0.0, rnd.uniform(-8, 8), rnd.uniform(-3, 3)))
    T = [p[0] + fwd[0] * d + side[0] * s, p[1] + rnd.choice((0.0, rnd.uniform(-1, 1))), p[2] + fwd[2] * d + side[2] * s]
    turn = rnd.choice((0.0, rnd.uniform(-1.2, 1.2), rnd.uniform(-0.3, 0.3)))
    c, sn = math.cos(turn), math.sin(turn)
    H1 = [fwd[0] * c - fwd[2] * sn, 0.0, fwd[0] * sn + fwd[2] * c]
    H0 = fwd if (fwd[0] or fwd[2]) else [0.0, 0.0, -1.0]
    R = rnd.choice((rnd.uniform(2, 20), 5.0, rnd.uniform(0.5, 4)))
    w += [fb(f32(x)) for x in p + H0 + T + H1 + [R]] + [rnd.getrandbits(1), rnd.getrandbits(1)]
    return w


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    fn, cell, heads, active, freeway, bld, thunk, start = (addr(a.tool, f) for f in (
        'F-47175', 'F-88455', 'F-99153', 'F-17857', 'F-31543', 'F-85722', 'F-36154', 'F-55960'))
    rnd = random.Random(16978)
    cs = [gen(rnd) for _ in range(a.cases)]
    # memory as traffic_select_reference.py's but the rails 0x20000 + 0x200 r (rail 0 the TCP rail: vtable the image's,
    # record +128 .. +292), the build's inputs 0x3000 (P, H0, T, H1)
    setup = r'''
static uint32_t OB(uint32_t i){return 0x10000+0x400*i;}
static uint32_t CA(uint32_t i){return 0x50000+0x400*i;}
static uint32_t RA(uint32_t r){return r==0xFFFFFFFFu?0:0x20000+0x200*r;}
static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}
static uint32_t ix(uint32_t a){return a==0?0xFFFFFFFFu:(a-0x10000)/0x400;}
static int NEAR14;
static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){uint32_t t=ctx.r3.u32;switch(a){
case 0x9001: ctx.r3.u64=t+0x40;return; case 0x9002: ctx.r3.u64=t+0xC0;return;
case 0x9003: {uint32_t r4=ctx.r4.u32;store(r4,load(t+0xE0));store(r4+4,load(t+0xE4));
  if(!injected&&r4==ctx.r1.u32+144){injected=1;uint32_t r1=ctx.r1.u32;store(r1+96,CANDS.size());for(size_t i=0;i<CANDS.size();i++)store(r1+240+4*i,OB(CANDS[i]));}
  return;}
case 0x9012: ctx.r3.u64=t+0x20;return; case 0x9013: ctx.r3.u64=t+0x20;return; case 0x9015: ctx.r3.u64=t+0x1A0;return;
case 0x9014: {uint32_t r=(ctx.r4.u32-0x20000)/0x200,i=(ctx.r5.u32-0x10070)/0x400;
  for(int j=0;j<3;j++)store(t+4*j,NR[r][i][j]);store(t+12,0);store(ctx.r6.u32,NR[r][i][3]);store(ctx.r7.u32,0);return;}
case 0x%s: NEAR14++;sub_%s(ctx,base);return;
case 0x%s: sub_%s(ctx,base);return;}
printf("bad call %%08x\n",a);throw std::runtime_error("call");}
''' % (thunk, thunk, start, start)
    body = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
std::ifstream in(argv[2]);std::string line;
put(0x82860950,0xFFFFFFFFu);put(0x82860954,0xFFFFFFFFu);put(0x82860958,0xFFFFFFFFu);put(0x8286095C,0);put(0x8286F1B4,0x5000);
for(int j=0;j<4;j++)put(0x828609E0+4*j,0x7F800000u);
while(std::getline(in,line)){std::vector<uint32_t> w;{const char*s=line.c_str();char*e;while(*s){uint32_t v=strtoul(s,&e,16);if(e==s)break;w.push_back(v);s=e;}}
memset(objects,0,sizeof(objects));size_t k=0;injected=0;CANDS.clear();NEAR14=0;
uint32_t mode=w[k++],nobj=w[k++],asker=w[k++],nr=w[k++],ra=w[k++],rb=w[k++],bs=w[k++];
{uint32_t n=w[k++];for(uint32_t j=0;j<n;j++)CANDS.push_back(w[k++]);}
store(0x2000+4,0x9001);store(0x2000+8,0x9002);store(0x2000+12,0x9003);
store(0x2100+48,0x9012);store(0x2100+52,0x9013);store(0x2100+56,0x9014);store(0x2100+60,0x9015);
for(uint32_t i=0;i<nobj;i++){uint32_t ob=OB(i),c=CA(i),bh=0x70000+0x100*i;store(ob,0x2000);uint32_t kind=w[k++];*pointer(ob+6)=uint8_t(kind);
  for(int j=0;j<3;j++)store(ob+0x40+4*j,w[k++]);for(int j=0;j<3;j++)store(ob+0x60+4*j,w[k++]);for(int j=0;j<3;j++)store(ob+0x70+4*j,w[k++]);
  for(int j=0;j<3;j++)store(ob+0xC0+4*j,w[k++]);store(ob+0xE0,w[k++]);store(ob+0xE4,w[k++]);
  uint32_t hascar=w[k++],speed=w[k++],act=w[k++],cab=w[k++],trl=w[k++],beh=w[k++],r188=w[k++],r192=w[k++],lane=w[k++],idle=w[k++],limit=w[k++];
  uint32_t drv=kind==0?ob:c;
  if(kind==2)store(ob+16,c);if(kind==9&&hascar)store(ob+312,c);
  store(c+412,speed);store(c+448,c+0x380);*pointer(c+0x380+111)=uint8_t(cab);*pointer(c+0x380+112)=uint8_t(trl);*pointer(c+0x3F8)=uint8_t(act);
  if(beh){store(c+452,bh);store(bh+188,RA(r188));store(bh+192,RA(r192));}
  if(lane){store(drv+640,drv+0x3F4);store(drv+0x3F4,lane==2);}
  store(drv+872,idle);store(drv+876,limit);}
for(uint32_t r=0;r<nr;r++){uint32_t rr=RA(r);store(rr,0x2100);store(rr+4,w[k++]);for(int j=0;j<3;j++)store(rr+0x20+4*j,w[k++]);
  store16(rr+0x1A0+28,1);store(rr+0x1A0+32,rr+0x1D0);store16(rr+0x1D0,0);store16(rr+0x1D2,0);
  for(uint32_t i=0;i<nobj;i++)for(int j=0;j<4;j++)NR[r][i][j]=w[k++];}
{uint32_t rr=RA(0);store(rr,0x820CD224u);for(int i=0;i<4;i++){for(int j=0;j<3;j++)store(0x3000+16*i+4*j,w[k++]);store(0x3000+16*i+12,0);}
  uint32_t R=w[k++],s0=w[k++],s1=w[k++];float fr;memcpy(&fr,&R,4);
  Context b{};b.r1.u32=0x7F000;b.r3.u32=rr+128;b.r4.u32=0x3000;b.r5.u32=0x3010;b.r6.u32=s0;b.r7.u32=0x3020;b.r8.u32=0x3030;b.r9.u32=s1;b.f1.f64=fr;
  sub_%s(b,nullptr);store(rr+4,0x1800);}
store(0x1200+8,0x1300);store(0x1300+56,0x1400);store(0x1400,0x1800);store16(0x1404,0);
Context ctx{};ctx.r1.u32=0x7F000;
store(0x1000,0);store(0x1004,0);store(0x1100,0x1200);store(0x1104,OB(asker));store(0x1100+12,RA(ra));store(0x1100+16,RA(rb));
store(0x1100+20,0);store(0x1100+32,bs);ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;sub_%s(ctx,nullptr);
if(!injected)printf("no inject ");printf("%%x %%x %%d %%d\n",ix(load(0x1000)),ix(load(0x1004)),int(*pointer(RA(0)+128+160)>>5&1),NEAR14);}
}
''' % (bld, fn)
    stubs = {'sub_' + cell: 'ctx.r3.u64=0;', 'sub_' + heads: 'for(int i=0;i<9;i++)store(ctx.r5.u32+4*i,0);',
             'sub_' + active: 'ctx.r3.u64=*pointer(ctx.r3.u32+0x3F8);', 'sub_' + freeway: 'ctx.r3.u64=load(ctx.r3.u32);'}
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fn, 'sub_' + bld, 'sub_' + thunk, 'sub_' + start], setup + body, a.out_dir, 'traffic_select_tcp',
                stubs=stubs, vector=True, scratch_end=0x80000)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'traffic_select_tcp_in.txt'
    inp.write_text(NL.join(' '.join('%x' % x for x in c) for c in cs) + NL)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True, check=True)
    out = [x for x in r.stdout.split(NL) if x]
    assert len(out) == len(cs), (len(out), len(cs), r.stdout[-300:])
    lines, calls = [], 0
    for c, o in zip(cs, out):
        f = o.split()
        calls += int(f[3])
        line = ' '.join('%x' % x for x in c) + ' -> ' + ' '.join(f[:3])
        assert not any(t.startswith('82') and len(t) == 8 for t in line.split()), line
        lines.append(line)
    (a.out_dir / 'traffic_select_tcp_ref.txt').write_text(NL.join(lines) + NL)
    res = [l.split(' -> ')[1].split() for l in lines]
    print(len(lines), 'cases; valid rails', sum(s[2] == '1' for s in res), '; near', sum(s[0] != 'ffffffff' for s in res),
          'blocking', sum(s[1] != 'ffffffff' for s in res), '; slot-14 calls on the TCP rail', calls)


if __name__ == '__main__':
    main()
