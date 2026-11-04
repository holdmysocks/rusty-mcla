"""The GPS map ring colour of types 7 / 8 / 9 run offline (recomp_dispatch): F-70194 on an element's colour word E+128. Writes
ui_element_colour_ref.txt, one case per line: word (decimal) -> R G B A (float bits, hex).
Usage: ui_element_colour_reference.py <D:/MCLA-static> <original-image.bin> <out.txt>  (MCLA_TOOL = the mcla-tool exe)"""
import os
import random
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

TOOL = os.environ.get('MCLA_TOOL', 'mcla-tool')

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 uint32_t w;
 while(scanf("%x",&w)==1){
  rt_reset();uint32_t e=rt_alloc(256,16),o=rt_alloc(16,16);store(e+128,w);
  Context c=fresh();c.r4.u32=e;c.r7.u32=o;sub_COL(c,nullptr);
  printf("OUT %x %x %x %x\n",load(o),load(o+4),load(o+8),load(o+12));fflush(stdout);
 }
}
'''


def main():
    gen, image, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    r = random.Random(70194)
    # (every byte value in each channel, then random words)
    words = [(v << 24) | (v << 16) | (v << 8) | v for v in range(256)] + [r.getrandbits(32) for _ in range(768)]
    col = 'sub_' + subprocess.run([TOOL, 'registry', 'addr', 'F-70194'], capture_output=True, text=True, check=True).stdout.split()[1]
    work = Path(out).parent / 'build_elcolour'
    work.mkdir(exist_ok=True)
    exe = rd.build(rd.index(gen), [col], MAIN.replace('sub_COL', col), work, 'elcolour', image=rd.load_image(image))
    p = subprocess.run([str(exe), str(image)], input=''.join('%x\n' % w for w in words), capture_output=True, text=True)
    res = [l[3:].strip() for l in p.stdout.split('\n') if l.startswith('OUT')]
    assert len(res) == len(words), (len(res), p.stderr[-800:])
    Path(out).write_text(''.join('%d -> %s\n' % (w, o) for w, o in zip(words, res)))
    print(len(res), 'cases')


if __name__ == '__main__':
    main()
