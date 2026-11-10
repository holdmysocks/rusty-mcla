"""Lamp, paint and reset routines of the vehicle code run offline: the generated code unchanged (vector mode, the recomp's
own -msse4.1 -mssse3), on flat guest memory. Each case line of <out-dir>/vehicles_small_ref.txt is
"<tag> <input words> -> <output words>" (hex words, memory words in the guest's big-endian reading; a word that reads
82xxxxxx is written with an h in front). Sections (the tag, the row, what is compared):

  paint    F-44484  paint rows: pow(x, 2.2) of three floats, the fourth passed through (the C library pow F-21955 real)
  lights   F-75961  an ambient car's lights-on getter (the hour getter F-22302 real; the object's virtual slot a stub)
  nitro    F-40379  the nitro counter's reset
  winreset F-42929  the window controller's reset (the sound stop F-31667 a stub that records its arguments)
  lampstate F-96984 the lamp state table by attachment and the light set's levels (the render helpers F-38254, F-31647,
                    F-75931 stubs; pow F-21955 real)
  lamps    F-10129  the lamp controller's targets and easing (the ease F-24995, the move F-64978, the hour F-22302 and
                    cos F-63589 real; the stealth check F-54047 and the engine speed F-12794 stubs)
  traffic  F-81135  a traffic car's lamp flags, easing, glow fade and level rows (the gate F-89245 a stub that
                    answers yes; the upload F-49426 a stub that records its two row arrays)

Globals the original fills at start-up are zero in the image and are filled here. No denormal is generated for a vector
input (the console flushes)."""
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
                         'static uint32_t REC[64][4], SV[16], WHICH, ncalls, NREC;' + chr(10) +
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}' + chr(10) +
                         'static uint32_t FB(double d){float r=float(d);uint32_t u;memcpy(&u,&r,4);return u;}' + chr(10) +
                         'static double FD(uint32_t u){float r;memcpy(&r,&u,4);return r;}' + chr(10))

NL = chr(92) + 'n'
nl = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def A(imm, off):
    """The guest address of a `lis` immediate (as the generated code prints it) plus an offset, as a C++ expression."""
    v = ((imm & 0xFFFFFFFF) + off) & 0xFFFFFFFF
    return '((0x%04Xu<<16)|0x%04Xu)' % (v >> 16, v & 0xFFFF)


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def bf(w):
    return struct.unpack('>f', struct.pack('>I', w))[0]


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
    """A float word: special values, a uniform number, or a power of two between 2^-30 and 2^30."""
    r = rnd.random()
    if r < 0.1:
        return rnd.choice((0, 0x80000000, 0x3f800000, 0xbf800000, 0x7f800000, 0xff800000, 0x7fc00000, 0x00800000 << 8))
    if r < 0.88:
        return fb(rnd.uniform(-scale, scale))
    return fb(rnd.choice((-1, 1)) * 2.0 ** rnd.uniform(-30, 30))


