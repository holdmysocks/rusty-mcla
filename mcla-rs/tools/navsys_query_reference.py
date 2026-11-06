"""Run the original's lane quadtree query F-58446 (with its node box test F-80185 and the lanes' own sphere test F-56243, called
through the lane class's vtable slot 5) offline on the city's real navsys resource, and record the answers for the port's test
(crates/formats/src/testdata/navsys_query_ref.txt; test navsys::tests::lanes_near_matches_the_original, needs MCLA_DATA_DIR).

The resource (resources/city/sc/0x9960fcd8.r66, 16-byte header, then the virtual block) is copied to guest memory at the
virtual block's own base (its pointers are 0x5....... there), so the tree is walked as the game walks it. The quadtree is the
navsys object's +304 (what the wrapper F-53328 passes). Each node's items get a vtable: a lane (the port's lane class code at
+0) the image's lane class vtable (the one whose slot 5 is F-56243), roads / junctions a table whose slot 5 throws (the masks
used here never select them - their flags are 2 / 4 / 16 / 32).

Queries: points in the root's box (y -20 .. 80), 40 % placed 0 - 3 m off a random lane's box corner, radius 0.25 - 150 m,
mask 1 / 0x2000 / 0x2001, max 100000 / 128 / 48 / 3. Line per query:
  <x> <y> <z> <w> (f32 bits as signed decimal) <mask> <max> <count> <fnv1a-32 of the result offsets (u32 LE bytes) in order, decimal> <first <= 6 offsets>
"""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp
import recomp_closure

# (the vector shim lacks the vector temporary's type and the condition register's setFromMask / so, as drawn_steer_reference.py
# adds them; built with -msse4.1 so the dot product is the native one, as in the game)
_vshim = recomp_closure.vector_shim


def _vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    return s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                     'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)


recomp_closure.vector_shim = _vector_shim
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)

QUERIES = 800
RES_BASE = 0x5 << 28
LINK_VT = 0x00479F0C
IMG = 0x82 << 24     # (the image base)

