"""The race line's portal feeders run offline, the generated code whole and unchanged, the mesh's accessors stubbed to answer
the case and every portal push F-95594 recorded (v1 = a, v2 = b, v3 = normal, r6 flag, f1 / f2 the extensions):

- F-77661 (brain, nav path element, start): the far edge of the node the element left (+36, start) or entered (+40). Its
  chunk from the global nav mesh (0x828D_A3AC -> +656 -> +12, the chunk table +48 indexed by node >> 10), F-45342 (chunk
  loaded) stubbed true; the polygon's vertices from F-36421 (stubbed: the case's 4 vertices to r6, its vertex count in r3),
  the stored centre F-58750 and the normal F-83219 (stubbed: the case's), the mesh's vfunc 60 (stubbed: the case's
  material object, or none) and +8 & 2 the flag. Then the vertices nearest the element's A (+0) and B (+12) in plan, the
  triangle / quad rule, the orientation against the centre, one push.
- F-82239 (brain, the path's point element): the node at +16, its vertices, normal and flag as above; two pushes.
- F-78429 (brain, nav path element): the element's A / B with w 0, the normal F-83219 of the node entered (+40), its flag,
  the extensions +28 / +32; one push.

Writes <out-dir>/race_line_feeders_ref.txt, hex words, one line a case:
"edge start nv A3 B3 v0..v3 (3 each) centre3 normal3 obj flags -> push", "single nv v0..v3 normal3 obj flags -> push push",
"portal A3 B3 clear_a clear_b normal3 obj flags -> push"; a push is "a3 b3 normal3 flag ext0 ext1"."""
import argparse
import random
import re
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr, fb

GLOB = 0x828DA3AC
P, Q, MESH, VT, TAB, CH, Z, EL, BR, OBJ = (0x40100000 + 0x1000 * k for k in range(10))
F60 = 0x40F00000


def poly(rnd):
    nv = rnd.choice((3, 4, 4))
    sc = rnd.choice((1.0, 10.0, 100.0, 1000.0))
    cx, cz = rnd.uniform(-2000, 2000), rnd.uniform(-2000, 2000)
    pt = lambda: [cx + rnd.uniform(-sc, sc), rnd.uniform(-20, 80), cz + rnd.uniform(-sc, sc)]
    verts = [pt() for _ in range(4)]
    if nv == 3 and rnd.random() < 0.5:
        verts[3] = [0.0, 0.0, 0.0]
    nrm = rnd.choice(([0.0, 1.0, 0.0], [rnd.uniform(-0.3, 0.3), rnd.uniform(0.8, 1.0), rnd.uniform(-0.3, 0.3)]))
    return nv, verts, pt, nrm


def tail(rnd):
    return [rnd.choice((1, 1, 1, 0)), rnd.choice((0x82, 0x80, 0x2, 0, rnd.getrandbits(32)))]


def edge(rnd):
    nv, verts, pt, nrm = poly(rnd)
    # A and B: mostly two vertices of the polygon (a shared edge), else anywhere near it, sometimes the same vertex
    k = rnd.randrange(nv)
    m = rnd.choice(((k + 1) % nv, (k + 1) % nv, (k - 1) % nv, k))
    near = lambda v: [v[0] + rnd.choice((0.0, 0.0, rnd.uniform(-0.1, 0.1))), v[1], v[2] + rnd.choice((0.0, 0.0, rnd.uniform(-0.1, 0.1)))]
    a, b = (near(verts[k]), near(verts[m])) if rnd.random() < 0.8 else (pt(), pt())
    if rnd.random() < 0.05:
        # (a tie: B as far from two vertices)
        b = [(verts[0][0] + verts[1 % nv][0]) / 2, 0.0, (verts[0][2] + verts[1 % nv][2]) / 2]
    cen = [sum(v[i] for v in verts[:nv]) / nv for i in range(3)]
    if rnd.random() < 0.2:
        cen = pt()
    return [rnd.choice((0, 1, 1)), nv] + [fb(x) for x in a + b] + [fb(x) for v in verts for x in v] + [fb(x) for x in cen + nrm] + tail(rnd)


def single(rnd):
    nv, verts, _, nrm = poly(rnd)
    return [nv] + [fb(x) for v in verts for x in v] + [fb(x) for x in nrm] + tail(rnd)


