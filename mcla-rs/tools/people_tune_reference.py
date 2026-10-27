"""The racer tune blend (F-83981: lerp of two aivVehicleTuning blobs by t, then the derived scales F-49493) run offline on the original's code.
Inputs are generated from a seed by the 32-bit xorshift below (the Rust test regenerates them); the destination starts as the sentinel word
0x7fc0dead, so the words the original does not write show up. Output per case: the 2195 words of the destination.
Lines: `TL seedA seedB t_bits -> 2195 words (hex)`
Usage: people_tune_reference.py --recomp D:/MCLA-static --image IMG --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

LERP = 'F-83981'
N = 2195
DEST, A, B = 0x40100000, 0x40200000, 0x40300000

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);uint32_t sa,sb,t;
 while(in>>sa>>sb>>std::hex>>t>>std::dec){rt_reset();
  for(int i=0;i<2195;i++){store(DEST_+4*i,0x7fc0deadu);}
  uint32_t s[2]={sa,sb};
  for(int k=0;k<2;k++){uint32_t x=s[k]?s[k]:1;for(int i=0;i<2195;i++){x^=x<<13;x^=x>>17;x^=x<<5;
    float f=(i==0)?20.0f+float(x>>20)/4096.0f*80.0f:float(x>>8)/16777216.0f*30.0f; uint32_t w;memcpy(&w,&f,4);store((k?B_:A_)+4*i,w);}}
  Context c=fresh();c.r3.u32=DEST_;c.r5.u32=A_;c.r6.u32=B_;float tf;memcpy(&tf,&t,4);c.f1.f64=double(tf);
  sub_LERP(c,nullptr);
  for(int i=0;i<2195;i++)printf("%08x ",load(DEST_+4*i));printf("\n");}
}
'''


def fid_addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


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
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    fn = 'sub_' + fid_addr(a.tool, LERP)
    src = MAIN.replace('sub_LERP', fn).replace('DEST_', hex(DEST)).replace('A_', hex(A)).replace('B_', hex(B))
    exe = rd.build(idx, [fn], src, a.work, 'people_tune', image=img)
    cases = [(11, 12, 0.0), (11, 12, 1.0), (11, 12, 0.5), (13, 14, 0.37), (15, 16, 0.8125), (17, 18, 0.123456), (19, 20, 0.999)]
    cf = a.work / 'people_tune_cases.txt'
    cf.write_text('\n'.join(f'{x} {y} {fb(t):08x}' for x, y, t in cases) + '\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    out = [f'TL {x} {y} {fb(t):08x} -> ' + o.strip() for (x, y, t), o in zip(cases, r.stdout.splitlines())]
    a.out.write_text('\n'.join(out) + '\n')
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
