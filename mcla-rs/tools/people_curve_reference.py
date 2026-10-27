"""The curve evaluator pair (F-71868: point + unscaled derivative) and its B-spline basis (F-75457) run offline on path curves built by the
original's own builder (F-86418, as traffic_curve_reference.py; vtable of the curve class from the image: recomp_dispatch).
Lines written: `C k flag <4k point words>` (a curve, built by the original's builder), then for it
  EV t_bits -> p0 p1 p2 p3 d0 d1 d2 d3        (point and derivative vectors, words as hex bits)
  BA i kk u_bits -> n_bits dn_bits
Usage: people_curve_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

BUILD, CLASS_CTOR, EVAL, BASIS = 'F-86418', 'F-33069', 'F-71868', 'F-75457'
PTS, THIS, OUT = 0x1200, 0x1000, 0x2000

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string kind;int k,flag;
 while(in>>kind>>k>>flag){rt_reset();
  for(int i=0;i<4*k;i++){uint32_t w;in>>std::hex>>w>>std::dec;store(PTS_+4*i,w);}
  store(THIS_+32,PTS_);store16(THIS_+36,k);store(THIS_+180,flag);
  {Context c=fresh();c.r3.u32=THIS_;sub_BUILD(c,nullptr);}
  uint32_t cur=load(THIS_+176)+124;
  if(kind=="EV"){uint32_t tb;in>>std::hex>>tb>>std::dec;
   Context c=fresh();c.r3.u32=OUT_;c.r4.u32=cur;c.r6.u32=OUT_+32;
   uint32_t tw=tb;float tf;memcpy(&tf,&tw,4);c.f1.f64=double(tf);
   for(int i=0;i<8;i++)store(OUT_+4*i,0x7fc00001u);
   sub_EVAL(c,nullptr);
   printf("EV");for(int i=0;i<4;i++)printf(" %08x",load(OUT_+4*i));for(int i=0;i<4;i++)printf(" %08x",load(OUT_+32+4*i));printf("\n");}
  else{int i,kk;uint32_t ub;in>>i>>kk>>std::hex>>ub>>std::dec;
   Context c=fresh();c.r3.u32=0;c.r4.u32=i;c.r5.u32=kk;c.r7.u32=cur+8;c.r8.u32=OUT_;
   uint32_t uw=ub;float uf;memcpy(&uf,&uw,4);c.f1.f64=double(uf);store(OUT_,0x7fc00001u);
   sub_BASIS(c,nullptr);float n=float(c.f1.f64);uint32_t nb;memcpy(&nb,&n,4);
   printf("BA %08x %08x\n",nb,load(OUT_));}
 }
}
'''


def fid_addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def f32bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def bits32(w):
    return struct.unpack('>f', struct.pack('>I', w))[0]


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
    build, ctor, ev, ba = ('sub_' + fid_addr(a.tool, f) for f in (BUILD, CLASS_CTOR, EVAL, BASIS))
    vt = rd.stored_pointers(idx, ctor, 3)[124]
    src = MAIN.replace('PTS_', hex(PTS)).replace('THIS_', hex(THIS)).replace('OUT_', hex(OUT)).replace('sub_BUILD', build).replace('sub_EVAL', ev).replace('sub_BASIS', ba)
    exe = rd.build(idx, [build, ev, ba], src, a.work, 'people_curve', image=img, vtables=[(vt, 4)])
    rnd = random.Random(20261027)
    cases = []
    for k in range(2, 13):
        for flag in (0, 1):
            if flag and k < 3:
                continue
            for _ in range(4):
                pts = []
                for _ in range(k):
                    pts += [f32bits(rnd.uniform(-1500, 1500)), f32bits(rnd.uniform(-5, 50)), f32bits(rnd.uniform(-1500, 1500)), f32bits(1.0)]
                head = f'{k} {flag} ' + ' '.join('%08x' % w for w in pts)
                used = k - 1 if flag else k
                p = max(1, min(3, used - 1))
                ts = [0.0, 1.0, 0.5, -0.25, 1.5] + [rnd.random() for _ in range(12)]
                for t in ts:
                    cases.append(f'EV {head} {f32bits(t):08x}')
                # basis: knot count = used + p + 1; u across the knot range, on knots too
                nk = used + p + 1
                for _ in range(10):
                    kk = rnd.randint(1, p + 1)
                    i = rnd.randint(0, nk - kk - 1)
                    u = rnd.choice([rnd.uniform(-0.5, used - p + 0.5), float(rnd.randint(0, used - p)), float(rnd.randint(0, used - p)) + 1e-6])
                    cases.append(f'BA {head} {i} {kk} {f32bits(u):08x}')
    cf = a.work / 'people_curve_cases.txt'
    cf.write_text('\n'.join(cases) + '\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    out, last = [], None
    for cl, ol in zip(cases, r.stdout.splitlines()):
        w = cl.split(' ')
        nh = 2 + 4 * int(w[1]) * 1 + 2   # kind k flag then 4k words
        head = ' '.join(w[1:3 + 4 * int(w[1])])
        if head != last:
            out.append('C ' + head)
            last = head
        out.append(' '.join([w[0]] + w[3 + 4 * int(w[1]):]) + ' -> ' + ol.split(' ', 1)[1])
    a.out.write_text('\n'.join(out) + '\n')
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