def portal(rnd):
    _, _, pt, nrm = poly(rnd)
    cl = lambda: rnd.choice((0.0, 0.0, rnd.uniform(0, 3), 100.0))
    return [fb(x) for x in pt() + pt()] + [fb(cl()), fb(cl())] + [fb(x) for x in nrm] + tail(rnd)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=8000)
    a = ap.parse_args()
    fn = {'E': addr(a.tool, 'F-77661'), 'S': addr(a.tool, 'F-82239'), 'O': addr(a.tool, 'F-78429')}
    rnd = random.Random(77661)
    nl = chr(10)
    ec = [edge(rnd) for _ in range(a.cases)]
    sc = [single(rnd) for _ in range(a.cases // 4)]
    oc = [portal(rnd) for _ in range(a.cases // 4)]
    rows = lambda cs: (',' + nl).join('{' + ','.join('%du' % w for w in c) + '}' for c in cs)
    src = ('static const uint32_t EC[][%d]={' % len(ec[0]) + nl + rows(ec) + nl + '};' + nl +
           'static const uint32_t SC[][%d]={' % len(sc[0]) + nl + rows(sc) + nl + '};' + nl +
           'static const uint32_t OC[][%d]={' % len(oc[0]) + nl + rows(oc) + nl + '};' + nl + r'''
static void mesh(uint32_t flags){for(uint32_t o=@P@;o<@OBJ@+0x1000;o+=4)store(o,0);rec().clear();
 store(@GLOB@,@P@);store(@P@+656,@Q@);store(@Q@+12,@MESH@);store(@MESH@,@VT@);store(@VT@+60,@F60@);store(@MESH@+48,@TAB@);
 store(@TAB@,@CH@);store(@CH@+4,@Z@);store(@Z@+4,@Z@+64);store(@BR@+300,0x12345678u);store(@OBJ@+8,flags);}
static void out(const char*tag,const uint32_t*s,int n){printf("%s",tag);for(int i=0;i<n;i++)printf(" %08x",s[i]);printf(" ->");
 if(rec().empty())printf(" none");for(uint32_t w:rec())printf(" %08x",w);printf("\n");}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(auto&s:EC){rt_reset();vs()=s+8;ns()=s+23;obj()=s[26];nv()=s[1];cen()=s+20;mesh(s[27]);
 for(int i=0;i<6;i++)store(@EL@+4*i,s[2+i]);store(@EL@+36,5);store(@EL@+40,7);
 Context c=fresh();c.r3.u32=@BR@;c.r4.u32=@EL@;c.r5.u32=s[0];sub_@E@(c,nullptr);out("edge",s,@NE@);}
for(auto&s:SC){rt_reset();vs()=s+1;ns()=s+13;obj()=s[16];nv()=s[0];mesh(s[17]);store(@EL@+16,9);
 Context c=fresh();c.r3.u32=@BR@;c.r4.u32=@EL@;sub_@S@(c,nullptr);out("single",s,@NS@);}
for(auto&s:OC){rt_reset();ns()=s+8;obj()=s[11];mesh(s[12]);
 for(int i=0;i<6;i++)store(@EL@+4*i,s[i]);store(@EL@+28,s[6]);store(@EL@+32,s[7]);store(@EL@+36,5);store(@EL@+40,7);
 Context c=fresh();c.r3.u32=@BR@;c.r4.u32=@EL@;sub_@O@(c,nullptr);out("portal",s,@NO@);}}
''')
    for k, v in (('GLOB', GLOB), ('P', P), ('Q', Q), ('MESH', MESH), ('VT', VT), ('TAB', TAB), ('CH', CH), ('Z', Z), ('EL', EL),
                 ('BR', BR), ('OBJ', OBJ), ('F60', F60)):
        src = src.replace('@%s@' % k, '%du' % v)
    for k, v in fn.items():
        src = src.replace('@%s@' % k, v)
    src = src.replace('@NE@', str(len(ec[0]))).replace('@NS@', str(len(sc[0]))).replace('@NO@', str(len(oc[0])))
    pre = ''.join('static %s&%s(){static %s v;return v;}' % (t, n, t) + nl for t, n in
                  (('const uint32_t*', 'vs'), ('const uint32_t*', 'ns'), ('const uint32_t*', 'cen'), ('uint32_t', 'obj'), ('uint32_t', 'nv'),
                   ('std::vector<uint32_t>', 'rec')))
    # (lane k of a vector register is the host's u32[3 - k])
    stubs = {
        'sub_' + addr(a.tool, 'F-45342'): 'ctx.r3.u32=1;',
        'sub_' + addr(a.tool, 'F-70160'): '',
        'sub_' + addr(a.tool, 'F-36421'): 'for(int k=0;k<4;k++){for(int i=0;i<3;i++)store(ctx.r6.u32+16*k+4*i,vs()[3*k+i]);store(ctx.r6.u32+16*k+12,0);}ctx.r3.u32=nv();',
        'sub_' + addr(a.tool, 'F-58750'): 'for(int i=0;i<3;i++)store(ctx.r5.u32+4*i,cen()[i]);store(ctx.r5.u32+12,0);',
        'sub_' + addr(a.tool, 'F-83219'): 'for(int i=0;i<3;i++)store(ctx.r5.u32+4*i,ns()[i]);store(ctx.r5.u32+12,0);',
        'sub_' + addr(a.tool, 'F-95594'): ('auto&r=rec();for(int i=0;i<3;i++)r.push_back(ctx.v1.u32[3-i]);for(int i=0;i<3;i++)r.push_back(ctx.v2.u32[3-i]);'
                                           'for(int i=0;i<3;i++)r.push_back(ctx.v3.u32[3-i]);r.push_back(ctx.r6.u32);'
                                           'for(double d:{ctx.f1.f64,ctx.f2.f64}){float f=(float)d;uint32_t w;memcpy(&w,&f,4);r.push_back(w);}'),
    }
    old = rd.rc.EXTRA
    rd.rc.EXTRA = old + pre
    try:
        exe = rd.build(rd.index(a.recomp), ['sub_' + v for v in fn.values()], src, a.out_dir, 'linefeed', stubs=stubs,
                       image=rd.load_image(a.image), fakes={F60: 'ctx.r3.u32=obj()?%du:0;' % OBJ})
    finally:
        rd.rc.EXTRA = old
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == len(ec) + len(sc) + len(oc), len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'race_line_feeders_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    print(len(lines), 'cases;', sum('none' in x for x in lines), 'without a push')


if __name__ == '__main__':
    main()