MAIN = r'''
static uint32_t bits(float f){uint32_t u;memcpy(&u,&f,4);return u;}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream rf(argv[2],std::ios::binary); std::vector<uint8_t> raw((std::istreambuf_iterator<char>(rf)),{});
 rt_reset(); memcpy(resmem,raw.data()+16,raw.size()-16);
 const uint32_t LVT=strtoul(argv[4],0,16), BADVT=0x40E00000u;
 for(int i=0;i<24;i++) store(BADVT+4*i,0x7F000020u);
 const uint32_t ROOT=load(RESB+304);
 // every node's items: lanes get the lane class's vtable, the rest the throwing one
 std::vector<uint32_t> st{ROOT}; int nodes=0, items=0;
 while(!st.empty()){ uint32_t n=st.back(); st.pop_back(); nodes++;
  uint32_t a=load(n+4); uint32_t cnt=load(n+8)>>16;
  for(uint32_t k=0;k<cnt;k++){ uint32_t it=load(a+4*k); if(!it) continue; uint32_t w=load(it); uint32_t le=__builtin_bswap32(w);
   store(it, le==LINKVT ? LVT : BADVT); items++; }
  for(int k=0;k<4;k++){ uint32_t c=load(n+28+4*k); if(c) st.push_back(c); } }
 fprintf(stderr,"nodes %d items %d\n",nodes,items);
 std::ifstream in(argv[3]); uint32_t xb,yb,zb,wb,mask; int mx; const uint32_t P=0x40D00000u, OUT=0x40100000u;
 while(in>>std::hex>>xb>>yb>>zb>>wb>>mask>>std::dec>>mx){
  store(P,xb); store(P+4,yb); store(P+8,zb); store(P+12,wb);
  Context c=fresh(); c.r3.u32=ROOT; c.r4.u32=P; c.r5.u32=OUT; c.r6.u32=(uint32_t)mx; c.r7.u32=mask; QUERY(c,nullptr);
  int n=(int)c.r3.u32; uint32_t h=2166136261u;
  for(int i=0;i<n;i++){ uint32_t o=load(OUT+4*i)-RESB; for(int b=0;b<4;b++){ h=(h^((o>>(8*b))&0xffu))*16777619u; } }
  printf("%d %d %d %d %x %d %d %u",(int)xb,(int)yb,(int)zb,(int)wb,mask,mx,n,h);
  for(int i=0;i<n&&i<6;i++) printf(" %x",load(OUT+4*i)-RESB);
  printf("\n"); }
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--data', type=Path, required=True, help='extracted/cache')
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-58446', 'F-80185', 'F-56243']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    img = rdisp.load_image(a.image)
    # the lane class: the vtable whose slot 5 is the sphere test
    want = struct.pack('>I', addr['F-56243'])
    hits = [i - 20 for i in range(0, len(img) - 3, 4) if img[i:i + 4] == want]
    lvt = [IMG + h for h in hits if rdisp.image_u32(img, IMG + h + 4) >> 24 == 0x82]
    assert len(lvt) >= 1, hits
    lvt = lvt[0]
    res = (a.data / 'resources/city/sc/0x9960fcd8.r66').read_bytes()
    d = res[16:]
    u32 = lambda o: struct.unpack_from('>I', d, o)[0]
    f32 = lambda o: struct.unpack_from('>f', d, o)[0]
    root = u32(304) & 0x0FFFFFFF
    x0, z0, x1, z1 = f32(root + 52), f32(root + 56), f32(root + 60), f32(root + 64)
    lanes = [o for o in range(0, len(d) - 0x30, 4) if struct.unpack_from('<I', d, o)[0] == LINK_VT]
    rnd = random.Random(20261106)
    fb = lambda v: struct.unpack('<I', struct.pack('<f', v))[0]
    lines = []
    for q in range(QUERIES):
        if rnd.random() < 0.4:
            o = rnd.choice(lanes)
            c = [f32(o + 16 + 4 * i) if rnd.random() < 0.5 else f32(o + 32 + 4 * i) for i in range(3)]
            p = [c[i] + rnd.uniform(-3, 3) for i in range(3)]
        else:
            p = [rnd.uniform(x0, x1), rnd.uniform(-20, 80), rnd.uniform(z0, z1)]
        w = rnd.choice([0.25, 1.0, 3.0, 6.0, 15.0, 30.0, 60.0, 150.0]) * rnd.uniform(0.9, 1.1)
        mask = rnd.choice([1, 1, 1, 0x2000, 0x2001])
        mx = rnd.choice([100000, 128, 48, 3])
        lines.append('%08x %08x %08x %08x %x %d' % (fb(p[0]), fb(p[1]), fb(p[2]), fb(w), mask, mx))
    a.out.mkdir(parents=True, exist_ok=True)
    cf = a.out / 'navsys_queries.txt'
    cf.write_text('\n'.join(lines) + '\n')
    orig_shim = rdisp.vector_shim
    rdisp.vector_shim = lambda: (orig_shim()
                                 .replace('static uint8_t* pointer(uint32_t a) {', 'static uint8_t* pointer(uint32_t a) {\n    if (a>=RESB && a<RESB+0x400000u) return resmem+a-RESB;', 1)
                                 .replace('static uint8_t arena[0x1000000]{};', 'static uint8_t arena[0x1000000]{};\nstatic uint8_t resmem[0x400000]{};', 1)
                                 .replace('RESB', '0x%08Xu' % RES_BASE))
    src = (MAIN.replace('QUERY', 'sub_%08X' % addr['F-58446']).replace('RESB', '0x%08Xu' % RES_BASE).replace('LINKVT', '0x%08Xu' % LINK_VT))
    exe = rdisp.build(rdisp.index(a.recomp), ['sub_%08X' % addr['F-58446']], src, a.out, 'navsys_query', image=img,
                      targets=[addr['F-56243']], fakes={0x7F000020: 'throw std::runtime_error("a non-lane item was tested");'}, limit=40)
    r = subprocess.run([str(exe), str(a.image), str(a.data / 'resources/city/sc/0x9960fcd8.r66'), str(cf), '%08X' % lvt],
                       capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    a.ref.write_text(r.stdout)
    print(r.stderr.strip(), '| lane vtable slot 5 found;', len(r.stdout.splitlines()), 'queries ->', a.ref)


if __name__ == '__main__':
    main()
