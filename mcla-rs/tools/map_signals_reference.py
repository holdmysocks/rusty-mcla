"""The signal update loop F-77331 run offline as generated over many junctions and many frames (cameras2-map4), written to
crates/game/src/testdata/map_signals_ref.txt for the port's tests. Built on tools/red_light_race_reference.py (same flat guest memory,
same vector-mode patches and stubs, imported), which checks the single junction update F-86376 on 3,000 cases; this run puts the loop
around it: the real F-77331 walks the junction array (element count through the rail-net getter, record stride, base) and hands every
junction the frame time. recomp_closure imports tools/memcap.py, so every process of this script (g++, the harness) runs in a 4 GB job.

Stubbed, as inputs of the case: the junction-clear test F-50959 (the case's per-junction, per-frame answer), the player getter F-65049
(an object whose virtual 4 returns the case's player), and the three lane callees of the box-hold test that the update does not reach.
The stub of the empty print routine is the image's own (a lone return).

A case is a list of hex words:
  m frames dt event clock0 clockstep flag0 flagframe flag1 forceframe forcej forceval forcered waitperiod waitj waitarm px py pz
  then per junction: n arms[8] cx cy cz current timer forced red demand[4] blockmask
Frame f of a case: the flag word is set (by the native F-15914) at f = flagframe; the forced arm / red byte of junction forcej are set
at f = forceframe; when waitperiod > 0 and f is a multiple of it, junction waitj's demand word waitarm is raised by one (a refused
car); the junction-clear answer is bit (f mod 32) of the junction's blockmask; the race clock (ms) is clock0 + f x clockstep, the
event byte is `event`; then F-77331 runs with dt. After each frame a 32-bit FNV-1a hash is taken over every junction's current arm,
timer bits and demand words (n of them). The line: "L <words> -> <hash per frame> | <final: per junction current timer demand[4]>".
Usage: map_signals_reference.py --recomp D:/MCLA-static --image IMG --out-dir DIR [--tool mcla-tool] [--cases N]"""
import argparse
import math
import random
import subprocess
from pathlib import Path
import struct
import recomp_dispatch as rd
import recomp_closure as rc


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def hi(x):
    return (x << 16) & 0xFFFFFFFF


_vs = rd.vector_shim


def _vector_shim():
    # (the SDK's own rex::ppc::simde_mm_cvtepu32_ps_, include/rex/ppc/intrinsics.h, for vcuxwfp128; the permute needs the AVX header)
    cvt = ('namespace rex { namespace ppc { inline simde__m128 simde_mm_cvtepu32_ps_(simde__m128i src1) {'
           'simde__m128i xmm1 = simde_mm_add_epi32(src1, simde_mm_set1_epi32(127));'
           'simde__m128i xmm0 = simde_mm_slli_epi32(src1, 31 - 8); xmm0 = simde_mm_srli_epi32(xmm0, 31);'
           'xmm0 = simde_mm_add_epi32(xmm0, xmm1); xmm0 = simde_mm_srai_epi32(xmm0, 8);'
           'xmm0 = simde_mm_add_epi32(xmm0, simde_mm_set1_epi32(0x4F800000)); simde__m128 xmm2 = simde_mm_cvtepi32_ps(src1);'
           'return simde_mm_blendv_ps(xmm2, simde_mm_castsi128_ps(xmm0), simde_mm_castsi128_ps(src1)); } } }')
    return '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <simde/x86/avx.h>' + chr(10) + cvt + chr(10) + _vs().replace('objects[8192]', 'objects[28672]').replace('a<12288', 'a<0x8000')


rd.vector_shim = _vector_shim

