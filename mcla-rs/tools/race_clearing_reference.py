"""The race traffic clearing (F-20685) run offline on the real city navsys: the generated code whole and unchanged, in
vector mode, with its callees as they are - the lane counter and getter (F-95725, F-12622, F-61293), the lane-leads-
to-a-record test F-91078 with the lane's record F-17030 - and the lane / curve slots the code calls dispatched to the
original's own functions (slot 15 F-58682, slot 12 F-88700, slot 13 F-93886, the curve's point F-48295). Slot 19 of a
lane (F-88133, the byte +104 store) prints the lane and the value.

Memory: the navsys file's virtual block at 0x50000000 and physical block at 0x60000000 (its own pointers), the lanes
given the run-time vtables (mcRailAmbient 0x820CD154, curve 0x8202B390), the vertex array global set to the file's
root +0x2c and the start-up xyz mask (0x82860950) filled as the running game has it. The railnet holder at 0x3100
(+8 the root, byte +69 from the case), the ambient manager pointer [0x8286D8E4] set so that +0x64140 is the holder,
S (r4) so that its byte +0x64251 is the case's, the -notrafficclearing word (0x828CCAD0) from the case, the tracker at
0x4000 (+2064 / +2080 the previous set, +2107 / +2108 bytes, +5872 / +5904 the records with their junctions +5888 /
+5920).

Writes <out-dir>/race_clearing_ref.txt: "<case words> -> <writes> N <count> <roads>". Case words (hex): switch, +2107,
S byte, +2108, railnet +69, record 0 direction x y z (float bits) and junction index (ffffffff none), record 1 the
same, the previous count and four road indices (ffffffff unused). Writes: "T <lane> <value>" per slot 19 call in
order; N: the set after the run (road indices). Lanes, roads and junctions are numbered as the port's NavSys numbers
them (lanes and roads in the file's object order, junctions by root +0x58)."""
import argparse
import math
import random
import struct
import subprocess
from fractions import Fraction
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build

LANE_VT, CURVE_VT = 0x820CD154, 0x8202B390


def patched_shim():
    s = recomp_leaf.shim(0x10000)
    old = 'uint64_t o=uint64_t(a)-0x82000000ull;'
    assert s.count(old) == 1
    s = s.replace(old, 'if ((a>>28)==5 && (a&0xFFFFFFF)+4<=resv.size()) return resv.data()+(a&0xFFFFFFF);\n'
                       '    if ((a>>28)==6 && (a&0xFFFFFFF)+4<=resp.size()) return resp.data()+(a&0xFFFFFFF);\n    ' + old, 1)
    s = s.replace('static std::vector<uint8_t> image;', 'static std::vector<uint8_t> image;\nstatic std::vector<uint8_t> resv, resp;', 1)
    # (extsh reads a register's low half signed)
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
NONE = 0xFFFFFFFF
CASE = 18


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def fround(q):
    return f32(float(q))


def dot(d, a):
    # fmuls a.y d.y, fmadds d.z a.z +, fmadds d.x a.x +
    f9 = fround(Fraction(a[1]) * Fraction(d[1]))
    f6 = fround(Fraction(d[2]) * Fraction(a[2]) + Fraction(f9))
    return fround(Fraction(d[0]) * Fraction(a[0]) + Fraction(f6))


def nextf(x, k):
    b = fb(x)
    if x >= 0:
        b += k
    else:
        b -= k
    return struct.unpack('>f', struct.pack('>I', b & 0xFFFFFFFF))[0]


