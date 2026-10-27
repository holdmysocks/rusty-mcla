"""Execute small original power-up functions offline and write the reference table for
`powerups::tests::small_functions_match_the_original` / `pu_bolt::tests::arc_init_matches_the_original`.

Each function runs unchanged (with its direct callees) on a flat guest memory (see recomp_closure.py). Nothing is stubbed.
Output lines (hex, big-endian words as the guest stores them):
  OPT  <byte0> <byte1>  <flag +22> <word +128> <roll k=0..n-1 ...>     options record -> allowed types word -> k-th type
  HLD  <kind or -1 no effect> <stealth getter> <megastealth getter>
  NEW  <the holder constructor's 0x100 bytes>
  LOOK <the look constructor's 0xE0 bytes>
  RAD  <r bits> <+116 bits> <+120 bits>
  ARC  (every number decimal behind an n) <rng lo> <rng carry> <6 floats of the tune ranges (bits)> -> <+16 +20 +24 +28 (bits)> <seed u64> <+48 +52> <rng lo> <rng carry>
"""
import argparse
import random
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def name(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--result', type=Path, required=True)
    a = ap.parse_args()
    idx = index(a.recomp)
    ids = dict(allowed='F-18300', roll='F-52455', stealth='F-73113', megastealth='F-63149', new='F-87508', look='F-88402',
               radius='F-52582', arc='F-63789')
    n = {k: name(a.tool, v)[4:] for k, v in ids.items()}
    rnd = random.Random(20261027)
    # the options record sits in the image at 0x82000000 + 0x700000; the pointer global -> object 0x1400 whose +48 = record - 0x2ADE0
    body = []
    body.append('''static uint32_t gf(float v){uint32_t b;std::memcpy(&b,&v,4);return b;}
''')
    m = ['int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});\n']
    # allowed + roll
    m.append('''for(int b0=0;b0<256;b0++)for(int b1=0;b1<256;b1+=0x20){ if(b1&0x1f)continue;
 std::memset(objects,0,sizeof(objects));store((0x82000000u+0x86D804u),0x1400);store(0x1430,(0x82000000u+0x700000u)-0x2ADE0u);*pointer((0x82000000u+0x700000u))=b0;*pointer((0x82000000u+0x700001u))=b1;
 Context ctx{};ctx.r3.u32=0x1000;ctx.r1.u32=0x2800;sub_%s(ctx,nullptr);
 uint32_t w=load(0x1080);std::printf("OPT %%02x %%02x %%02x %%08x",b0,b1,*pointer(0x1016),w);
 for(int k=0;k<12;k++){Context c2{};c2.r3.u32=0x1080;c2.r4.u32=k;c2.r1.u32=0x2800;sub_%s(c2,nullptr);std::printf(" %%d",(int)c2.r3.s32);}
 std::puts("");}
''' % (n['allowed'], n['roll']))
    m.append('''for(int k=-1;k<=30;k++){std::memset(objects,0,sizeof(objects));if(k>=0){store(0x1000+236,0x1200);store(0x1204,k);}
 Context a{};a.r3.u32=0x1000;a.r1.u32=0x2800;sub_%s(a,nullptr);Context b{};b.r3.u32=0x1000;b.r1.u32=0x2800;sub_%s(b,nullptr);
 std::printf("HLD %%d %%d %%d\\n",k,(int)a.r3.s32,(int)b.r3.s32);}
''' % (n['stealth'], n['megastealth']))
    m.append('''{std::memset(objects,0,sizeof(objects));for(int i=0;i<0x100;i++)objects[i]=0xAA;Context c{};c.r3.u32=0x1000;c.r1.u32=0x2800;sub_%s(c,nullptr);
 std::printf("NEW");for(int i=0;i<0x100;i++)std::printf(" %%02x",objects[i]);std::puts("");}
''' % n['new'])
    m.append('''{std::memset(objects,0,sizeof(objects));Context c{};c.r3.u32=0x1000;c.r1.u32=0x2800;sub_%s(c,nullptr);
 std::printf("LOOK");for(int i=0;i<0xE0;i++)std::printf(" %%02x",objects[i]);std::puts("");}
''' % n['look'])
    m.append('''for(float r:{4.0f,0.0f,1.5f,-2.0f,3.3f}){std::memset(objects,0,sizeof(objects));Context c{};c.r3.u32=0x1000;c.f1.f64=r;c.r1.u32=0x2800;sub_%s(c,nullptr);
 std::printf("RAD %%08x %%08x %%08x\\n",gf(r),load(0x1000+116),load(0x1000+120));}
''' % n['radius'])
    cases = []
    for _ in range(1200):
        lo, ca = rnd.getrandbits(32), rnd.getrandbits(32)
        def rng():
            x = rnd.uniform(0.0, 20.0)
            return x, x + rnd.uniform(0.01, 30.0)
        g, gs, ft, fs = rng(), rng(), rng(), rng()
        cases.append((lo, ca, g, gs, ft, fs))
    arr = ','.join('{%du,%du,%.9gf,%.9gf,%.9gf,%.9gf,%.9gf,%.9gf,%.9gf,%.9gf}' % (c[0], c[1], c[2][0], c[2][1], c[3][0], c[3][1], c[4][0], c[4][1], c[5][0], c[5][1]) for c in cases)
    m.append('''{static const struct{uint32_t lo,ca;float v[8];}T[]={%s};
 for(auto&t:T){std::memset(objects,0,sizeof(objects));store((0x82000000u+0x860880u),t.lo);store((0x82000000u+0x860884u),t.ca);
  // tune at 0x1800: +48/52 ArcGrowth, +56/60 GrowthSpeed, +64/68 FluxTangent, +80/84 FlickerSpeed
  putf(0x1800+48,t.v[0]);putf(0x1800+52,t.v[1]);putf(0x1800+56,t.v[2]);putf(0x1800+60,t.v[3]);putf(0x1800+64,t.v[4]);putf(0x1800+68,t.v[5]);putf(0x1800+80,t.v[6]);putf(0x1800+84,t.v[7]);
  Context c{};c.r3.u32=0x1000;c.r4.u32=0x1800;c.r1.u32=0x2800;sub_%s(c,nullptr);
  std::printf("ARC n%%u n%%u",t.lo,t.ca);for(int i=0;i<8;i++)std::printf(" n%%u",gf(t.v[i]));
  std::printf(" n%%u n%%u n%%u n%%u n%%llu n%%u n%%u n%%u n%%u\\n",load(0x1010),load(0x1014),load(0x1018),load(0x101c),(unsigned long long)load64(0x1028),load(0x1030),load(0x1034),load((0x82000000u+0x860880u)),load((0x82000000u+0x860884u)));}}
''' % (arr, n['arc']))
    m.append('}\n')
    funcs = [n[k] and 'sub_' + n[k] for k in ids]
    exe = build(idx, funcs, ''.join(body) + ''.join(m), a.out, 'powerups_small')
    a.result.write_text(run(exe, a.image))


if __name__ == '__main__':
    main()
