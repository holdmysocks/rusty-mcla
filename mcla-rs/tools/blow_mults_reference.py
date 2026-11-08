"""Run mcCarDamage's blow multipliers F-19797 and its flag setters F-65434 (OR) / F-43282 (clear) offline (static
recompilation, native, flat guest memory) and print what they leave, for the port's `physics::car::blow_mults` to be
compared with (test blow_mults_match_the_original).

F-19797 (asm, docs/reference/hookmen/triage_small_h0.md): from the flag byte +88, +12 (against anything not a vehicle)
and +16 (against a vehicle) = 1.0, then bit 0x4 stores 0.0, bit 0x8 and bit 0x2 multiply by 0.0, bit 0x1 by 4.8 / 10,
bit 0x10 by 16, bit 0x20 by +24 (the difficulty factor). The setters OR / clear the argument's low byte into +88 and
tail-call F-19797.

Cases: every flag byte 0 .. 255 with +24 = each of 1.0, 0.5, 3.0, -2.0, +inf, NaN; then 256 setter pairs (start byte k,
OR (k x 37) & 255, clear (k x 91) & 255) with +24 = 1.0.
Output, one line a case:
  m <flags> <+24 bits> <+12 bits> <+16 bits>
  s <k> <byte after or> <+12> <+16> <byte after clear> <+12> <+16>
"""
import argparse
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 const uint32_t B=0x40A00000u; const uint32_t D[6]={0x3f800000u,0x3f000000u,0x40400000u,0xc0000000u,0x7f800000u,0x7fc00000u};
 rt_reset();
 for(int d=0;d<6;d++) for(int f=0;f<256;f++){
  for(int i=0;i<96;i+=4) store(B+i,0xA5A5A5A5u);
  store(B+24,D[d]); store(B+88,(uint32_t)f<<24|0xA5A5A5u);
  Context c=fresh(); c.r3.u32=B; MULTS(c,nullptr);
  printf("m %d %08x %08x %08x\n",f,D[d],load(B+12),load(B+16)); }
 for(int k=0;k<256;k++){
  for(int i=0;i<96;i+=4) store(B+i,0xA5A5A5A5u);
  store(B+24,0x3f800000u); store(B+88,(uint32_t)k<<24|0xA5A5A5u);
  Context c=fresh(); c.r3.u32=B; c.r4.u32=0xFFFFFF00u|((k*37)&255); SETF(c,nullptr);
  printf("s %d %u %08x %08x",k,load(B+88)>>24,load(B+12),load(B+16));
  Context e=fresh(); e.r3.u32=B; e.r4.u32=0xFFFFFF00u|((k*91)&255); CLRF(e,nullptr);
  printf(" %u %08x %08x\n",load(B+88)>>24,load(B+12),load(B+16)); }
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
    ids = ['F-19797', 'F-65434', 'F-43282']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    main_src = MAIN.replace('MULTS', name('F-19797')).replace('SETF', name('F-65434')).replace('CLRF', name('F-43282'))
    exe = rdisp.build(rdisp.index(a.recomp), [name(i) for i in ids], main_src, a.out, 'blow_mults', image=img, limit=10)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    if a.ref:
        a.ref.write_text(r.stdout)
    print(r.stdout[:600], end='')


if __name__ == '__main__':
    main()
