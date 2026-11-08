"""The engine sound's start F-44510 (vt+52) run offline (the generated code whole and unchanged, vector mode) over every value
of its two tune words: XF+99 (u16, the ignition delay in ms) and XF+105 (s16, the start boost in dB x 100), with the ignition
flag 0 and 1. The object's vt+112 (stop every loop) is a host fake that counts its calls; db -> linear F-42985's code is the
original's (compiled in the closure).

Writes <out-dir>/engine_start_ref.txt: one case a line, "flag u99 s105 -> state timer boost stops" (u99 / s105 as the
words' 16 bits in hex, timer and boost as hex float bits, state as the word +1928 in hex, stops decimal)."""
import argparse
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd

# db -> linear's vector shifts (the build has no AVX2): the same shim as category_record_reference.py
recomp_closure.EXTRA += r'''
#ifndef simde_mm_sllv_epi32
static inline __m128i simde_mm_sllv_epi32(__m128i a, __m128i n) {
    alignas(16) uint32_t x[4], k[4]; _mm_store_si128((__m128i*)x, a); _mm_store_si128((__m128i*)k, n);
    for (int i = 0; i < 4; i++) x[i] = k[i] > 31 ? 0 : x[i] << k[i];
    return _mm_load_si128((__m128i*)x);
}
#endif
#ifndef simde_mm_srlv_epi32
static inline __m128i simde_mm_srlv_epi32(__m128i a, __m128i n) {
    alignas(16) uint32_t x[4], k[4]; _mm_store_si128((__m128i*)x, a); _mm_store_si128((__m128i*)k, n);
    for (int i = 0; i < 4; i++) x[i] = k[i] > 31 ? 0 : x[i] >> k[i];
    return _mm_load_si128((__m128i*)x);
}
#endif
namespace rex { namespace ppc {
// the SDK's intrinsics.h (vcfpsxws128)
inline simde__m128i simde_mm_vctsxs(simde__m128 src1) {
  simde__m128 xmm2 = simde_mm_cmpunord_ps(src1, src1);
  simde__m128i xmm0 = simde_mm_cvttps_epi32(src1);
  simde__m128i xmm1 = simde_mm_cmpeq_epi32(xmm0, simde_mm_set1_epi32(INT_MIN));
  xmm1 = simde_mm_andnot_si128(simde_mm_castps_si128(src1), xmm1);
  simde__m128 dest = simde_mm_blendv_ps(simde_mm_castsi128_ps(xmm0), simde_mm_castsi128_ps(simde_mm_set1_epi32(INT_MAX)),
                                        simde_mm_castsi128_ps(xmm1));
  return simde_mm_andnot_si128(simde_mm_castps_si128(xmm2), simde_mm_castps_si128(dest));
}
} }
'''

OBJ = 0x40100000     # the engine sound object (this)
VT = 0x40102000      # its fake vtable
XF = 0x40103000      # its tune record (+1216 points here)
FAKE_STOP = 0x7F000020
CNT = 0x40104000     # the fake's call count


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:].upper()


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--step', type=int, default=17, help='every n-th 16-bit value (plus the ends and the sign edge)')
    a = ap.parse_args()
    fn = addr(a.tool, 'F-44510')
    idx = rd.index(a.recomp)
    vals = sorted(set(range(0, 65536, a.step)) | {0, 1, 2, 999, 1000, 1001, 0x7FFE, 0x7FFF, 0x8000, 0x8001, 0xFFFE, 0xFFFF, 600, 65535 - 600})
    nl = chr(10)
    main_src = (
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'static const uint16_t V[]={' + ','.join(str(v) for v in vals) + '};' + nl +
        'for(int fl=0;fl<2;fl++)for(uint16_t u:V){uint16_t s=(uint16_t)(u*40503u+fl*7u);rt_reset();store(%du,0u);'
        'memset(pointer(%du),0xCD,4096);memset(pointer(%du),0,256);memset(pointer(%du),0xAB,256);' % (CNT, OBJ, VT, XF) +
        'store(%du,%du);store(%du+112,%du);store(%du+1216,%du);' % (OBJ, VT, VT, FAKE_STOP, OBJ, XF) +
        'uint8_t*x=(uint8_t*)pointer(%du);x[99]=u>>8;x[100]=u&255;x[105]=s>>8;x[106]=s&255;' % XF +
        'Context ctx=fresh();ctx.r3.u32=%du;ctx.r4.u32=fl?0x12345601u:0x12345600u;sub_%s(ctx,nullptr);' % (OBJ, fn) +
        'printf("%%d %%04x %%04x -> %%08x %%08x %%08x %%u\\n",fl,u,s,load(%du+1928),load(%du+288),load(%du+152),load(%du));}}' % (OBJ, OBJ, OBJ, CNT) + nl)
    exe = rd.build(idx, ['sub_' + fn], main_src, a.out_dir, 'engine_start', image=rd.load_image(a.image),
                   fakes={FAKE_STOP: 'store(%du,load(%du)+1);' % (CNT, CNT)})
    out = [x.strip() for x in rd.run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == 2 * len(vals), (len(out), len(vals))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'engine_start_ref.txt').write_text(nl.join(out) + nl)
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
