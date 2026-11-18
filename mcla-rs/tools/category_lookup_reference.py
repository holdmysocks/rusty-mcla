"""The category lookup F-80535 run offline: the generated code whole and unchanged, on the table the original's loader builds
(the load as in category_tree_reference.py: F-45518 through a stand-in stream, the allocator a bump arena). Per load, the
manager's +72 / +76 search table is searched for every hash in it, for random hashes (none in 0x82......), for hashes next to them (+-1) and for 0.
Inputs: the disc's categories.dat and copies with random own fields.
Writes <out-dir>/category_lookup_ref.txt: per load 'L <n> <+72> <+80>' then 'O <hash> <object hex>' per object (the file's own
bytes), then 'K <key> <hash of the record found, or 0 for none>' per search (hex)."""
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
    ap.add_argument('--loads', type=int, default=4)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = {k: addr(a.tool, k) for k in ('F-45518', 'F-80535', 'F-42985')}
    disc = a.categories.read_bytes()
    r = random.Random(80535)
    files = [disc] + [mutate(disc, r) for _ in range(a.loads - 1)]
    keys = []
    for d in files:
        hs = [joaat(name) for name, _o, _s in objects(d)]
        k = [0, 1, 0xffffffff] + [r.getrandbits(32) for _ in range(200)]
        for h in hs:
            k += [h, (h + 1) & 0xffffffff, (h - 1) & 0xffffffff]
        # (no random key that reads as an address of the executable: the public files cite none)
        keys.append([x for x in k if x in hs or x >> 24 != 0x82])
    fdata = ','.join('{%s}' % ','.join(str(b) for b in d) for d in files)
    flens = ','.join(str(len(d)) for d in files)
    kdata = ','.join('{%s}' % ','.join('%du' % x for x in k) for k in keys)
    klens = ','.join(str(len(k)) for k in keys)
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
static const uint32_t KD[][@MAXK@]={@KDATA@};
static const size_t KL[]={@KLENS@};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(size_t L=0;L<sizeof(FL)/sizeof(FL[0]);L++){
 rt_reset();fbuf=FD[L];fcur=0;flen=FL[L];store(0x82834DECu,0x3f800000u);
 Context ctx=fresh();ctx.r1.u32=0x40F00000u;ctx.r3.u32=@MGR@u;sub_@LOAD@(ctx,nullptr);
 uint32_t cap=load(@MGR@u+72),used=load(@MGR@u+80),lut=load(@MGR@u+76);
 printf("L %zu %u %u\n",L,cap,used);
 for(size_t i=0;i<KL[L];i++){uint32_t key=KD[L][i];
  Context s=fresh();s.r1.u32=0x40F00000u;s.r3.u32=@MGR@u;s.r4.u64=key;sub_@FIND@(s,nullptr);uint32_t rec=s.r3.u32,h=0;
  if(rec){h=0xdeadbeefu;for(uint32_t j=0;j<=cap;j++)if(load(lut+8*j)==rec){h=load(lut+8*j+4);break;}}
  printf("K %08x %08x\n",key,h);}}
}
"""
    for k, v in (('FDATA', fdata), ('FLENS', flens), ('KDATA', kdata), ('KLENS', klens), ('MAXLEN', max(len(d) for d in files)),
                 ('MAXK', max(len(k) for k in keys)), ('MGR', MGR), ('LOAD', f['F-45518']), ('FIND', f['F-80535'])):
        c = c.replace('@%s@' % k, str(v))
    img = rd.load_image(a.image)
    exe = rd.build(index(a.recomp), ['sub_' + f['F-45518'], 'sub_' + f['F-80535']], c, a.out_dir, 'category_lookup', stubs=stubs, image=img)
    out = run(exe, a.image).split(chr(10))
    res, li = [], 0
    for line in out:
        if line.startswith('L '):
            d = files[li]
            res.append(line)
            for name, off, sz in objects(d):
                res.append('O %08x %s' % (joaat(name), d[8 + off:8 + off + sz].hex()))
            li += 1
        elif line:
            res.append(line)
    assert li == len(files), (li, len(files))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'category_lookup_ref.txt').write_text(chr(10).join(res) + chr(10))
    print(len(files), 'loads,', sum(1 for x in res if x.startswith('K ')), 'searches')


if __name__ == '__main__':
    main()
