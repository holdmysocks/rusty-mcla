"""The agro ability's speed factor F-91892 run offline: unchanged generated code on an ability object (+4 -> +8 the car's sim, its
speed at +224; ThresholdSpeed at +672) over speeds and thresholds with the edge cases (0, -0, the threshold itself, NaN, inf).
Line:  AGS <speed> <threshold> -> <factor>        (floats as hex bits)
Usage: agro_speed_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string k;
 const uint32_t A=0x40010000,B=0x40010100,S=0x40010200;
 while(in>>k){rt_reset();uint32_t v,t;in>>std::hex>>v>>t>>std::dec;
  store(A+4,B);store(B+8,S);store(S+224,v);store(A+672,t);
  Context cx=fresh();cx.r3.u32=A;sub_F(cx,nullptr);
  float r=float(cx.f1.f64);uint32_t u;std::memcpy(&u,&r,4);printf("%08x\n",u);}
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
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    name = 'sub_' + subprocess.run([a.tool, 'registry', 'addr', 'F-91892'], capture_output=True, text=True, check=True).stdout.split()[1]
    exe = rd.build(idx, [name], MAIN.replace('sub_F(', name + '('), a.work, 'agro_speed', image=img)
    rnd = random.Random(91892)
    edge = [0.0, -0.0, float('nan'), float('inf'), -float('inf'), 1e-30, -1.0]
    cases = []
    for t in [13.41119, 20.0, 0.0, -0.0, float('nan'), float('inf'), 1e-30, -5.0]:
        for v in edge + [t, rnd.uniform(0, 40)]:
            cases.append((v, t))
    for _ in range(3000):
        t = rnd.choice([13.41119, 20.0, rnd.uniform(0.01, 60)])
        v = rnd.choice([rnd.uniform(-5, 60), t, rnd.uniform(0, t), struct.unpack('>f', struct.pack('>I', fb(t) + rnd.choice([-1, 1])))[0]])
        cases.append((v, t))
    lines = [f'AGS {fb(v):08x} {fb(t):08x}' for v, t in cases]
    cf = a.work / 'agro_speed_cases.txt'
    cf.write_text('\n'.join(lines) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    rows = p.stdout.split()
    assert len(rows) == len(lines), (len(rows), len(lines))
    out = [l + ' -> ' + r for l, r in zip(lines, rows)]
    keep = [l for l in out if not re.search(r'(?<![0-9a-zA-Z])82[0-9a-f]{6}(?![0-9a-zA-Z])', l)]      # tracked data must not hold raw-address-looking words
    a.out.write_text('\n'.join(keep) + '\n')
    print(len(keep), 'of', len(out), 'cases')


if __name__ == '__main__':
    main()
