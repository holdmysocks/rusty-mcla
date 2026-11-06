"""GET_CONTROLLER_LEFTX's getter run offline (recomp_dispatch): F-80539 with the UI-capture test stubbed (case 0: input
open, case 1: captured) and the player's pad index fixed at 0; the pad record's left-stick byte (+16) set to each of
0..255. Writes ui_leftx_ref.txt:  <captured> <byte> -> <the f32 result as bits>
Usage: ui_leftx_reference.py <D:/MCLA-static> <original-image.bin> <out.txt>  (MCLA_TOOL = the mcla-tool exe)"""
import os
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

TOOL = os.environ.get('MCLA_TOOL', 'mcla-tool')


def addr(fid):
    return subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 uint32_t cap,b;
 while(scanf("%u %u",&cap,&b)==2){
  rt_reset();
  CAPTURED=cap;
  *pointer(PADREC+16)=uint8_t(b);
  Context c=fresh();c.r3.u32=0;
  sub_GET(c,nullptr);
  float f=float(c.f1.f64);uint32_t w;memcpy(&w,&f,4);
  printf("OUT %u",w);putchar(10);fflush(stdout);
 }
}
'''


def main():
    gen, image, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    idx = rd.index(gen)
    img = rd.load_image(image)
    work = Path(out).parent / 'build_leftx'
    work.mkdir(exist_ok=True)
    get = 'sub_' + addr('F-80539')
    # (the getter's callees: the UI-capture test and the player's pad index; the pad records' table as the getter's code
    # builds it, -2104492032 + 21384, 64 bytes a pad)
    calls = ['sub_' + addr('F-99388'), 'sub_' + addr('F-18285')]
    rec = (-2104492032 + 21384) & 0xFFFFFFFF
    main_src = ('uint32_t CAPTURED=0;\nstatic const uint32_t PADREC=%uu;\n' % rec + MAIN).replace('sub_GET', get)
    exe = rd.build(idx, [get], main_src, work, 'leftx', image=img,
                   stubs={calls[0]: '{extern uint32_t CAPTURED;ctx.r3.u64=CAPTURED;}', calls[1]: '{ctx.r3.u64=0;}'})
    cases = [(c, b) for c in (0, 1) for b in range(256)]
    p = subprocess.run([str(exe), str(image)], input=''.join(f'{c} {b}\n' for c, b in cases), capture_output=True, text=True)
    res = [l[4:] for l in p.stdout.split('\n') if l.startswith('OUT ')]
    assert len(res) == len(cases), (len(res), p.stderr[-800:])
    Path(out).write_text(''.join(f'{c} {b} -> {o}\n' for (c, b), o in zip(cases, res)))
    print(len(res), 'cases')


if __name__ == '__main__':
    main()
