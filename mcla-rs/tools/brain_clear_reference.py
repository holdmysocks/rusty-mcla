"""Run the racing brain's Clear F-76382 (the cop brain's attach F-43897 tail-calls it) offline (static recompilation, native,
flat guest memory) on a brain object filled with A5A5A5A5, and print what it leaves, for the port's fresh police brain to be
compared with (`brain.rs` test police_brain_is_the_cleared_brain).

Clear (asm, docs/reference/hookmen/brain_build.md): the base call F-27233 (stubbed: it marks a word; its query allocation is the
AI topic's), the aiv job +304 freed through F-58498 when set (stubbed: marks a word) and zeroed, the goal arrays +176 / +184
(pointer, u16 count +4, u16 capacity +6) emptied - count 0, and pointer and capacity 0 when the capacity is 0 -, then
+212 / +216 / +220 = 0, +332 = 0, +336 (byte) = 0, +224 = 1, +292 = the image's -1.0, +252 (byte) = 0, +296 = the image's 0.0,
+272 = -1.

Two cases: capacities 0 (both arrays) and capacities 5 / 7. Output, one line a case:
  case <k> base <0|1> free <0|1> w<offset>=<word> ... (offsets 176 .. 340)
"""
import argparse
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 const uint32_t B=0x40A00000u, MARK=0x40B00000u;
 for(int k=0;k<2;k++){ rt_reset();
  for(int i=0;i<600;i+=4) store(B+i,0xA5A5A5A5u);
  store(B+304,0x12345678u);
  store(B+180,(k?5u:0u)|0xA5A50000u); store(B+188,(k?7u:0u)|0xA5A50000u);  // u16 count +4 = A5A5, capacity +6
  store(MARK,0); store(MARK+4,0);
  Context c=fresh(); c.r3.u32=B; CLEAR(c,nullptr);
  printf("case %d base %u free %u",k,load(MARK),load(MARK+4));
  for(int i=176;i<=340;i+=4) printf(" w%d=%08x",i,load(B+i)); printf("\n"); }
 printf("end\n");
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, help='write the output here')
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-76382', 'F-27233', 'F-58498']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    stubs = {name('F-27233'): 'store(0x40B00000u,1);', name('F-58498'): 'store(0x40B00004u,1);'}
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-76382')], MAIN.replace('CLEAR', name('F-76382')), a.out, 'brain_clear', image=img,
                      stubs=stubs, limit=50)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    print(r.stdout, end='')
    if a.ref:
        a.ref.write_text(r.stdout)


if __name__ == '__main__':
    main()
