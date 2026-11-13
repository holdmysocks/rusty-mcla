"""The race marker of one racer F-50704 run offline as tools/race_marker_reference.py does, but with the Stealth getter F-54047
and its type getters (F-73113 / F-63149) as generated instead of the stub: the racer object (+128 of the racer, 0x4600) holds
the car at +48, the car its power-up holder at +68, the holder its effect at +236, the effect its type at +4 (-1 .. 30); each
present or null. One marker that is shown otherwise (100 m out, icons 0 / 10 / 18).

Writes <out-dir>/race_marker_stealth_ref.txt, one state a line: "car holder effect type icon -> queued" (decimal)."""
import argparse
from pathlib import Path
import recomp_closure
import race_marker_reference as rm
from recomp_closure import index, build, run


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fr, fdepth, fbound = (rm.addr(a.tool, f) for f in ('F-50704', 'F-26769', 'F-98453'))
    nl = chr(10)
    body = (
        'static void put3(uint32_t a,float x,float y,float z,float w){putf(a,x);putf(a+4,y);putf(a+8,z);putf(a+12,w);}' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
        'for(int k=0;k<4;k++)store(0x828609E0+4*k,0x7f800000);' + nl +
        'for(int icon:{0,10,18})for(int car=0;car<2;car++)for(int holder=0;holder<2;holder++)for(int effect=0;effect<2;effect++)for(int type=-1;type<=30;type++){' + nl +
        'memset(objects,0,sizeof(objects));' + nl +
        # the icon manager as race_marker_reference.py: offset scale 1.9, scale (1, 1.4, 1), fade 20 / 45 / 175 / 40, white colours
        'store(0x1000+68,0x4900);putf(0x1000+128,1.9f);put3(0x1000+176,1,1.4f,1,0);put3(0x1000+192,0,0,0,0);'
        'for(int k=0;k<4;k++){putf(0x1000+208+4*k,1);putf(0x1000+240+4*k,1);putf(0x1000+256+4*k,1);}'
        'putf(0x1000+272,20);putf(0x1000+276,45);putf(0x1000+280,175);putf(0x1000+284,40);' + nl +
        'store(0x4400+8,0x4800);store(0x4400+128,0x4600);store(0x4600+116,uint32_t(icon));'
        'for(uint32_t m:{0x4400u+144,0x4400u+208}){for(int r=0;r<4;r++)for(int q=0;q<4;q++)putf(m+16*r+4*q,r==q?1.f:0.f);}'
        'put3(0x4400+144+48,0,0,100,1);' + nl +
        'putf(0x4800+16+4,1.5f);putf(0x4800+32+4,-0.5f);store(0x4900,0x4A00);store(0x4A00+60,0x82000000);' + nl +
        'store(0x828CCFC4,0x4B00);store(0x4C00,0x4D00);' + nl +
        'store(0x82839704,0x5000);put3(0x5000+96,0,0,-1,0);put3(0x5000+112,0,0,0,1);store(0x827E5640,0);store(0x8287E064,0x5200);put3(0x5200+240,0,0,0,1);' + nl +
        # the Stealth getter's chain
        'store(0x4600+48,car?0x6000:0);store(0x6000+68,holder?0x6100:0);store(0x6100+236,effect?0x6400:0);store(0x6400+4,uint32_t(type));' + nl +
        '{Context ctx{};ctx.r1.u32=0x8F00;ctx.r13.u32=0x4C00;ctx.r3.u32=0x1000;ctx.r4.u32=0x4400;sub_%s(ctx,nullptr);}' % fr +
        'printf("%%d %%d %%d %%d %%d -> %%u%s",car,holder,effect,type,icon,load(0x1000+12672));}' % (chr(92) + 'n') + nl + '}' + nl)
    stubs = {'sub_' + fdepth: 'ctx.r3.u64=0x5555;', 'sub_' + fbound: 'ctx.r3.u64=0x4800;'}
    recomp_closure.EXTRA += rm.VIS
    exe = build(index(a.recomp), ['sub_' + fr], body, a.out_dir, 'race_marker_stealth', stubs=stubs, vector=True, scratch_end=0x9000)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == 3 * 8 * 32, len(out)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'race_marker_stealth_ref.txt').write_text(nl.join(out) + nl)
    print(len(out), 'states,', sum(x.endswith('-> 1') for x in out), 'queued')


if __name__ == '__main__':
    main()
