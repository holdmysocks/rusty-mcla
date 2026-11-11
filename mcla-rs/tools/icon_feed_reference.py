"""Run the original's 3D icon feed F-87716 offline and record the movie globals it sets, for the port's `hookicons::feed_globals`
(crates/game/src/testdata/icon_feed_ref.txt; test hookicons::tests::icon_feed_matches_the_original).

F-87716 as its caller F-76855 calls it (r3 the icon manager: +288 IconFadeoutDist, +340 + 4 slot the slot's movie holder, whose
+0's +56 is the movie; r4 the icon type, r5 the category, r6 the place (2 m up), r7 the colour, r8 the slot, r9 / r10 0, f1 the
bomb meter; on the stack the label id 0, the labels byte 0, the vehicle's +113, the candidate's +68 and +56). The camera is the
viewport table's (image globals: the index and the base, +240 the place). Logged: every global set (F-59146 int, F-11028 bool,
F-16600 string, F-66095 float) and the show (F-67094); the colour word F-44647 runs as it is.

  t<type> k<category> p<x,y,z> c<r,g,b,a> s<slot> f<fadeout> e<cam x,y,z> b<+113> <+68> <+56> m<bomb> -> <name=value;...>
"""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

VPI = 0x8288B9F0                # the viewport index (-17936 off 0x82890000)
VPB = 0x8287E064                # the viewport table base (-8092 off 0x82880000)

MAIN = r'''
static uint32_t fb(float f){uint32_t u; memcpy(&u,&f,4); return u;}
static void sb(uint32_t a,uint8_t b){uint32_t w=load(a&~3u);int sh=(3-(a&3))*8;w=(w&~(0xffu<<sh))|((uint32_t)b<<sh);store(a&~3u,w);}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t M=0x40800000u, P=0x40810000u, C=0x40810040u, V=0x40820000u, H=0x40830000u, MV=0x40840000u;
 while(in>>tok){
  if(tok!="case") break; int32_t ty,ka; uint32_t slot,b113,b68,b56; float px,py,pz,cr,cg,cb,ca,fo,ex,ey,ez,bomb;
  in>>ty>>ka>>px>>py>>pz>>cr>>cg>>cb>>ca>>slot>>fo>>ex>>ey>>ez>>b113>>b68>>b56>>bomb;
  rt_reset(); for(uint32_t a=M;a<M+0x400;a+=4) store(a,0);
  store(M+288,fb(fo)); for(uint32_t s=0;s<3;s++){ store(M+340+4*s,H+16*s); store(H+16*s,MV+0x100*s); store(MV+0x100*s+56,MV+0x100*s+0x80); }
  store(P,fb(px)); store(P+4,fb(py)); store(P+8,fb(pz)); store(P+12,fb(1.0f));
  store(C,fb(cr)); store(C+4,fb(cg)); store(C+8,fb(cb)); store(C+12,fb(ca));
  store(VPIDX,1); store(VPBASE,V); store(V+64+240,fb(ex)); store(V+64+244,fb(ey)); store(V+64+248,fb(ez));
  Context c=fresh(); uint32_t sp=c.r1.u32;
  store(sp+84,0); sb(sp+95,0); sb(sp+103,b113); sb(sp+111,b68); sb(sp+119,b56);
  c.r3.u32=M; c.r4.u64=(uint64_t)(int64_t)ty; c.r5.u64=(uint64_t)(int64_t)ka; c.r6.u32=P; c.r7.u32=C; c.r8.u32=slot; c.r9.u64=0; c.r10.u64=0; c.f1.f64=(double)bomb;
  FEED(c,nullptr); printf("\n"); }
 printf("end\n");
}
'''


def f32(v):
    return struct.unpack('<f', struct.pack('<f', v))[0]


def fbits(v):
    return struct.unpack('<I', struct.pack('<f', v))[0]


def cases():
    g = random.Random(87716)
    out = []
    for _ in range(3000):
        ty = g.choice((1, 6, 9, 10, 11, 13, 14, 16, 0, 2, 4))
        ka = g.choice((0, 1, 1, 2))
        cam = [f32(g.uniform(-3000, 3000)), f32(g.uniform(-10, 150)), f32(g.uniform(-3000, 3000))]
        r = g.choice((5.0, 50.0, 150.0, 399.0, 400.0, 401.0, 800.0, g.uniform(0, 600)))
        a, e = g.uniform(0, 6.2832), g.uniform(-0.3, 0.3)
        import math
        p = [f32(cam[0] + r * math.cos(a)), f32(cam[1] + r * e), f32(cam[2] + r * math.sin(a))]
        if g.random() < 0.05:
            p = list(cam)
        col = [f32(g.choice((0.0, 1.0, g.random(), 0.501961))) for _ in range(3)] + [f32(g.choice((1.0, 0.5, 0.0)))]
        fo = g.choice((400.0, 400.0, 400.0, 250.0, 1000.0))
        out.append((ty, ka, p, col, g.randrange(3), f32(fo), cam, int(g.random() < 0.1), int(g.random() < 0.3), int(g.random() < 0.2),
                    f32(g.choice((0.0, 0.0, 0.25, 1.0, g.random())))))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-87716', 'F-59146', 'F-11028', 'F-16600', 'F-66095', 'F-53775', 'F-67094']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    assert len(addr) == len(ids), addr
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    nm = 'for(uint32_t q=ctx.r4.u32;;q++){uint8_t ch=(uint8_t)(load(q&~3u)>>((3-(q&3))*8)); if(!ch) break; putchar(ch);}'
    stubs = {
        name('F-59146'): nm + 'printf("=%d;",(int)ctx.r5.u32);',
        name('F-11028'): nm + 'printf("=b%u;",(unsigned)(ctx.r5.u32&0xff));',
        name('F-16600'): nm + 'printf("=s%08x;",ctx.r5.u32);',
        name('F-66095'): nm + '{float f=(float)ctx.f1.f64; uint32_t u; memcpy(&u,&f,4); printf("=f%08x;",u);}',
        name('F-53775'): 'ctx.r3.u64=0x40900000u+ctx.r4.u32;',
        name('F-67094'): 'printf("show%u;",ctx.r4.u32);',
    }
    src = MAIN.replace('FEED', name('F-87716')).replace('VPIDX', '0x%08Xu' % VPI).replace('VPBASE', '0x%08Xu' % VPB)
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-87716')], src, a.out, 'feed', image=img, stubs=stubs)
    cs = cases()
    cf = a.out / 'feed_cases.txt'
    with cf.open('w') as f:
        for ty, ka, p, col, s, fo, cam, b113, b68, b56, bomb in cs:
            f.write('case %d %d %r %r %r %r %r %r %r %d %r %r %r %r %d %d %d %r\n' % (ty, ka, *p, *col, s, fo, *cam, b113, b68, b56, bomb))
        f.write('stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True, timeout=120)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    lines = r.stdout.split('\n')[:-2]
    assert len(lines) == len(cs)
    xs = lambda v: ','.join('%08x' % fbits(q) for q in v)
    rows = []
    for (ty, ka, p, col, s, fo, cam, b113, b68, b56, bomb), l in zip(cs, lines):
        rows.append('t%d k%d p%s c%s s%d f%08x e%s b%d %d %d m%08x -> %s' % (ty, ka, xs(p), xs(col), s, fbits(fo), xs(cam), b113, b68, b56, fbits(bomb), l or '.'))
    a.ref.write_text('\n'.join(rows) + '\n')
    print('cases', len(rows), '->', a.ref)


if __name__ == '__main__':
    main()
