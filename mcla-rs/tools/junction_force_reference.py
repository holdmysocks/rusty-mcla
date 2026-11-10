"""The race events' intersection fire F-11036 (`vIntersectionCenter`, `vDirection`, `bGreenElseAllRed`) and its junction lookup
F-56988 run offline: the generated code whole and unchanged, in vector mode, on the real city navsys (cameras2-map5). F-11036 does
nothing while the command-line flag word 0x828CCD08 (`-notriggeredintersections`) is set or the intersection manager (word
0x8286D8E4) is missing; F-56988 asks the quadtree query F-44023 (type mask 4, at most 8; each junction's vfunc 4 is the box test
F-35347) and keeps the hit nearest its middle; a vDirection whose x, y and z are all zero leaves everything; otherwise the junction's
arm of the point F-97162 (the point with y = 0 minus the junction's centre), then the junction's +176 (the arm, or -1) and +180
(all red).

Shim, memory and file reading as tools/road_density_reference.py (imported: navsys virtual block at 0x50000000, physical at
0x60000000, holder at 0x3100, the roads' and junctions' vtable at 0x3800 with F-35347 at +16); the manager word holds
0x3100 - 0x64140; the vector mask the zero test ands with (0x82860950, zero in the static image, set at start) holds what
the running original holds there (read from the recomp's memory: xyz all ones, w 0); every junction's +176 starts as 0x5A5A5A5A and +180 as 0xA5, so the writes show.

Writes <out-dir>/junction_force_ref.txt, one case a line:
  "F <flag> <manager> <green byte> <centre x y z> <point x y z> -> <junction of F-56988> | <writes>"  (words hex; junctions as
  indices into the file's junction array root +58, -1 none); writes "<junction> <+176> <+180>" for every junction whose words
  changed, or "-". No word reads as an address of the image (cases that would hold one are left out)."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import road_density_reference as rdr
from road_density_reference import addr, fb, f32, nextf, nl
from recomp_closure import index, build

# (road_density_reference patches the shims for an older recomp_closure whose shims took no argument: keep its file memory
# mapping, take the current vector shim, and make its flush-mode calls set the SSE flush bits as that tool does)
recomp_closure.shim = lambda scratch_end=0x10000: rdr.patched_shim()


def _vector_shim(scratch_end=0x10000):
    s = rdr._vshim(scratch_end)
    fp = ('struct { void disableFlushMode() {} void enableFlushMode() {} void disableFlushModeUnconditional() {}'
          ' void enableFlushModeUnconditional() {} } fpscr;')
    on, off = '{_mm_setcsr(_mm_getcsr()|0x8040);}', '{_mm_setcsr(_mm_getcsr()&~0x8040u);}'
    if s.count(fp) == 1:
        s = s.replace(fp, 'struct { void disableFlushMode() %s void enableFlushMode() %s void disableFlushModeUnconditional() %s'
                      ' void enableFlushModeUnconditional() %s } fpscr;' % (off, on, off, on), 1)
    return '#include <xmmintrin.h>' + nl + s


recomp_closure.vector_shim = _vector_shim
# F-97162's vrlimi128 / vcuxwfp128, as tools/arm_towards_reference.py defines them (the SDK's intrinsics.h)
recomp_closure.EXTRA += (
    '#define simde_mm_permute_ps(a,imm) simde_mm_shuffle_ps(a,a,imm)\n'
    'namespace rex { namespace ppc {\n'
    'inline simde__m128 simde_mm_cvtepu32_ps_(simde__m128i src1) {\n'
    '  simde__m128i xmm1 = simde_mm_add_epi32(src1, simde_mm_set1_epi32(127));\n'
    '  simde__m128i xmm0 = simde_mm_slli_epi32(src1, 31 - 8);\n'
    '  xmm0 = simde_mm_srli_epi32(xmm0, 31);\n'
    '  xmm0 = simde_mm_add_epi32(xmm0, xmm1);\n'
    '  xmm0 = simde_mm_srai_epi32(xmm0, 8);\n'
    '  xmm0 = simde_mm_add_epi32(xmm0, simde_mm_set1_epi32(0x4F800000));\n'
    '  simde__m128 xmm2 = simde_mm_cvtepi32_ps(src1);\n'
    '  return simde_mm_blendv_ps(xmm2, simde_mm_castsi128_ps(xmm0), simde_mm_castsi128_ps(src1));\n'
    '} } }\n')


class Nav(rdr.Nav):
    """The junctions' boxes, centres and arms, read from the navsys file (to make the cases)."""

    def __init__(self, path):
        super().__init__(path)
        o, vs, self.junctions = 0, len(self.d), []
        while o + 0x30 <= vs:
            vt = struct.unpack('<I', self.d[o:o + 4])[0]
            if vt == 0x00479F0C:
                o += 0x70
            elif vt == 0x0047A13C:
                o += 0xB0
            elif vt == 0x0047A0C4:
                self.junctions.append(o)
                o += 0xC0
            else:
                o += 4

    def vec(self, o):
        return [self.f(o), self.f(o + 4), self.f(o + 8)]


