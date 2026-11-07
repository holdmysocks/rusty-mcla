"""The category record's volume and rolloff scale (F-31694) run offline: the generated code whole and unchanged, through the
category loader F-41684 (r3 = the record, whose +0 points at the categories.dat object, r4 = the object; it reads the
global blend factor 0x82834D90+92, set here to 1.0, its only value - SOUND_FORMAT.md 5.1 - and passes r5 = 0, so
F-31694 does not walk the children). Inputs: the four fields it reads (+11 / +13
the two volumes in 0.01 dB, s16; +21 / +23 the two rolloff scales in 0.01, u16), from every object of the disc's
categories.dat and random values. dB2lin F-42985 is stubbed with the arithmetic of the recomp's vector powf (as
busfx.rs::db2lin, checked bit-exact against the original); the vector log2 estimate of the record's lin2dB is the
generated code's own.
The loader's own copies come along: +12 / +16 = +25 / +27 x 0.01 (the reverb sends).
Writes <out-dir>/category_record_ref.txt: 'v<+11> v<+13> s<+21> s<+23> r<+25> r<+27> -> d<+4 bits> k<+8 bits>
a<+12 bits> b<+16 bits>' (decimal inputs, f32 bits as hex)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd
from recomp_closure import index, run

recomp_closure.EXTRA += r'''
static float pow10v(float y){uint32_t kb=0x40549A78u;float k;memcpy(&k,&kb,4);float x=k*y;double n=floor((double)x);
 float fr=(float)((double)x-n);float p=1.8775767e-3f;const float cs[5]={8.9893397e-3f,5.5826318e-2f,2.4015361e-1f,6.9315308e-1f,1.0f};
 for(int i=0;i<5;i++)p=p*fr+cs[i];return (float)((double)p*ldexp(1.0,(int)n));}
static float d2l(float db){uint32_t kb=0x3D4CCCCDu;float k;memcpy(&k,&kb,4);return (db-(-100.0f))>=0.0f?pow10v(db*k):0.0f;}
#ifndef simde_mm_sllv_epi32
static inline __m128i simde_mm_sllv_epi32(__m128i a, __m128i n) {
    alignas(16) uint32_t x[4], k[4]; _mm_store_si128((__m128i*)x, a); _mm_store_si128((__m128i*)k, n);
    for (int i = 0; i < 4; i++) x[i] = k[i] > 31 ? 0 : x[i] << k[i];
    return _mm_load_si128((__m128i*)x);
}
#endif
'''

REC = 0x40100000
OBJ = 0x40110000
STACK = 0x40F00000


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def categories(path):
    d = open(path, 'rb').read()
    size = struct.unpack_from('>I', d, 4)[0]
    data = d[8:8 + size]
    p = 8 + size
    p += 4 + struct.unpack_from('>I', d, p)[0]
    count = struct.unpack_from('>I', d, p)[0]
    p += 8
    out = []
    for _ in range(count):
        n = d[p]
        p += 1 + n
        off, _sz = struct.unpack_from('>II', d, p)
        p += 8
        o = data[off:]
        out.append((struct.unpack_from('>h', o, 11)[0], struct.unpack_from('>h', o, 13)[0],
                    struct.unpack_from('>H', o, 21)[0], struct.unpack_from('>H', o, 23)[0],
                    struct.unpack_from('>H', o, 25)[0], struct.unpack_from('>H', o, 27)[0]))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--categories', type=Path, required=True, help="the disc's audio/X360/config/categories.dat")
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn, d2l = addr(a.tool, 'F-41684'), addr(a.tool, 'F-42985')
    img = rd.load_image(a.image)
    idx = index(a.recomp)
    cases = categories(a.categories)
    r = random.Random(31694)
    edge = (0, 1, -1, 100, -100, 600, -600, -9999, -10000, -10001, -12000, 3500, 32767, -32768, 1200, -1200)
    for _ in range(3000):
        v11 = r.choice(edge) if r.random() < 0.2 else r.randrange(-10500, 6000)
        v13 = 0 if r.random() < 0.6 else (r.choice(edge) if r.random() < 0.3 else r.randrange(-10500, 6000))
        s21 = r.choice((0, 1, 35, 100, 65535, 5000)) if r.random() < 0.2 else r.randrange(0, 10000)
        s23 = 100 if r.random() < 0.6 else r.randrange(0, 65536)
        cases.append((v11, v13, s21, s23, r.randrange(0, 65536), r.choice((0, 100, 35, r.randrange(0, 65536)))))
    data = ','.join('%du,%du,%du,%du,%du,%du' % (v11 & 0xffff, v13 & 0xffff, s21, s23, r25, r27) for v11, v13, s21, s23, r25, r27 in cases)
    stubs = {'sub_' + d2l: 'ctx.f1.f64=(double)d2l((float)ctx.f1.f64);'}
    c = r"""
static const uint32_t C[]={@DATA@};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(size_t i=0;i<sizeof(C)/4;i+=6){
 memset(pointer(@REC@u),0,0x200);memset(pointer(@OBJ@u),0,0x100);
 store(@REC@u,@OBJ@u);store16(@OBJ@u+11,C[i]);store16(@OBJ@u+13,C[i+1]);store16(@OBJ@u+21,C[i+2]);store16(@OBJ@u+23,C[i+3]);
 store16(@OBJ@u+25,C[i+4]);store16(@OBJ@u+27,C[i+5]);store(0x82834DECu,0x3f800000u);
 Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@REC@u;ctx.r4.u32=@OBJ@u;sub_@FN@(ctx,nullptr);
 printf("v%d v%d s%u s%u r%u r%u -> d%08x k%08x a%08x b%08x\n",(int)(int16_t)C[i],(int)(int16_t)C[i+1],C[i+2],C[i+3],C[i+4],C[i+5],
  load(@REC@u+4),load(@REC@u+8),load(@REC@u+12),load(@REC@u+16));}}
"""
    for k, v in (('DATA', data), ('REC', REC), ('OBJ', OBJ), ('STACK', STACK), ('FN', fn)):
        c = c.replace('@%s@' % k, str(v))
    exe = rd.build(idx, ['sub_' + fn], c, a.out_dir, 'category_record', stubs=stubs, image=img)
    out = run(exe, a.image)
    lines = [x for x in out.split(chr(10)) if x]
    assert len(lines) == len(cases), (len(lines), len(cases))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'category_record_ref.txt').write_text(chr(10).join(lines) + chr(10))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
