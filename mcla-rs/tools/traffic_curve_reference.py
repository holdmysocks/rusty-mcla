"""The race path's curve builder (F-86418 -> curve constructor F-80887 -> knot padding F-42351) run offline, with the class's own vtable
(the constructor's slot 1 is a virtual call: recomp_dispatch). Cases: the control-point count k (2..12) and the 'closed' flag word
at +180 (non-zero: one point less), random control points. Writes one line per case:
  CV k flag px py pz pw ... -> degree count knots.. trange tmax      (floats as hex bits; after tmax: the stored point count and its points, 4 words each)
Usage: traffic_curve_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

CURVE_BUILD, CURVE_CTOR, CURVE_CLASS_CTOR = 'F-86418', 'F-80887', 'F-33069'
PTS, THIS = 0x1200, 0x1000        # control points; the path object (+32 points, +36 count, +180 flag, +176 result)

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);int k,flag;
 while(in>>k>>flag){rt_reset();
  for(int i=0;i<4*k;i++){uint32_t w;in>>std::hex>>w>>std::dec;store(PTS_+4*i,w);}
  store(THIS_+32,PTS_);store16(THIS_+36,k);store(THIS_+180,flag);
  Context c=fresh();c.r3.u32=THIS_;sub_BUILD(c,nullptr);
  uint32_t cur=load(THIS_+176);uint32_t kn=load(cur+124+8+4);uint32_t n=load(cur+124+8+8);
  printf("CV %d %d ->",k,flag);printf(" %d %u",(int)*pointer(cur+124+31),n);
  for(uint32_t i=0;i<n;i++)printf(" %08x",load(kn+4*i));
  printf(" %08x %08x",load(cur+124+20),load(cur+124+24));
  uint32_t np=load16(cur+124+28)+1u,pp=load(cur+124+4);printf(" %u",np);for(uint32_t i=0;i<4*np;i++)printf(" %08x",load(pp+4*i));printf("\n");}
}
'''


def fid_addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


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
    img = rd.load_image(a.image)
    build, ctor = 'sub_' + fid_addr(a.tool, CURVE_BUILD), 'sub_' + fid_addr(a.tool, CURVE_CLASS_CTOR)
    vt = rd.stored_pointers(idx, ctor, 3)[124]
    src = MAIN.replace('PTS_', hex(PTS)).replace('THIS_', hex(THIS)).replace('sub_BUILD', build)
    exe = rd.build(idx, [build], src, a.work, 'traffic_curve', image=img, vtables=[(vt, 4)])
    rnd = random.Random(2026)
    lines = []
    for k in range(2, 13):
        for flag in (0, 1):
            if flag and k < 3:
                continue
            for _ in range(6):
                pts = []
                for _ in range(k):
                    pts += [f32bits(rnd.uniform(-1500, 1500)), f32bits(rnd.uniform(-5, 50)), f32bits(rnd.uniform(-1500, 1500)), f32bits(1.0)]
                lines.append(f'{k} {flag} ' + ' '.join('%08x' % w for w in pts))
    cases = a.work / 'traffic_curve_cases.txt'
    cases.write_text('\n'.join(lines) + '\n')
    r = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    out = []
    for cl, ol in zip(lines, r.stdout.splitlines()):
        out.append(ol.replace(' ->', ' ' + ' '.join(cl.split()[2:]) + ' ->', 1))
    a.out.write_text('\n'.join(out) + '\n')
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
