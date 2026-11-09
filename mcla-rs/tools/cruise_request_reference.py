"""The cruising brain's route + request F-25539 run offline (the generated code whole and unchanged, vector mode), with the
ambient request's default ctor F-86587, its fill F-45833 and the route record's init / copy (F-18347 / F-29143) as
generated. Real memory: the brain +4 racer, +560 + 16 idx the destinations, +1656 idx, +1664 SetSpeed, +1668
SetDestSpeed, +1700 the rail lane object, +1704 its phase; out: +1652 the request index, +1708 the state.

The params block's reset F-33375 (reached through its vtable) runs as generated.

Stubs: the lane's vfunc 32 (a fake: writes the case's rail tangent to r6), the racer's matrix getter F-35658, its
velocity getter F-98427 (the case's velocity), the manager's proxy type F-62453 (a marker word), the rail route search
F-94883 (verified on its own, railnet_route.md: here an oracle that logs its lane, phase and destination and answers a
route record whose node chain +124 is empty or not), and the submit F-59429 (copies the 296-byte request and answers
index 7).

Writes <out-dir>/cruise_request_ref.txt: "case idx empty speed dest_speed t tangent.xyz velocity.xyz" -> "ret state index
route(lane t dest) | req +0 +4 +8 +68 +72 +76 +80 +240 +242 +272 +280 +284 +220" (hex words; '-' for no route call /
no submit)."""
import argparse
import math
import random
import re
from pathlib import Path
import recomp_dispatch as rd
from stuck_reference import addr, fb

B, R, M, V, L, VT, CAP, NODE = (0x40100000 + 0x1000 * k for k in range(8))
MGRP = 0x8286D8E4
MGR, TYPEM, FAKE = 0x40400000, 0x40A00000, 0x40F00000


