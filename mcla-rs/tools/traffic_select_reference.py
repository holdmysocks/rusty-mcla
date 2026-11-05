"""The ambient rider's choice of the car ahead (F-47175's selection loop after the gather, with F-70116 and F-87404)
run offline: the generated code whole and unchanged in vector mode (the VMX flush mode as the host's DAZ / FTZ), on
flat guest memory, as tools/traffic_gather_reference.py runs the gather.

Three kinds of case (first word):
  0  F-47175 from its start with the result words 0 (so the head does not keep the last search); the gather runs on
     an empty world (grid lists empty, lanes without cars); the asker's slot 3 (called right after the gather with
     r4 = stack + 144) writes the case's candidates into the gather's list (stack + 240, count stack + 96). Out: the
     result's two words (obstacle index, ffffffff none).
  1  F-70116 (obstacle, asker) called directly. Out: its answer.
  2  F-87404 (r4 obstacle, r5 a point, r6 asker) called directly. Out: its answer.
The world is the case's objects read through their vtables: an object (slot 1 its matrix: rows 0 / 2 / 3; slot 2 its
velocity; slot 3 its size, two floats at r4; kind byte +6; kind 2: +16 its car; kind 9: +312 a driver's car; kind 0:
the driver fields on itself), a car (+412 speed, +448 its type with bytes +111 / +112, +452 its behaviour with the
rails +188 / +192, +640 a lane token, +872 / +876), rails (word +4 flags, 0x1800 a run-time rail; slot 12 its start,
slot 14 the case's nearest point and phase for each object, found by its place's address r5). Stubbed with the case's
answers: F-17857 (a car physically active) and F-31543 (a lane token's freeway answer); F-96472 runs. The mask words
0x82860950 (zero in the image, filled at run time) are set to x / y / z and 0x828609E0 to +inf (the normalisation's
infinite-estimate test).

Writes <out-dir>/traffic_select_ref.txt, a case a line: the case's words, ' -> ', the answer (layout in `traffic.rs`
test `select_matches_the_original`)."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build

recomp_closure.shim = lambda *_: recomp_leaf.shim(0x80000)
_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <xmmintrin.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    s = s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                  'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)
    reg = 'uint16_t u16; uint8_t u8;'
    assert s.count(reg) == 1
    s = s.replace(reg, 'uint16_t u16; int16_t s16; uint8_t u8;', 1)
    fl = 'void disableFlushMode() {} void enableFlushMode() {}'
    assert s.count(fl) == 1
    return s.replace(fl, 'void disableFlushMode() {_mm_setcsr(_mm_getcsr()&~0x8040u);} void enableFlushMode() {_mm_setcsr(_mm_getcsr()|0x8040u);}', 1)


recomp_closure.vector_shim = vector_shim
recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n'
                         'static uint32_t NR[8][16][4];static int injected;static std::vector<uint32_t> CANDS;\n')
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)

NL = chr(10)
NONE = 0xFFFFFFFF
NAN = float('nan')


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def bf(u):
    return struct.unpack('>f', struct.pack('>I', u & 0xFFFFFFFF))[0]


def f32(x):
    return bf(fb(x))


def gen(rnd, mode):
    nobj = rnd.randint(2, 7) if mode == 0 else 2
    asker = rnd.randrange(nobj)
    nr = rnd.randint(1, 3)
    flags = [rnd.choice((0, 0, 0x1800)) for _ in range(nr)]
    rail = lambda: rnd.choice((NONE,) + tuple(range(nr)))
    a, b = rail(), rail()
    if rnd.random() < 0.15:
        a = NONE
    b_start = rnd.choice((0.0, 0.0, 0.25, 0.5, -0.0))
    yaw = rnd.choice((0.0, math.pi / 2, math.pi, None, None))
    if yaw is None:
        yaw = rnd.uniform(-math.pi, math.pi)
    ar2 = (f32(math.sin(yaw)), f32(rnd.choice((0.0, 0.0, rnd.uniform(-0.05, 0.05)))), f32(math.cos(yaw)))
    ar0 = (f32(math.cos(yaw)), 0.0, f32(-math.sin(yaw)))
    base = tuple(f32(rnd.choice((0.0, rnd.uniform(-900, 900)))) for _ in range(3))
    vlim = math.sqrt(4.9961185)
    objs, places = [], []
    for i in range(nobj):
        kind = rnd.choice((0, 2, 2, 2, 2, 9, 1, 3)) if i != asker else rnd.choice((2, 2, 0))
        if i == asker:
            r0, r2, p = ar0, ar2, base
        else:
            k = rnd.random()
            if k < 0.3:
                # near the asker's heading: the dot about cos 70 degrees
                y = yaw + rnd.choice((0.0, math.pi, rnd.uniform(-1.4, 1.4), 1.2217305, -1.2217305, rnd.uniform(1.15, 1.3)))
            else:
                y = rnd.uniform(-math.pi, math.pi)
            r2 = (f32(math.sin(y)), f32(rnd.choice((0.0, rnd.uniform(-0.05, 0.05)))), f32(math.cos(y)))
            r0 = (f32(math.cos(y)), f32(rnd.choice((0.0, rnd.uniform(-0.05, 0.05)))), f32(-math.sin(y)))
            if places and rnd.random() < 0.12:
                p = rnd.choice(places)
            elif rnd.random() < 0.03:
                p = (NAN, base[1], base[2])
            else:
                t = rnd.choice((rnd.uniform(-10, 60), rnd.uniform(0, 30), 0.0))
                s = rnd.choice((0.0, rnd.uniform(-3, 3), rnd.uniform(-10, 10)))
                p = (f32(base[0] - ar2[0] * t + s * ar0[0]), f32(base[1] + rnd.choice((0.0, rnd.uniform(-1, 1)))),
                     f32(base[2] - ar2[2] * t + s * ar0[2]))
            places.append(p)
        vk = rnd.random()
        if vk < 0.45:
            v = rnd.choice(((0.0, 0.0, 0.0), (-0.0, 0.0, 0.0)))
        elif vk < 0.7:
            m = rnd.choice((bf(fb(vlim) - 1), f32(vlim), bf(fb(vlim) + 1), rnd.uniform(0, 2.2), rnd.uniform(2.2, 2.3)))
            v = (f32(m), 0.0, 0.0) if rnd.random() < 0.5 else (0.0, f32(m * 0.6), f32(m * 0.8))
        else:
            v = tuple(f32(rnd.uniform(-15, 15)) for _ in range(3))
        e = (f32(rnd.choice((rnd.uniform(1.6, 2.2), 2.0, 0.0))), f32(rnd.choice((rnd.uniform(3.5, 5.5), 4.5))))
        hascar = 1 if kind == 2 else (int(rnd.random() < 0.85) if kind == 9 else 0)
        speed = rnd.choice((0.0, 0.0, -0.0, 0.3, f32(0.44704), bf(fb(0.44704) - 1), bf(fb(0.44704) + 1), 3.0, 12.0, NAN))
        active = int(rnd.random() < 0.4)
        cab, trl = int(rnd.random() < 0.15), int(rnd.random() < 0.15)
        beh = int(rnd.random() < 0.8)
        pick = lambda: rnd.choice((NONE, a, b, a, b) + tuple(range(nr)))
        r188, r192 = pick(), pick()
        lane = rnd.choice((0, 0, 1, 2))
        limit = rnd.choice((40.0, 40.0, 10.0))
        idle = rnd.choice((0.0, f32(limit - 0.1), limit, f32(limit + 0.1), 100.0))
        objs.append([kind] + [fb(x) for x in r0 + r2 + p + v + e] + [hascar, fb(speed), active, cab, trl, beh, r188, r192, lane,
                                                                      fb(idle), fb(limit)])
    rails = []
    for r in range(nr):
        ang = rnd.uniform(-math.pi, math.pi)
        dist = rnd.choice((75.0, f32(75.0 + 1e-4), f32(75.0 - 1e-4), rnd.uniform(0, 70), rnd.uniform(80, 150)))
        st = (f32(base[0] + dist * math.cos(ang)), f32(base[1] + rnd.uniform(-2, 2)), f32(base[2] + dist * math.sin(ang)))
        row = [flags[r]] + [fb(x) for x in st]
        for i in range(nobj):
            o = objs[i]
            px, py, pz = (bf(o[7]), bf(o[8]), bf(o[9]))
            q = (f32(px + rnd.uniform(-4, 4)), f32(py + rnd.uniform(-1, 1)), f32(pz + rnd.uniform(-4, 4)))
            if rnd.random() < 0.1:
                q = (px, py, pz)
            t = rnd.choice((rnd.uniform(0, 1), rnd.uniform(0, 1), 0.0, -0.0, 1.0, bf(fb(1.0) - 1), -0.2, 1.3, 0.25, 0.5, NAN, 1e-30))
            row += [fb(x) for x in q] + [fb(t)]
        rails.append(row)
    words = [mode, nobj, asker, nr, a, b, fb(b_start)]
    if mode == 0:
        others = [i for i in range(nobj) if i != asker]
        rnd.shuffle(others)
        cands = others[:rnd.randint(0, len(others))]
        if cands and rnd.random() < 0.1:
            cands.insert(rnd.randrange(len(cands) + 1), rnd.choice(cands))
        words += [len(cands)] + cands
    else:
        o = 1 - asker
        words += [o]
        if mode == 2:
            ob = objs[o]
            px, py, pz = (bf(ob[7]), bf(ob[8]), bf(ob[9]))
            k = rnd.random()
            if k < 0.08:
                q = (px, py, pz)
            elif k < 0.12:
                q = (px, f32(py + 1), pz)
            else:
                rr = rnd.choice((rnd.uniform(0, 1.5), rnd.uniform(0, 4), rnd.uniform(2, 8)))
                ang = rnd.uniform(-math.pi, math.pi)
                q = (f32(px + rr * math.cos(ang)), f32(py + rnd.uniform(-1, 1)), f32(pz + rr * math.sin(ang)))
            words += [fb(x) for x in q]
    for ob in objs:
        words += ob
    for rr in rails:
        words += rr
    return words


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', default='1500,1500,2500')
    a = ap.parse_args()
    fn, blk, clr, cell, heads, active, freeway = (addr(a.tool, f) for f in ('F-47175', 'F-70116', 'F-87404', 'F-88455', 'F-99153',
                                                                            'F-17857', 'F-31543'))
    rnd = random.Random(47176)
    counts = [int(x) for x in a.cases.split(',')]
    cs = [gen(rnd, m) for m in range(3) for _ in range(counts[m])]
    # memory: result 0x1000, context 0x1100, railnet 0x1200 (+8 -> root 0x1300, +56 -> table 0x1400: vertex 0's list
    # 0x1800, empty), the point 0x1E00, vtables 0x2000 (object) / 0x2100 (rail), objects 0x10000 + 0x400 i (matrix
    # +0x40, velocity +0xC0, size +0xE0, lane token +0x3F4), rails 0x20000 + 0x100 r (start +0x20, curve +0xA0: +28 = 1,
    # +32 -> +0xD0 vertex indices 0, 0), cars 0x50000 + 0x400 i (type +0x380, lane token +0x3F4, active +0x3F8),
    # behaviours 0x70000 + 0x100 i; stack below 0x7F000
    setup = r'''
static uint32_t OB(uint32_t i){return 0x10000+0x400*i;}
static uint32_t CA(uint32_t i){return 0x50000+0x400*i;}
static uint32_t RA(uint32_t r){return r==0xFFFFFFFFu?0:0x20000+0x100*r;}
static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}
static uint32_t ix(uint32_t a){return a==0?0xFFFFFFFFu:(a-0x10000)/0x400;}
static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){uint32_t t=ctx.r3.u32;switch(a){
case 0x9001: ctx.r3.u64=t+0x40;return; case 0x9002: ctx.r3.u64=t+0xC0;return;
case 0x9003: {uint32_t r4=ctx.r4.u32;store(r4,load(t+0xE0));store(r4+4,load(t+0xE4));
  if(!injected&&r4==ctx.r1.u32+144){injected=1;uint32_t r1=ctx.r1.u32;store(r1+96,CANDS.size());for(size_t i=0;i<CANDS.size();i++)store(r1+240+4*i,OB(CANDS[i]));}
  return;}
case 0x9012: ctx.r3.u64=t+0x20;return; case 0x9013: ctx.r3.u64=t+0x20;return; case 0x9015: ctx.r3.u64=t+0xA0;return;
case 0x9014: {uint32_t r=(ctx.r4.u32-0x20000)/0x100,i=(ctx.r5.u32-0x10070)/0x400;
  for(int j=0;j<3;j++)store(t+4*j,NR[r][i][j]);store(t+12,0);store(ctx.r6.u32,NR[r][i][3]);store(ctx.r7.u32,0);return;}}
printf("bad call %08x\n",a);throw std::runtime_error("call");}
'''
    body = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
std::ifstream in(argv[2]);std::string line;
put(0x82860950,0xFFFFFFFFu);put(0x82860954,0xFFFFFFFFu);put(0x82860958,0xFFFFFFFFu);put(0x8286095C,0);put(0x8286F1B4,0x5000);
for(int j=0;j<4;j++)put(0x828609E0+4*j,0x7F800000u);
while(std::getline(in,line)){std::vector<uint32_t> w;{const char*s=line.c_str();char*e;while(*s){uint32_t v=strtoul(s,&e,16);if(e==s)break;w.push_back(v);s=e;}}
memset(objects,0,sizeof(objects));size_t k=0;injected=0;CANDS.clear();
uint32_t mode=w[k++],nobj=w[k++],asker=w[k++],nr=w[k++],ra=w[k++],rb=w[k++],bs=w[k++],o=0,pt[3]={0,0,0};
if(mode==0){uint32_t n=w[k++];for(uint32_t j=0;j<n;j++)CANDS.push_back(w[k++]);}else{o=w[k++];if(mode==2)for(int j=0;j<3;j++)pt[j]=w[k++];}
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
  store16(rr+0xA0+28,1);store(rr+0xA0+32,rr+0xD0);store16(rr+0xD0,0);store16(rr+0xD2,0);
  for(uint32_t i=0;i<nobj;i++)for(int j=0;j<4;j++)NR[r][i][j]=w[k++];}
store(0x1200+8,0x1300);store(0x1300+56,0x1400);store(0x1400,0x1800);store16(0x1404,0);
Context ctx{};ctx.r1.u32=0x7F000;
if(mode==0){store(0x1000,0);store(0x1004,0);store(0x1100,0x1200);store(0x1104,OB(asker));store(0x1100+12,RA(ra));store(0x1100+16,RA(rb));
  store(0x1100+20,0);store(0x1100+32,bs);ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;sub_%s(ctx,nullptr);
  if(!injected)printf("no inject ");printf("%%x %%x\n",ix(load(0x1000)),ix(load(0x1004)));}
else if(mode==1){ctx.r3.u32=OB(o);ctx.r4.u32=OB(asker);sub_%s(ctx,nullptr);printf("%%x\n",ctx.r3.u32&0xFF);}
else{for(int j=0;j<3;j++)store(0x1E00+4*j,pt[j]);store(0x1E0C,0);ctx.r3.u32=0x1000;ctx.r4.u32=OB(o);ctx.r5.u32=0x1E00;ctx.r6.u32=OB(asker);
  sub_%s(ctx,nullptr);printf("%%x\n",ctx.r3.u32&0xFF);}}
}
''' % (fn, blk, clr)
    stubs = {'sub_' + cell: 'ctx.r3.u64=0;', 'sub_' + heads: 'for(int i=0;i<9;i++)store(ctx.r5.u32+4*i,0);',
             'sub_' + active: 'ctx.r3.u64=*pointer(ctx.r3.u32+0x3F8);', 'sub_' + freeway: 'ctx.r3.u64=load(ctx.r3.u32);'}
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fn, 'sub_' + blk, 'sub_' + clr], setup + body, a.out_dir, 'traffic_select', stubs=stubs, vector=True,
                scratch_end=0x80000)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'traffic_select_in.txt'
    inp.write_text(NL.join(' '.join('%x' % x for x in c) for c in cs) + NL)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True, check=True)
    out = [x for x in r.stdout.split(NL) if x]
    assert len(out) == len(cs), (len(out), len(cs), r.stdout[-300:])
    lines = []
    for c, o in zip(cs, out):
        line = ' '.join('%x' % x for x in c) + ' -> ' + o.strip()
        assert not any(t.startswith('82') and len(t) == 8 for t in line.split()), line
        lines.append(line)
    (a.out_dir / 'traffic_select_ref.txt').write_text(NL.join(lines) + NL)
    sel = [l.split(' -> ')[1].split() for l in lines if l.startswith('0 ')]
    print(len(lines), 'cases;', 'selection: near', sum(s[0] != 'ffffffff' for s in sel), 'blocking', sum(s[1] != 'ffffffff' for s in sel),
          'of', len(sel), '; F-70116 true', sum(l.endswith('-> 1') for l in lines if l.startswith('1 ')),
          '; F-87404 clear', sum(l.endswith('-> 1') for l in lines if l.startswith('2 ')))


if __name__ == '__main__':
    main()
