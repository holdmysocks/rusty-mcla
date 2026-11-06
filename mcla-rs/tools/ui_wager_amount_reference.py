"""The wager form's amount set-up run offline (recomp_dispatch): F-19872 with the slider's money call F-86598 stubbed to
write a given base into its +60 (the form's +1564), the slider reset F-57833 stubbed to record that it was called, and the
form's vt+184 at +672 faked. Each case: base (int), difficulty (+1216), the old amount (+1464). Writes
ui_wager_amount_ref.txt:  <base> <diff> <old> -> <+1456 lo> <+1460 hi> <+1464 amount> <+1468 step> <reset called>
Usage: ui_wager_amount_reference.py <D:/MCLA-static> <original-image.bin> <out.txt> [cases]  (MCLA_TOOL = the mcla-tool exe)"""
import os
import random
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

TOOL = os.environ.get('MCLA_TOOL', 'mcla-tool')
FAKE_VT = 0x7F000010


def addr(fid):
    return subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


MAIN = r'''
uint32_t BASE=0,RESET=0;
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 int32_t b;uint32_t d,old;
 while(scanf("%d %u %u",&b,&d,&old)==3){
  rt_reset();
  uint32_t F=rt_alloc(2048,16),V=rt_alloc(256,16),W=rt_alloc(16,16);
  for(int i=0;i<2048;i+=4) store(F+i,0);
  for(int i=0;i<256;i+=4) store(V+i,FAKEVT);
  store(F+672,V);store(F+1216,d);store(F+1464,old);
  BASE=uint32_t(b);RESET=0;
  Context c=fresh();c.r3.u32=F;
  sub_AMT(c,nullptr);
  printf("OUT %d %d %d %d %u",int32_t(load(F+1456)),int32_t(load(F+1460)),int32_t(load(F+1464)),int32_t(load(F+1468)),RESET);putchar(10);fflush(stdout);
 }
}
'''


def main():
    gen, image, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    count = int(sys.argv[4]) if len(sys.argv) > 4 else 4000
    r = random.Random(19872)
    cases = []
    for i in range(count):
        b = r.choice([r.randint(0, 3000), r.randint(0, 60000), r.randint(0, 400000), r.randint(-500, 500), r.randrange(0, 100000, 10)])
        d = r.choice([0, 1, 2, 3, 0, 1, 2, 3, 4, 7])
        old = r.choice([0, 1, r.randint(0, 100000)])
        cases.append((b, d, old))
    idx = rd.index(gen)
    img = rd.load_image(image)
    work = Path(out).parent / 'build_wageramt'
    work.mkdir(exist_ok=True)
    amt, money, reset = ('sub_' + addr(x) for x in ('F-19872', 'F-86598', 'F-57833'))
    src = MAIN.replace('sub_AMT', amt).replace('FAKEVT', '%uu' % FAKE_VT)
    exe = rd.build(idx, [amt], src, work, 'wageramt', image=img,
                   stubs={money: '{extern uint32_t BASE;store(ctx.r3.u32+60,BASE);}', reset: '{extern uint32_t RESET;RESET=1;}'},
                   fakes={FAKE_VT: ''})
    p = subprocess.run([str(exe), str(image)], input=''.join('%d %u %u\n' % c for c in cases), capture_output=True, text=True)
    res = [l[4:] for l in p.stdout.split('\n') if l.startswith('OUT ')]
    assert len(res) == len(cases), (len(res), p.stderr[-800:])
    Path(out).write_text(''.join('%d %u %u -> %s\n' % (c + (o,)) for c, o in zip(cases, res)))
    print(len(res), 'cases')


if __name__ == '__main__':
    main()
