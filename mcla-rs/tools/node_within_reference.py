"""Run the original's road / junction lookups F-55639 / F-85231 (the quadtree walk F-21575 with its node box test F-80185
and the node rectangle distance F-46547, called through the items' vtable slot 3) offline on the city's real navsys resource,
and record the answers for the port's test (crates/formats/src/testdata/node_within_ref.txt; test
navsys::tests::node_within_matches_the_original, needs MCLA_DATA_DIR).

The resource is copied to guest memory at its own base as in navsys_query_reference.py. The navsys object is a fake whose +8
is the resource (the wrappers read +8, then its +304). Each node item gets a vtable: a road / junction one of the image's node
vtables (slot 3 F-46547, the same in all of them), a lane a table that throws (masks 2 / 4 never select a lane).

Queries: points in the root's box, 40 % placed 0 - 30 m off a random road's or junction's centre, radius 0 - 150 m. Line per
query: <x> <y> <z> <r> (f32 bits as signed decimal) <road offset or -1> <junction offset or -1>, then for the walk F-21575
with mask 6 capped at 64, 8 and 3: |<count> <the hits' offsets in order>
"""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp
import recomp_closure
import memcap

memcap.apply()

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

QUERIES = 1500
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
   store(it, le==LINKVT ? BADVT : LVT); items++; }
  for(int k=0;k<4;k++){ uint32_t c=load(n+28+4*k); if(c) st.push_back(c); } }
 const uint32_t OBJ=0x40C00000u; store(OBJ+8, RESB);
 fprintf(stderr,"nodes %d items %d\n",nodes,items);
 std::ifstream in(argv[3]); uint32_t xb,yb,zb,wb; const uint32_t P=0x40D00000u;
 while(in>>std::hex>>xb>>yb>>zb>>wb){
  store(P,xb); store(P+4,yb); store(P+8,zb); store(P+12,wb);
  Context c=fresh(); c.r3.u32=OBJ; c.r4.u32=P; ROADQ(c,nullptr); uint32_t rd_=c.r3.u32;
  Context d=fresh(); d.r3.u32=OBJ; d.r4.u32=P; JUNCQ(d,nullptr); uint32_t jn=d.r3.u32;
  printf("%d %d %d %d %d %d",(int)xb,(int)yb,(int)zb,(int)wb, rd_?(int)(rd_-RESB):-1, jn?(int)(jn-RESB):-1);
  // the walk itself with mask 6 (the traffic region list's query), capped at 64 / 8 / 3
  for(int mx: {64, 8, 3}){ const uint32_t OUT=0x40100000u; Context e=fresh(); e.r3.u32=ROOT; e.r4.u32=P; e.r5.u32=OUT; e.r6.u32=(uint32_t)mx; e.r7.u32=6;
   WALK(e,nullptr); int n=(int)e.r3.u32; printf(" |%d",n); for(int i=0;i<n;i++) printf(" %d",(int)(load(OUT+48*i+32)-RESB)); }
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
    ids = ['F-55639', 'F-85231', 'F-46547', 'F-80185', 'F-21575']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    img = rdisp.load_image(a.image)
    # the lane class: the vtable whose slot 5 is the sphere test
    want = struct.pack('>I', addr['F-46547'])
    hits = [i - 12 for i in range(0, len(img) - 3, 4) if img[i:i + 4] == want]
    lvt = [IMG + h for h in hits if rdisp.image_u32(img, IMG + h + 4) >> 24 == 0x82]
    assert len(lvt) >= 1, hits
    lvt = lvt[0]
    res = (a.data / 'resources/city/sc/0x9960fcd8.r66').read_bytes()
    d = res[16:]
    u32 = lambda o: struct.unpack_from('>I', d, o)[0]
    f32 = lambda o: struct.unpack_from('>f', d, o)[0]
    root = u32(304) & 0x0FFFFFFF
    x0, z0, x1, z1 = f32(root + 52), f32(root + 56), f32(root + 60), f32(root + 64)
    rnd = random.Random(20261108)
    fb = lambda v: struct.unpack('<I', struct.pack('<f', v))[0]
    # roads / junctions: items whose flags (+4) have 2 / 4, their centres at +52 / +56
    nodes, st = set(), [root]
    while st:
        n = st.pop()
        cnt = u32(n + 8) >> 16
        arr = u32(n + 4) & 0x0FFFFFFF
        for k in range(cnt):
            it = u32(arr + 4 * k) & 0x0FFFFFFF
            if it and u32(it + 4) & 6:
                nodes.add(it)
        st += [u32(n + 28 + 4 * k) & 0x0FFFFFFF for k in range(4) if u32(n + 28 + 4 * k)]
    nodes = sorted(nodes)
    lines = []
    for q in range(QUERIES):
        if rnd.random() < 0.4:
            o = rnd.choice(nodes)
            p = [f32(o + 52) + rnd.uniform(-30, 30), rnd.uniform(-20, 80), f32(o + 56) + rnd.uniform(-30, 30)]
        else:
            p = [rnd.uniform(x0, x1), rnd.uniform(-20, 80), rnd.uniform(z0, z1)]
        w = rnd.choice([0.0, 1.0, 3.0, 6.0, 15.0, 30.0, 60.0, 150.0]) * rnd.uniform(0.9, 1.1)
        lines.append('%08x %08x %08x %08x' % (fb(p[0]), fb(p[1]), fb(p[2]), fb(w)))
    a.out.mkdir(parents=True, exist_ok=True)
    cf = a.out / 'navsys_queries.txt'
    cf.write_text('\n'.join(lines) + '\n')
    orig_shim = rdisp.vector_shim
    rdisp.vector_shim = lambda: (orig_shim()
                                 .replace('static uint8_t* pointer(uint32_t a) {', 'static uint8_t* pointer(uint32_t a) {\n    if (a>=RESB && a<RESB+0x400000u) return resmem+a-RESB;', 1)
                                 .replace('static uint8_t arena[0x1000000]{};', 'static uint8_t arena[0x1000000]{};\nstatic uint8_t resmem[0x400000]{};', 1)
                                 .replace('RESB', '0x%08Xu' % RES_BASE))
    src = (MAIN.replace('ROADQ', 'sub_%08X' % addr['F-55639']).replace('JUNCQ', 'sub_%08X' % addr['F-85231']).replace('WALK', 'sub_%08X' % addr['F-21575']).replace('RESB', '0x%08Xu' % RES_BASE).replace('LINKVT', '0x%08Xu' % LINK_VT))
    exe = rdisp.build(rdisp.index(a.recomp), ['sub_%08X' % addr['F-55639'], 'sub_%08X' % addr['F-85231']], src, a.out, 'node_within',
                      image=img, targets=[addr['F-46547']], fakes={0x7F000020: 'throw std::runtime_error("a lane item was tested");'}, limit=60)
    r = subprocess.run([str(exe), str(a.image), str(a.data / 'resources/city/sc/0x9960fcd8.r66'), str(cf), '%08X' % lvt],
                       capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    a.ref.write_text(r.stdout)
    print(r.stderr.strip(), '| node vtable %08X;' % lvt, '', len(r.stdout.splitlines()), 'queries ->', a.ref)


if __name__ == '__main__':
    main()
