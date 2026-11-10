"""The racing brain handing a found nav path to the aiv race line, run offline: F-47058 (brain), the generated code whole and
unchanged with its feeders F-77661 / F-82239 / F-78429, the aiv's portal push F-95594 and pop F-82126 and the goal insert F-89800
run as generated; the mesh's accessors stubbed to answer the case (as in race_line_feeders_reference.py: F-45342 true, F-70160
the polygon, F-36421 its vertices and count, F-58750 its centre, F-83219 its normal, vfunc 60 its material object), the
quickest-path result F-43679 (the case's list), the goal push F-29767 and the nav-state call F-71636 recorded.

Brain: +176 goals (32 bytes: pos, +12 w, +16 second point, +28 speed), +180 / +188 counts, +184 flags words, +192 the final
heading, +208 the final speed, +216 the goal driven to, +220 the goal planned to, +258 looping, +300 the aiv. The aiv (31,232
bytes, zero but for): the portal ring header +22544 tail, +22548 start, +22552 count, +22560 base and the flag ring's +22704 /
+22708 / +22712, the planes-built index +30832, the car size +25168, the race-line tuning +25332 (the recorded block). The
result list: +12 the first 56-byte element (+0 type: 2 portal, 0 the point; +4 A, +16 B, +32 / +36 clearances, +40 left, +44
entered, +48 next; the point's node at +20), +20 the element count.

Writes <out-dir>/path_to_aiv_ref.txt, hex words, one line a case:
"case start count base planes size3 current next looping heading3 speed ngoals node nport" + per goal (pos3 w p1_3 speed flags)
+ 4 polygons (nv v0..v3 centre3 normal3 obj flags) + per portal one word (left | entered << 3 | ia << 6 | ib << 8 | clear_a << 10 |
clear_b << 12, the clearances from (0, 0.4, 1.5, 100)) + "-> nav start count base ncells cells_hash npush [dir3 goal8 flags speed]
goals flags next goals_hash flags_hash" (hashes: FNV-1a 32 over the words; a cell = a b sa sb n (3 words each) and the flag)."""
import argparse
import random
import re
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr, fb

GLOB = 0x828DA3AC
BR, GL, FL, RES, ELS = 0x40100000, 0x40110000, 0x40118000, 0x40120000, 0x40130000
AIV = 0x40200000
P, Q, MESH, VT, TAB, CH, Z, OBJ = (0x40300000 + 0x1000 * k for k in range(8))
F60 = 0x40F00000
CLEAR = (0.0, 0.4, 1.5, 100.0)
TUNE = [0x14, 0, 1, 0x3f000000, 0x41c80000, 3, 0x3e4ccccd, 0x3dcccccd, 0x40a00000, 0x3f800000, 0x41200000, 0x3dcccccd, 0x3f400000,
        0x3dcccccd, 0x3f800000, 0x3dcac083, 0x3f800000, 0x40c00000, 0x3f800000, 0x3f800000, 0x3f400000, 0x3f800000, 0x3f000000,
        0x40a00000, 0x41c80000, 0x42c80000, 0x40d68f5c]


def poly(rnd, cx, cz):
    nv = rnd.choice((3, 4, 4))
    sc = rnd.choice((3.0, 10.0, 30.0))
    pt = lambda: [cx + rnd.uniform(-sc, sc), rnd.uniform(-5, 20), cz + rnd.uniform(-sc, sc)]
    verts = [pt() for _ in range(4)]
    if nv == 3:
        verts[3] = [0.0, 0.0, 0.0]
    cen = [sum(v[i] for v in verts[:nv]) / nv for i in range(3)]
    nrm = rnd.choice(([0.0, 1.0, 0.0], [rnd.uniform(-0.3, 0.3), rnd.uniform(0.8, 1.0), rnd.uniform(-0.3, 0.3)]))
    return [nv] + [fb(x) for v in verts for x in v] + [fb(x) for x in cen + nrm] + [rnd.choice((1, 1, 0)), rnd.choice((0x82, 0x80, 0x2, 0))]


