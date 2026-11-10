"""Small hull / contact-record functions run offline: the generated code unchanged (vector mode, the recomp's own
-msse4.1 -mssse3), on flat guest memory. Each section is one function; every case line of <out-dir>/hull_small_ref.txt is
"<tag> <input words> -> <output words>" (hex words, memory words in the guest's big-endian reading).

  centroid  F-67839  hull centroid setter: the vector stored at +48, byte +5 = any of x y z not equal to zero
  reset     F-51639  contact record reset: 10 vectors, +164 kind, +168..+174 u16 zeroed, byte +165 |= 0xC0
  ctor      F-34223  new contact record from six vectors and two element ids
  copy      F-86309  one contact record copied over another (byte +165: only bits 0x80 and 0x40 from the source)
  detector  F-15933  the GJK detector's constructor
  zero      F-54406  the push solve's state clear: velocity (+0) and angular velocity (+16) of every 128-byte state
  water     F-12574  mcCarFluid's in-bit (+12 bit 0)
  blow      F-61340  the damage multiplier: own +16 x the other vehicle's +20, else own +12
  material  F-17693  a polyhedron bound's material id of a polygon

The guest vectors: ctx.vN.u32[3-k] holds lane k (x = lane 0). The start-up constants the code reads (the xyz mask
0x82860950) are set as in hull_element_reference.py. No denormal is generated for a vector: the SDK's flush mode is a
no-op in this build, and the console flushes them."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
from cite_safe import cite_safe
import recomp_closure

_vector_shim = recomp_closure.vector_shim
# (vrlimi128 -> simde_mm_permute_ps: AVX)
recomp_closure.vector_shim = lambda scratch_end=0x3000: '#include <simde/x86/avx2.h>' + chr(10) + _vector_shim(scratch_end)

NL = chr(92) + 'n'
nl = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def is_denormal(w):
    return (w >> 23) & 0xFF == 0 and (w & 0x7FFFFF) != 0


def word(rnd, denorm=True):
    r = rnd.random()
    if r < 0.12:
        w = rnd.choice((0, 0x80000000, 0x3f800000, 0xbf800000, 0x7f800000, 0xff800000, 0x7fc00000, 0xffc00000, 0x7fa00000, 0x00000001, 0x80000001, 0x007fffff, 0x00800000, 0x322bcc77))
    elif r < 0.7:
        w = fb(rnd.uniform(-100, 100))
    else:
        w = rnd.getrandbits(32)
    if not denorm and is_denormal(w):
        w = 0
    return w


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = {k: addr(a.tool, v) for k, v in dict(centroid='F-67839', reset='F-51639', ctor='F-34223', copy='F-86309', detector='F-15933',
                                              zero='F-54406', water='F-12574', blow='F-61340', material='F-17693').items()}
    rnd = random.Random(67839)
    secs = []   # (tag, cases, setup, call, out-count, outcode)

    def vec(denorm=False):
        return [word(rnd, denorm) for _ in range(4)]

    # centroid: 4 vector words, 16 words of prefilled object
    cs = []
    for i in range(4000):
        v = vec()
        if i % 5 == 0:
            for k in rnd.sample(range(3), rnd.randrange(4)):
                v[k] = rnd.choice((0, 0x80000000))
            if rnd.random() < 0.5:
                v = [0, 0, 0, v[3]]
        cs.append(v + [rnd.getrandbits(32) for _ in range(16)])
    secs.append(('centroid', cs, 'for(int i=0;i<16;i++)store(0x1000+4*i,c[4+i]);for(int k=0;k<4;k++)store(0x1100+4*k,c[k]);' +
                 'ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;', f['centroid'], 16, 'load(0x1000+4*i)'))
    # reset
    cs = [[rnd.getrandbits(32) for _ in range(48)] for _ in range(2000)]
    secs.append(('reset', cs, 'for(int i=0;i<48;i++)store(0x1000+4*i,c[i]);ctx.r3.u32=0x1000;', f['reset'], 48, 'load(0x1000+4*i)'))
    # ctor: v1..v6, r4, r5, prefill
    cs = []
    for _ in range(4000):
        c = []
        for k in range(6):
            c += vec()
        c += [rnd.getrandbits(32), rnd.getrandbits(32)]
        c += [rnd.getrandbits(32) for _ in range(48)]
        cs.append(c)
    secs.append(('ctor', cs, 'for(int i=0;i<48;i++)store(0x1000+4*i,c[26+i]);ctx.r3.u32=0x1000;' +
                 'ctx.r4.u32=c[24];ctx.r5.u32=c[25];ctx.r1.u32=0x2F00;' +
                 'uint32_t* V[6]={ctx.v1.u32,ctx.v2.u32,ctx.v3.u32,ctx.v4.u32,ctx.v5.u32,ctx.v6.u32};' +
                 'for(int v=0;v<6;v++)for(int k=0;k<4;k++)V[v][3-k]=c[4*v+k];', f['ctor'], 48, 'load(0x1000+4*i)'))
    # copy: dst 48 words, src 48 words (src bytes 164/165 random)
    cs = []
    for _ in range(3000):
        cs.append([rnd.getrandbits(32) for _ in range(96)])
    secs.append(('copy', cs, 'for(int i=0;i<48;i++){store(0x1000+4*i,c[i]);store(0x1100+4*i,c[48+i]);}' +
                 'ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;ctx.r1.u32=0x2F00;', f['copy'], 48, 'load(0x1000+4*i)'))
    # detector ctor: r4..r7, prefill 16 words
    cs = [[rnd.getrandbits(32) for _ in range(4 + 16)] for _ in range(1500)]
    secs.append(('detector', cs, 'for(int i=0;i<16;i++)store(0x1000+4*i,c[4+i]);ctx.r3.u32=0x1000;' +
                 'ctx.r4.u32=c[0];ctx.r5.u32=c[1];ctx.r6.u32=c[2];ctx.r7.u32=c[3];', f['detector'], 16, 'load(0x1000+4*i)'))
    # zero: n, 8 states x 32 words
    cs = []
    for _ in range(1500):
        cs.append([rnd.choice((-1, 0, 1, 2, 3, 4, 5, 8)) & 0xFFFFFFFF] + [rnd.getrandbits(32) for _ in range(256)])
    secs.append(('zero', cs, 'store(0x1000,0x1100);store(0x1004,c[0]);for(int i=0;i<256;i++)store(0x1100+4*i,c[1+i]);ctx.r3.u32=0x1000;',
                 f['zero'], 256, 'load(0x1100+4*i)'))
    # water
    cs = [[rnd.getrandbits(32) if rnd.random() < 0.7 else rnd.choice((0, 1, 2, 3, 0xFFFFFFFF, 0xFFFFFFFE))] for _ in range(300)]
    secs.append(('water', cs, 'store(0x1000+12,c[0]);ctx.r3.u32=0x1000;', f['water'], 1, 'ctx.r3.u32'))
    # blow: own +12, own +16, mode, flags, other's factor
    cs = []
    for _ in range(3000):
        cs.append([word(rnd, True), word(rnd, True), rnd.choice((0, 1, 2, 3, 3, 3, 4)), rnd.choice((0, 4, 8, 12, 0x10, 0x14, 0xFFFFFFF3, rnd.getrandbits(32))), word(rnd, True)])
    secs.append(('blow', cs, 'store(0x1000+12,c[0]);store(0x1000+16,c[1]);' +
                 'ctx.r3.u32=0x1000;ctx.r4.u32=c[2]==0?0:0x1100;' +
                 'if(c[2]>=1)store(0x1100+16,c[2]==1?0:0x1200);store(0x1200+4,0x1300);store(0x1300+16,c[3]);' +
                 'store(0x1200+80,0x1400);store(0x1400+20,0x1500);store(0x1500+20,c[4]);', f['blow'], 1,
                 '[&]{float r=float(ctx.f1.f64);uint32_t u;memcpy(&u,&r,4);return u;}()'))
    # material: nt, default, nm, idx, 8 table words, 32 bytes
    cs = []
    for _ in range(4000):
        nm = rnd.randrange(0, 9)
        nt = rnd.randrange(0, 24)
        dflt = rnd.choice([-1] + list(range(0, nm + 3)))
        idx = rnd.randrange(-3, nt + 4)
        c = [nt, dflt & 0xFFFFFFFF, nm, idx & 0xFFFFFFFF] + [rnd.getrandbits(32) for _ in range(8)]
        c += [rnd.randrange(0, nm + 4) if rnd.random() < 0.9 else rnd.randrange(256) for _ in range(32)]
        cs.append(c)
    secs.append(('material', cs, 'store(0x1000+204,c[0]);store(0x1000+208,0x1200);store(0x1000+212,c[1]);store(0x1000+216,0x1300);' +
                 '*pointer(0x1000+220)=c[2];for(int i=0;i<8;i++)store(0x1200+4*i,c[4+i]);for(int i=0;i<32;i++)*pointer(0x1300+i)=c[12+i];' +
                 'ctx.r3.u32=0x1000;ctx.r4.u32=c[3];', f['material'], 1, 'ctx.r3.u32'))

    body = ''
    for tag, cs, setup, fn, nout, outc in secs:
        w = len(cs[0])
        rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cs)
        body += ('{static const uint32_t C[][%d]={%s%s%s};' % (w, nl, rows, nl) +
                 'for(auto&c:C){memset(objects,0,sizeof(objects));Context ctx{};ctx.r1.u32=0x2F00;' + setup +
                 'sub_%s(ctx,nullptr);' % fn +
                 'for(int i=0;i<%d;i++)printf("%%08x ",uint32_t(%s));printf("%s");}}' % (nout, outc, NL) + nl)
    consts = ('for(int k=0;k<3;k++){store(0x82860950+4*k,0xFFFFFFFF);store(0x82860930+4*k,0);}store(0x8286095C,0);' +
              'store(0x8286093C,0x3F800000);')
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' +
                nl + consts + nl + body + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + secs_f for secs_f in f.values()], main_src, a.out_dir, 'hull_small', vector=True,
                flags=('-msse4.1', '-mssse3'))
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == sum(len(s[1]) for s in secs), (len(out), sum(len(s[1]) for s in secs))
    lines = []
    k = 0
    for tag, cs, *_ in secs:
        for c in cs:
            lines.append('%s %s -> %s' % (tag, ' '.join('%08x' % x for x in c), out[k].strip()))
            k += 1
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'hull_small_ref.txt').write_text(cite_safe(nl.join(lines) + nl))
    print({s[0]: len(s[1]) for s in secs})


if __name__ == '__main__':
    main()
