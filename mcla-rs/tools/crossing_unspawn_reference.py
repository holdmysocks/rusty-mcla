"""The traffic crossing unspawner (F-54093) run offline: the generated code whole and unchanged, in vector mode, on flat
guest memory, with the object grid's cell code (F-88455 -> F-59709, F-99153, F-49551), the crossing test (F-95591 ->
F-66886), the freeway test (F-31543) and the probe helper (F-13384 with its record copy F-13185) running as they are.
What the code asks of the world is stubbed: the network / cutscene / front-end tests (F-24338, F-97032, F-95688) answer
from the case; the remote test F-23063, the behaviour's slots 18 / 19 / 22 (F-86955, F-74598, F-90466) and the road
speed F-87157 answer from the car; the car's place F-58860 hands back car +176 and its forward F-50349 the car's
vector; the view test F-92805 prints its arguments and answers the car's "seen" bit; the probe F-57005 prints its
record (origin +32, end +48, the words +16 / +20, byte +24, its r4) and answers the car's bit for that ray (hit: +96 set);
the removal F-51512 prints the car and the reason.

Guest memory: the manager M 0x1000 (+176 focus, +208 velocity, +224 view, +2109 the enable byte, +2152 S = 0x1A00;
the arrays it writes, +328 .. +959, filled with 0xA5 before the run), the grid 0x1B00 (+0 w, +4 h, +8 count, +12 / +16
min, +20 / +24 max = min + 40 n, +28 the cell heads 0x1C00; [0x8286F1B4] points at it), the obstacles 0x2000 + 0x40 k
(vtable 0x2E00: slot 1 -> car +128, slot 2 -> car +208; +6 kind, +12 next, +16 car; inserted in car order at the
tail of the cell F-88455 gives for their place, none when off the grid), the cars 0x3000 + 0x200 k (+40 item, +176
place, +208 velocity, +256 forward, +448 type 0x9000 / 0x9100 (+116 = 0 / 2), +452 behaviour 0x9800 + 0x100 k (vtable
0xC800, +188 lane 0xC900 + 0x20 k, +8 owner 0xD000 + 0x80 k, its parent +0x40), +476 id k, +492 radius, +500 flags).

Writes <out-dir>/crossing_unspawn_ref.txt: "<case words> -> <calls> M <arrays>" (or "X" when the code left before
its clear), floats as hex bits. Case words: net cut fe en focus(3) vel(3) view(3) w h minx minz n, then per car kind
pos(3) vel(3) bits owner-mode speed fwd(3) radius. Bits: 1 +500 bit 0x20, 2 remote, 4 no behaviour, 8 bound group 2,
16 item, 32 slot 18, 64 slot 19, 128 slot 22, 256 no lane, 512 seen, 1024 ray 0 hit, 2048 ray 1 hit. Owner mode: 0 the
lane's owner a road (flag 2), 1 the owner not a road but its parent is, 2 neither, 3 no owner."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build

recomp_closure.shim = lambda: recomp_leaf.shim(0x10000)
recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n')
recomp_closure.EXTRA += r'''static const uint32_t*cur;static int curk,ray;
static uint32_t bit(int k,uint32_t b){return cur[18+14*k+7]&b;}
static void pv(uint32_t a){printf(" %08x %08x %08x",load(a),load(a+4),load(a+8));}
static auto fbits=[](double d){float f=float(d);uint32_t u;memcpy(&u,&f,4);return u;};
static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}
'''
_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <xmmintrin.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    s = s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                  'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)
    fp = ('struct { void disableFlushMode() {} void enableFlushMode() {} void disableFlushModeUnconditional() {}'
          ' void enableFlushModeUnconditional() {} } fpscr;')
    assert s.count(fp) == 1
    on, off = '{_mm_setcsr(_mm_getcsr()|0x8040);}', '{_mm_setcsr(_mm_getcsr()&~0x8040u);}'
    return s.replace(fp, 'struct { void disableFlushMode() %s void enableFlushMode() %s void disableFlushModeUnconditional() %s'
                     ' void enableFlushModeUnconditional() %s } fpscr;' % (off, on, off, on), 1)


recomp_closure.vector_shim = vector_shim
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)

nl = chr(10)
HDR, CAR = 18, 14
FREEWAY = struct.unpack('>f', bytes.fromhex('41e81a69'))[0]


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def unit(rnd, y=0.0):
    a = rnd.uniform(-math.pi, math.pi)
    v = [math.sin(a), y, -math.cos(a)]
    n = math.sqrt(sum(x * x for x in v))
    return [x / n for x in v]


def cases(rnd, count):
    out = []
    for _ in range(count):
        u = rnd.random()
        net, cut, fe, en = int(u < 0.02), int(0.02 <= u < 0.04), int(0.04 <= u < 0.06), int(not 0.06 <= u < 0.08)
        w, h = rnd.randint(2, 10), rnd.randint(2, 10)
        minx, minz = -40.0 * rnd.randint(0, 5), -40.0 * rnd.randint(0, 5)
        fx = rnd.uniform(minx - 15, minx + 40 * w + 15)
        fz = rnd.uniform(minz - 15, minz + 40 * h + 15)
        if rnd.random() < 0.05:
            fx = minx + 40 * rnd.randint(0, w)
        # (a crowd: 32 .. 40 cars round the point, none skipped, crossing ones seen and in sight - past the 32 looked at)
        crowd = rnd.random() < 0.06
        if crowd:
            w, h = rnd.randint(6, 10), rnd.randint(6, 10)
            fx, fz = rnd.uniform(minx + 100, minx + 40 * w - 100), rnd.uniform(minz + 100, minz + 40 * h - 100)
        focus = [fx, rnd.uniform(-10, 60), fz]
        m = rnd.random()
        if crowd:
            vel = [rnd.uniform(-8, 8), 0.0, rnd.uniform(-8, 8)]
        elif m < 0.08:
            vel = [0.0, 0.0, 0.0]
        elif m < 0.2:
            vel = [rnd.choice((0.0, rnd.uniform(-40, 40))), 0.0, rnd.choice((0.0, rnd.uniform(-40, 40)))]
        elif m < 0.25:
            vel = [rnd.uniform(-1e-5, 1e-5), 0.0, rnd.uniform(-1e-5, 1e-5)]
        else:
            vel = [rnd.uniform(-40, 40), rnd.uniform(-3, 3), rnd.uniform(-40, 40)]
        view = unit(rnd, rnd.choice((0.0, 0.0, rnd.uniform(-0.2, 0.2))))
        focus, vel, view = [f32(x) for x in focus], [f32(x) for x in vel], [f32(x) for x in view]
        a = (focus[0], focus[2])
        b = (f32(a[0] + vel[0] * 2), f32(a[1] + vel[2] * 2))
        n = rnd.choice((0, 1, 2, 3, 5, 8, 12, 20, rnd.randint(1, 30)))
        if crowd:
            n = rnd.choice((32, 33, 36, 40))
        px, pz = f32(a[0] + f32(f32(b[0] - a[0]) * 0.75)), f32(a[1] + f32(f32(b[1] - a[1]) * 0.75))
        cars = []
        for _k in range(n):
            if crowd:
                pos = [px + rnd.uniform(-25, 25), focus[1], pz + rnd.uniform(-25, 25)]
                v = [rnd.uniform(-30, 30), 0.0, rnd.uniform(-30, 30)]
                bits = 512 | (2048 if rnd.random() < 0.5 else 0) | (8 if rnd.random() < 0.3 else 0)
                cars.append([2] + [fb(f32(x)) for x in pos + v] + [bits, 0, fb(f32(rnd.uniform(5, 25)))] +
                            [fb(f32(x)) for x in unit(rnd)] + [fb(f32(rnd.uniform(1.0, 9.0)))])
                continue
            kind = 2 if rnd.random() < 0.93 else rnd.choice((0, 1, 3, 9))
            m = rnd.random()
            if m < 0.35:
                # through a point of the player's next 2 s (its ends included) at a phase of its own 5 s
                s = rnd.choice((0.0, 1.0, rnd.random(), rnd.random()))
                x = (a[0] + (b[0] - a[0]) * s, a[1] + (b[1] - a[1]) * s)
                v = unit(rnd)
                sp = rnd.choice((rnd.uniform(0, 30), 4.0, 10.0))
                v = [v[0] * sp, 0.0, v[2] * sp]
                if rnd.random() < 0.15:
                    v[rnd.choice((0, 2))] = 0.0
                t = rnd.choice((0.0, 1.0, rnd.random(), rnd.uniform(-0.2, 1.2)))
                pos = [x[0] - 5 * v[0] * t, 0.0, x[1] - 5 * v[2] * t]
            elif m < 0.45:
                # parallel to the player's, on its line or beside it
                k = rnd.choice((1.0, -1.0, 0.4, 2.5, rnd.uniform(-3, 3)))
                v = [vel[0] * k, 0.0, vel[2] * k]
                off = rnd.choice((0.0, 0.0, rnd.uniform(-5, 5)))
                s = rnd.uniform(-0.5, 1.5)
                pos = [a[0] + (b[0] - a[0]) * s + off, 0.0, a[1] + (b[1] - a[1]) * s - off]
            else:
                pos = [focus[0] + rnd.uniform(-70, 70), 0.0, focus[2] + rnd.uniform(-70, 70)]
                v = rnd.choice(([0.0, 0.0, 0.0], [rnd.uniform(-30, 30), rnd.uniform(-2, 2), rnd.uniform(-30, 30)]))
            pos[1] = focus[1] + rnd.choice((0.0, rnd.uniform(-4, 4), 3.0, -3.0, 2.0, -2.0, rnd.uniform(-2.5, 2.5)))
            bits = 0
            for bit, p in ((1, 0.04), (2, 0.04), (4, 0.04), (8, 0.25), (16, 0.04), (32, 0.04), (64, 0.04), (128, 0.04), (256, 0.15),
                           (512, 0.5), (1024, 0.6), (2048, 0.5)):
                if rnd.random() < p:
                    bits |= bit
            mode = rnd.choice((0, 0, 0, 1, 2, 3))
            speed = rnd.choice((FREEWAY, f32(math.nextafter(FREEWAY, 0)), rnd.uniform(5, 40), 13.4))
            fwd = unit(rnd, rnd.choice((0.0, rnd.uniform(-0.1, 0.1))))
            cars.append([kind] + [fb(f32(x)) for x in pos + v] + [bits, mode, fb(f32(speed))] + [fb(f32(x)) for x in fwd] +
                        [fb(f32(rnd.uniform(1.0, 9.0)))])
        hdr = [net, cut, fe, en] + [fb(x) for x in focus + vel + view] + [w, h, fb(minx), fb(minz), n]
        assert len(hdr) == HDR and all(len(c) == CAR for c in cars)
        out.append(hdr + [x for c in cars for x in c])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    idx = index(a.recomp)
    nm = {f: addr(a.tool, f) for f in ('F-54093', 'F-24338', 'F-97032', 'F-95688', 'F-23063', 'F-58860', 'F-87157', 'F-92805',
                                        'F-50349', 'F-57005', 'F-51512', 'F-88455')}
    rows = cases(random.Random(54093), a.cases)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'crossing_unspawn_inputs.bin'
    inp.write_bytes(b''.join(struct.pack('<%dI' % len(r), *r) for r in rows))
    car = '(0x3000+0x200*k)'
    disp = (r'''static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){
 uint32_t r3=ctx.r3.u32;
 if(a==0x82FF0001){ctx.r3.u64=load(r3+16)+128;return;}
 if(a==0x82FF0002){ctx.r3.u64=load(r3+16)+208;return;}
 int k=(int(r3)-0x9800)/0x100;
 if(a==0x82FF0012){ctx.r3.u64=bit(k,32)?1:0;return;}
 if(a==0x82FF0013){ctx.r3.u64=bit(k,64)?1:0;return;}
 if(a==0x82FF0016){ctx.r3.u64=bit(k,128)?1:0;return;}
 throw std::runtime_error("unexpected indirect call");}
''')
    stubs = {
        'sub_' + nm['F-24338']: 'ctx.r3.u64=cur[0];',
        'sub_' + nm['F-97032']: 'ctx.r3.u64=cur[1];',
        'sub_' + nm['F-95688']: 'ctx.r3.u64=cur[2];',
        'sub_' + nm['F-23063']: 'ctx.r3.u64=bit(int(ctx.r3.u32),2)?1:0;',
        'sub_' + nm['F-58860']: 'ctx.r3.u64=ctx.r3.u32+176;',
        'sub_' + nm['F-87157']: 'ctx.f1.f64=getf(ctx.r3.u32+16);',
        'sub_' + nm['F-92805']: ('{int k=(int(ctx.r4.u32)-176-0x3000)/0x200;printf(" S %d",k);pv(ctx.r4.u32);'
                                 'printf(" %08x %d %d",fbits(ctx.f1.f64),int(ctx.r6.u32&0xFF),ctx.r3.u32==0x1000);ctx.r3.u64=bit(k,512)?1:0;}'),
        'sub_' + nm['F-50349']: ('{curk=(int(ctx.r4.u32)-0x3000)/0x200;ray=0;for(int j=0;j<16;j+=4)store(ctx.r3.u32+j,load(ctx.r4.u32+256+j));}'),
        'sub_' + nm['F-57005']: ('{uint32_t q=ctx.r3.u32;printf(" Q %d",curk);pv(q+32);pv(q+48);printf(" %08x %08x %02x %d",load(q+16),load(q+20),'
                                 '*pointer(q+24),int(ctx.r4.u32));int hit=bit(curk,ray?2048:1024)!=0;ray++;store(q+96,hit?0x10:0);}'),
        'sub_' + nm['F-51512']: 'printf(" D %d %d %d",(int(ctx.r4.u32)-0x3000)/0x200,int(ctx.r5.u32),ctx.r3.u32==0x1A00);',
    }
    body = (r'''FILE*fi=fopen(argv[2],"rb");std::vector<uint32_t> in;uint32_t w_;while(fread(&w_,4,1,fi)==1)in.push_back(w_);fclose(fi);
size_t i=0;while(i<in.size()){const uint32_t*c=in.data()+i;cur=c;int n=int(c[17]);i+=18+14*n;
 memset(objects,0,0xF000);
 const uint32_t M=0x1000,G=0x1B00;
 for(uint32_t o=328;o<960;o++)*pointer(M+o)=0xA5;
 store(M+2152,0x1A00);*pointer(M+2109)=uint8_t(c[3]);
 for(int j=0;j<3;j++){store(M+176+4*j,c[4+j]);store(M+208+4*j,c[7+j]);store(M+224+4*j,c[10+j]);}
 store(G,c[13]);store(G+4,c[14]);store(G+8,c[13]*c[14]);store(G+12,c[15]);store(G+16,c[16]);
 putf(G+20,getf(G+12)+40.0f*float(c[13]));putf(G+24,getf(G+16)+40.0f*float(c[14]));store(G+28,0x1C00);
 put(0x8286F1B4,G);
 store(0x2E00+4,0x82FF0001);store(0x2E00+8,0x82FF0002);
 store(0xC800+72,0x82FF0012);store(0xC800+76,0x82FF0013);store(0xC800+88,0x82FF0016);
 store(0x9100+116,2);
 for(int k=0;k<n;k++){const uint32_t*d=c+18+14*k;uint32_t o=0x2000+0x40*k,car=0x3000+0x200*k,beh=0x9800+0x100*k,lane=0xC900+0x20*k,ow=0xD000+0x80*k,pa=ow+0x40;
  store(o,0x2E00);*pointer(o+6)=uint8_t(d[0]);store(o+16,car);
  for(int j=0;j<3;j++){store(car+176+4*j,d[1+j]);store(car+208+4*j,d[4+j]);store(car+256+4*j,d[10+j]);}
  store(car+492,d[13]);*pointer(car+500)=(d[7]&1)?0x20:0;store(car+476,k);store(car+40,(d[7]&16)?0x1234:0);
  store(car+448,(d[7]&8)?0x9100:0x9000);
  if(!(d[7]&4)){store(car+452,beh);store(beh,0xC800);
   if(!(d[7]&256)){store(beh+188,lane);
    if(d[8]!=3){store(lane+8,ow);store(ow+16,d[9]);store(pa+16,d[9]);
     store(ow+4,d[8]==0?2:5);store(ow+8,pa);store(pa+4,d[8]==1?6:4);}}}
  Context g{};g.r1.u32=0xFF00;g.r3.u32=G;g.r4.u32=car+176;sub_'''
            + nm['F-88455'] + r'''(g,nullptr);uint32_t cell=g.r3.u32&0xFFFF;
  if(cell!=0xFFFF&&cell<load(G+8)){uint32_t hp=0x1C00+4*cell;while(load(hp))hp=load(hp)+12;store(hp,o);}}
 Context ctx{};ctx.r1.u32=0xFF00;ctx.r13.u32=0x5000;ctx.r3.u32=M;ctx.r4.u32=0x1A00;sub_''' + nm['F-54093'] + r'''(ctx,nullptr);
 bool kept=true;for(uint32_t o=328;o<960;o++)kept=kept&&*pointer(M+o)==0xA5;
 if(kept){printf(" X\n");continue;}
 printf(" M");for(int j=0;j<22;j++)printf(" %08x",load(M+328+4*j));
 int last=-1;for(int s=0;s<32;s++){if(load(M+416+8*s)|load(M+420+8*s)|load(M+672+8*s)|load(M+676+8*s)|*pointer(M+928+s))last=s;}
 printf(" %d",last+1);for(int s=0;s<=last;s++)printf(" %08x %08x %08x %08x %d",load(M+416+8*s),load(M+420+8*s),load(M+672+8*s),load(M+676+8*s),int(*pointer(M+928+s)));
 printf("\n");}
''')
    main_src = disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(idx, ['sub_' + nm['F-54093'], 'sub_' + nm['F-88455']], main_src, a.out_dir, 'crossing_unspawn', stubs=stubs, vector=True)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True, check=True)
    out = [x.strip() for x in r.stdout.split(nl) if x.strip()]
    assert len(out) == len(rows), (len(out), len(rows))
    lines = [' '.join('%08x' % x for x in row) + ' -> ' + o for row, o in zip(rows, out)]
    # (no word that reads as an address of the image: such a case is left out)
    keep = [x for x in lines if not any(len(t) == 8 and t.startswith('82') for t in x.split())]
    (a.out_dir / 'crossing_unspawn_ref.txt').write_text(nl.join(keep) + nl)
    print(len(keep), 'cases kept of', len(lines), '|', sum(' D ' in x for x in keep), 'removals,',
          sum(x.endswith(' X') for x in keep), 'left before the clear,', sum(x.count(' S ') for x in keep), 'view tests,',
          sum(x.count(' Q ') for x in keep), 'probes')


if __name__ == '__main__':
    main()