def centre_case(rnd, nav):
    j = rnd.choice(nav.junctions)
    m = rnd.random()
    c = nav.vec(j + 0x80)
    y = rnd.choice((c[1], c[1], 0.0, f32(c[1] + rnd.uniform(-20, 20))))
    if m < 0.35:
        return [c[0], y, c[2]]
    mx, mz = nav.f(j + 0x50), nav.f(j + 0x58)
    dx, dz = nav.f(j + 0x60), nav.f(j + 0x68)
    sx, sz = nav.f(j + 0x70), nav.f(j + 0x78)
    w, ln = nav.f(j + 0x3C), nav.f(j + 0x40)
    if m < 0.7:
        a, b = rnd.uniform(-0.6, 0.6) * w, rnd.uniform(-0.6, 0.6) * ln
        return [f32(mx + a * sx + b * dx), y, f32(mz + a * sz + b * dz)]
    if m < 0.9:
        if rnd.random() < 0.5:
            a, b = rnd.choice((-0.5, 0.5)) * w, rnd.uniform(-0.45, 0.45) * ln
        else:
            a, b = rnd.uniform(-0.45, 0.45) * w, rnd.choice((-0.5, 0.5)) * ln
        x, z = f32(mx + a * sx + b * dx), f32(mz + a * sz + b * dz)
        return [nextf(x, rnd.randint(-3, 3)), y, nextf(z, rnd.randint(-3, 3))]
    lx, lz, hx, hz = (nav.f(nav.root + k) for k in (0x34, 0x38, 0x3C, 0x40))
    return [f32(rnd.uniform(lx, hx)), y, f32(rnd.uniform(lz, hz))]


def point_case(rnd, nav, centre):
    m = rnd.random()
    if m < 0.06:
        return rnd.choice(([0.0, 0.0, 0.0], [-0.0, 0.0, 0.0], [0.0, -0.0, -0.0], [0.0, 0.0, 1e-30], [0.0, 1.0, 0.0], [float('nan'), 0.0, 0.0]))
    # the junction nearest the centre (by its +80) and its arms, to put points on and between them
    j = min(nav.junctions, key=lambda o: (nav.f(o + 0x80) - centre[0]) ** 2 + (nav.f(o + 0x88) - centre[2]) ** 2)
    c = nav.vec(j + 0x80)
    n = nav.u32(j + 0x30) >> 16
    arms = []
    ap = nav.u32(j + 0x2C)
    if ap >> 28 == 5:
        arms = [nav.vec((ap & 0xFFFFFFF) + 16 * k) for k in range(min(n, 16))]
    r = rnd.choice((1.0, 5.0, 20.0, 60.0, rnd.uniform(0.5, 150)))
    y = rnd.choice((c[1], 0.0, f32(c[1] + rnd.uniform(-10, 10))))
    if arms and m < 0.55:
        a = rnd.choice(arms)
        if m < 0.3 and len(arms) > 1:
            b = rnd.choice(arms)
            a = [a[0] + b[0], 0.0, a[2] + b[2]]
        ln = math.hypot(a[0], a[2]) or 1.0
        x, z = f32(c[0] + a[0] / ln * r), f32(c[2] + a[2] / ln * r)
        return [nextf(x, rnd.randint(-2, 2)), y, nextf(z, rnd.randint(-2, 2))]
    ang = rnd.uniform(0, 2 * math.pi)
    return [f32(c[0] + math.sin(ang) * r), y, f32(c[2] + math.cos(ang) * r)]


DISPATCH = r'''static std::vector<uint32_t> jun_off;
static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){
 if(a==0xBOX){sub_BOX(ctx,base);return;}
 throw std::runtime_error("unexpected indirect call");}
'''

