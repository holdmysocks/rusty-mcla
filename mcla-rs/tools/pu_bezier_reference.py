"""The lightning arcs' de Casteljau routine (F-29412) run offline: unchanged generated code. Signature (from the asm): r3 = result (16 bytes),
r6 = n 16-byte points, r7 = n, f1 = t; the points are copied to a stack buffer and reduced in place over n-1 rounds; p[0] is stored at r3.
Writes one line per case: BZ n t p0x p0y p0z p0w ... -> rx ry rz rw   (floats as hex bits)
Usage: pu_bezier_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

BEZIER = 'F-29412'
PTS, RES = 0x1200, 0x1100

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);int n;uint32_t tb;
 while(in>>n>>std::hex>>tb>>std::dec){rt_reset();
  for(int i=0;i<4*n;i++){uint32_t w;in>>std::hex>>w>>std::dec;store(PTS_+4*i,w);}
  Context c=fresh();c.r3.u32=RES_;c.r6.u32=PTS_;c.r7.u32=n;float t;std::memcpy(&t,&tb,4);c.f1.f64=t;sub_BEZ(c,nullptr);
  printf("R");for(int i=0;i<4;i++)printf(" %08x",load(RES_+4*i));printf("\n");}
}
'''


def f32bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    addr = subprocess.run([a.tool, 'registry', 'addr', BEZIER], capture_output=True, text=True, check=True).stdout.split()[1]
    fn = 'sub_' + addr
    src = MAIN.replace('PTS_', hex(PTS)).replace('RES_', hex(RES)).replace('sub_BEZ', fn)
    exe = rd.build(idx, [fn], src, a.work, 'pu_bezier', image=rd.load_image(a.image))
    rnd = random.Random(2027)
    lines = []
    for n in list(range(1, 11)):
        ts = [0.0, 1.0, 0.5] + [rnd.random() for _ in range(9)]
        for t in ts:
            pts = []
            for _ in range(n):
                pts += [f32bits(rnd.uniform(-50, 50)), f32bits(rnd.uniform(-5, 20)), f32bits(rnd.uniform(-50, 50)), f32bits(rnd.uniform(-1, 1))]
            lines.append(f'{n} {f32bits(t):08x} ' + ' '.join('%08x' % w for w in pts))
    cases = a.work / 'pu_bezier_cases.txt'
    cases.write_text('\n'.join(lines) + '\n')
    r = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    out = r.stdout.splitlines()
    res = []
    for cl, ol in zip(lines, out):
        p = cl.split()
        res.append('BZ ' + ' '.join(p) + ' -> ' + ol.split(' ', 1)[1])
    assert len(res) == len(lines)
    a.out.write_text('\n'.join(res) + '\n')
    print(len(res), 'cases')


if __name__ == '__main__':
    main()
