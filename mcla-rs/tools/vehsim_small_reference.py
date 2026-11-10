"""Small routines of the vehicle simulation run offline: the generated code unchanged (vector mode, the recomp's own
-msse4.1 -mssse3), on flat guest memory. Each case line of <out-dir>/vehsim_small_ref.txt is
"<tag> <input words> -> <output words>" (hex words, memory words in the guest's big-endian reading; a word that reads
82xxxxxx is written with an h in front). Sections (the tag, the row, what is compared):

  idle     F-86064  idle torque at an engine speed: the fallback (no idle curve) formula; the curve branch is a stub
  pedal    F-48349  torque for a pedal position (torque F-71482 / curve and idle F-86064 are stubs answering from the case)
  init     F-23005  engine init (the real ctor F-48129 first, the engine's curve builder and the curves' own
                    virtual functions real, the allocator a bump pointer): +68 +72 +76 +84 +88 and the torque +20
  drag     F-31519  the aero's drag numbers from the tune (the C library pow real)
  lying    F-75615  lying on the ground: air time (slot 13, a stub) and the up row's y
  hydload  F-60656  every wheel's static load for the hydraulics (the wheel's setter F-51861 a stub that records its argument)
  hydtune  F-10823  the front and back wheel tunes the hydraulics write, then hydload
  stuck    F-42631  vehStuck ctor
  fpoint   F-96311  a force at a point: the force, then the torque about the centre of mass (slots 32 and 33 record)
  thelper  F-29993  the body's slot 28, then slot 27 with the minus cross product (slots record)
  lever    F-61274  suspension lever's two reciprocals

Where a table is needed it is at 0x2800 with every slot 0x7F0000nn; `dispatch` maps the slots it needs. The guest vectors:
ctx.vN.u32[3-k] holds lane k. No denormal is generated for a vector input (the console flushes)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
import recomp_closure
from cite_safe import cite_safe

_vector_shim = recomp_closure.vector_shim
recomp_closure.vector_shim = lambda scratch_end=0x3000: '#include <simde/x86/avx2.h>' + chr(10) + _vector_shim(scratch_end).replace('int32_t s32;', 'int32_t s32; int16_t s16; int8_t s8;', 1).replace('throw std::runtime_error("unexpected guest access")', '{fprintf(stderr,"bad address %08x ",a);fflush(stderr);throw std::runtime_error("unexpected guest access");}')
recomp_closure.EXTRA += ('namespace rex { namespace ppc { static simde__m128 simde_mm_cvtepu32_ps_(simde__m128i v) { alignas(16) uint32_t a[4]; alignas(16) float f[4];' +
                         ' simde_mm_store_si128((simde__m128i*)a, v); for (int i = 0; i < 4; i++) f[i] = float(a[i]); return simde_mm_load_ps(f); } } }' + chr(10) +
                         '#define ctr r2' + chr(10) + 'static void dispatch(Context&,const uint8_t*,uint32_t);' + chr(10) +
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)' + chr(10) +
                         'static uint32_t REC[16][16], SV[16], WHICH, ncalls, ALLOC; static uint32_t NW, TERM; static uint32_t WREC[8][2]; static const int OFF[4]={76,88,92,100};' + chr(10) +
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}' + chr(10) +
                         'static uint32_t FB(double d){float r=float(d);uint32_t u;memcpy(&u,&r,4);return u;}' + chr(10) +
                         'static double FD(uint32_t u){float r;memcpy(&r,&u,4);return r;}' + chr(10))

NL = chr(92) + 'n'
nl = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def is_denormal(w):
    return (w >> 23) & 0xFF == 0 and (w & 0x7FFFFF) != 0


def word(rnd, scale=10.0):
    r = rnd.random()
    if r < 0.12:
        w = rnd.choice((0, 0x80000000, 0x3f800000, 0xbf800000, 0x7f800000, 0xff800000, 0x7fc00000, 0x00800000))
    elif r < 0.9:
        w = fb(rnd.uniform(-scale, scale))
    else:
        w = rnd.getrandbits(32)
    return 0 if is_denormal(w) else w


def fw(rnd, scale=10.0):
    """A float word for the vector routines: special values, a uniform number, or a power of two between 2^-30 and 2^30."""
    r = rnd.random()
    if r < 0.1:
        return rnd.choice((0, 0x80000000, 0x3f800000, 0xbf800000, 0x7f800000, 0xff800000, 0x7fc00000, 0x00800000 << 8))
    if r < 0.88:
        return fb(rnd.uniform(-scale, scale))
    return fb(rnd.choice((-1, 1)) * 2.0 ** rnd.uniform(-30, 30))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--only', default='')
    a = ap.parse_args()
    f = {k: addr(a.tool, v) for k, v in dict(idle='F-86064', pedal='F-48349', init='F-23005', ctor='F-48129', drag='F-31519', lying='F-75615',
                                              hydload='F-60656', hydtune='F-10823', stuck='F-42631', fpoint='F-96311', thelper='F-29993',
                                              lever='F-61274', settle='F-68237', setdown='F-72737', rider='F-32386').items()}
    # (the stubs, the curves' virtuals and the start-up initialisers are named by address: not rows of this batch)
    A = {k: addr(a.tool, v) for k, v in dict(idle_curve='F-47353', torque_curve='F-36517', torque_flat='F-91323', alloc='F-15488', wheel_setter='F-51861',
                                              crt1='F-50173', crt2='F-33402', crt3='F-26199', crt4='F-55749', slot24='F-96575', slot28='F-64958',
                                              curve_eval='F-39947', gear_for_speed='F-24547', is_rider='F-54299', rider_body='F-17180', rider_vel='F-84540',
                                              rider_flag='F-14919', rider_x1='F-74220', rider_x2='F-10364', rider_x3='F-55794', rider_x4='F-35412',
                                              rider_x5='F-58287').items()}
    rnd = random.Random(86064)
    secs = []   # (tag, cases, setup, fn, nout, outcode)
    FP = '{float x;memcpy(&x,&c[%d],4);ctx.f%d.f64=x;}'
    VT = 'for(int k=0;k<64;k++)store(0x2800+4*k,0x7F000000+k);'
    GLOBALS = 'for(int k=0;k<4;k++){store((0x8286u<<16)|(0x0950u+4*k),k<3?0xFFFFFFFFu:0);store((0x8286u<<16)|(0x0930u+4*k),k<3?0:0x3F800000u);}'
    FLAG = 'store8(((0x828Du<<16)|0xA40Cu),%s);'
    vload = lambda base, o: ';'.join('store(%s+%d,c[%d])' % (base, 4 * k, o + k) for k in range(4)) + ';'

    # idle: w, max hp, opt, idle, curve count, flag
    cs = []
    for _ in range(3000):
        opt = rnd.choice((fb(rnd.uniform(100, 1000)), word(rnd, 1000.0)))
        idle = rnd.choice((fb(rnd.uniform(50, 200)), word(rnd, 300.0), opt))
        w = rnd.choice((idle, idle, fb(rnd.uniform(0, 1100)), word(rnd, 1000.0), opt, 0, 0x80000000))
        cs.append([w, rnd.choice((fb(rnd.uniform(50, 900)), word(rnd, 900.0))), opt, idle, rnd.choice((0, 0, 0, 0, 12)), rnd.choice((1, 1, 0))])
    secs.append(('idle', cs, 'store(0x1000+4,0x1100);store(0x1100+4,c[1]);store(0x1000+72,c[2]);store(0x1000+76,c[3]);store16(0x1000+200,c[4]);' +
                 FLAG % 'c[5]' + FP % (0, 1) + 'ctx.r3.u32=0x1000;', f['idle'], 2, '(i==0?FB(ctx.f1.f64):ncalls)'))

    # pedal: w, pedal, timer, multiplier, boost duration, boost torque, torque (curve), torque (flat), opt, idle, max hp, curve count, flag
    cs = []
    for _ in range(3000):
        opt = rnd.choice((fb(rnd.uniform(100, 1000)), word(rnd, 1000.0)))
        cs.append([word(rnd, 900.0), rnd.choice((fb(rnd.uniform(0, 1)), fb(rnd.uniform(0, 1)), 0x3f000000, 0, 0x3f800000, 0x80000000, word(rnd, 2.0))),
                   rnd.choice((fb(rnd.uniform(0, 3)), word(rnd, 3.0))), rnd.choice((0x3f800000, fb(rnd.uniform(0.5, 1.5)), word(rnd, 2.0))),
                   rnd.choice((fb(rnd.uniform(0, 3)), 0, 0x80000000, word(rnd, 3.0))), rnd.choice((fb(rnd.uniform(0, 200)), word(rnd, 200.0))),
                   word(rnd, 400.0), word(rnd, 400.0), opt, rnd.choice((fb(rnd.uniform(50, 200)), word(rnd, 300.0), opt)), fb(rnd.uniform(50, 900)),
                   rnd.choice((0, 12, 12)), rnd.choice((0, 1, 1))])
    secs.append(('pedal', cs, 'store(0x1000+4,0x1100);store(0x1100+40,c[4]);store(0x1100+44,c[5]);store(0x1100+4,c[10]);store(0x1000+24,c[1]);store(0x1000+28,c[2]);' +
                 'store(0x1000+32,c[3]);store16(0x1000+120,c[11]);store(0x1000+72,c[8]);store(0x1000+76,c[9]);' + FLAG % 'c[12]' +
                 'SV[0]=c[6];SV[1]=c[7];WHICH=0;' + FP % (0, 1) + 'ctx.r3.u32=0x1000;',
                 f['pedal'], 3, '(i==0?FB(ctx.f1.f64):i==1?WHICH:REC[0][0])'))

    # drag: high gear mph, DragMin, DragMax, Down
    cs = []
    for _ in range(3000):
        cs.append([rnd.choice((fb(rnd.uniform(80, 250)), fb(rnd.uniform(0, 400)), word(rnd, 400.0), 0, 0x80000000)),
                   rnd.choice((fb(rnd.uniform(0, 5)), word(rnd, 5.0))), rnd.choice((fb(rnd.uniform(0, 5)), word(rnd, 5.0))),
                   rnd.choice((fb(rnd.uniform(0, 5)), word(rnd, 5.0)))])
    secs.append(('drag', cs, 'store(0x1000+12,0x1100);store(0x1100+368,0x1200);store(0x1200+148,0x1300);store(0x1300+12,c[0]);store(0x1000+8,0x1400);' +
                 'store(0x1400+88,c[1]);store(0x1400+92,c[2]);store(0x1400+68,c[3]);ctx.r3.u32=0x1000;', f['drag'], 3, 'load(0x1000+(i==0?24:i==1?28:32))'))

    # lying: air time, up y, sim present
    cs = [[rnd.choice((0, 0x80000000, fb(rnd.uniform(-1, 3)), fb(rnd.uniform(0, 0.01)), 0x7fc00000, 0x3f800000, 0xbf800000)),
           rnd.choice((fb(rnd.uniform(-1, 1)), fb(rnd.uniform(0.09, 0.11)), 0x3dcccccd, 0x3dccccce, 0x3dcccccc, 0, 0x80000000, 0x7fc00000)), rnd.choice((1, 1, 1, 0))]
          for _ in range(2000)]
    secs.append(('lying', cs, 'store(0x1000+8,0x1100);store(0x1100+8,c[2]?0x1200:0);store(0x1200,0x2800);' + VT + 'store(0x1200+260,c[1]);ctx.r3.u32=0x1000;SV[0]=c[0];',
                 f['lying'], 1, 'ctx.r3.u32&0xFF'))

    # hydload / hydtune: fitted byte, on, pattern, stiffness, mass, wheel count, four wheels' +428, then (hydtune) the extent and damp
    def hyd_case(tune):
        c = [rnd.choice((0, 1, 1, 1, 2)), rnd.choice((0, 0, 1, 2)), rnd.choice((0, 0, 3, 1)),
             rnd.choice((fb(rnd.uniform(0, 14)), fb(rnd.uniform(0.5, 1.5)), 0x3f800000, 0x41200000, 0x41300000, 0x7fc00000, word(rnd, 20.0))),
             rnd.choice((fb(rnd.uniform(500, 3000)), word(rnd, 3000.0))), rnd.choice((0, 2, 4, 4, 4, 1, 3)),
             *[rnd.choice((0, 0, fb(rnd.uniform(-5, 800)), word(rnd, 100.0))) for _ in range(4)]]
        if tune:
            c += [word(rnd, 1.0), word(rnd, 1.0)]
        return c

    hyd_setup = ('store8(0x1000+12,c[0]);store(0x1000+16,c[1]);store(0x1000+20,c[2]);store(0x1000+4,0x1100);store(0x1100+36,c[3]);' +
                 'store(0x1000+8,0x1200);store(0x1200+8,0x1300);store(0x1300+368,0x2000);store(0x2000+112,c[4]);store(0x1300+24,c[5]);store(0x1300+28,0x1500);' +
                 'for(int k=0;k<4;k++){store(0x1500+4*k,0x3000+0x200*k);store(0x3000+0x200*k,0x2800);store(0x3000+0x200*k+428,c[6+k]);}' + VT + 'NW=0;')
    hyd_out = '(i==0?NW:i==1?ncalls:(i-2<NW?WREC[i-2][1]:0))'
    secs.append(('hydload', [hyd_case(False) for _ in range(3000)], hyd_setup + 'ctx.r3.u32=0x1000;', f['hydload'], 6, hyd_out))
    tune_setup = ('store(0x1200+92,0x1700);store(0x1700+136,0x1800);store(0x1700+140,0x1900);for(int k=0;k<32;k++){store(0x1800+4*k,0xA5A50000+k);store(0x1900+4*k,0xA5A60000+k);}' +
                  'store(0x1100+28,c[10]);store(0x1100+32,c[11]);')
    secs.append(('hydtune', [hyd_case(True) for _ in range(3000)], hyd_setup + tune_setup + 'ctx.r3.u32=0x1000;', f['hydtune'], 14,
                 '(i<6?' + hyd_out + ':load((i<10?0x1800:0x1900)+OFF[(i-6)%4]))'))

    # stuck ctor: every word of the object prefilled
    secs.append(('stuck', [[rnd.getrandbits(32) for _ in range(20)] for _ in range(1000)], 'for(int i=0;i<20;i++)store(0x1000+4*i,c[i]);ctx.r3.u32=0x1000;',
                 f['stuck'], 20, 'load(0x1000+4*i)'))

    def vec4(scale=10.0):
        return [fw(rnd, scale) for _ in range(4)]

    # force at a point: force, point, the centre vector, four rows
    cs = []
    for _ in range(3000):
        scale = rnd.choice((10.0, 1.0, 0.1))
        cs.append(vec4(scale) + vec4(scale) + vec4(scale) + [fw(rnd, 2.0) if (k % 4) != 3 else rnd.choice((0, 0x3f800000, fw(rnd, 2.0))) for k in range(16)])
    secs.append(('fpoint', cs, 'store(0x1000+8,0x1100);store(0x1100+128,0x1200);store(0x1200,0x2800);' + VT + GLOBALS +
                 vload('0x1300', 0) + vload('0x1310', 4) + vload('0x1100+112', 8) +
                 'for(int k=0;k<16;k++)store(0x1100+304+4*k,c[12+k]);ctx.r3.u32=0x1000;ctx.r4.u32=0x1300;ctx.r5.u32=0x1310;', f['fpoint'], 9,
                 '(i<4?REC[0][i]:i<8?REC[1][i-4]:ncalls)'))

    # torque helper: V (r4), U (r5), the vector at the constant address
    cs = []
    for _ in range(3000):
        scale = rnd.choice((10.0, 1.0, 0.1))
        cs.append(vec4(scale) + vec4(scale) + vec4(scale))
    secs.append(('thelper', cs, 'store(0x1000+8,0x1100);store(0x1100+128,0x1200);store(0x1200,0x2800);' + VT +
                 vload('0x1300', 0) + vload('0x1310', 4) + vload('((0x8286u<<16)|0x08D0u)', 8) + 'ctx.r3.u32=0x1000;ctx.r4.u32=0x1300;ctx.r5.u32=0x1310;', f['thelper'], 15,
                 '(i<4?REC[0][i]:i<8?REC[1][i-4]:i<12?REC[2][i-8]:i==12?REC[3][0]:i==13?REC[3][1]:ncalls)'))

    # lever init: bone present, its 16 words (rows 0-2 and the place at +48), the wheel's vector at +384
    cs = []
    for _ in range(3000):
        scale = rnd.choice((3.0, 0.3, 0.05))
        c = [rnd.choice((1, 1, 1, 0))] + [fw(rnd, scale if k % 4 != 3 else 3.0) for k in range(16)] + [fw(rnd, scale) for _ in range(4)]
        cs.append(c)
    secs.append(('lever', cs, 'store(0x1000+8,c[0]?0x1100:0);store(0x1000+84,0x1300);for(int k=0;k<16;k++)store(0x1100+4*k,c[1+k]);' +
                 'for(int k=0;k<4;k++)store(0x1300+384+4*k,c[17+k]);store(0x1000+100,0xA5A5A5A5);store(0x1000+104,0xA5A5A5A5);ctx.r3.u32=0x1000;',
                 f['lever'], 2, 'load(0x1000+(i==0?100:104))'))

    # settle: the CG callback. this: cg (4 words), tune +272, tune +36, the u16 flag at (A+28)+10; n contacts of
    # (flag byte, self is side A, other kind 0 none / 1 id 0xFFFF / 2 present, table word, normal x y z w)
    cs = []
    for _ in range(3000):
        n = rnd.choice((0, 1, 1, 2, 2, 3, 4, 5))
        c = [n] + [fw(rnd, 2.0) for _ in range(4)] + [rnd.choice((fb(rnd.uniform(0, 1)), fw(rnd, 2.0))), rnd.choice((fb(rnd.uniform(0, 1)), fw(rnd, 2.0))),
                                                  rnd.choice((0x10, 0, 0xFFFF, 0x0F, 0x1234))]
        for k in range(5):
            ny = rnd.choice((fb(rnd.uniform(-0.2, 0.2)), fb(rnd.uniform(-0.2, 0.2)), 0x3dcccccd, 0xbdcccccd, 0x3dccccce, 0x3dcccccc, 0, 0x80000000, 0x7fc00000, fb(rnd.uniform(-1, 1))))
            c += [rnd.choice((0x80, 0x80, 0x80, 0xFF, 0x7F, 0, 0x81)), rnd.choice((0, 1)), rnd.choice((2, 2, 2, 0, 1)), (rnd.getrandbits(30) << 2) | rnd.choice((2, 2, 0, 1, 3)),
                  fw(rnd, 1.0), ny, fw(rnd, 1.0), fw(rnd, 1.0)]
        cs.append(c)
    secs.append(('settle', cs,
                 'store(0x1000+112,0);ctx.r3.u32=0x1000;ctx.r4.u32=0x1400;' + vload('0x1000+112', 1) +
                 'store(0x1000+368,0x1100);store(0x1100+272,c[5]);store(0x1100+36,c[6]);store(0x1000+4,0x1200);store(0x1200+28,0x1300);store16(0x1300+10,c[7]);' +
                 'store(0x1000+588,0xA5A5A5A5);store(0x1400,0x1500);store(0x1400+4,0x1600);store(0x1400+8,0);store(0x1400+12,c[0]);' +
                 'store((0x828Du<<16)|0xA464u,0x2000);store(0x2000+112,0x2100);' +
                 'for(int k=0;k<5;k++){uint32_t b=8+8*k,ct=0x1800+0x100*k,mf=0x1700+0x20*k,ob=0x4000+0x40*k;store(0x1500+4*k,ct);store8(ct+165,c[b]);store(ct+160,mf);' +
                 'for(int j=0;j<4;j++)store(ct+80+4*j,c[b+4+j]);uint32_t other=c[b+2]==0?0:ob;if(c[b+2]==1)store16(ob+8,0xFFFF);else store16(ob+8,k);' +
                 'store(0x2100+8*k+4,c[b+3]);if(c[b+1]){store(mf,0x1600);store(mf+4,other);}else{store(mf,other);store(mf+4,0x1600);}}',
                 f['settle'], 5, '(i<4?load(0x1000+112+4*i):load(0x1000+588))'))

    # setdown: the body (slots 41 and 43), the shafts, the gear for the speed (a stub; its answer from the case, the gear setter real) and the engine
    cs = []
    for _ in range(3000):
        c = [fw(rnd, 30.0) for _ in range(3)] + [fw(rnd, 1.0) for _ in range(3)] + [rnd.getrandbits(32)]
        c += [rnd.choice((1, 1, 1, 0)), rnd.choice((1, 1, 1, 0)), rnd.choice((0, 1, 2, 2, 3)), rnd.choice((0, 1, 2, 2, 3))]
        c += [rnd.choice((fb(rnd.uniform(0.2, 0.5)), fw(rnd, 1.0))) for _ in range(6)]
        c += [rnd.choice((1, 1, 1, 0)), rnd.choice((1, 1, 1, 0)), rnd.choice((fb(rnd.uniform(500, 900)), fw(rnd, 900.0))), rnd.choice((fb(rnd.uniform(50, 150)), fw(rnd, 300.0)))]
        c += [rnd.choice((0, 1, 1, 2)), rnd.choice((-1, 0, 1, 2, 3, 4, 5)) & 0xFFFFFFFF, rnd.choice((0, 1, 2, 3, 6)), rnd.randrange(-1, 7) & 0xFFFFFFFF]
        c += [rnd.choice((fb(rnd.uniform(-5, 5)), fw(rnd, 5.0))) for _ in range(16)]
        cs.append(c)
    secs.append(('setdown', cs,
                 'store(0x1000+128,0x1100);store(0x1100,0x2800);' + VT + 'store(0x1000+272,c[3]);store(0x1000+276,c[4]);store(0x1000+280,c[5]);' +
                 'for(int j=0;j<3;j++)store(0x1800+4*j,c[j]);ctx.r4.u32=0x1800;ctx.r5.u32=c[6];if(c[7])store(0x1000+400,0x1200);if(c[8])store(0x1000+404,0x1300);' +
                 'for(int s=0;s<2;s++){uint32_t sh=s?0x1300:0x1200;store(sh+20,c[9+s]);for(int k=0;k<3;k++){uint32_t wh=0x3000+0x800*s+0x200*k;store(sh+24+4*k,wh);putf(wh+420,FD(c[11+3*s+k]));store(wh+552,0xA5A5A5A5);}' +
                 'store(sh+48,0xA5A5A5A5);store(sh+52,0xA5A5A5A5);}' +
                 'if(c[17])store(0x1000+16,0x1400);if(c[18])store(0x1000+20,0x1500);store(0x1400+72,c[19]);store(0x1400+76,c[20]);store(0x1400+8,0xA5A5A5A5);' +
                 'store(0x1500+16,c[21]);store(0x1500+20,c[22]);store(0x1500+4,0x1600);store(0x1600,c[23]);store(0x1500+24,0xA5A5A5A5);store(0x1500+28,0xA5A5A5A5);' +
                 'ctx.r3.u32=0x1000;SV[0]=c[24];for(int k=0;k<16;k++)store(0x1500+32+4*k,c[25+k]);ncalls=0;',
                 f['setdown'], 19,
                 '(i==0?load(0x1200+48):i==1?load(0x1200+52):i<5?load(0x3000+0x200*(i-2)+552):i==5?load(0x1300+48):i==6?load(0x1300+52):i<10?load(0x3800+0x200*(i-7)+552):i==10?load(0x1400+8):i==11?REC[0][0]:i==12?REC[0][1]:i==13?REC[1][0]:i==14?REC[1][1]:i==15?REC[1][2]:i==16?load(0x1500+20):i==17?load(0x1500+24):load(0x1500+28))'))

    # rider: the damage object's frame update F-32386 finds the thrown rider's term by the one comparison it makes: the timer (+44)
    # runs down when the term is under the constant at ((0x8203u<<16)|0x5460u) (4.47). The harness searches that constant for the smallest value
    # above the term (a binary search over the bits of positive floats, a run each) and gives the term's own bits; a term that is
    # NaN or not under the largest float gives 0xFFFFFFFF. The callees are stubs answering from the case. Case: asks, rider found,
    # his +52, his velocity (3), the vtable's answer equal to the sim's word
    cs = []
    for _ in range(3000):
        scale = rnd.choice((5.0, 1.0, 0.3, 0.05, 30.0))
        c = [rnd.choice((1, 1, 1, 1, 1, 0)), rnd.choice((1, 1, 1, 1, 1, 0)), rnd.choice((fb(rnd.uniform(-70, 390)), fb(rnd.uniform(-70, 390)), fb(rnd.uniform(-70, 390)), fb(rnd.uniform(-70, 390)), 0xc2960000, 0x43c80000, 0, 0x7fc00000, fb(-80.0), fb(450.0)))]
        c += [fw(rnd, scale) for _ in range(3)] + [1]
        if rnd.random() < 0.1:
            c[3 + rnd.randrange(3)] = rnd.choice((0, 0x80000000, 0x7f800000, 0xff800000, 0x7fc00000, fb(1e19), fb(3e18)))
        cs.append(c)
    rider_prep = ('auto prep=[&](uint32_t thr){memset(objects,0,sizeof(objects));store(0x1000+4,0x1100);store(0x1100+128,0x1800);store(0x1100+8,0x1200);' +
                  'store(0x1200+224,0x7F7FFFFF);store(0x1200+24,0x1234);store(0x1200,0x2800);store(0x1000+44,0x3f800000);store(0x1000+28,0);' + VT +
                  'store(0x1900+52,c[2]);store8(0x1000+81,0);SV[1]=c[0];SV[2]=c[1];SV[3]=c[3];SV[4]=c[4];SV[5]=c[5];SV[6]=c[6]?0x1234:0x4321;' +
                  'store(((0x827Du<<16)|0x7558u),0x3f000000);store(((0x8203u<<16)|0x5460u),thr);Context k{};k.r1.u32=0x7F00;k.r3.u32=0x1000;sub_%s(k,nullptr);return load(0x1000+44)!=0x3f800000;};' % f['rider'] +
                  'uint32_t lo=0,hi=0x7F7FFFFF;if(!prep(hi)){TERM=0xFFFFFFFF;}else{while(hi-lo>1){uint32_t mid=lo+(hi-lo)/2;if(prep(mid))hi=mid;else lo=mid;}TERM=hi-1;}' +
                  'store(((0x8203u<<16)|0x5460u),0x408f0a3d);ctx.r3.u32=0;')
    secs.append(('rider', cs, rider_prep.replace('ctx.r3.u32=0;', 'ctx.r3.u32=0x1000;'), f['rider'], 1, 'TERM'))

    # init: the engine's ctor first, then init on a tune (hp, idle, opt, max rpm, boost duration, +60 word, count n, torque values, idle values)
    cs = []
    for k in range(2500):
        n = rnd.choice((0, 0, 3, 4, 5, 6, 7, 8, 9, 10, 10))
        opt = rnd.uniform(2500, 9000)
        mx = opt + rnd.choice((rnd.uniform(100, 4000), 1000.0, 500.0, 0.0, -300.0))
        c = [fb(x) for x in (rnd.choice((rnd.uniform(40, 900), 150.0)), rnd.uniform(500, 1500), opt, mx, rnd.choice((0.0, 1.0, 2.0, rnd.uniform(0, 5))))]
        c += [rnd.choice((0, 1, 1)), n]
        peak = rnd.uniform(100, 600)
        tv = [int(max(0, peak * rnd.uniform(0.3, 1.0))) for _ in range(n)] + [0] * (10 - n)
        if n and rnd.random() < 0.5:
            tv = [int(peak * (1 - abs(i - n * rnd.uniform(0.3, 0.8)) / n)) for i in range(n)] + [0] * (10 - n)
        if rnd.random() < 0.1:
            tv = [rnd.choice((-5, 0, 100, 400)) for _ in range(10)]
        iv = [int(rnd.uniform(-800, 200)) for _ in range(10)]
        c += [x & 0xFFFFFFFF for x in tv] + [x & 0xFFFFFFFF for x in iv]
        cs.append(c)
    init_sec = ('init', cs,
                'ALLOC=0x4000;store(0x1400+4,c[0]);store(0x1400+8,c[1]);store(0x1400+12,c[2]);store(0x1400+16,c[3]);store(0x1400+40,c[4]);store(0x1400+52,c[6]);' +
                'for(int k=0;k<10;k++){store(0x1400+56+4*k,c[7+k]);store(0x1400+100+4*k,c[17+k]);}' +
                'ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);store(0x1004,0x1400);store(0x1000+60,c[5]);store8(((0x828Du<<16)|0xA40Cu),1);' % f['ctor'] +
                'ctx=Context{};ctx.r1.u32=0x7F00;ctx.r3.u32=0x1000;', f['init'], 8,
                '(i==0?load(0x1000+68):i==1?load(0x1000+72):i==2?load(0x1000+76):i==3?load(0x1000+84):i==4?load(0x1000+88):i==5?load(0x1000+20):i==6?load(0x1000+28):uint32_t(load16(0x1000+120)))')

    if a.only:
        secs = [s for s in secs if s[0] in a.only.split(',')]
        if 'init' not in a.only.split(','):
            init_sec = None
    stubs_a = {
        'sub_' + A['idle_curve']: 'ncalls++;ctx.f1.f64=FD(0x3e8f5c29);',
        'sub_' + A['torque_curve']: '{float x=float(ctx.f1.f64);memcpy(&REC[0][0],&x,4);WHICH|=1;ctx.f1.f64=FD(SV[0]);}',
        'sub_' + A['torque_flat']: '{float x=float(ctx.f1.f64);memcpy(&REC[0][0],&x,4);WHICH|=2;ctx.f1.f64=FD(SV[1]);}',
        'sub_' + A['wheel_setter']: 'WREC[NW][0]=ctx.r3.u32;WREC[NW][1]=FB(ctx.f1.f64);NW++;',
    }
    disp_a = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){switch(a){' +
              'case 0x7F000020:{for(int k=0;k<4;k++)REC[0][k]=ctx.v1.u32[3-k];ncalls++;}return;' +
              'case 0x7F000021:{for(int k=0;k<4;k++)REC[1][k]=ctx.v1.u32[3-k];ncalls++;}return;' +
              'case 0x7F00001C:{for(int k=0;k<4;k++)REC[0][k]=ctx.v1.u32[3-k];ncalls++;}return;' +
              'case 0x7F00001B:{for(int k=0;k<4;k++)REC[1][k]=ctx.v1.u32[3-k];for(int k=0;k<4;k++)REC[2][k]=load(ctx.r4.u32+4*k);REC[3][0]=ctx.r4.u32;REC[3][1]=ctx.r5.u32;ncalls++;}return;' +
              'case 0x7F000029:REC[1][0]++;return;case 0x7F00002B:REC[1][1]=ctx.r4.u32;REC[1][2]++;return;' +
              'case 0x7F00000B:ctx.r3.u32=SV[6];return;case 0x7F00000D:ctx.f1.f64=FD(SV[0]);return;case 0x7F000001:ncalls++;return;' +
              'default:throw std::runtime_error("unexpected indirect call");}}')
    stubs_a.update({'sub_' + A['is_rider']: 'ctx.r3.u32=SV[1];', 'sub_' + A['rider_body']: 'ctx.r3.u32=SV[2]?0x1900:0;',
                    'sub_' + A['rider_vel']: 'store(ctx.r4.u32,SV[3]);store(ctx.r4.u32+4,SV[4]);store(ctx.r4.u32+8,SV[5]);', 'sub_' + A['rider_flag']: 'ctx.r3.u32=0;',
                    'sub_' + A['rider_x1']: '', 'sub_' + A['rider_x2']: '', 'sub_' + A['rider_x3']: '', 'sub_' + A['rider_x4']: '', 'sub_' + A['rider_x5']: ''})
    stubs_a['sub_' + A['gear_for_speed']] = 'REC[0][0]=FB(ctx.f1.f64);REC[0][1]++;ctx.r3.u32=SV[0];'
    stubs_b = {'sub_' + A['alloc']: '{uint32_t r=ALLOC;ALLOC+=0x400;ctx.r3.u32=r;}'}
    disp_b = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){switch(a){' +
              ''.join('case 0x%s:sub_%s(ctx,base);return;' % (A[k], A[k]) for k in ('slot24', 'slot28', 'curve_eval')) +
              'default:throw std::runtime_error("unexpected indirect call");}}')
    plan = [('a', secs, stubs_a, disp_a)]
    if init_sec:
        plan.append(('b', [init_sec], stubs_b, disp_b))
    results = {}
    for gname, gsecs, gstubs, gdisp in plan:
        if not gsecs:
            continue
        body = ''
        for tag, cs, setup, fn, nout, outc in gsecs:
            w = max(len(c) for c in cs)
            rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c + [0] * (w - len(c))) for c in cs)
            pre = ''
            if gname == 'b':
                pre = 'Context g{};g.r1.u32=0x7F00;sub_%s(g,nullptr);sub_%s(g,nullptr);sub_%s(g,nullptr);sub_%s(g,nullptr);' % (A['crt1'], A['crt2'], A['crt3'], A['crt4'])
            body += ('{' + pre + 'static const uint32_t C[][%d]={%s%s%s};' % (w, nl, rows, nl) +
                     'for(auto&c:C){memset(objects,0,sizeof(objects));Context ctx{};ctx.r1.u32=0x7F00;ncalls=0;memset(REC,0,sizeof(REC));' + setup +
                     'sub_%s(ctx,nullptr);' % fn +
                     'for(int i=0;i<%d;i++)printf("%%08x ",uint32_t(%s));printf("%s");fflush(stdout);}}' % (nout, outc, NL) + nl)
        names = {'sub_' + s[3] for s in gsecs}
        if gname == 'b':
            names |= {'sub_' + A[k] for k in ('crt1', 'crt2', 'crt3', 'crt4', 'slot24', 'slot28', 'curve_eval')}
            names.add('sub_' + f['ctor'])
        names -= set(gstubs)
        main_src = ('static void store8(uint32_t a,uint8_t v){*pointer(a)=v;}' + nl + gdisp + nl +
                    'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
        exe = build(index(a.recomp), sorted(names), main_src, a.out_dir, 'vehsim_small_' + gname, stubs=gstubs, vector=True, scratch_end=0x8000, flags=('-msse4.1', '-mssse3'))
        out = [x for x in run(exe, a.image).split(nl) if x]
        assert len(out) == sum(len(s[1]) for s in gsecs), (gname, len(out), sum(len(s[1]) for s in gsecs))
        k = 0
        for tag, cs, *_ in gsecs:
            for c in cs:
                results.setdefault(tag, []).append('%s %s -> %s' % (tag, ' '.join('%08x' % x for x in c), out[k].strip()))
                k += 1
    lines = [l for tag in results for l in results[tag]]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'vehsim_small_ref.txt').write_text(cite_safe(nl.join(lines) + nl))
    print({t: len(v) for t, v in results.items()})


if __name__ == '__main__':
    main()
