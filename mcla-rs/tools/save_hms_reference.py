"""The seconds format F-68183 run offline (recomp_dispatch); its sprintf F-32585 is a stub that prints the text through
the harness's own printf. Writes save_hms_ref.txt, one case per line: seconds (float bits, decimal) -> text.
Usage: save_hms_reference.py <D:/MCLA-static> <original-image.bin> <out.txt>  (MCLA_TOOL = the mcla-tool exe)"""
import os
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

TOOL = os.environ.get('MCLA_TOOL', 'mcla-tool')

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 uint32_t b;
 while(scanf("%x",&b)==1){
  rt_reset();uint32_t buf=rt_alloc(64,16);float f;memcpy(&f,&b,4);
  printf("OUT ");Context c=fresh();c.f1.f64=f;c.r5.u32=buf;c.r6.u32=64;sub_HMS(c,nullptr);printf("\n");fflush(stdout);
 }
}
'''


def addr(fid):
    return subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def main():
    gen, image, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    r = random.Random(68183)
    vals = [0.0, 0.5, -0.5, 0.49999997, 1.5, 59.5, 3599.5, 3600.0, 356399.0, 356400.0, 359999.4, 360000.0, 1e7, -1.0, -59.5, -3600.5]
    vals += [r.uniform(0, 600) for _ in range(400)] + [r.uniform(0, 400000) for _ in range(300)] + [r.uniform(-4000, 0) for _ in range(100)]
    vals += [n + 0.5 for n in range(0, 200)]
    hms, spr = 'sub_' + addr('F-68183'), 'sub_' + addr('F-32585')
    work = Path(out).parent / 'build_hms'
    work.mkdir(exist_ok=True)
    exe = rd.build(rd.index(gen), [hms], MAIN.replace('sub_HMS', hms), work, 'hms', image=rd.load_image(image),
                   stubs={spr: 'VaIn w{&ctx,1,0,0};printf("%s",fmt_c(cfmt(ctx.r5.u32),w).c_str());'})
    bits = ['%x' % struct.unpack('>I', struct.pack('>f', v))[0] for v in vals]
    p = subprocess.run([str(exe), str(image)], input='\n'.join(bits) + '\n', capture_output=True, text=True)
    res = [l[4:] for l in p.stdout.split('\n') if l.startswith('OUT ')]
    assert len(res) == len(vals), (len(res), p.stderr[-800:])
    Path(out).write_text(''.join('%d -> %s\n' % (int(b, 16), t) for b, t in zip(bits, res)))
    print(len(res), 'cases')


if __name__ == '__main__':
    main()
