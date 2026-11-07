"""The Doppler ratio (F-42745) run offline: the generated code whole and unchanged. f1 = the distance change, f2 = the
voice's Doppler factor, r5 = the time step in ms (the caller's li r5,30; 0 gives 1.0), r3 = the environment with
+3668 the speed of sound (340) and +3672 the largest relative speed (100), as the game's audio xml sets them.
Writes <out-dir>/doppler_ratio_ref.txt: 'd<f1 bits> f<f2 bits> t<r5> -> r<f1 bits>' (hex)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
from recomp_closure import index, run

ENV = 0x40100000
STACK = 0x40F00000


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = addr(a.tool, 'F-42745')
    r = random.Random(42745)
    cases = []
    edge = [0.0, -0.0, 1e-7, -1e-7, 3.0, -3.0, 2.9999998, 3.0000002, -2.9999998, -3.0000002, 1.0, -1.0, 100.0, -100.0]
    for _ in range(4000):
        d = r.choice(edge) if r.random() < 0.15 else r.uniform(-6, 6) * r.choice((1, 0.1, 0.01, 0.001))
        f = r.choice((0.0, 1.0, 1.0, 1.0, 0.5, r.random(), 0.033333335))
        t = 30 if r.random() < 0.9 else r.choice((0, 16, 33))
        cases.append((fb(d), fb(f), t))
    data = ','.join('%du,%du,%du' % c for c in cases)
    c = r"""
static const uint32_t C[]={@DATA@};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
memset(pointer(@ENV@u),0,0x1000);store(@ENV@u+3668,0x43AA0000u);store(@ENV@u+3672,0x42C80000u);
for(size_t i=0;i<sizeof(C)/4;i+=3){float d,f;memcpy(&d,&C[i],4);memcpy(&f,&C[i+1],4);
 Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@ENV@u;ctx.r5.u32=C[i+2];ctx.f1.f64=d;ctx.f2.f64=f;sub_@FN@(ctx,nullptr);
 float o=(float)ctx.f1.f64;uint32_t ob;memcpy(&ob,&o,4);printf("d%08x f%08x t%u -> r%08x\n",C[i],C[i+1],C[i+2],ob);}}
"""
    for k, v in (('DATA', data), ('ENV', ENV), ('STACK', STACK), ('FN', fn)):
        c = c.replace('@%s@' % k, str(v))
    exe = rd.build(index(a.recomp), ['sub_' + fn], c, a.out_dir, 'doppler_ratio', image=rd.load_image(a.image))
    lines = [x for x in run(exe, a.image).split(chr(10)) if x]
    assert len(lines) == len(cases), (len(lines), len(cases))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'doppler_ratio_ref.txt').write_text(chr(10).join(lines) + chr(10))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
