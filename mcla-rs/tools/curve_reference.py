"""The audio curves run offline: every curves.dat object set up by the curve init F-57265 (the curve store F-67395
stubbed to hand it the object) and evaluated by F-33490, the generated code whole and unchanged, at fixed and random
inputs; and the rolloff F-98578 (scale f1, distance f2; r4 the curve's slot in the table at r3 + 3780, 40 bytes a
slot, 0 = the built-in DEFAULT_ROLLOFF) on random scales
and distances, with no curve and with each curve.
Writes <out-dir>/curve_ref.txt (each number led by a letter, so no column reads as an address): per curve
'K c<object bytes hex>', then 'X x r' (f32 bits); then 'D k s d r' (k = the curve's index, or n for none; f32 bits)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd
from recomp_closure import index, run
import occlusion_reference  # noqa: F401  (its vector shim: the Gauss curve's vector powf, the SDK's flush mode)

OBJS = 0x40100000    # the curves.dat objects, one stride each
COBJ = 0x40300000    # the runtime curve (64 bytes)
STACK = 0x40F00000


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def curves(path):
    d = open(path, 'rb').read()
    size = struct.unpack_from('>I', d, 4)[0]
    p = 8 + size
    p += 4 + struct.unpack_from('>I', d, p)[0]
    count = struct.unpack_from('>I', d, p)[0]
    p += 8
    out = []
    for _ in range(count):
        n = d[p]
        p += 1 + n
        off, sz = struct.unpack_from('>II', d, p)
        p += 8
        out.append(d[8 + off:8 + off + sz])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--curves-dat', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    cinit, ceval, roll, store_ = (addr(a.tool, f) for f in ('F-57265', 'F-33490', 'F-98578', 'F-67395'))
    objs = curves(a.curves_dat)
    stride = (max(len(o) for o in objs) + 15) & ~15
    img = rd.load_image(a.image)
    idx = index(a.recomp)
    r = random.Random(33490)
    fixed = [0.0, -0.0, 1.0, -1.0, 0.5, 2.0, 0.25, 0.75, 1e-7, 0.999999, 1.000001, 5.0, 4.99, 128.0, 127.99, 10.0, 100.0,
             1000.0, 20000.0, -5.0, float('inf'), float('-inf'), float('nan')]
    xs = []
    for o in objs:
        own = fixed + [struct.unpack_from('>f', o, k)[0] for k in range(13, len(o) - 3, 4)]
        own = [v for v in own if v == v or v != v]
        own += [r.uniform(-0.5, 1.5) for _ in range(150)] + [r.uniform(-10, 200) for _ in range(150)]
        own += [r.uniform(-1, 30000) for _ in range(40)] + [float(r.randint(-2, 40)) for _ in range(30)]
        xs.append([fb(v) for v in own])
    ds = []
    scales = (0.0, 1.0, 0.5, 2.0, 0.3, 1.7, 3.0, 0.1, 10.0, 0.999, 1.001, 0.75, 4.5)
    for k in [-1] * 6 + list(range(len(objs))):
        for _ in range(120 if k < 0 else 40):
            s = r.choice(scales) if r.random() < 0.6 else r.uniform(0.01, 8)
            d = r.choice((0.0, 5.0, 4.99, 128.0, 100.0, r.uniform(0, 10))) if r.random() < 0.3 else r.uniform(0, 400)
            ds.append((k & 0xffffffff, fb(s), fb(d)))
    blob = b''.join(o.ljust(stride, b'\0') for o in objs)
    xdata = ','.join('%d,%s' % (len(x), ','.join('%du' % v for v in x)) for x in xs)
    ddata = ','.join('%du,%du,%du' % t for t in ds)
    recomp_closure.EXTRA += ('static uint32_t fbits(float f){uint32_t b;memcpy(&b,&f,4);return b;}' + chr(10) +
                             'static float fbits2(uint32_t b){float f;memcpy(&f,&b,4);return f;}' + chr(10))
    stubs = {'sub_' + store_: '{uint32_t h=ctx.r4.u32,p=0;if(h>=0x1000u&&h<0x1000u+@N@u)p=@OBJS@u+(h-0x1000u)*@STRIDE@u;'
                              'store(ctx.r5.u32,p);ctx.r3.u32=p?1:0;}'}
    c = r"""
static const uint8_t B[]={@BLOB@};
static const uint32_t XS[]={@XS@};
static const uint32_t DS[]={@DS@};
static void init(uint32_t i){memset(pointer(@COBJ@u),0,64);Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@COBJ@u;ctx.r4.u32=0x1000u+i;
 sub_@CINIT@(ctx,nullptr);}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
for(size_t q=0;q<sizeof(B);q++)*pointer(@OBJS@u+(uint32_t)q)=B[q];
size_t p=0;for(uint32_t i=0;i<@N@u;i++){uint32_t n=XS[p++];printf("K i%u\n",i);
 for(uint32_t j=0;j<n;j++){uint32_t x=XS[p++];init(i);Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@COBJ@u;ctx.f1.f64=(double)fbits2(x);
  sub_@CEVAL@(ctx,nullptr);printf("X x%08x r%08x\n",x,fbits((float)ctx.f1.f64));}}
for(size_t i=0;i<sizeof(DS)/4;i+=3){uint32_t k=DS[i];uint32_t o=0;if(k!=0xffffffffu){init(k);o=1;}
 Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@COBJ@u-3780u-40u;ctx.r4.u32=o;ctx.f1.f64=(double)fbits2(DS[i+1]);ctx.f2.f64=(double)fbits2(DS[i+2]);
 sub_@ROLL@(ctx,nullptr);if(k==0xffffffffu)printf("D kn");else printf("D k%u",k);printf(" s%08x d%08x r%08x\n",DS[i+1],DS[i+2],fbits((float)ctx.f1.f64));}}
"""
    for k, v in (('BLOB', ','.join(str(b) for b in blob)), ('XS', xdata), ('DS', ddata), ('N', len(objs)), ('STRIDE', stride), ('OBJS', OBJS),
                 ('COBJ', COBJ), ('STACK', STACK), ('CINIT', cinit), ('CEVAL', ceval), ('ROLL', roll)):
        c = c.replace('@%s@' % k, str(v))
        stubs = {n: b.replace('@%s@' % k, str(v)) for n, b in stubs.items()}
    exe = rd.build(idx, ['sub_' + cinit, 'sub_' + ceval, 'sub_' + roll], c, a.out_dir, 'curves', stubs=stubs, image=img)
    out = run(exe, a.image)
    lines = out.split(chr(10))
    o = []
    for l in lines:
        if l.startswith('K i'):
            o.append('K c' + objs[int(l[3:])].hex())
        elif l:
            o.append(l)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'curve_ref.txt').write_text(chr(10).join(o) + chr(10))
    print(len(objs), 'curves,', sum(l.startswith('X') for l in o), 'evaluations,', sum(l.startswith('D') for l in o), 'rolloffs')


if __name__ == '__main__':
    main()