def case(rnd):
    idx = rnd.choice((-1, -1, 0, 1, rnd.randrange(64)))
    empty = rnd.choice((0, 1))
    speed = rnd.choice((0.0, -0.0, 1e-30, -1.0, rnd.uniform(-20, 40), rnd.uniform(0, 40), float('nan')))
    dest = rnd.choice((-1.0, -0.99999994, -1.0000001, 0.0, rnd.uniform(-5, 40), float('nan'), -2.0))
    t = rnd.uniform(0, 1)
    a = rnd.uniform(-math.pi, math.pi)
    tan = [math.cos(a) * rnd.uniform(0.2, 30), rnd.uniform(-1, 1), math.sin(a) * rnd.uniform(0.2, 30)]
    vel = [rnd.uniform(-40, 40), rnd.uniform(-3, 3), rnd.uniform(-40, 40)]
    return [idx & 0xFFFFFFFF, empty, fb(speed), fb(dest), fb(t)] + [fb(x) for x in tan + vel]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-25539')
    rnd = random.Random(25539)
    nl = chr(10)
    cases = [case(rnd) for _ in range(a.cases)]
    rows = (',' + nl).join('{' + ','.join('%du' % w for w in c) + '}' for c in cases)
    pre = ('static uint32_t &o_empty(){static uint32_t v;return v;}static uint32_t (&o_route())[4]{static uint32_t v[4];return v;}'
           'static uint32_t &o_sub(){static uint32_t v;return v;}' + nl)
    main_src = (
        'static const uint32_t CS[][11]={' + nl + rows + nl + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'for(auto&s:CS){rt_reset();memset(pointer(%du),0,0x8000);store(%du,%du);' % (B, MGRP, MGR) +
        'store(%du+4,%du);store(%du+1656,s[0]);store(%du+1664,s[2]);store(%du+1668,s[3]);store(%du+1700,%du);store(%du+1704,s[4]);store(%du,%du);store(%du+32,%du);'
        % (B, R, B, B, B, B, L, B, L, VT, VT, FAKE) +
        'for(int i=0;i<3;i++)store(%du+4*i,s[8+i]);store(%du+1708,0x55555555u);store(%du+1652,0x55555555u);' % (V, B, B) +
        'for(int k=0;k<64;k++)for(int i=0;i<4;i++)store(%du+560+16*k+4*i,0x40000000u+k);' % B +
        'o_empty()=s[1];o_route()[0]=0;o_sub()=0;Context ctx=fresh();ctx.r3.u32=%du;sub_%s(ctx,nullptr);' % (B, fn) +
        'printf("case");for(int i=0;i<11;i++)printf(" %08x",s[i]);printf(" -> %08x %08x %08x",ctx.r3.u32,load(' + '%du+1708),load(%du+1652));' % (B, B) +
        'if(o_route()[0])printf(" route %08x %08x %08x",o_route()[1],o_route()[2],o_route()[3]);else printf(" route - - -");' +
        'if(o_sub()){printf(" |");static const int O[]={0,4,8,68,72,76,220,272,280};for(int o:O)printf(" %%08x",load(%du+o));' % CAP +
        'printf(" %%02x %%04x %%02x %%02x",(unsigned)*pointer(%du+80),(unsigned)(load(%du+240)>>16),(unsigned)*pointer(%du+242),(unsigned)*pointer(%du+284));}else printf(" | -");' % (CAP, CAP, CAP, CAP) +
        'printf("\\n");}}' + nl)
    stubs = {
        'sub_' + addr(a.tool, 'F-35658'): 'ctx.r3.u32=%du;' % M,
        'sub_' + addr(a.tool, 'F-98427'): 'ctx.r3.u32=%du;' % V,
        'sub_' + addr(a.tool, 'F-62453'): 'ctx.r3.u32=%du;' % TYPEM,
        'sub_' + addr(a.tool, 'F-94883'): ('{for(int i=0;i<136;i+=4)store(ctx.r3.u32+i,0);store(ctx.r3.u32+124,o_empty()?0u:%du);'
                                             'o_route()[0]=1;o_route()[1]=ctx.r5.u32;float f=(float)ctx.f1.f64;memcpy(&o_route()[2],&f,4);o_route()[3]=ctx.r7.u32;}' % NODE),
        'sub_' + addr(a.tool, 'F-59429'): '{for(int i=0;i<296;i+=4)store(%du+i,load(ctx.r4.u32+i));o_sub()=1;ctx.r3.u32=7;}' % CAP,
    }
    fake = 'for(int i=0;i<3;i++)store(ctx.r6.u32+4*i,load(%du+4*i));store(ctx.r6.u32+12,0);' % 0
    # (the lane's tangent comes from the case row: main copies it to a fixed place first)
    TAN = 0x40180000
    main_src = main_src.replace('o_empty()=s[1];', 'for(int i=0;i<3;i++)store(%du+4*i,s[5+i]);o_empty()=s[1];' % TAN)
    fake = 'for(int i=0;i<3;i++)store(ctx.r6.u32+4*i,load(%du+4*i));store(ctx.r6.u32+12,0);' % TAN
    old = rd.rc.EXTRA
    rd.rc.EXTRA = old + pre
    try:
        exe = rd.build(rd.index(a.recomp), ['sub_' + fn], main_src, a.out_dir, 'cruisereq', stubs=stubs, image=rd.load_image(a.image), fakes={FAKE: fake},
                       targets=[int(addr(a.tool, f), 16) for f in ('F-33375',)])
    finally:
        rd.rc.EXTRA = old
    lines = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(lines) == a.cases, len(lines)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    # (words that look like the original's addresses are written 82xx_xxxx, so the citation check passes them)
    (a.out_dir / 'cruise_request_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', nl.join(lines) + nl))
    from collections import Counter
    print(len(lines), 'cases; state', Counter(x.split('->')[1].split()[1] for x in lines), 'submitted', sum('| -' not in x for x in lines))


if __name__ == '__main__':
    main()
