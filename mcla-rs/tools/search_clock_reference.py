"""Run the original's Search-clock test F-73289 (the Search state's vt+80: its clock +16 > 0, read by the tactics' vt+44 F-77220
as "the target is lost") offline on every one of the 2^32 bit patterns of the clock, and write which give true, as ranges of bit
patterns, for the port's test (crates/game/src/testdata/search_clock_ref.txt; test search_clock_matches_the_original).

The function compares the clock (lfs, so widened to double) with a float constant of the image (+0.0) by fcmpu and returns
1 on greater: NaN, -0.0 and +0.0 give 0. Output:
  true <lo>-<hi>      (one line per run of consecutive true patterns, hex)
  count <n>
"""
import argparse
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 rt_reset(); const uint32_t S=0x40A00000u; uint64_t n=0; bool in=false; uint32_t lo=0; Context c0=fresh();
 for(uint64_t b=0;b<=0xFFFFFFFFull;b++){ store(S+16,(uint32_t)b); Context c=c0; c.r3.u32=S; TEST(c,nullptr); bool t=(c.r3.u32&1u)!=0;
  if(t){ n++; if(!in){in=true;lo=(uint32_t)b;} } else if(in){ in=false; printf("true %08x-%08x\n",lo,(uint32_t)(b-1)); } }
 if(in) printf("true %08x-ffffffff\n",lo); printf("count %llu\n",(unsigned long long)n);
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    r = subprocess.run([a.tool, 'registry', 'addr', 'F-73289'], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = 'sub_%08X' % addr['F-73289']
    img = rdisp.load_image(a.image)
    exe = rdisp.build(rdisp.index(a.recomp), [name], MAIN.replace('TEST', name), a.out, 'search_clock', image=img, limit=5)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    a.ref.write_text(r.stdout)
    print(r.stdout, end='')


if __name__ == '__main__':
    main()