def case(rnd):
    cx, cz = rnd.uniform(-2000, 2000), rnd.uniform(-2000, 2000)
    nport = rnd.choice((0, 0, 1, 1, 2, 3, 5, 8, 15, 30, 60, 93, 94, 95, 110, 127))
    count = rnd.choice((0, 0, 1, 1, 2, 3, rnd.randint(0, 40), rnd.randint(0, 127), 128 - min(nport, 64) - 1, 128 - min(nport, 64) - 2))
    base = rnd.choice((0, 0, 0, 1, rnd.randint(0, 600)))
    start = rnd.randrange(128)
    planes = max(0, base + count + rnd.choice((0, 1, 1, 2, -1, -5)))
    ng = rnd.randint(1, 5)
    nxt = rnd.choice((0, rnd.randrange(ng), ng - 1, ng - 1))
    cur = rnd.choice((nxt, nxt, max(nxt - 1, 0), nxt + 1, 0))
    goals = []
    for _ in range(ng):
        p0 = [cx + rnd.uniform(-300, 300), rnd.uniform(-5, 20), cz + rnd.uniform(-300, 300)]
        p1 = [cx + rnd.uniform(-300, 300), rnd.uniform(-5, 20), cz + rnd.uniform(-300, 300)]
        gate = rnd.choice((0, 0, 1))
        w = rnd.uniform(0, 1) if gate else rnd.uniform(3, 30)
        flags = (rnd.getrandbits(19) << 13 if rnd.random() < 0.2 else 0) | (rnd.randrange(128) << 6) | (rnd.randrange(16) << 2) | 2 | gate
        goals += [fb(x) for x in p0] + [fb(w)] + [fb(x) for x in p1] + [fb(rnd.choice((-1.0, 0.0, rnd.uniform(5, 40))))] + [flags]
    polys = [w for _ in range(4) for w in poly(rnd, cx, cz)]
    ports = []
    left = rnd.randrange(4)
    for _ in range(nport):
        ent = rnd.choice(((left + 1) % 4, rnd.randrange(4)))
        nvl, nve = polys[21 * left], polys[21 * ent]
        ports.append(left | ent << 3 | rnd.randrange(nvl) << 6 | rnd.randrange(nve) << 8 | rnd.randrange(4) << 10 | rnd.randrange(4) << 12)
        left = ent
    size = [rnd.uniform(1.6, 2.4), 1.4, rnd.uniform(4.0, 5.5)]
    head = [rnd.uniform(-1, 1), 0.0, rnd.uniform(-1, 1)]
    return ([start, count, base, planes] + [fb(x) for x in size] + [cur, nxt, rnd.choice((0, 0, 1))] + [fb(x) for x in head] +
            [fb(rnd.choice((-1.0, 20.0, 0.0))), ng, rnd.randrange(4), nport] + goals + polys + ports)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    ap.add_argument('--raw', type=int, default=-1, help="(debugging) print case N's portal cells to stderr")
    a = ap.parse_args()
    fn = addr(a.tool, 'F-47058')
    rnd = random.Random(47058)
    nl = chr(10)
    cases = [case(rnd) for _ in range(a.cases)]
    flat, offs = [], []
    for c in cases:
        offs.append(len(flat))
        flat += c
    src = ('static const uint32_t CS[]={%s};' % ','.join('%du' % w for w in flat) + nl +
           'static const uint32_t OF[]={%s};' % ','.join('%du' % o for o in offs) + nl +
           'static const uint32_t LN[]={%s};' % ','.join('%du' % len(c) for c in cases) + nl +
           'static const uint32_t TU[]={%s};' % ','.join('%du' % w for w in TUNE) + nl +
           'static const float CL[]={0.0f,0.4f,1.5f,100.0f};' + nl + r'''
static void st16(uint32_t a,uint32_t v){pointer(a)[0]=(uint8_t)(v>>8);pointer(a)[1]=(uint8_t)v;}
static uint32_t ld16(uint32_t a){return (uint32_t)pointer(a)[0]<<8|pointer(a)[1];}
static uint32_t fnv(uint32_t h,uint32_t v){for(int b=0;b<4;b++){h^=(v>>(8*b))&0xFF;h*=16777619u;}return h;}
static uint32_t fbits(float f){uint32_t w;memcpy(&w,&f,4);return w;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(int k=0;k<@C@;k++){const uint32_t*s=CS+OF[k];rt_reset();nav()=0;rec().clear();
 for(uint32_t o=@BR@;o<@ELS@+0x4000;o+=4)store(o,0);for(uint32_t o=@AIV@;o<@AIV@+31232;o+=4)store(o,0);
 for(uint32_t o=@P@;o<@OBJ@+0x1000;o+=4)store(o,0);
 uint32_t st=s[0],ct=s[1],bs=s[2],ng=s[14],nd=s[15],np=s[16];
 const uint32_t*g=s+17,*pl=g+9*ng,*pt=pl+84;poly()=pl;
 // aiv
 store(@AIV@+22544,(st+ct)%128);store(@AIV@+22548,st);store(@AIV@+22552,ct);store(@AIV@+22560,bs);
 store(@AIV@+22704,(st+ct)%128);store(@AIV@+22708,st);store(@AIV@+22712,ct);store(@AIV@+30832,s[3]);
 for(int i=0;i<3;i++)store(@AIV@+25168+4*i,s[4+i]);for(int i=0;i<27;i++)store(@AIV@+25332+4*i,TU[i]);
 // brain
 store(@BR@+176,@GL@);st16(@BR@+180,ng);st16(@BR@+182,64);store(@BR@+184,@FL@);st16(@BR@+188,ng);st16(@BR@+190,64);
 for(uint32_t i=0;i<ng;i++){for(int j=0;j<8;j++)store(@GL@+32*i+4*j,g[9*i+j]);store(@FL@+4*i,g[9*i+8]);}
 store(@BR@+216,s[7]);store(@BR@+220,s[8]);*pointer(@BR@+258)=(uint8_t)s[9];
 for(int i=0;i<3;i++)store(@BR@+192+4*i,s[10+i]);store(@BR@+208,s[13]);store(@BR@+300,@AIV@);
 // the result list: the portals then the point
 store(@RES@+20,np+1);store(@RES@+12,@ELS@);
 for(uint32_t i=0;i<=np;i++){uint32_t e=@ELS@+56*i;store(e+48,i<np?e+56:0);
  if(i==np){store(e,0);store(e+20,nd);continue;}
  uint32_t w=pt[i],l=w&7,n=(w>>3)&7,ia=(w>>6)&3,ib=(w>>8)&3;store(e,2);
  for(int j=0;j<3;j++){store(e+4+4*j,pl[21*l+1+3*ia+j]);store(e+16+4*j,pl[21*l+1+3*ib+j]);}
  store(e+32,fbits(CL[(w>>10)&3]));store(e+36,fbits(CL[(w>>12)&3]));store(e+40,l);store(e+44,n);}
 // the mesh
 store(@GLOB@,@P@);store(@P@+656,@Q@);store(@Q@+12,@MESH@);store(@MESH@,@VT@);store(@VT@+60,@F60@);store(@MESH@+48,@TAB@);
 store(@TAB@,@CH@);store(@CH@+4,@Z@);store(@Z@+4,@Z@+64);
 for(int i=0;i<4;i++)store(@OBJ@+16*i+8,pl[21*i+20]);
 Context c=fresh();c.r3.u32=@BR@;sub_@F@(c,nullptr);
 printf("case");for(uint32_t i=0;i<LN[k];i++)printf(" %08x",s[i]);
 uint32_t a0=load(@AIV@+22548),a1=load(@AIV@+22552),a2=load(@AIV@+22560);
 printf(" -> %08x %08x %08x %08x",nav(),a0,a1,a2);
 uint32_t h=2166136261u;for(uint32_t i=0;i<a1;i++){uint32_t sl=(a0+i+1)%128,o=@AIV@+16+176*sl;
  for(int q=0;q<5;q++)for(int j=0;j<3;j++)h=fnv(h,load(o+16*q+4*j));h=fnv(h,(pointer(@AIV@+22576+sl)[0]&0x80)?1:0);}
 if(k==@RAW@)for(uint32_t i=0;i<a1;i++){uint32_t sl=(a0+i+1)%128,o=@AIV@+16+176*sl;fprintf(stderr,"cell %u",i);
  for(int q=0;q<5;q++)for(int j=0;j<3;j++)fprintf(stderr," %08x",load(o+16*q+4*j));fprintf(stderr," %u\n",(pointer(@AIV@+22576+sl)[0]&0x80)?1:0);}
 printf(" %08x %08x %08x",a1,h,(uint32_t)rec().size()/13);for(uint32_t w:rec())printf(" %08x",w);
 uint32_t gc=ld16(@BR@+180),fc=ld16(@BR@+188);printf(" %08x %08x %08x",gc,fc,load(@BR@+220));
 h=2166136261u;for(uint32_t i=0;i<gc;i++)for(int j=0;j<8;j++)h=fnv(h,load(@GL@+32*i+4*j));printf(" %08x",h);
 h=2166136261u;for(uint32_t i=0;i<fc;i++)h=fnv(h,load(@FL@+4*i));printf(" %08x\n",h);}}
''')
    for k, v in (('GLOB', GLOB), ('BR', BR), ('GL', GL), ('FL', FL), ('RES', RES), ('ELS', ELS), ('AIV', AIV), ('P', P), ('Q', Q),
                 ('MESH', MESH), ('VT', VT), ('TAB', TAB), ('CH', CH), ('Z', Z), ('OBJ', OBJ), ('F60', F60)):
        src = src.replace('@%s@' % k, '%du' % v)
    src = src.replace('@RAW@', str(a.raw))
    src = src.replace('@C@', str(a.cases)).replace('@F@', fn)
    pre = ''.join('static %s&%s(){static %s v;return v;}' % (t, n, t) + nl for t, n in
                  (('const uint32_t*', 'poly'), ('uint32_t', 'cur'), ('uint32_t', 'nav'), ('std::vector<uint32_t>', 'rec')))
    # a polygon's record: nv, v0..v3, centre, normal, obj, flags (21 words); lane k of a vector register is the host's u32[3 - k]
    stubs = {
        'sub_' + addr(a.tool, 'F-43679'): 'ctx.r3.u32=%du;' % RES,
        'sub_' + addr(a.tool, 'F-71636'): 'nav()=1;',
        'sub_' + addr(a.tool, 'F-29767'): ('auto&r=rec();for(int i=0;i<3;i++)r.push_back(ctx.v1.u32[3-i]);for(int j=0;j<8;j++)r.push_back(load(ctx.r4.u32+4*j));'
                                           'r.push_back(load(ctx.r5.u32));{float f=(float)ctx.f1.f64;uint32_t w;memcpy(&w,&f,4);r.push_back(w);}'),
        'sub_' + addr(a.tool, 'F-45342'): 'ctx.r3.u32=1;',
        # (memset(r3, r4, r5): the push clears a cell's planes +80 .. +176)
        'sub_' + addr(a.tool, 'F-29294'): 'for(uint32_t i=0;i<ctx.r5.u32;i++)pointer(ctx.r3.u32+i)[0]=(uint8_t)ctx.r4.u32;',
        'sub_' + addr(a.tool, 'F-70160'): 'cur()=ctx.r4.u32;',
        'sub_' + addr(a.tool, 'F-36421'): ('const uint32_t*p=poly()+21*cur();for(int k=0;k<4;k++){for(int i=0;i<3;i++)store(ctx.r6.u32+16*k+4*i,p[1+3*k+i]);'
                                           'store(ctx.r6.u32+16*k+12,0);}ctx.r3.u32=p[0];'),
        'sub_' + addr(a.tool, 'F-58750'): 'const uint32_t*p=poly()+21*ctx.r4.u32;for(int i=0;i<3;i++)store(ctx.r5.u32+4*i,p[13+i]);store(ctx.r5.u32+12,0);',
        'sub_' + addr(a.tool, 'F-83219'): 'const uint32_t*p=poly()+21*ctx.r4.u32;for(int i=0;i<3;i++)store(ctx.r5.u32+4*i,p[16+i]);store(ctx.r5.u32+12,0);',
    }
    old = rd.rc.EXTRA
    rd.rc.EXTRA = old + pre
    try:
        exe = rd.build(rd.index(a.recomp), ['sub_' + fn], src, a.out_dir, 'pathaiv', stubs=stubs, image=rd.load_image(a.image),
                       fakes={F60: 'ctx.r3.u32=poly()[21*ctx.r4.u32+19]?%du+16*ctx.r4.u32:0;' % OBJ})
    finally:
        rd.rc.EXTRA = old
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'path_to_aiv_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    nav = [x.split('->')[1].split()[0] for x in lines]
    print(len(lines), 'cases;', nav.count('00000001'), 'consumed')


if __name__ == '__main__':
    main()
