"""MANAGER_PLAYER_SPEED run offline: the mixer's F-15595 (each record's target from the two speed inputs, the controller's
level slewed toward it at rate x dt) with SetVolumeDB F-10740, the generated code whole and
unchanged, on the game's own MANAGER_PLAYER_SPEED table and on random tables, through random frames: the camera mode
(+1744, 5 = cinematic: inputs taken as 0), the two inputs (+1748 / +1752; the game only ever stores 0 there) and the frame
time (0x827D7500 +92). Stubbed: dB2lin F-42985 with the arithmetic of the recomp's vector powf (as busfx.rs::db2lin, itself
checked bit-exact against the original in busfx_is_the_originals; the generated vector code does not build in this
harness), the controller command post F-42774 (records the command's value and, as the mixer's
command pass does before the next frame, puts it in the controller's +4).
Writes <out-dir>/player_speed_ref.txt (each number led by a letter, so no column reads as an address): per table 'T n'
and its records 'r hash alo ahi blo bhi mblo mbhi rate' (f32 / s16 bits), then per frame 'F mode a b dt' '->' every
controller's new linear level (f32 bits)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd
from recomp_closure import index, run

recomp_closure.EXTRA += 'static uint32_t VAL[64],CTL[64];static uint32_t NV;' + chr(10)
# (the vector exp of dB2lin uses a shift the toolchain's SIMDe lacks; as solver_reference.py)
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

MIX = 0x40100000
TAB = 0x40110000     # the table object (records from +11)
CTRL = 0x40120000    # controllers, 64 bytes each
ARR = 0x40130000     # +1732 the controller pointers
STACK = 0x40F00000
DT = 0x827D7500 + 92


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def gamedat(path):
    d = open(path, 'rb').read()
    size = struct.unpack_from('>I', d, 4)[0]
    p = 8 + size
    p += 4 + struct.unpack_from('>I', d, p)[0]
    count = struct.unpack_from('>I', d, p)[0]
    p += 8
    for _ in range(count):
        n = d[p]
        name = d[p + 1:p + 1 + n].decode('latin1')
        p += 1 + n
        off, sz = struct.unpack_from('>II', d, p)
        p += 8
        if name == 'MANAGER_PLAYER_SPEED':
            return d[8 + off:8 + off + sz]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--game-dat', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn, post, d2l = (addr(a.tool, f) for f in ('F-15595', 'F-42774', 'F-42985'))
    img = rd.load_image(a.image)
    idx = index(a.recomp)
    real = gamedat(a.game_dat)
    r = random.Random(15595)
    tables = []
    n = (len(real) - 11) // 28
    tables.append([struct.unpack_from('>I4I2HI', real, 11 + 28 * i) for i in range(n)])
    vals = (0.0, 0.5, 1.0, 0.75, 40.0, 70.0, 100.0, 300.0, 50.0, -1.0)
    for _ in range(60):
        recs = []
        for _ in range(r.randrange(1, 24)):
            alo, ahi = sorted((r.choice(vals), r.choice(vals))) if r.random() < 0.8 else (r.choice(vals), r.choice(vals))
            blo, bhi = r.choice(vals), r.choice(vals)
            mlo, mhi = r.choice((-2400, -600, -200, 0, 100, 300, 800, -10000)), r.choice((-1200, -150, 0, 200, 750, -10000))
            recs.append((r.getrandbits(32), fb(alo), fb(ahi), fb(blo), fb(bhi), mlo & 0xffff, mhi & 0xffff, fb(r.choice((3.0, 6.0, 9.0, 0.0, 100.0)))))
        tables.append(recs)
    cases = []
    for t in tables:
        frames = []
        for _ in range(r.randrange(30, 150)):
            mode = r.choice((0, 2, 4, 5, 5, 6))
            sa = 0.0 if r.random() < 0.4 else r.choice((0.0, 20.0, 45.0, 60.0, 99.0, 150.0, 0.3, 0.9, 1.0))
            sb = 0.0 if r.random() < 0.4 else r.choice((0.0, 0.3, 0.5, 0.75, 0.9, 1.0, 2.0))
            dt = r.choice((1 / 60, 1 / 30, 0.0, 0.1, 0.5))
            frames.append((mode, fb(sa), fb(sb), fb(dt)))
        cases.append((t, frames))
    data = ','.join('%d,%s,%d,%s' % (len(t), ','.join(','.join('%du' % v for v in rec) for rec in t), len(fr),
                                     ','.join(','.join('%du' % v for v in f) for f in fr)) for t, fr in cases)
    stubs = {'sub_' + post: 'CTL[NV]=ctx.r4.u32;VAL[NV++]=load(ctx.r5.u32+8);', 'sub_' + d2l: 'ctx.f1.f64=(double)d2l((float)ctx.f1.f64);'}
    c = r"""
static const uint32_t C[]={@DATA@};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(size_t i=0;i<sizeof(C)/4;){uint32_t n=C[i];const uint32_t*rec=C+i+1;i+=1+8*n;uint32_t m=C[i];const uint32_t*fr=C+i+1;i+=1+4*m;
 printf("T n%u\n",n);
 memset(pointer(@MIX@u),0,0x1000);memset(pointer(@TAB@u),0,0x1000);memset(pointer(@CTRL@u),0,0x1000);
 for(uint32_t k=0;k<n;k++){const uint32_t*w=rec+8*k;uint32_t b=@TAB@u+11+28*k;
  store(b,w[0]);store(b+4,w[1]);store(b+8,w[2]);store(b+12,w[3]);store(b+16,w[4]);store16(b+20,w[5]);store16(b+22,w[6]);store(b+24,w[7]);
  printf("r h%u a%08x a%08x b%08x b%08x m%u m%u t%08x\n",w[0],w[1],w[2],w[3],w[4],w[5],w[6],w[7]);
  store(@ARR@u+4*k,@CTRL@u+64*k);store(@CTRL@u+64*k+4,0x3f800000u);}
 store(@MIX@u+16,@TAB@u);store(@MIX@u+28,n);store(@MIX@u+1732,@ARR@u);
 for(uint32_t j=0;j<m;j++){const uint32_t*f=fr+4*j;
  store(@MIX@u+1744,f[0]);store(@MIX@u+1748,f[1]);store(@MIX@u+1752,f[2]);store(@DT@u,f[3]);NV=0;
  Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@MIX@u;sub_@FN@(ctx,nullptr);
  printf("F c%u s%08x s%08x d%08x ->",f[0],f[1],f[2],f[3]);
  for(uint32_t k=0;k<NV;k++){store(CTL[k]+4,VAL[k]);printf(" l%08x",VAL[k]);}
  printf("\n");}
 printf("=\n");}}
"""
    for k, v in (('DATA', data), ('MIX', MIX), ('TAB', TAB), ('CTRL', CTRL), ('ARR', ARR), ('STACK', STACK), ('DT', DT), ('FN', fn)):
        c = c.replace('@%s@' % k, str(v))
    exe = rd.build(idx, ['sub_' + fn], c, a.out_dir, 'player_speed', stubs=stubs, image=img)
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'player_speed_ref.txt').write_text(out)
    fr = [l for l in out.split(chr(10)) if l.startswith('F ')]
    print(len(cases), 'tables,', len(fr), 'frames,', sum(l.count(' l') for l in fr), 'controller steps')


if __name__ == '__main__':
    main()
