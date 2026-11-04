"""The road at a point (F-86195, the lookup of AmbientMgr_SetRoadDensity F-84942 and of the race scripts' road density
event F-36201) and the road density index (F-85897), run offline: the generated code whole and unchanged, in vector
mode, on the real city navsys. F-86195 calls the quadtree query F-44023 (type mask 2, at most 8), which calls each
road's vfunc 4, the box test F-35347 (dispatched to the original's function through a vtable written for the roads
and junctions). F-44023 is also run on its own with the same point to log every hit.

Memory: the navsys file's virtual block at 0x50000000 and physical block at 0x60000000 (its own pointers), the holder
at 0x3100 (+8 the root), the roads' and junctions' first word the vtable at 0x3800 (+16 = F-35347).

Writes <out-dir>/road_density_ref.txt:
  "R <x> <y> <z> -> <road> | <hits>"  x y z as float bits (hex); road the index of F-86195's road (roads in the file's
  object order) or -1; hits "<road>:<dist2 bits>" for every record F-44023 counted, in order.
  "D <global> <road +164> <cut byte> <front-end byte> <front-end n> <road +152> -> <index>" for F-85897 (all hex).
No word reads as an address of the image (hex words that would start with 82 are not written: the cases avoid them)."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build


def patched_shim():
    s = recomp_leaf.shim(0x10000)
    old = 'uint64_t o=uint64_t(a)-0x82000000ull;'
    assert s.count(old) == 1
    s = s.replace(old, 'if ((a>>28)==5 && (a&0xFFFFFFF)+4<=resv.size()) return resv.data()+(a&0xFFFFFFF);\n'
                       '    if ((a>>28)==6 && (a&0xFFFFFFF)+4<=resp.size()) return resp.data()+(a&0xFFFFFFF);\n    ' + old, 1)
    s = s.replace('static std::vector<uint8_t> image;', 'static std::vector<uint8_t> image;\nstatic std::vector<uint8_t> resv, resp;', 1)
    reg = 'uint16_t u16; uint8_t u8;'
    assert s.count(reg) == 1
    return s.replace(reg, 'uint16_t u16; int16_t s16; uint8_t u8; int8_t s8;', 1)


recomp_closure.shim = patched_shim
recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n')
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


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def nextf(x, k):
    if x == 0.0:
        return struct.unpack('>f', struct.pack('>I', (k if k > 0 else 0x80000000 - k) & 0xFFFFFFFF))[0] if k else x
    b = fb(x)
    b = b + k if x >= 0 else b - k
    return struct.unpack('>f', struct.pack('>I', b & 0xFFFFFFFF))[0]


class Nav:
    """The roads' boxes and the quadtree's nodes, read from the navsys file (to make the cases)."""

    def __init__(self, path):
        raw = path.read_bytes()
        vs = struct.unpack('<I', raw[12:16])[0]
        self.d = raw[16:16 + vs]
        o, self.roads = 0, []
        while o + 0x30 <= vs:
            vt = struct.unpack('<I', self.d[o:o + 4])[0]
            if vt == 0x00479F0C:
                o += 0x70
                continue
            if vt == 0x0047A13C:
                self.roads.append(o)
                o += 0xB0
                continue
            if vt == 0x0047A0C4:
                o += 0xC0
                continue
            o += 4
        self.nodes = []
        root = self.u32(0x130) & 0xFFFFFFF
        todo = [root]
        while todo:
            n = todo.pop()
            self.nodes.append(n)
            for k in range(4):
                c = self.u32(n + 0x1C + 4 * k)
                if c >> 28 == 5:
                    todo.append(c & 0xFFFFFFF)
        self.root = root

    def u32(self, o):
        return struct.unpack('>I', self.d[o:o + 4])[0]

    def f(self, o):
        return struct.unpack('>f', self.d[o:o + 4])[0]


def point(rnd, nav):
    m = rnd.random()
    y = rnd.choice((0.0, rnd.uniform(-50, 300), f32(21.0)))
    if m < 0.35:
        # inside or about a road's box
        r = rnd.choice(nav.roads)
        mx, mz = nav.f(r + 0x50), nav.f(r + 0x58)
        dx, dz = nav.f(r + 0x60), nav.f(r + 0x68)
        sx, sz = nav.f(r + 0x70), nav.f(r + 0x78)
        w, ln = nav.f(r + 0x3C), nav.f(r + 0x40)
        a = rnd.uniform(-0.6, 0.6) * w
        b = rnd.uniform(-0.6, 0.6) * ln
        return [f32(mx + a * sx + b * dx), y, f32(mz + a * sz + b * dz)]
    if m < 0.6:
        # on a road's box edge, to the float (across or along)
        r = rnd.choice(nav.roads)
        mx, mz = nav.f(r + 0x50), nav.f(r + 0x58)
        dx, dz = nav.f(r + 0x60), nav.f(r + 0x68)
        sx, sz = nav.f(r + 0x70), nav.f(r + 0x78)
        w, ln = nav.f(r + 0x3C), nav.f(r + 0x40)
        if rnd.random() < 0.5:
            a, b = rnd.choice((-0.5, 0.5)) * w, rnd.uniform(-0.45, 0.45) * ln
        else:
            a, b = rnd.uniform(-0.45, 0.45) * w, rnd.choice((-0.5, 0.5)) * ln
        x, z = f32(mx + a * sx + b * dx), f32(mz + a * sz + b * dz)
        return [nextf(x, rnd.randint(-3, 3)), y, nextf(z, rnd.randint(-3, 3))]
    if m < 0.8:
        # on a quadtree node's split or box edge, to the float
        n = rnd.choice(nav.nodes)
        sx, sz = nav.f(n + 0x2C), nav.f(n + 0x30)
        lx, lz, hx, hz = nav.f(n + 0x34), nav.f(n + 0x38), nav.f(n + 0x3C), nav.f(n + 0x40)
        x = rnd.choice((sx, lx, hx, f32(sx - 1e-6), rnd.uniform(lx, hx)))
        z = rnd.choice((sz, lz, hz, f32(sz - 1e-6), rnd.uniform(lz, hz)))
        return [nextf(x, rnd.randint(-2, 2)), y, nextf(z, rnd.randint(-2, 2))]
    if m < 0.97:
        lx, lz, hx, hz = (nav.f(nav.root + k) for k in (0x34, 0x38, 0x3C, 0x40))
        return [f32(rnd.uniform(lx - 50, hx + 50)), y, f32(rnd.uniform(lz - 50, hz + 50))]
    return rnd.choice(([float('nan'), 0.0, 0.0], [0.0, 0.0, float('nan')], [f32(-234.0), f32(21.0), f32(-733.0)], [f32(1772.0), f32(-3.0), f32(90.0)]))


def density_cases(rnd, count):
    out = []
    for _ in range(count):
        g = rnd.choice((0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF, 0, 2, 6, 0x80000000, 0x7FFFFFFF))
        s = rnd.choice((0xFFFFFFFF, 0xFFFFFFFF, 0, 1, 3, 6, 0xFFFFFFFE))
        cut = rnd.choice((0, 0, 0, 1, 0xFF))
        fe = rnd.choice((0, 0, 0, 1, 0x40))
        fen = rnd.choice((0, 3, 4, 6, 0xFFFFFFFF))
        own = rnd.choice((0xFFFFFFFF, 0xFFFFFFFF, 0, 1, 2, 3, 5, 6, 0xFFFFFFFE))
        out.append([g, s, cut, fe, fen, own])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--nav', type=Path, default=Path('D:/MCLA/extracted/cache/resources/city/sc/0x9960fcd8.r66'))
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    ap.add_argument('--dcases', type=int, default=600)
    a = ap.parse_args()
    idx = index(a.recomp)
    nm = {f: addr(a.tool, f) for f in ('F-86195', 'F-44023', 'F-35347', 'F-85897', 'F-84942', 'F-36201')}
    nav = Nav(a.nav)
    rnd = random.Random(86195)
    pts = [point(rnd, nav) for _ in range(a.cases)]
    dc = density_cases(random.Random(85897), a.dcases)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'road_density_inputs.bin'
    inp.write_bytes(b''.join(struct.pack('<3I', *(fb(x) for x in p)) for p in pts))
    dinp = a.out_dir / 'road_density_dinputs.bin'
    dinp.write_bytes(b''.join(struct.pack('<6I', *c) for c in dc))
    # the setters: the native (kind 0) and the race event's fire (kind 1) - point, index, manager there, the event's guard
    srnd = random.Random(84942)
    sc = [[k % 2] + [fb(x) for x in pts[k]] + [srnd.choice((0, 1, 3, 6, 0xFFFFFFFF, 0x7FFFFFFF)), srnd.choice((1, 1, 1, 0)),
          0 if srnd.random() < 0.85 else srnd.choice((1, 0x40000000))] for k in range(min(1200, len(pts)))]
    sinp = a.out_dir / 'road_density_sinputs.bin'
    sinp.write_bytes(b''.join(struct.pack('<7I', *c) for c in sc))
    box = nm['F-35347']
    disp = (r'''static std::vector<uint32_t> road_off;
static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){
 if(a==0x''' + box + r'''){sub_''' + box + r'''(ctx,base);return;}
 throw std::runtime_error("unexpected indirect call");}
static int road_ix(uint32_t p){if((p>>28)!=5)return p?-2:-1;p&=0xFFFFFFF;auto it=std::lower_bound(road_off.begin(),road_off.end(),p);
 return (it!=road_off.end()&&*it==p)?int(it-road_off.begin()):-3;}
''')
    body = (r'''std::ifstream nv(argv[3],std::ios::binary);std::vector<uint8_t> raw((std::istreambuf_iterator<char>(nv)),{});
uint32_t vs=raw[12]|raw[13]<<8|raw[14]<<16|(uint32_t)raw[15]<<24;
resv.assign(raw.begin()+16,raw.begin()+16+vs);resp.assign(raw.begin()+16+vs,raw.end());resv.resize(resv.size()+16);
{size_t o=0;while(o+0x30<=vs){uint32_t vt=resv[o]|resv[o+1]<<8|resv[o+2]<<16|(uint32_t)resv[o+3]<<24;
 if(vt==0x00479f0c){o+=0x70;continue;}
 if(vt==0x0047a13c||vt==0x0047a0c4){if(vt==0x0047a13c)road_off.push_back(o);uint32_t v=0x3800;for(int i=0;i<4;i++)resv[o+i]=v>>(24-8*i);o+=vt==0x0047a13c?0xb0:0xc0;continue;}
 o+=4;}}
auto rd=[&](uint32_t o){return uint32_t(resv[o])<<24|resv[o+1]<<16|resv[o+2]<<8|resv[o+3];};
auto wr=[&](uint32_t o,uint32_t v){for(int k=0;k<4;k++)resv[o+k]=uint8_t(v>>(24-8*k));};
FILE*fi=fopen(argv[2],"rb");std::vector<uint32_t> in;uint32_t w_;while(fread(&w_,4,1,fi)==1)in.push_back(w_);fclose(fi);
for(size_t i=0;i+3<=in.size();i+=3){
 memset(objects,0,0xF000);
 store(0x3800+16,0x''' + box + r''');store(0x3100+8,0x50000000u);
 store(0x3000,in[i]);store(0x3004,in[i+1]);store(0x3008,in[i+2]);
 Context ctx{};ctx.r1.u32=0xFF00;ctx.r13.u32=0x8000;ctx.r3.u32=0x3100;ctx.r4.u32=0x3000;
 sub_''' + nm['F-86195'] + r'''(ctx,nullptr);
 printf("%d |",road_ix(ctx.r3.u32));
 Context c2{};c2.r1.u32=0xFF00;c2.r13.u32=0x8000;c2.r3.u32=rd(0x130);c2.r4.u32=0x3000;c2.r5.u32=0x4000;c2.r6.u32=8;c2.r7.u32=2;
 sub_''' + nm['F-44023'] + r'''(c2,nullptr);
 for(uint32_t k=0;k<c2.r3.u32;k++){uint32_t b=0x4000+48*k;printf(" %d:%08x",road_ix(load(b+32)),load(b+36));}
 printf("\n");}
FILE*fd=fopen(argv[4],"rb");std::vector<uint32_t> dn;while(fread(&w_,4,1,fd)==1)dn.push_back(w_);fclose(fd);
for(size_t i=0;i+6<=dn.size();i+=6){const uint32_t*c=dn.data()+i;
 memset(objects,0,0xF000);
 store(0x8282E930,c[0]);store(0x3200+164,c[1]);*pointer(0x829095FD)=uint8_t(c[2]);*pointer(0x829095FE)=uint8_t(c[3]);
 store(0x8282A7D8,c[4]);store(0x3200+152,c[5]);
 Context ctx{};ctx.r1.u32=0xFF00;ctx.r13.u32=0x8000;ctx.r3.u32=0x3200;
 sub_''' + nm['F-85897'] + r'''(ctx,nullptr);
 printf("D %08x\n",ctx.r3.u32);}
FILE*fs_=fopen(argv[5],"rb");std::vector<uint32_t> sn;while(fread(&w_,4,1,fs_)==1)sn.push_back(w_);fclose(fs_);
for(size_t i=0;i+7<=sn.size();i+=7){const uint32_t*c=sn.data()+i;
 memset(objects,0,0xF000);
 for(uint32_t o:road_off)wr(o+164,0x5A5A5A5Au);
 store(0x3800+16,0x''' + box + r''');store(0x3100+8,0x50000000u);
 store(0x8286D8E4,c[5]?0x3100u-0x64140u:0);store(0x828CCE34,c[6]);
 Context ctx{};ctx.r1.u32=0xFF00;ctx.r13.u32=0x8000;
 if(c[0]==0){store(0x3420,c[1]);store(0x3424,c[2]);store(0x3428,c[3]);store(0x3410,0x3420);store(0x3414,c[4]);store(0x3400+8,0x3410);
  ctx.r3.u32=0x3400;sub_''' + nm['F-84942'] + r'''(ctx,nullptr);}
 else{store(0x3500+16,c[1]);store(0x3500+20,c[2]);store(0x3500+24,c[3]);store(0x3500+32,c[4]);
  ctx.r3.u32=0x3500;sub_''' + nm['F-36201'] + r'''(ctx,nullptr);}
 printf("S");int nw=0;for(size_t r=0;r<road_off.size();r++){uint32_t v=rd(road_off[r]+164);if(v!=0x5A5A5A5Au){printf(" %d %08x",int(r),v);nw++;}}
 if(!nw)printf(" -");printf("\n");}
''')
    main_src = ('#include <algorithm>' + nl + disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    names = ['sub_' + nm[f] for f in ('F-86195', 'F-44023', 'F-35347', 'F-85897', 'F-84942', 'F-36201')]
    exe = build(idx, names, main_src, a.out_dir, 'road_density', vector=True, scratch_end=0x10000)
    r = subprocess.run([str(exe), str(a.image), str(inp), str(a.nav), str(dinp), str(sinp)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[:2000])
    out = [x.strip() for x in r.stdout.split(nl) if x.strip()]
    so = [x[1:].strip() for x in out if x.startswith('S')]
    ro = [x for x in out if not x.startswith('D ') and not x.startswith('S')]
    do = [x[2:] for x in out if x.startswith('D ')]
    assert len(ro) == len(pts) and len(do) == len(dc), (len(ro), len(do))
    lines = ['R ' + ' '.join('%08x' % fb(x) for x in p) + ' -> ' + o for p, o in zip(pts, ro)]
    lines += ['D ' + ' '.join('%08x' % x for x in c) + ' -> ' + o for c, o in zip(dc, do)]
    assert len(so) == len(sc)
    lines += ['S ' + ' '.join('%08x' % x for x in c) + ' -> ' + o for c, o in zip(sc, so)]
    bad = [x for x in lines if any(len(t) == 8 and t.startswith('82') for t in x.replace(':', ' ').split())]
    keep = [x for x in lines if x not in bad]
    (a.out_dir / 'road_density_ref.txt').write_text(nl.join(keep) + nl)
    found = [x for x in keep if x.startswith('R ') and not x.split(' -> ')[1].startswith('-1')]
    print(len(keep), 'lines kept of', len(lines), '|', len(found), 'points on a road,',
          sum(x.split(' |')[1].count(':') > 1 for x in keep if x.startswith('R ')), 'with more than one hit,',
          sum(x.split(' |')[1].count(':') == 8 for x in keep if x.startswith('R ')), 'with 8 hits')


if __name__ == '__main__':
    main()