class Nav:
    """The parts of the navsys file the cases are made from."""

    def __init__(self, path):
        raw = path.read_bytes()
        vs = struct.unpack('<I', raw[12:16])[0]
        self.d = raw[16:16 + vs]
        self.p = raw[16 + vs:]
        self.vs = vs
        lanes, roads, o = [], [], 0
        while o + 0x30 <= vs:
            vt = struct.unpack('<I', self.d[o:o + 4])[0]
            if vt == 0x00479F0C:
                lanes.append(o)
                o += 0x70
                continue
            if vt == 0x0047A13C:
                roads.append(o)
                o += 0xB0
                continue
            if vt == 0x0047A0C4:
                o += 0xC0
                continue
            o += 4
        self.lanes, self.roads = lanes, roads
        self.road_ix = {r: i for i, r in enumerate(roads)}
        self.jarr, self.jn = self.ptr(0x58), self.u32(0xC8)
        conn = self.ptr(0x3C)
        self.items, self.arms = [], []
        for j in range(self.jn):
            J = self.jarr + 0xC0 * j
            e = conn + 16 * self.u32(J + 0x20)
            a = self.ptr(e)
            a8 = self.ptr(e + 8)
            n = self.u16(e + 4)
            self.items.append([(self.road_ix.get(self.ptr(a + 4 * k), -1), struct.unpack('>i', self.d[a8 + 4 * k:a8 + 4 * k + 4])[0] if a8 else -1)
                               for k in range(n)] if a else [])
            d0 = self.ptr(J + 0x2C)
            self.arms.append([(self.f(d0 + 16 * k), self.f(d0 + 16 * k + 4), self.f(d0 + 16 * k + 8)) for k in range(self.u16(J + 0x30))] if d0 else [])
        gv = self.ptr(0x2C)
        self.jlane_dirs = []
        for j in range(self.jn):
            out = []
            self.walk(self.jarr + 0xC0 * j, False, out)
            dirs = []
            for lo in out:
                c = self.u16(lo + 0x4C) + 1
                arr = self.ptr(lo + 0x50)
                vi = [self.u16(arr + 2 * k) for k in range(c)]
                if len(vi) < 2:
                    continue
                for p, q in ((vi[0], vi[1]), (vi[-2], vi[-1])):
                    P = [self.f(gv + 16 * p + 4 * k) for k in range(3)]
                    Q = [self.f(gv + 16 * q + 4 * k) for k in range(3)]
                    dirs.append([f32(Q[k] - P[k]) for k in range(3)])
            self.jlane_dirs.append(dirs)

    def u32(self, o):
        return struct.unpack('>I', self.d[o:o + 4])[0]

    def u16(self, o):
        return struct.unpack('>H', self.d[o:o + 2])[0]

    def f(self, o):
        return struct.unpack('>f', self.d[o:o + 4])[0]

    def ptr(self, o):
        p = self.u32(o)
        return (p & 0xFFFFFFF) if p >> 28 == 5 else None

    def walk(self, o, below, out):
        n = o
        while n is not None:
            a = self.ptr(n + 0x14)
            if a is not None:
                out += [self.ptr(a + 4 * k) for k in range(self.u16(n + 0x18))]
            if below and self.ptr(n + 0x10) is not None:
                self.walk(self.ptr(n + 0x10), True, out)
            n = self.ptr(n + 0x0C)
            below = True


def unit(v):
    n = math.sqrt(sum(x * x for x in v)) or 1.0
    return [f32(x / n) for x in v]


def direction(rnd, nav, j):
    arms = nav.arms[j]
    used = sorted({a for _, a in nav.items[j] if a >= 0})
    m = rnd.random()
    if m < 0.25 and nav.jlane_dirs[j]:
        # a junction lane's first or last span, as F-95137 records it (with the Newton step left to the port)
        return unit(rnd.choice(nav.jlane_dirs[j])), 'lane'
    if m < 0.5 and arms:
        a = rnd.choice(arms)
        t = rnd.choice((0.0, rnd.uniform(-0.3, 0.3)))
        s = rnd.choice((1.0, -1.0))
        v = [s * (a[0] * math.cos(t) - a[2] * math.sin(t)), rnd.choice((0.0, a[1], rnd.uniform(-0.05, 0.05))), s * (a[0] * math.sin(t) + a[2] * math.cos(t))]
        return unit(v), 'arm'
    if m < 0.8 and used:
        # about the cut-off for one arm: |dot| = 0.643 to the float, and the floats round it
        a = arms[rnd.choice(used)]
        c = f32(0.643)
        th = math.acos(c) * rnd.choice((1, -1)) + (math.pi if rnd.random() < 0.5 else 0.0)
        v = unit([a[0] * math.cos(th) - a[2] * math.sin(th), rnd.choice((0.0, 0.0, rnd.uniform(-0.02, 0.02))), a[0] * math.sin(th) + a[2] * math.cos(th)])
        best = None
        for kx in range(-6, 7):
            for kz in range(-6, 7):
                w = [nextf(v[0], kx), v[1], nextf(v[2], kz)]
                e = abs(dot(w, a)) - c
                if best is None or abs(e) < abs(best[0]):
                    best = (e, w)
                if e == 0:
                    break
        w = best[1]
        k = rnd.choice((0, 0, 0, 1, -1, 2, -2))
        if k:
            w = [nextf(w[0], k), w[1], w[2]]
        return w, 'edge'
    if m < 0.95:
        return unit([rnd.uniform(-1, 1), rnd.uniform(-0.3, 0.3), rnd.uniform(-1, 1)]), 'random'
    return rnd.choice(([0.0, 0.0, 0.0], [float('nan'), 0.0, 0.0], [float('inf'), 0.0, 1.0], [1e-30, 0.0, -1e-30])), 'odd'