BODY = r'''std::ifstream nv(argv[3],std::ios::binary);std::vector<uint8_t> raw((std::istreambuf_iterator<char>(nv)),{});
uint32_t vs=raw[12]|raw[13]<<8|raw[14]<<16|(uint32_t)raw[15]<<24;
resv.assign(raw.begin()+16,raw.begin()+16+vs);resp.assign(raw.begin()+16+vs,raw.end());resv.resize(resv.size()+16);
{size_t o=0;while(o+0x30<=vs){uint32_t vt=resv[o]|resv[o+1]<<8|resv[o+2]<<16|(uint32_t)resv[o+3]<<24;
 if(vt==0x00479f0c){o+=0x70;continue;}
 if(vt==0x0047a13c||vt==0x0047a0c4){if(vt==0x0047a0c4)jun_off.push_back(o);uint32_t v=0x3800;for(int i=0;i<4;i++)resv[o+i]=v>>(24-8*i);o+=vt==0x0047a13c?0xb0:0xc0;continue;}
 o+=4;}}
auto rd=[&](uint32_t o){return uint32_t(resv[o])<<24|resv[o+1]<<16|resv[o+2]<<8|resv[o+3];};
auto wr=[&](uint32_t o,uint32_t v){for(int k=0;k<4;k++)resv[o+k]=uint8_t(v>>(24-8*k));};
uint32_t jarr=rd(0x58)&0xFFFFFFF;
auto jix=[&](uint32_t p)->long{if(!p)return -1;if((p>>28)!=5)return -2;p&=0xFFFFFFF;if(p<jarr||(p-jarr)%0xc0)return -3;return long((p-jarr)/0xc0);};
FILE*fi=fopen(argv[2],"rb");std::vector<uint32_t> in;uint32_t w_;while(fread(&w_,4,1,fi)==1)in.push_back(w_);fclose(fi);
for(size_t i=0;i+9<=in.size();i+=9){const uint32_t*c=in.data()+i;
 memset(objects,0,0xF000);
 for(uint32_t o:jun_off){wr(o+176,0x5A5A5A5Au);resv[o+180]=0xA5;}
 store(0x3800+16,0xBOX);store(0x3100+8,0x50000000u);
 store(0x8286D8E4,c[1]?0x3100u-0x64140u:0);store(0x828CCD08,c[0]);
 for(int k=0;k<3;k++)store(0x82860950+4*k,0xFFFFFFFFu);store(0x8286095C,0);
 *pointer(0x3500+16)=uint8_t(c[2]);for(int k=0;k<3;k++){store(0x3500+32+4*k,c[3+k]);store(0x3500+48+4*k,c[6+k]);}
 store(0x3500+44,0);store(0x3500+60,0);
 Context q{};q.r1.u32=0xFF00;q.r13.u32=0x8000;q.r3.u32=0x3100;q.r4.u32=0x3500+32;
 sub_LOOKUP(q,nullptr);
 printf("%ld |",jix(q.r3.u32));
 Context ctx{};ctx.r1.u32=0xFF00;ctx.r13.u32=0x8000;ctx.r3.u32=0x3500;
 sub_FIRE(ctx,nullptr);
 int nw=0;for(uint32_t o:jun_off){uint32_t v=rd(o+176);uint8_t b=resv[o+180];if(v!=0x5A5A5A5Au||b!=0xA5){printf(" %ld %08x %02x",jix(0x50000000u|o),v,b);nw++;}}
 if(!nw)printf(" -");printf("\n");}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--nav', type=Path, default=Path('D:/MCLA/extracted/cache/resources/city/sc/0x9960fcd8.r66'))
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    idx = index(a.recomp)
    nm = {f: addr(a.tool, f) for f in ('F-11036', 'F-56988', 'F-44023', 'F-35347', 'F-97162')}
    nav = Nav(a.nav)
    rnd = random.Random(11036)
    cases = []
    for _ in range(a.cases):
        c = centre_case(rnd, nav)
        p = point_case(rnd, nav, c)
        flag = 0 if rnd.random() < 0.97 else rnd.choice((1, 0x10000))
        mgr = 1 if rnd.random() < 0.98 else 0
        green = rnd.choice((0, 1, 1, 0x80))
        cases.append([flag, mgr, green] + [fb(x) for x in c] + [fb(x) for x in p])
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'junction_force_inputs.bin'
    inp.write_bytes(b''.join(struct.pack('<9I', *c) for c in cases))
    disp = DISPATCH.replace('BOX', nm['F-35347'])
    body = BODY.replace('BOX', nm['F-35347']).replace('sub_LOOKUP', 'sub_' + nm['F-56988']).replace('sub_FIRE', 'sub_' + nm['F-11036'])
    main_src = ('#include <algorithm>' + nl + disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    names = ['sub_' + nm[f] for f in ('F-11036', 'F-56988', 'F-44023', 'F-35347', 'F-97162')]
    exe = build(idx, names, main_src, a.out_dir, 'junction_force', vector=True, scratch_end=0x10000)
    r = subprocess.run([str(exe), str(a.image), str(inp), str(a.nav)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[:2000])
    out = [x.strip() for x in r.stdout.split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['F ' + ' '.join('%08x' % x for x in c) + ' -> ' + o for c, o in zip(cases, out)]
    keep = [x for x in lines if not any(len(t) == 8 and t.startswith('82') for t in x.split())]
    (a.out_dir / 'junction_force_ref.txt').write_text(nl.join(keep) + nl)
    hit = sum(not x.split(' -> ')[1].startswith('-1') for x in keep)
    wrote = sum(not x.endswith(' -') for x in keep)
    print(len(keep), 'cases kept of', len(lines), '|', hit, 'with a junction,', wrote, 'with a write')


if __name__ == '__main__':
    main()
