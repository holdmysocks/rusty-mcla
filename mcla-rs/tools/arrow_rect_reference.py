"""The top GPS arrow's blit F-29366 run offline as generated, its render-state calls stubbed and the quad draw F-66926 stubbed to
record the rectangle it is handed (f1 left, f2 top, f3 right, f4 bottom). Inputs: the screen object's lines (+692, an integer)
and its scale (+652); the control's BlitPosition.y (+1668), BlitScale (+1672), the shape ratio (+1676) and the centre column
(+1680); the fade word (the control's +248 for index 0) is 0, so the fade's alpha branch is not taken.
Line:  <lines> <scale> <blit y> <blit scale> <ratio> <centre> -> <left> <top> <right> <bottom>   (floats as their 32 bits, in decimal)
Usage: arrow_rect_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);
 const uint32_t C=0x40010000,S=0x40012000,P=0x40012800,R=0x40013000;
 const uint32_t SCR=uint32_t(-2105278464)-26880,IDX=uint32_t(-2105671680)+22080;
 int lines;uint32_t w[5];
 while(in>>lines){for(auto&x:w)in>>x;rt_reset();
  for(uint32_t a=C;a<0x40013100;a+=4)store(a,0);
  store(SCR,S);store(S+692,uint32_t(lines));store(S+652,w[0]);store(IDX,0);
  store(C+1648,P);store(C+1668,w[1]);store(C+1672,w[2]);store(C+1676,w[3]);store(C+1680,w[4]);
  Context cx=fresh();cx.r3.u32=C;sub_BLIT(cx,nullptr);
  printf("%u %u %u %u\n",load(R),load(R+4),load(R+8),load(R+12));
 }
}
'''


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    addr = lambda fid: 'sub_' + subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]
    blit = addr('F-29366')
    rec = ''.join('{float f=float(ctx.f%d.f64);uint32_t b;memcpy(&b,&f,4);store(0x40013000u+%d,b);}' % (k, 4 * (k - 1)) for k in range(1, 5))
    stubs = {addr(i): '' for i in ('F-13093', 'F-33110', 'F-81995', 'F-28789', 'F-60960', 'F-56814', 'F-17339', 'F-39265')}
    stubs[addr('F-66926')] = rec
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    exe = rd.build(idx, [blit], MAIN.replace('sub_BLIT(', blit + '('), a.work, 'arrow_rect', image=img, stubs=stubs)
    rnd = random.Random(29366)
    lines = []
    for n in range(4000):
        h = rnd.choice([480, 576, 600, 720, 768, 800, 900, 1024, 1080, 1200, 1440, 1600, 2160, rnd.randint(100, 4000)])
        w = rnd.choice([h * 16 // 9, h * 4 // 3, h * 21 // 9, h * 16 // 10, rnd.randint(100, 8000)])
        scale = rnd.choice([1.0, 1.0, 1.0, 0.5, 2.0, rnd.uniform(0.3, 2.5)])
        by = rnd.choice([-0.089999989, 0.0, rnd.uniform(-0.5, 0.5)])
        bs = rnd.choice([1.0, rnd.uniform(0.2, 3.0)])
        shape = w / h
        ratio = rnd.choice([shape / (16 / 9) if shape > 1.5 else shape / (4 / 3), rnd.uniform(0.5, 1.6)])
        centre = rnd.choice([w * 0.5, rnd.uniform(0, 4000)])
        lines.append(f'{h} ' + ' '.join(str(fb(v)) for v in (scale, by, bs, ratio, centre)))
    cases = a.work / 'arrow_rect_cases.txt'
    cases.write_text('\n'.join(lines) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    rows = p.stdout.splitlines()
    assert len(rows) == len(lines), (len(rows), len(lines))
    a.out.write_text(''.join(l + ' -> ' + r + '\n' for l, r in zip(lines, rows)))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