def cases(rnd, nav, count):
    out = []
    for _ in range(count):
        sw = 0 if rnd.random() < 0.93 else rnd.choice((1, 0x80000000, 0xFFFFFFFF))
        b2107 = 0 if rnd.random() < 0.9 else rnd.choice((1, 0x80, 0xFF))
        sb = rnd.choice((1, 1, 1, 1, 0x40, 0))
        b2108 = 1 if rnd.random() < 0.9 else rnd.choice((0, 0xFF))
        rn = 1 if rnd.random() < 0.85 else 0
        j0 = rnd.randrange(nav.jn) if rnd.random() < 0.92 else None
        d0 = direction(rnd, nav, j0)[0] if j0 is not None else unit([rnd.uniform(-1, 1), 0.0, rnd.uniform(-1, 1)])
        j1 = None
        if j0 is not None and rnd.random() < 0.15:
            room = 8 - len(nav.items[j0])
            ok = [j for j in range(nav.jn) if len(nav.items[j]) <= room]
            if ok:
                j1 = rnd.choice(ok + [j0] * (len(nav.items[j0]) <= room))
        d1 = direction(rnd, nav, j1)[0] if j1 is not None else [0.0, 0.0, 0.0]
        pool = [r for j in (j0, j1) if j is not None for r, _ in nav.items[j] if r >= 0]
        n = rnd.choice((0, 0, 1, 2, 2, 3, 4, 4))
        prev = []
        for _k in range(n):
            r = rnd.choice(pool) if pool and rnd.random() < 0.75 else rnd.randrange(len(nav.roads))
            if r not in prev:
                prev.append(r)
        words = [sw, b2107, sb, b2108, rn] + [fb(x) for x in d0] + [NONE if j0 is None else j0] + [fb(x) for x in d1] + \
                [NONE if j1 is None else j1, len(prev)] + prev + [NONE] * (4 - len(prev))
        assert len(words) == CASE
        out.append(words)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--nav', type=Path, default=Path('D:/MCLA/extracted/cache/resources/city/sc/0x9960fcd8.r66'))
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    idx = index(a.recomp)
    nm = {f: addr(a.tool, f) for f in ('F-20685', 'F-58682', 'F-88700', 'F-93886', 'F-48295', 'F-88133')}
    nav = Nav(a.nav)
    rows = cases(random.Random(20685), nav, a.cases)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'race_clearing_inputs.bin'
    inp.write_bytes(b''.join(struct.pack('<%dI' % len(r), *r) for r in rows))
    slot = lambda f: '0x' + nm[f]
    disp = (r'''static std::vector<uint32_t> lane_off, road_off;
static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){
 if(a==''' + slot('F-58682') + r'''){sub_''' + nm['F-58682'] + r'''(ctx,base);return;}
 if(a==''' + slot('F-88700') + r'''){sub_''' + nm['F-88700'] + r'''(ctx,base);return;}
 if(a==''' + slot('F-93886') + r'''){sub_''' + nm['F-93886'] + r'''(ctx,base);return;}
 if(a==''' + slot('F-48295') + r'''){sub_''' + nm['F-48295'] + r'''(ctx,base);return;}
 if(a==''' + slot('F-88133') + r'''){uint32_t o=ctx.r3.u32-0x50000000u;auto it=std::lower_bound(lane_off.begin(),lane_off.end(),o);
  if(it==lane_off.end()||*it!=o)throw std::runtime_error("slot 19 on a non-lane");
  printf(" T %d %d",int(it-lane_off.begin()),int(ctx.r4.u32&0xFF));sub_''' + nm['F-88133'] + r'''(ctx,base);return;}
 throw std::runtime_error("unexpected indirect call");}
''')
    body = (r'''std::ifstream nv(argv[3],std::ios::binary);std::vector<uint8_t> raw((std::istreambuf_iterator<char>(nv)),{});
uint32_t vs=raw[12]|raw[13]<<8|raw[14]<<16|(uint32_t)raw[15]<<24;
resv.assign(raw.begin()+16,raw.begin()+16+vs);resp.assign(raw.begin()+16+vs,raw.end());resv.resize(resv.size()+16);
{size_t o=0;while(o+0x30<=vs){uint32_t vt=resv[o]|resv[o+1]<<8|resv[o+2]<<16|(uint32_t)resv[o+3]<<24;
 if(vt==0x00479f0c){lane_off.push_back(o);uint32_t v1=LANEVT,v2=CURVEVT;for(int i=0;i<4;i++){resv[o+i]=v1>>(24-8*i);resv[o+0x30+i]=v2>>(24-8*i);}o+=0x70;continue;}
 if(vt==0x0047a13c){road_off.push_back(o);o+=0xb0;continue;}
 if(vt==0x0047a0c4){o+=0xc0;continue;}
 o+=4;}}
auto rd=[&](uint32_t o){return uint32_t(resv[o])<<24|resv[o+1]<<16|resv[o+2]<<8|resv[o+3];};
uint32_t jarr=rd(0x58)&0xFFFFFFF;
store(0x828DA6B4,rd(0x2c));
store(0x82860950,0xFFFFFFFF);store(0x82860954,0xFFFFFFFF);store(0x82860958,0xFFFFFFFF);store(0x8286095C,0);
store(0x8286D8E4,0x3100u-0x64140u);
FILE*fi=fopen(argv[2],"rb");std::vector<uint32_t> in;uint32_t w_;while(fread(&w_,4,1,fi)==1)in.push_back(w_);fclose(fi);
// (the lanes' byte +104 as the file has it, restored before each case)
std::vector<uint8_t> b104(lane_off.size());for(size_t l=0;l<lane_off.size();l++)b104[l]=resv[lane_off[l]+104];
for(size_t i=0;i+18<=in.size();i+=18){const uint32_t*c=in.data()+i;
 memset(objects,0,0xF000);
 for(size_t l=0;l<lane_off.size();l++)resv[lane_off[l]+104]=b104[l];
 store(0x828CCAD0,c[0]);
 const uint32_t M=0x4000,H=0x3100;
 store(H+8,0x50000000u);*pointer(H+69)=uint8_t(c[4]);
 *pointer(0x3000)=uint8_t(c[2]);
 *pointer(M+2107)=uint8_t(c[1]);*pointer(M+2108)=uint8_t(c[3]);
 for(int r=0;r<2;r++){uint32_t b=M+5872+32*r;store(b,c[5+4*r]);store(b+4,c[6+4*r]);store(b+8,c[7+4*r]);
  store(b+16,c[8+4*r]==0xFFFFFFFFu?0:0x50000000u+jarr+0xc0*c[8+4*r]);}
 store(M+2080,c[13]);for(int k=0;k<4;k++)store(M+2064+4*k,c[14+k]==0xFFFFFFFFu?0:0x50000000u+road_off[c[14+k]]);
 Context ctx{};ctx.r1.u32=0xFF00;ctx.r13.u32=0x8000;ctx.r3.u32=M;ctx.r4.u32=0x3000u-0x64251u;
 sub_''' + nm['F-20685'] + r'''(ctx,nullptr);
 uint32_t n=load(M+2080);printf(" N %u",n);
 for(uint32_t k=0;k<n&&k<4;k++){uint32_t p=load(M+2064+4*k)-0x50000000u;auto it=std::lower_bound(road_off.begin(),road_off.end(),p);
  if(it!=road_off.end()&&*it==p)printf(" %d",int(it-road_off.begin()));else printf(" X%08x",p);}
 printf("\n");}
''').replace('LANEVT', hex(LANE_VT)).replace('CURVEVT', hex(CURVE_VT))
    main_src = ('#include <algorithm>' + nl + disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    names = ['sub_' + nm[f] for f in ('F-20685', 'F-58682', 'F-88700', 'F-93886', 'F-48295', 'F-88133')]
    exe = build(idx, names, main_src, a.out_dir, 'race_clearing', vector=True, scratch_end=0x10000)
    r = subprocess.run([str(exe), str(a.image), str(inp), str(a.nav)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[:2000])
    out = [x.strip() for x in r.stdout.split(nl) if x.strip()]
    assert len(out) == len(rows), (len(out), len(rows))
    lines = [' '.join('%08x' % x for x in row) + ' -> ' + o for row, o in zip(rows, out)]
    # (a run that leaves more than 4 in the set has written a road over its count, +2080 - the original's own
    # overflow, which the city's junction lanes never reach: left out; no word that reads as an address of the image)
    over = [x for x in lines if int(x.split(' N ')[1].split()[0]) > 4]
    keep = [x for x in lines if x not in over and not any(len(t) == 8 and t.startswith('82') for t in x.split())]
    (a.out_dir / 'race_clearing_ref.txt').write_text(nl.join(keep) + nl)
    print(len(keep), 'cases kept of', len(lines), '|', len(over), 'overflowing the set,',
          sum(' T ' in x and ' 1' in x.split(' N ')[0] for x in keep), 'with lanes set,',
          sum(x.count(' T ') for x in keep), 'slot 19 calls,',
          sum(x.split(' -> ')[1].split(' N ')[0].count(' 0') > 0 for x in keep), 'with lanes cleared')


if __name__ == '__main__':
    main()
