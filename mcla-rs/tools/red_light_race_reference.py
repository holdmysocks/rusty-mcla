"""The red light race flag's two readers run offline: the junction update F-86376 and the riders' light F-16850, the
generated code whole and unchanged in vector mode, with the flag setter F-15914, the getter F-12073, the arm of a point
F-97162 and the static init of the box-hold limit F-81092 (the image holds 0.0 there until it runs; it is run once
before the cases) in the closure, on flat guest memory. The flag (the image's byte at lis -32111 / -21536) is set by running the
native F-15914 with the case's argument word, as the script does.

Stubbed, as inputs of the case: the junction-clear test F-50959 (the case's answer), the player getter F-65049 (an
object whose virtual 4 returns the case's player; position at +48), the lane's junction F-41621 (the case's junction),
the lane's green test F-78185 (the case's answer) and the car's position F-58860 (the case's point).

Junction J at 0x1100: +44 the arms (16 bytes each, 0x1200), +48 their count, +64 the reach, +80 the centre, +144 the
demand words (0x1300) and +148 their count, +152 the time stamps (0x1340) and +156 their count, +160 current, +164
timer, +176 forced arm, +180 forced red. The ambient manager pointer (the image's word at lis -32121 / -10012) points so that its +0x64238
(ambient clock), +0x64251 (the race / event byte) and +0x64254 (the race clock, ms) land in scratch at 0x2800.

Writes <out-dir>/red_light_race_ref.txt, one case a line: 's' (F-86376) or 'w' (F-16850), the case's words, '->', then
for 's' current, the timer bits and the demand words; for 'w' the returned go and the demand words (all hex)."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

_vshim = recomp_closure.vector_shim


def vector_shim():
    # (the SDK's own rex::ppc::simde_mm_cvtepu32_ps_, include/rex/ppc/intrinsics.h, for vcuxwfp128)
    cvt = ('namespace rex { namespace ppc { inline simde__m128 simde_mm_cvtepu32_ps_(simde__m128i src1) {'
           'simde__m128i xmm1 = simde_mm_add_epi32(src1, simde_mm_set1_epi32(127));'
           'simde__m128i xmm0 = simde_mm_slli_epi32(src1, 31 - 8); xmm0 = simde_mm_srli_epi32(xmm0, 31);'
           'xmm0 = simde_mm_add_epi32(xmm0, xmm1); xmm0 = simde_mm_srai_epi32(xmm0, 8);'
           'xmm0 = simde_mm_add_epi32(xmm0, simde_mm_set1_epi32(0x4F800000)); simde__m128 xmm2 = simde_mm_cvtepi32_ps(src1);'
           'return simde_mm_blendv_ps(xmm2, simde_mm_castsi128_ps(xmm0), simde_mm_castsi128_ps(src1)); } } }')
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <simde/x86/avx.h>' + chr(10) + cvt + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    return s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                     'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)


recomp_closure.vector_shim = vector_shim
recomp_closure.EXTRA += ('#define ctr r2\nstatic const uint32_t *K;static uint32_t BLK;\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n')
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)

MGR = (0x2800 - 0x64238) & 0xFFFFFFFF
# image addresses as the code builds them (lis hi, offset lo)
MGR_PTR = (-32121 << 16) + -10012 & 0xFFFFFFFF
LIMIT = (-32111 << 16) + -21532 & 0xFFFFFFFF


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    step, light, setter, getter, arm, init, clear, player, lanej, green, carpos = (addr(a.tool, f) for f in (
        'F-86376', 'F-16850', 'F-15914', 'F-12073', 'F-97162', 'F-81092', 'F-50959', 'F-65049', 'F-41621', 'F-78185',
        'F-58860'))
    rnd = random.Random(15914)

    def arms(n):
        base = rnd.uniform(0, 2 * math.pi)
        out = []
        for k in range(n):
            t = base + k * 2 * math.pi / n + rnd.uniform(-0.3, 0.3)
            out += [fb(f32(math.cos(t))), fb(f32(math.sin(t)))]
        return out + [0] * (8 - len(out))

    def centre():
        return [fb(f32(rnd.uniform(-1500, 1500))), fb(f32(rnd.uniform(-20, 60))), fb(f32(rnd.uniform(-1500, 1500)))]

    # F-86376: n, arms[8], centre[3], current, timer, forced, red, flag word, event, clock, blocked, dt, demand[4], player[3]
    steps = []
    for _ in range(a.cases):
        n = rnd.choice((2, 3, 4, 4))
        c = centre()
        cur = rnd.randrange(n)
        dt = rnd.choice((1 / 60, 1 / 30, 0.0333333, rnd.uniform(0, 0.1)))
        r = rnd.random()
        timer = (rnd.uniform(0, 16) if r < 0.5 else rnd.choice((10.0, 15.0)) - rnd.choice((0.0, dt, dt * 0.5)) if r < 0.8
                 else rnd.choice((0.0, 6.5, 8.5, 9.99, 10.0, 10.01, 14.99, 15.0, 15.01)))
        forced = rnd.randrange(n) if rnd.random() < 0.3 else -1
        red = int(rnd.random() < 0.15)
        flag = rnd.choice((0, 0, 1, 2, 0xFFFFFFFF, 0x100))
        ev = int(rnd.random() < 0.6)
        clock = rnd.choice((rnd.randrange(-8000, 0), -1, 0, 1499, 1500, rnd.randrange(0, 1500), rnd.randrange(1500, 600000)))
        blocked = int(rnd.random() < 0.35)
        dem = [rnd.choice((0, 0, 1, 2, 3, 5)) for _ in range(n)] + [0] * (4 - n)
        ang = rnd.uniform(0, 2 * math.pi)
        dist = rnd.uniform(1, 80)
        p = [fb(f32(f32(struct.unpack('>f', struct.pack('>I', c[0]))[0]) + dist * math.cos(ang))), c[1],
             fb(f32(f32(struct.unpack('>f', struct.pack('>I', c[2]))[0]) + dist * math.sin(ang)))]
        steps.append([n] + arms(n) + c + [cur, fb(f32(timer)), forced & 0xFFFFFFFF, red, flag, ev, clock & 0xFFFFFFFF, blocked,
                                          fb(f32(dt))] + dem + p)
    # F-16850: n, arms[8], centre[3], reach, green, b500, flag word, blocked, demand[4], pos[3]
    waits = []
    for _ in range(a.cases):
        n = rnd.choice((2, 3, 4, 4))
        c = centre()
        reach = f32(rnd.uniform(5, 40))
        ang = rnd.uniform(0, 2 * math.pi)
        dist = rnd.choice((rnd.uniform(0, 1.3 * reach), reach, f32(reach * 0.9999999), f32(reach * 1.0000001)))
        cx, cz = (struct.unpack('>f', struct.pack('>I', c[k]))[0] for k in (0, 2))
        p = [fb(f32(cx + dist * math.cos(ang))), fb(f32(rnd.uniform(-20, 60))), fb(f32(cz + dist * math.sin(ang)))]
        dem = [rnd.choice((0, 0, 1, 2, 7)) for _ in range(n)] + [0] * (4 - n)
        waits.append([n] + arms(n) + c + [fb(reach), int(rnd.random() < 0.5), rnd.choice((0, 0x20, 0x10, 0x30, 0xFF, 0x40)),
                                          rnd.choice((0, 1, 0, 0xFFFFFFFF)), int(rnd.random() < 0.35)] + dem + p)
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = lambda cs: nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cs)
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){'
            'if(a==0x9100)ctx.r3.u32=0x1500;else throw std::runtime_error("unexpected indirect call");}')
    junction = ('memset(objects,0,sizeof(objects));K=c;'
                'store(0x1100+44,0x1200);store16(0x1100+48,c[0]);for(int k=0;k<4;k++){store(0x1200+16*k,c[1+2*k]);'
                'store(0x1200+16*k+8,c[2+2*k]);}store(0x1100+80,c[9]);store(0x1100+84,c[10]);store(0x1100+88,c[11]);'
                'store(0x1100+144,0x1300);store16(0x1100+148,c[0]);store(0x1100+152,0x1340);store16(0x1100+156,c[0]);')
    run_flag = 'Context f{};f.r1.u32=0x2700;f.r3.u32=0x2F00;store(0x2F08,0x2F10);store(0x2F10,%s);sub_%s(f,nullptr);'
    s_body = ('static const uint32_t S[][%d]={' % len(steps[0]) + nl + rows(steps) + nl + '};' + nl +
              'for(auto&c:S){' + junction +
              'store(0x1100+160,c[12]);store(0x1100+164,c[13]);store(0x1100+176,c[14]);objects[0x1100-0x1000+180]=c[15];'
              + run_flag % ('c[16]', setter) +
              'objects[0x2800-0x1000+0x19]=c[17];store(0x2800+0x1C,c[18]);store(0x2800,0x41200000);'
              'for(int k=0;k<4;k++)store(0x1300+4*k,c[21+k]);'
              'store(0x1400,0x1480);store(0x1480+4,0x9100);store(0x1500+48,c[25]);store(0x1500+52,c[26]);store(0x1500+56,c[27]);'
              'BLK=c[19];Context ctx{};ctx.r1.u32=0x2700;ctx.r3.u32=0x1100;{float d;uint32_t b=c[20];memcpy(&d,&b,4);ctx.f1.f64=d;}'
              'sub_%s(ctx,nullptr);' % step +
              ('printf("%%08x %%08x %%08x %%08x %%08x %%08x%s",load(0x1100+160),load(0x1100+164),load(0x1300),load(0x1304),'
               'load(0x1308),load(0x130C));}' % NL) + nl)
    w_body = ('static const uint32_t W[][%d]={' % len(waits[0]) + nl + rows(waits) + nl + '};' + nl +
              'for(auto&c:W){' + junction +
              'store(0x1100+64,c[12]);' + run_flag % ('c[15]', setter) +
              'for(int k=0;k<4;k++)store(0x1300+4*k,c[17+k]);'
              'store(0x1000+16,0x1800);objects[0x1800-0x1000+500]=c[14];store(0x1C00,c[21]);store(0x1C04,c[22]);store(0x1C08,c[23]);'
              'BLK=c[16];Context ctx{};ctx.r1.u32=0x2700;ctx.r3.u32=0x1000;ctx.r4.u32=0x1A00;sub_%s(ctx,nullptr);' % light +
              ('printf("%%02x %%08x %%08x %%08x %%08x%s",ctx.r3.u8,load(0x1300),load(0x1304),load(0x1308),load(0x130C));}' % NL)
              + nl)
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
                # the ambient manager pointer and the static init of the box-hold limit
                'store(0x%08x,0x%08x);{Context i{};i.r1.u32=0x2700;sub_%s(i,nullptr);}' % (MGR_PTR, MGR, init) +
                'printf("limit %08x' + NL + '",load(0x%08x));' % LIMIT + nl + s_body + w_body + '}' + nl)
    stubs = {
        'sub_' + clear: 'ctx.r3.u32=BLK;',
        'sub_' + player: 'ctx.r3.u32=0x1400;',
        'sub_' + lanej: 'ctx.r3.u32=0x1100;',
        'sub_' + green: 'ctx.r3.u32=K[13];',
        'sub_' + carpos: 'ctx.r3.u32=0x1C00;',
    }
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + step, 'sub_' + light, 'sub_' + setter, 'sub_' + getter, 'sub_' + init], main_src, a.out_dir,
                'red_light_race', stubs=stubs, vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert out[0] == 'limit 41700000', out[0]
    out = out[1:]
    assert len(out) == len(steps) + len(waits), (len(out), len(steps), len(waits))
    lines = (['s ' + ' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(steps, out)] +
             ['w ' + ' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(waits, out[len(steps):])])
    for l in lines:
        for wd in l.split():
            assert not (len(wd) == 8 and wd.startswith('82')), l
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'red_light_race_ref.txt').write_text(nl.join(lines) + nl)
    so = out[:len(steps)]
    wo = out[len(steps):]
    print(len(steps), 'update cases (flag on', sum(c[16] != 0 for c in steps), '),', len(waits), 'light cases (flag on',
          sum(c[15] != 0 for c in waits), '); update: timer 10 held', sum(o.split()[1] == '41200000' for o in so),
          '; light: go', sum(o.split()[0] == '01' for o in wo), ', go by the flag on red',
          sum(o.split()[0] == '01' and c[13] == 0 for c, o in zip(waits, wo)))


if __name__ == '__main__':
    main()