def unit(rnd):
    """A state or level: mostly 0..1, with the edges."""
    return rnd.choice((0, 0x3f800000, 0x80000000, fb(rnd.uniform(0, 1)), fb(rnd.uniform(0, 1)), fb(rnd.uniform(0, 1)), fb(rnd.uniform(0.98, 1.0)),
                       fb(rnd.uniform(0, 0.02)), fw(rnd, 2.0)))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--only', default='')
    a = ap.parse_args()
    ids = dict(paint='F-44484', lights='F-75961', nitro='F-40379', winreset='F-42929', lampstate='F-96984', lamps='F-10129', traffic='F-81135',
               pow='F-21955', move='F-64978', hour='F-22302', stealth='F-54047', engine_speed='F-12794', cosine='F-63589',
               helper_a='F-38254', helper_b='F-31647', helper_c='F-75931', gate='F-89245', upload='F-49426', stop='F-31667', ease='F-24995')
    f = {k: addr(a.tool, v) for k, v in ids.items()}
    rnd = random.Random(44484)
    secs = []   # (tag, cases, setup, fn, nout, outcode)
    VT = 'for(int k=0;k<64;k++)store(0x1200+4*k,0x7F000000+k);'

    # paint: x y z w -> out x y z w
    def pw(r):
        c = r.random()
        if c < 0.25:
            return fb(r.uniform(0, 2))
        if c < 0.5:
            return fb(2.0 ** r.uniform(-40, 8))
        if c < 0.6:
            return r.choice((0, 0x80000000, 0x3f800000, 0x7f800000, 0xff800000, 0x7fc00000, 0xbf800000, 0x00000001, 0x007fffff, 0x00800000, 0x7f7fffff))
        if c < 0.7:
            return fb(r.uniform(-3, 0))
        return fb(r.uniform(0, 1))
    cs = [[pw(rnd), pw(rnd), pw(rnd), rnd.choice((fb(rnd.uniform(0, 1)), 0x3f800000, 0, word(rnd, 3.0)))] for _ in range(4000)]
    secs.append(('paint', cs, 'for(int k=0;k<3;k++)store(0x1200+4*k,c[k]);ctx.f1.f64=FD(c[3]);ctx.r3.u32=0x1100;ctx.r4.u32=0x1200;',
                 f['paint'], 4, 'load(0x1100+4*i)'))

    # lights: fade (+484), has the light object, its slot's answer, selector byte, the two indices, the table rows' +8 and +12 (for
    # both indices), sunrise, sunset, hour (for both slots), offset
    cs = []
    for _ in range(4000):
        hr = rnd.choice((fb(rnd.uniform(0, 24)), fb(rnd.uniform(5.5, 7.0)), fb(rnd.uniform(17.0, 18.5)), word(rnd, 30.0)))
        off = rnd.choice((fb(rnd.uniform(0, 0.25)), fb(rnd.uniform(0, 0.25)), 0, 0x80000000, word(rnd, 0.5)))
        sr = rnd.choice((fb(6.0), fb(6.0), fb(rnd.uniform(5, 7)), word(rnd, 10.0)))
        ss = rnd.choice((fb(18.0), fb(18.0), fb(rnd.uniform(17, 19)), word(rnd, 30.0)))
        # (an hour a hair off the edge: from the rounded edge itself)
        if rnd.random() < 0.15:
            e = rnd.choice(((ss, 1), (sr, 2)))
            v = bf(e[0])
            hr = fb(v - 0.333 - bf(off)) if e[1] == 1 else fb(v + 0.5 + bf(off))
            hr = (hr + rnd.choice((-2, -1, 0, 0, 1, 2))) & 0xFFFFFFFF
            hr = 0 if is_denormal(hr) else hr
        cs.append([rnd.choice((fb(rnd.uniform(0, 1)), 0x3dcccccd, 0x3dccccce, 0x3dcccccc, 0, 0x3f800000, word(rnd, 2.0))), rnd.choice((1, 1, 1, 0)), rnd.choice((0, 0, 0, 1)),
                   rnd.choice((0, 1, 5)), rnd.randrange(0, 3), rnd.randrange(0, 3),
                   rnd.choice((0, 0, fb(rnd.uniform(0, 1)), word(rnd, 1.0))), rnd.choice((0, 0, fb(rnd.uniform(0, 1)), word(rnd, 1.0))),
                   rnd.choice((0, 0, fb(rnd.uniform(0, 1)), word(rnd, 1.0))), rnd.choice((0, 0, fb(rnd.uniform(0, 1)), word(rnd, 1.0))),
                   sr, ss, hr, hr ^ rnd.choice((0, 0, 0x3f800000)), off])
    G1, G2, TBL, MGR = A(-2105671680, 22080), A(-2104950784, -17936), A(-2105671680, 6928), A(-2105016320, -7572)
    secs.append(('lights', cs,
                 'store(0x1000+484,c[0]);store(0x1000+452,c[1]?0x1100:0);store(0x1100,0x1200);' + VT + 'SV[0]=c[2];ctx.r13.u32=0x1300;store(0x1300,0x1400);store8(0x1400+72,c[3]);' +
                 'store(%s,c[4]);store(%s,c[5]);' % (G1, G2) +
                 'for(int k=0;k<3;k++){store(%s+16*k+8,0);store(%s+16*k+12,0);}' % (TBL, TBL) +
                 'store(%s+16*c[4]+8,c[6]);store(%s+16*c[4]+12,c[7]);store(%s+16*c[5]+8,c[8]);store(%s+16*c[5]+12,c[9]);' % (TBL, TBL, TBL, TBL) +
                 'store(%s,0x2000);store8(0x2000+14546,0);store(0x2000+14564,c[10]);store(0x2000+14568,c[11]);store(0x2000+14532+4*c[4],c[12]);store(0x2000+14532+8,c[13]);' % MGR +
                 'store(0x1000+3568,c[14]);ctx.r3.u32=0x1000;',
                 f['lights'], 1, '(ctx.r3.u32&0xFF)'))

    # nitro: the tune's s16, the cap s16 (+16), prefilled +14 +18 +32 +24 +28
    cs = [[rnd.choice((0, 3, 100, 101, 99, 0xFFFF, 0x8000, 0x7FFF, 1, 2, rnd.getrandbits(16), rnd.randrange(0, 200))),
           rnd.choice((100, 100, 100, 0, 50, 0xFFFF, 0x7FFF, 0x8000, rnd.getrandbits(16), rnd.randrange(0, 200)))] for _ in range(3000)]
    secs.append(('nitro', cs, 'store(0x1000+4,0x1100);store16(0x1100,c[0]);store16(0x1000+16,c[1]);store16(0x1000+14,0xA5A5);store16(0x1000+18,0xA5A5);store8(0x1000+32,0xA5);' +
                 'store(0x1000+24,0xA5A5A5A5);store(0x1000+28,0xA5A5A5A5);ctx.r3.u32=0x1000;', f['nitro'], 5,
                 '(i==0?load16(0x1000+14):i==1?load16(0x1000+18):i==2?*pointer(0x1000+32):i==3?load(0x1000+24):load(0x1000+28))'))

    # winreset: the sound handle (+20) present or not, every other word prefilled
    cs = [[rnd.choice((0, 1)), rnd.getrandbits(32)] for _ in range(1000)]
    secs.append(('winreset', cs, 'for(int k=0;k<24;k++)store(0x1000+4*k,0xA5A50000+k);store(0x1000+20,c[0]?0x1100:0);ctx.r3.u32=0x1000;ctx.r4.u32=0x55;NREC=0;', f['winreset'], 8,
                 '(i==0?load(0x1000+76):i==1?load(0x1000+64):i==2?load(0x1000+8):i==3?load(0x1000+12):i==4?NREC:i==5?REC[0][0]:i==6?REC[0][1]:load(0x1000+16))'))


    img_size = a.image.stat().st_size
    CL = A(-2105737216, 29952)   # the frame clock's record: +8 the frame time, +24 the clock
    GARAGE_OBJ = A(-2104426496, -29340)
    RAIN = A(-2104950784, -6728)
    FLAG20 = A(-2104950784, -8768)

    def rate(r):
        return r.choice((fb(9.0), fb(9.0), fb(18.0), fb(r.uniform(0, 30)), 0, fw(r, 20.0)))

    def dtw(r):
        return r.choice((0x3c888889, 0x3c888889, fb(r.uniform(0, 0.1)), fb(0.0), fb(1.0), 0x80000000, fw(r, 0.2)))

    # lampstate: the six eased states (+168 +188 +208 +228 +248 +308; +188 equal to +168 in 3 cases of 4), +392, +404, +408, +268, byte +6466,
    # R+368, R+372, then 19 entries of 9 words (bulb 4, cover 4, attachment)
    cs = []
    for _ in range(1000):
        h = unit(rnd)
        c = [h, h if rnd.random() < 0.75 else unit(rnd), unit(rnd), unit(rnd), unit(rnd), unit(rnd), unit(rnd), unit(rnd), unit(rnd), rnd.choice((0x3f800000, 0x3f800000, unit(rnd))),
             rnd.choice((0, 0, 1)), fw(rnd, 3.0), fw(rnd, 3.0)]
        for _ in range(19):
            c += [fw(rnd, 3.0) for _ in range(8)] + [rnd.choice((rnd.randrange(0, 17), rnd.randrange(0, 17), rnd.randrange(0, 12)))]
        cs.append(c)
    ent = ('for(int i=0;i<19;i++){for(int j=0;j<9;j++){uint32_t o=j<4?4*j:j<8?16+4*(j-4):32;store(0x3100+560+48*i+o,c[13+9*i+j]);}}')
    secs.append(('lampstate', cs,
                 'store(0x1000+168,c[0]);store(0x1000+188,c[1]);store(0x1000+208,c[2]);store(0x1000+228,c[3]);store(0x1000+248,c[4]);store(0x1000+308,c[5]);' +
                 'store(0x1000+392,c[6]);store(0x1000+404,c[7]);store(0x1000+408,c[8]);store(0x1000+268,c[9]);store8(0x1000+6466,c[10]);' +
                 'store(0x1000+20,0x3000);store(0x3000+132,0x3100);store(0x1000+1056,0x4000);store(0x3100+64+304,c[11]);store(0x3100+64+308,c[12]);' + ent +
                 'store(%s,0);ctx.r3.u32=0x1000;' % FLAG20,
                 f['lampstate'], 33,
                 '(i<19?load(0x4000+400+4*i):i==19?REC[19][3]:i<23?REC[19][i-20]:i==23?load(0x1000+2672):i==24?load(0x1000+2676):i==25?load(0x1000+2680):i==26?load(0x1000+2976):i==27?load(0x1000+2980):i==28?load(0x1000+2984):load(0x1000+2624+4*(i-29)))'))

    # traffic: depth, gate, thread index, three flag bytes, dt, five rates, five values, three glow scales, factor (rec+84), override flag byte, 17 override
    # words, then 19 entries of 9 words
    cs = []
    for _ in range(1000):
        c = [rnd.choice((fb(rnd.uniform(0, 400)), fb(50.0), fb(rnd.uniform(40, 60)), fb(375.0), 0x7f7fffff, 0, 0x80000000, fw(rnd, 500.0))), rnd.choice((1, 1, 1, 0)),
             rnd.randrange(0, 3), rnd.getrandbits(8), rnd.getrandbits(8), rnd.getrandbits(8), dtw(rnd)]
        c += [rate(rnd) for _ in range(5)] + [unit(rnd) for _ in range(5)] + [rnd.choice((fb(rnd.uniform(0, 2)), 0x3f800000, fw(rnd, 2.0))) for _ in range(3)]
        c += [rnd.choice((fb(rnd.uniform(0, 2)), 0x3f800000, fw(rnd, 2.0))), rnd.choice((0, 0, 0, 0, 0, 1))]
        c += [unit(rnd) for _ in range(17)]
        for _ in range(19):
            c += [fw(rnd, 3.0) for _ in range(8)] + [rnd.choice((rnd.randrange(0, 17), rnd.randrange(0, 17), rnd.randrange(0, 12)))]
        cs.append(c)
    ent = ('for(int i=0;i<19;i++){for(int j=0;j<9;j++){uint32_t o=j<4?4*j:j<8?16+4*(j-4):32;store(0x4000+48*i+o,c[39+9*i+j]);}}')
    secs.append(('traffic', cs,
                 'store(0x1000+20,0x1800);store(0x1800+264,0x4000);store(0x1000+528,0x3000);store(%s,c[2]);' % G1 +
                 'store8(0x1000+557,c[3]);store8(0x1000+558,c[4]);store8(0x1000+559,c[5]);store(%s+8,c[6]);' % CL +
                 'for(int k=0;k<5;k++){store(0x1000+56+20*k,c[7+k]);store(0x1000+64+20*k,c[12+k]);store(0x1000+60+20*k,0xA5A5A5A5);}' +
                 'for(int k=0;k<3;k++)store(0x1000+44+4*k,c[17+k]);store(0x1800+84,c[20]);store8(0x1800+66,c[21]);for(int k=0;k<17;k++)store(0x1800+88+4*k,c[22+k]);' + ent +
                 'for(int k=0;k<200;k++)store(0x3000+4*k,0xA5A5A5A5);SV[0]=c[1];ctx.f1.f64=FD(c[0]);ctx.r3.u32=0x1000;',
                 f['traffic'], 124,
                 '(i<5?load(0x1000+64+20*i):i<10?(uint32_t)*pointer(0x1000+60+20*(i-5)):i<29?load(0x3000+400+4*(i-10)):i<48?REC[i-29][3]:load(0x3000+96+4*(i-48)))'))

    # lamps (the index of each input is in tools' notes: see the test)
    cs = []
    for _ in range(2000):
        hr = rnd.choice((fb(rnd.uniform(0, 24)), fb(rnd.uniform(5.5, 6.5)), fb(rnd.uniform(17.5, 18.5)), word(rnd, 30.0)))
        sr = rnd.choice((fb(6.0), fb(6.0), word(rnd, 10.0)))
        ss = rnd.choice((fb(18.0), fb(18.0), word(rnd, 30.0)))
        if rnd.random() < 0.15:
            hr = rnd.choice((sr, ss, (sr + 1) & 0xFFFFFFFF, (ss - 1) & 0xFFFFFFFF))
            hr = 0 if is_denormal(hr) else hr
        wthr = lambda: rnd.choice((0, 0, fb(rnd.uniform(0, 1)), fb(0.85), 0x3f59999b, 0x3f599999, word(rnd, 1.0)))
        c = [rnd.choice((1, 1, 1, 0)), rnd.choice((1, 1, 1, 0)), rnd.choice((fb(rnd.uniform(-0.01, 1)), 0x3a83126f, 0x3a831270, 0x3a83126e, 0, word(rnd, 1.0))),
             rnd.choice((0, 1, 1)), rnd.choice((0, 0, 0, 1)), rnd.choice((0, 0, 1, 2, 3, 4, 5, 6, 7))]
        c += [rnd.choice((fb(rnd.uniform(0, 1000)), fb(rnd.uniform(0, 0.01)), 0, word(rnd, 1000.0))) for _ in range(10)]
        c += [rnd.choice((fb(rnd.uniform(0, 3)), word(rnd, 3.0))), rnd.choice((fb(rnd.uniform(0, 1.0)), fb(rnd.uniform(0, 100)), fw(rnd, 60.0)))]
        c += [rnd.choice((0, 0, 0, 1)), hr, sr, ss, rnd.choice((0, 1, 5)), rnd.randrange(0, 3), rnd.randrange(0, 3), wthr(), wthr(),
              rnd.choice((0, 1)), rnd.choice((0, 0, 0, 1)), fw(rnd, 1000.0), rnd.choice((0, 0, 1, 0xFF, 0xFE, 5, 0x80, rnd.getrandbits(8))), rnd.choice((0, 0, 1, 0xFF, 7, 0x80, rnd.getrandbits(8)))]
        c += [rate(rnd) for _ in range(8)] + [unit(rnd) for _ in range(8)] + [rnd.getrandbits(1) for _ in range(8)] + [rate(rnd), unit(rnd), rnd.getrandbits(1)]
        c += [dtw(rnd), rnd.choice((fw(rnd, 3.0), fw(rnd, 3.0), unit(rnd))), rnd.choice((unit(rnd), 0x3f800000))]
        cs.append(c)
    LAMP_OFFS = [164, 168, 184, 188, 204, 208, 224, 228, 244, 248, 264, 268, 284, 288, 304, 308, 368, 388, 392, 404, 408]
    pre = ('store(0x1000+20,0x6000);store(0x6000+4,c[1]?0x6300:0);store(0x6000+8,0x6100);store(0x6000+128,c[3]?0x1234:0);store(0x6000+84,0x6800);store(0x6000+132,0x6400);' +
           'store(0x6100+224,c[17]);store(0x6100+16,0x6150);store(0x6100+20,0x6200);store(0x6200+20,c[5]);for(int k=0;k<10;k++)store(0x6200+4*(k+19),c[6+k]);' +
           'store(0x6300+16,c[2]);store8(0x1000+597,c[0]);SV[5]=c[4];SV[6]=c[16];' +
           'store(%s,0x2000);store8(0x2000+14546,c[18]);store(0x2000+14564,c[20]);store(0x2000+14568,c[21]);store(0x2000+14532+4*c[23],c[19]);store(0x2000+14540,c[19]);' % MGR +
           'ctx.r13.u32=0x1300;store(0x1300,0x1400);store8(0x1400+72,c[22]);store(%s,c[23]);store(%s,c[24]);' % (G1, G2) +
           'for(int k=0;k<3;k++){store(%s+16*k+8,0);store(%s+16*k+12,0);}uint32_t sel=c[22]?c[23]:c[24];store(%s+16*sel+8,c[25]);store(%s+16*sel+12,c[26]);' % (TBL, TBL, TBL, TBL) +
           'store(%s,0x82000000u+%d-0x20000u-39229u);store8(0x82000000u+%d-0x20000u,c[27]);store8(%s,c[28]);' % (GARAGE_OBJ, img_size, img_size, RAIN) +
           'store(%s+24,c[29]);store(%s+8,c[59]);' % (CL, CL) +
           'store8(0x6400+396,c[30]);store8(0x6400+397,c[31]);for(int k=0;k<8;k++){store(0x1000+160+20*k,c[32+k]);store(0x1000+168+20*k,c[40+k]);store8(0x1000+164+20*k,c[48+k]);}' +
           'store(0x1000+384,c[56]);store(0x1000+392,c[57]);store8(0x1000+388,c[58]);store(0x1000+268,c[60]);store(0x1000+404,c[61]);' +
           'store(0x1000+360,0x3c888889);store(0x1000+364,0x3c888889);store(0x1000+368,0);store(0x1000+372,0);store8(0x6800+2161,0xA5);store8(0x6800+2162,0xA5);ctx.r3.u32=0x1000;')
    outc = '(i<21?(LBYTE[i]?(uint32_t)*pointer(0x1000+LOFF[i]):load(0x1000+LOFF[i])):(uint32_t)*pointer(0x6800+2161+(i-21)))'
    secs.append(('lamps', cs, pre, f['lamps'], 23, outc))
    extra_decl = ('static const uint32_t LOFF[21]={' + ','.join(str(o) for o in LAMP_OFFS) + '};' +
                  'static const uint32_t LBYTE[21]={' + ','.join('1' if o in (164, 184, 204, 224, 244, 264, 284, 304, 388) else '0' for o in LAMP_OFFS) + '};')

    if a.only:
        secs = [s for s in secs if s[0] in a.only.split(',')]
    stubs = {
        'sub_' + f['stop']: 'REC[NREC][0]=ctx.r3.u32;REC[NREC][1]=ctx.r4.u32;NREC++;',
        'sub_' + f['stealth']: 'ctx.r3.u32=SV[5]&0xFF;',
        'sub_' + f['engine_speed']: 'ctx.f1.f64=FD(SV[6]);',
        'sub_' + f['gate']: 'ctx.r3.u32=SV[0]&0xFF;',
        'sub_' + f['upload']: 'for(int k=0;k<19;k++){for(int j=0;j<4;j++){REC[k][j]=load(ctx.r5.u32+16*k+4*j);}}for(int k=0;k<19;k++){for(int j=0;j<4;j++){REC[19+k][j]=load(ctx.r4.u32+16*k+4*j);}}NREC=ctx.r6.u32;',
        'sub_' + f['helper_a']: '',
        'sub_' + f['helper_b']: 'for(int k=0;k<20;k++)for(int j=0;j<4;j++)REC[k][j]=load(ctx.r5.u32+16*k+4*j);',
        'sub_' + f['helper_c']: '',
    }
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){switch(a){' +
            'case 0x7F000012:ctx.r3.u32=SV[0];return;' +
            'default:throw std::runtime_error("unexpected indirect call");}}')
    results = {}
    body = ''
    for tag, cs, setup, fn, nout, outc in secs:
        w = max(len(c) for c in cs)
        rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c + [0] * (w - len(c))) for c in cs)
        body += ('{static const uint32_t C[][%d]={%s%s%s};' % (w, nl, rows, nl) +
                 'for(auto&c:C){memset(objects,0,sizeof(objects));Context ctx{};ctx.r1.u32=0x7F00;ncalls=0;memset(REC,0,sizeof(REC));' + setup +
                 'sub_%s(ctx,nullptr);' % fn +
                 'for(int i=0;i<%d;i++)printf("%%08x ",uint32_t(%s));printf("%s");fflush(stdout);}}' % (nout, outc, NL) + nl)
    names = {'sub_' + s[3] for s in secs} | {'sub_' + f['pow']}
    names -= set(stubs)
    main_src = ('static void store8(uint32_t a,uint8_t v){*pointer(a)=v;}' + nl + extra_decl + nl + disp + nl +
                'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    exe = build(index(a.recomp), sorted(names), main_src, a.out_dir, 'vehicles_small', stubs=stubs, vector=True, scratch_end=0x8000, flags=('-msse4.1', '-mssse3'))
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == sum(len(s[1]) for s in secs), (len(out), sum(len(s[1]) for s in secs))
    k = 0
    for tag, cs, *_ in secs:
        for c in cs:
            results.setdefault(tag, []).append('%s %s -> %s' % (tag, ' '.join('%08x' % x for x in c), out[k].strip()))
            k += 1
    lines = [l for tag in results for l in results[tag]]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'vehicles_small_ref.txt').write_text(cite_safe(nl.join(lines) + nl))
    print({t: len(v) for t, v in results.items()})


if __name__ == '__main__':
    main()