MGR = (0x2800 - 0x64238) & 0xFFFFFFFF
MGR_PTR = hi(-32121) - 10012     # the ambient manager pointer the update reads its race state through
LIMIT = hi(-32111) - 21532       # the box-hold limit (static init F-81092 fills it)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=600)
    ap.add_argument('--frames', type=int, default=240)
    a = ap.parse_args()
    if not (0 < a.cases <= 5000 and 0 < a.frames <= 2000):
        raise SystemExit('case or frame count out of range')
    loop, step, setter, getter, arm, init, clear, player, lanej, green, carpos = (addr(a.tool, f) for f in (
        'F-77331', 'F-86376', 'F-15914', 'F-12073', 'F-97162', 'F-81092', 'F-50959', 'F-65049', 'F-41621', 'F-78185', 'F-58860'))
    rnd = random.Random(77331)

    def arms(n):
        base = rnd.uniform(0, 2 * math.pi)
        out = []
        for k in range(n):
            t = base + k * 2 * math.pi / n + rnd.uniform(-0.3, 0.3)
            out += [fb(f32(math.cos(t))), fb(f32(math.sin(t)))]
        return out + [0] * (8 - len(out))

    cases = []
    for _ in range(a.cases):
        m = rnd.choice((1, 2, 3, 4, 4, 5, 6))
        dt = rnd.choice((1 / 60, 1 / 30, 0.0333333, 0.05, 0.1, rnd.uniform(0.01, 0.2)))
        event = int(rnd.random() < 0.5)
        clockstep = rnd.choice((16, 33, 50, 100))
        clock0 = rnd.choice((-3000, -1000, -200, 0, 1000, 1400, 5000, rnd.randrange(-6000, 6000)))
        flag0 = rnd.choice((0, 0, 0, 1, 2))
        flagframe = rnd.choice((-1, -1, rnd.randrange(0, a.frames)))
        flag1 = rnd.choice((0, 1, 0xFFFFFFFF))
        forceframe = rnd.choice((-1, -1, rnd.randrange(0, a.frames)))
        forcej = rnd.randrange(m)
        forceval = rnd.choice((-1, -1, 0, 1, 2))
        forcered = int(rnd.random() < 0.3)
        waitperiod = rnd.choice((0, 0, 5, 11, 23, 40))
        waitj = rnd.randrange(m)
        waitarm = rnd.randrange(2)
        pos = [fb(f32(rnd.uniform(-1500, 1500))), fb(f32(rnd.uniform(-20, 60))), fb(f32(rnd.uniform(-1500, 1500)))]
        words = [m, a.frames, fb(f32(dt)), event, clock0 & 0xFFFFFFFF, clockstep, flag0, flagframe & 0xFFFFFFFF, flag1, forceframe & 0xFFFFFFFF,
                 forcej, forceval & 0xFFFFFFFF, forcered, waitperiod, waitj, waitarm] + pos
        for j in range(m):
            n = rnd.choice((2, 3, 4, 4))
            c = [fb(f32(rnd.uniform(-1500, 1500))), fb(f32(rnd.uniform(-20, 60))), fb(f32(rnd.uniform(-1500, 1500)))]
            r = rnd.random()
            timer = (rnd.uniform(0, 16) if r < 0.4 else rnd.choice((10.0, 15.0)) - rnd.choice((0.0, dt, dt * 0.5)) if r < 0.6
                     else rnd.choice((0.0, 6.5, 8.5, 9.99, 10.0, 10.01, 14.99, 15.0, 15.01)))
            forced = rnd.randrange(n) if rnd.random() < 0.15 else -1
            dem = [rnd.choice((0, 0, 1, 2, 3, 5)) for _ in range(n)] + [0] * (4 - n)
            words += [n] + arms(n) + c + [rnd.randrange(n), fb(f32(timer)), forced & 0xFFFFFFFF, int(rnd.random() < 0.05)] + dem + [rnd.getrandbits(32)]
        cases.append(words)

    rc.EXTRA += 'static uint32_t BLKJ[8];' + chr(10)
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) + '/*%d*/' % len(c) for c in cases)
    run_flag = 'Context f=fresh();f.r3.u32=0x2F00;store(0x2F08,0x2F10);store(0x2F10,%s);sub_%s(f,nullptr);'
    body = (
        'static const std::vector<std::vector<uint32_t>> C={' + nl + rows + nl + '};' + nl +
        'for(auto&c:C){memset(objects,0,sizeof(objects));'
        'uint32_t m=c[0],frames=c[1],dtb=c[2],event=c[3],clock0=c[4],cstep=c[5],flag0=c[6],flagframe=c[7],flag1=c[8],forceframe=c[9],forcej=c[10],'
        'forceval=c[11],forcered=c[12],waitperiod=c[13],waitj=c[14],waitarm=c[15];'
        # the manager, its element table, the junction array
        'store(0x3000+8,0x3100);store(0x3100+88,0x4000);store(0x3100+144,0x100);store(0x3100+200,m);'
        'uint32_t BLKM[8]={0};uint32_t n[8];'
        'for(uint32_t j=0;j<m;j++){const uint32_t*w=&c[19+j*21];uint32_t J=0x4000+0x100*j,AR=0x5000+0x40*j,DM=0x5400+0x20*j,ST=0x5600+0x20*j;n[j]=w[0];'
        'store(J+44,AR);store16(J+48,w[0]);for(int k=0;k<4;k++){store(AR+16*k,w[1+2*k]);store(AR+16*k+8,w[2+2*k]);}'
        'store(J+80,w[9]);store(J+84,w[10]);store(J+88,w[11]);store(J+144,DM);store16(J+148,w[0]);store(J+152,ST);store16(J+156,w[0]);'
        'store(J+160,w[12]);store(J+164,w[13]);store(J+176,w[14]);objects[J-0x1000+180]=w[15];for(int k=0;k<4;k++)store(DM+4*k,w[16+k]);BLKM[j]=w[20];}'
        'store(0x1400,0x1480);store(0x1480+4,0x9100);store(0x1500+48,c[16]);store(0x1500+52,c[17]);store(0x1500+56,c[18]);'
        'store(0x2800,0x41200000);objects[0x2800-0x1000+0x19]=event;'
        + run_flag % ('flag0', setter) +
        'float dt;memcpy(&dt,&dtb,4);printf("L");'
        'for(uint32_t f=0;f<frames;f++){'
        'if(f==flagframe){' + run_flag % ('flag1', setter) + '}'
        'if(f==forceframe){uint32_t J=0x4000+0x100*forcej;store(J+176,forceval);objects[J-0x1000+180]=forcered;}'
        'if(waitperiod>0&&f%waitperiod==0&&waitarm<n[waitj]){uint32_t DM=0x5400+0x20*waitj;store(DM+4*waitarm,load(DM+4*waitarm)+1);}'
        'for(uint32_t j=0;j<m;j++)BLKJ[j]=(BLKM[j]>>(f%32))&1;'
        'store(0x2800+0x1C,clock0+f*cstep);'
        'Context ctx=fresh();ctx.r3.u32=0x3000;ctx.f1.f64=dt;sub_LOOP_(ctx,nullptr);' +
        'uint32_t h=2166136261u;auto mix=[&](uint32_t v){for(int i=0;i<4;i++){h^=(v>>(8*i))&0xFF;h*=16777619u;}};'
        'for(uint32_t j=0;j<m;j++){uint32_t J=0x4000+0x100*j;mix(load(J+160));mix(load(J+164));for(uint32_t k=0;k<n[j];k++)mix(load(0x5400+0x20*j+4*k));}'
        'printf(" %08x",h);}'
        'printf(" |");'
        'for(uint32_t j=0;j<m;j++){uint32_t J=0x4000+0x100*j;printf(" %08x %08x",load(J+160),load(J+164));for(int k=0;k<4;k++)printf(" %08x",load(0x5400+0x20*j+4*k));}'
        'printf("' + NL + '");}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
                'store(0x%08x,0x%08x);{Context i=fresh();sub_%s(i,nullptr);}' % (MGR_PTR, MGR, init) +
                'printf("limit %08x' + NL + '",load(0x%08x));' % LIMIT + nl + body + '}' + nl)
    stubs = {
        'sub_' + clear: '{int j=int((ctx.r3.u32-0x4000)/0x100);if(j<0||j>=8)throw std::runtime_error("clear test: not a junction");ctx.r3.u32=BLKJ[j];}',
        'sub_' + player: 'ctx.r3.u32=0x1400;',
        'sub_' + lanej: 'ctx.r3.u32=0x4000;',
        'sub_' + green: 'ctx.r3.u32=0;',
        'sub_' + carpos: 'ctx.r3.u32=0x1C00;',
    }
    main_src = main_src.replace('sub_LOOP_', 'sub_' + loop)
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    exe = rd.build(idx, ['sub_' + loop, 'sub_' + step, 'sub_' + setter, 'sub_' + getter, 'sub_' + init], main_src, a.out_dir, 'map_signals',
                stubs=stubs, scratch_end=0x8000, image=img, fakes={0x9100: 'ctx.r3.u32=0x1500;'})
    out = [x for x in subprocess.run([str(exe), str(a.image)], capture_output=True, text=True, check=True).stdout.split(nl) if x]
    assert out[0] == 'limit 41700000', out[0]
    out = out[1:]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['L ' + ' '.join('%x' % x for x in c) + ' -> ' + o[2:] for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'map_signals_ref.txt').write_text(nl.join(lines) + nl)
    print(len(cases), 'cases x', a.frames, 'frames')


if __name__ == '__main__':
    main()
