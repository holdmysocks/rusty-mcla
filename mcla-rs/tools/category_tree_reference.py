"""The category tree run offline: the generated code whole and unchanged from the file to the resolved table. The loader
F-45518 (the .dat reader F-35660 and its readers, the build from BASE F-48971 with F-31533 / F-26947 / F-41684 / F-31694, the
sort) reads categories.dat through a stand-in stream (the file's bytes in order; open / close / path formatting stubbed, the
allocator F-51856 a bump arena). Then per case the runtime fields are set through the original's setters (F-87261 volume dB
+40, F-60329 pitch cents +68, F-99639 LPF +70, F-29180 HPF +72) and the update F-85160 walks the tree (F-52968) into the
resolved table (36 B a category). dB2lin F-42985 as in category_record_reference.py (the recomp's vector powf).
Inputs: the disc's categories.dat and copies with random own fields (+9 / +10 flags, volumes, pitch, LPF, HPF, rolloff
scales, reverb sends); per load 8 runtime sets (all zero / default, random, wrapping pitch, LPF above and below 23900).
Writes <out-dir>/category_tree_ref.txt: per load 'L <n>' then 'O <hash> <object hex>' per object (the file's own bytes),
per case 'C' then 'R <hash> <vol bits> <pitch> <lpf> <hpf>' per category whose runtime is not the default, and 'T <hash>
<resolved table: 7 f32 words, then the 3 u16 (pitch, LPF, HPF)>' per category (hex)."""
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
static const uint8_t *fbuf; static size_t fcur, flen;
#ifndef simde_mm_sllv_epi32
static inline __m128i simde_mm_sllv_epi32(__m128i a, __m128i n) {
    alignas(16) uint32_t x[4], k[4]; _mm_store_si128((__m128i*)x, a); _mm_store_si128((__m128i*)k, n);
    for (int i = 0; i < 4; i++) x[i] = k[i] > 31 ? 0 : x[i] << k[i];
    return _mm_load_si128((__m128i*)x);
}
#endif
'''

MGR = 0x82834D90   # the global category manager (F-26947 adds the children to it; +92 the blend factor)
STREAM = 0x40180000
SETS = 8
TABLE_PTR, RECORDS_PTR = 0x8283681C, 0x82836820


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def objects(d):
    """(name, offset into data, size) of a .dat file, and the data block's start in the file."""
    be = lambda o: struct.unpack_from('>I', d, o)[0]
    size = be(4)
    p = 8 + size
    p += 4 + be(p)
    count = be(p)
    p += 8
    out = []
    for _ in range(count):
        n = d[p]
        name = d[p + 1:p + 1 + n].decode()
        p += 1 + n
        out.append((name, be(p), be(p + 4)))
        p += 8
    return out


def joaat(s):
    h = 0
    for c in s.lower().encode():
        h = (h + c) & 0xffffffff
        h = (h + (h << 10)) & 0xffffffff
        h ^= h >> 6
    h = (h + (h << 3)) & 0xffffffff
    h ^= h >> 11
    return (h + (h << 15)) & 0xffffffff


def mutate(d, r):
    d = bytearray(d)
    for _, off, _sz in objects(bytes(d)):
        o = 8 + off
        if r.random() < 0.5:
            d[o + 9] = r.getrandbits(8)
        if r.random() < 0.3:
            d[o + 10] = r.getrandbits(8)
        for f, gen in ((11, lambda: r.randrange(-10500, 3000)), (13, lambda: r.choice((0, -10000, r.randrange(-10500, 3000)))),
                       (15, lambda: r.choice((0, 1200, -1200, 32767, -32768, r.randrange(-32768, 32768)))),
                       (17, lambda: r.choice((23900, 24000, 65535, 0, r.randrange(0, 65536)))),
                       (19, lambda: r.choice((0, 65535, r.randrange(0, 65536)))),
                       (21, lambda: r.randrange(0, 1000)), (23, lambda: r.choice((100, r.randrange(0, 1000)))),
                       (25, lambda: r.randrange(0, 200)), (27, lambda: r.randrange(0, 200)),
                       (29, lambda: r.randrange(0, 65536)), (31, lambda: r.randrange(0, 65536)), (33, lambda: r.randrange(0, 65536))):
            if r.random() < 0.5:
                struct.pack_into('>H', d, o + f, gen() & 0xffff)
    return bytes(d)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--categories', type=Path, required=True, help="the disc's audio/X360/config/categories.dat")
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--loads', type=int, default=6)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = {k: addr(a.tool, k) for k in ('F-45518', 'F-85160', 'F-87261', 'F-60329', 'F-99639', 'F-29180', 'F-42985')}
    disc = a.categories.read_bytes()
    r = random.Random(85160)
    files = [disc] + [mutate(disc, r) for _ in range(a.loads - 1)]
    n = len(objects(disc))
    # runtime sets: (kind, per-category values); kind 0 = defaults
    loads = []
    for fi, d in enumerate(files):
        sets = []
        for k in range(SETS):
            rt = []
            for i in range(n):
                if k == 0 or r.random() < 0.6:
                    rt.append((0, 0, 23900, 0))
                    continue
                vol = r.choice((0.0, -100.0, 6.0, r.uniform(-100, 12), r.uniform(-3, 3)))
                pit = r.choice((0, 100, -2400, r.randrange(-1200, 1200), r.randrange(-32768, 32768) if k % 4 == 3 else 0))
                lpf = r.choice((23900, 23900, 24000, 65535, r.randrange(0, 23900), r.randrange(0, 65536)))
                hpf = r.choice((0, 0, r.randrange(0, 3000), r.randrange(0, 65536)))
                rt.append((struct.unpack('>I', struct.pack('>f', vol))[0], pit & 0xffff, lpf, hpf))
            sets.append(rt)
        loads.append(sets)
    fdata = ','.join('{%s}' % ','.join(str(b) for b in d) for d in files)
    flens = ','.join(str(len(d)) for d in files)
    rdata = ','.join('%du,%du,%du,%du' % v for sets in loads for rt in sets for v in rt)
    # stubbed: the allocator and its free, the file exists / open / close, the path formatting and copy, the stream read
    s_ = {k: addr(a.tool, k) for k in ('F-51856', 'F-29135', 'F-47409', 'F-56701', 'F-10154', 'F-98741', 'F-17218', 'F-23353')}
    stubs = {
        'sub_' + f['F-42985']: 'ctx.f1.f64=(double)d2l((float)ctx.f1.f64);',
        'sub_' + s_['F-51856']: 'ctx.r3.u32=rt_alloc(ctx.r4.u32,ctx.r5.u32);',
        'sub_' + s_['F-29135']: '',
        'sub_' + s_['F-47409']: 'ctx.r3.u64=1;',
        'sub_' + s_['F-56701']: 'ctx.r3.u64=%du;' % STREAM,
        'sub_' + s_['F-10154']: 'ctx.r3.u64=0;',
        'sub_' + s_['F-98741']: '',
        'sub_' + s_['F-17218']: '',
        'sub_' + s_['F-23353']: 'uint32_t k=ctx.r5.u32;if(fcur+k>flen)k=(uint32_t)(flen-fcur);for(uint32_t i=0;i<k;i++)*pointer(ctx.r4.u32+i)=fbuf[fcur+i];'
                        'fcur+=k;ctx.r3.u64=k;',
    }
    c = r"""
static const uint8_t FD[][@MAXLEN@]={@FDATA@};
static const size_t FL[]={@FLENS@};
static const uint32_t RT[]={@RDATA@};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
size_t q=0;
for(size_t L=0;L<sizeof(FL)/sizeof(FL[0]);L++){
 rt_reset();fbuf=FD[L];fcur=0;flen=FL[L];store(0x82834DECu,0x3f800000u);
 Context ctx=fresh();ctx.r1.u32=0x40F00000u;ctx.r3.u32=@MGR@u;sub_@LOAD@(ctx,nullptr);
 uint32_t ok=ctx.r3.u32&0xff,cap=load(@MGR@u+72),used=load(@MGR@u+80),lut=load(@MGR@u+76),recs=load(@RECS@u),tab=load(@TAB@u);
 printf("L %zu ok%u used%u cap%u\n",L,ok,used,cap);
 for(int k=0;k<@SETS@;k++){
  printf("C %d\n",k);
  for(uint32_t i=0;i<cap;i++){uint32_t rec=load(lut+8*i),h=load(lut+8*i+4);const uint32_t*v=RT+q;q+=4;if(!rec)continue;
   Context s=fresh();s.r1.u32=0x40F00000u;s.r3.u32=rec;float vol;memcpy(&vol,&v[0],4);s.f1.f64=vol;sub_@SVOL@(s,nullptr);
   s.r3.u32=rec;s.r4.u64=v[1];sub_@SPIT@(s,nullptr);s.r3.u32=rec;s.r4.u64=v[2];sub_@SLPF@(s,nullptr);s.r3.u32=rec;s.r4.u64=v[3];sub_@SHPF@(s,nullptr);
   if(v[0]||v[1]||v[2]!=23900||v[3])printf("R %08x %08x %u %u %u\n",h,v[0],v[1],v[2],v[3]);}
  Context u=fresh();u.r1.u32=0x40F00000u;u.r3.u32=@MGR@u;sub_@UPD@(u,nullptr);
  for(uint32_t i=0;i<cap;i++){uint32_t rec=load(lut+8*i),h=load(lut+8*i+4);if(!rec)continue;uint32_t ix=(uint16_t)load16(rec+82);uint32_t t=tab+36*ix;
   printf("T %08x",h);for(int w=0;w<7;w++)printf(" %08x",load(t+4*w));printf(" %04x %04x %04x\n",load16(t+28),load16(t+30),load16(t+32));}}}
}
"""
    for k, v in (('FDATA', fdata), ('FLENS', flens), ('RDATA', rdata), ('MAXLEN', max(len(d) for d in files)), ('MGR', MGR), ('SETS', SETS),
                 ('RECS', RECORDS_PTR), ('TAB', TABLE_PTR), ('LOAD', f['F-45518']), ('UPD', f['F-85160']), ('SVOL', f['F-87261']),
                 ('SPIT', f['F-60329']), ('SLPF', f['F-99639']), ('SHPF', f['F-29180'])):
        c = c.replace('@%s@' % k, str(v))
    img = rd.load_image(a.image)
    names = ['sub_' + f[k] for k in ('F-45518', 'F-85160', 'F-87261', 'F-60329', 'F-99639', 'F-29180')]
    exe = rd.build(index(a.recomp), names, c, a.out_dir, 'category_tree', stubs=stubs, image=img)
    out = run(exe, a.image).split(chr(10))
    # the file's objects, so the port's test can build the same Dat
    res, li = [], 0
    for line in out:
        if line.startswith('L '):
            d = files[li]
            res.append(line)
            size = struct.unpack_from('>I', d, 4)[0]
            for name, off, sz in objects(d):
                res.append('O %08x %s' % (joaat(name), d[8 + off:8 + off + sz].hex()))
            li += 1
        elif line:
            res.append(line)
    assert li == len(files), (li, len(files))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'category_tree_ref.txt').write_text(chr(10).join(res) + chr(10))
    print(len(files), 'loads,', sum(1 for x in res if x.startswith('T ')), 'resolved records')


if __name__ == '__main__':
    main()
