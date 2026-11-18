"""The parked spot's request fill F-23864 (called by the parked spawner F-74446) run offline: the generated code whole on
flat guest memory with the real sine / cosine; the request's defaults F-47257 stubbed (the record starts zeroed, but for
the old bytes +80 / +242 / +284 of the case).

Writes <out-dir>/park_request_ref.txt, a case a line: the spot's heading (float bits), the old +80, +242, +284 bytes
' -> ' the record's +0, +4, +12, +280 (timeout, type, spot, M+972), +32..+44 (the row), +48..+60 (the place), +68, +72
words and its +80, +240 (the colour, u16), +242, +284 (test `a_parked_spots_request_matches_the_original`)."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build

NL = chr(10)


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
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    fn, ctor = (addr(a.tool, f) for f in ('F-23864', 'F-47257'))
    rnd = random.Random(23865)
    cs = []
    for k in range(a.cases):
        h = rnd.choice((rnd.uniform(-math.pi, math.pi), rnd.uniform(-40, 40), rnd.uniform(0, 2 * math.pi),
                        rnd.choice((0.0, -0.0, math.pi, -math.pi, math.pi / 2, -math.pi / 2, 1e-7, 3 * math.pi))))
        cs.append((fb(h), rnd.randrange(256), rnd.randrange(256), rnd.randrange(256)))
    stubs = {'sub_' + ctor: ''}
    body = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
std::ifstream in(argv[2]);uint32_t w[4];
while(in>>std::hex>>w[0]){for(int i=1;i<4;i++)in>>std::hex>>w[i];memset(objects,0,sizeof(objects));
  uint32_t R=0x10000,S=0x20000,C=0x30000;
  for(int i=0;i<80;i++)store(R+4*i,0);
  store(R+80,(load(R+80)&0x00FFFFFF)|(w[1]<<24));store(R+240,(load(R+240)&0xFFFF00FF)|(w[2]<<8));
  store(R+284,(load(R+284)&0x00FFFFFF)|(w[3]<<24));
  store(S+48,0x44FA1234);store(S+52,0x42C80000);store(S+56,0xC4A10000);store(S+60,w[0]);store(C,0xBEEF0000);
  Context ctx{};ctx.r1.u32=0x7F000;ctx.r3.u32=R;ctx.f1.f64=1.0;ctx.r4.u32=0x777;ctx.r5.u32=S;ctx.r6.u32=C;ctx.r7.u32=0x972;
  sub_%s(ctx,nullptr);
  int o[]={0,4,12,280,32,36,40,44,48,52,56,60,68,72};for(int i:o)printf("%%08x ",load(R+i));
  printf("%%02x %%04x %%02x %%02x\n",load(R+80)>>24,load(R+240)>>16,(load(R+240)>>8)&255,load(R+284)>>24);}
}
''' % fn
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fn], body, a.out_dir, 'park_request', stubs=stubs, vector=True, scratch_end=0x80000)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'park_request_in.txt'
    inp.write_text(NL.join(' '.join('%x' % x for x in c) for c in cs) + NL)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True, check=True)
    out = [x for x in r.stdout.split(NL) if x]
    assert len(out) == len(cs), (len(out), len(cs), r.stdout[-300:])
    lines = ['%08x %02x %02x %02x -> %s' % (*c, o.strip()) for c, o in zip(cs, out)]
    (a.out_dir / 'park_request_ref.txt').write_text(NL.join(lines) + NL)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
