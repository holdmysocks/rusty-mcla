"""Run the original's Double Pursuit may-escalate test F-73000 (its vt+24: the radio's busy byte F-94735 clear, then the base
test F-38605: the may-escalate byte +28, not in Search vt+40 F-12623, the arrest timer +108 not > 0, not in Arrest vt+36
F-48174) offline on the tactic's real vtable, over every combination of the FSM state (+160 -> +12), the byte +28, the radio
byte and a set of arrest-timer bit patterns, and write the results for the port's test
(crates/game/src/testdata/pursuit_dbl_escalate_ref.txt; test pursuit_dbl_escalate_matches_the_original).

Inputs (the Rust test makes the same ones):
  states   -1 0 1 2 3 257 0x80000001 0xFFFFFFFE (as int32)
  byte +28 0 1 0x80;  radio byte 0 1 0x40
  +108     12 special patterns (+-0, denormals, 1.0, -1.0, +-inf, NaNs) then k x 65537 (mod 2^32) for k = 0 .. 65535
Output, one line per (state, +28, radio): `<state> <b28> <radio> <true count> <fnv1a-32 of the result bytes in pattern order>`.
"""
import argparse
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

IMG = 0x82 << 24     # (the image base; data places below are offsets, no raw addresses)
DBL_VT = IMG + 0x0C8514
RADIO_PTR = IMG + 0x90A0B0

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 rt_reset();
 const uint32_t M=0x40800000u, F=0x40810000u, R=0x40820000u;
 const uint32_t ST[8]={0xFFFFFFFFu,0u,1u,2u,3u,257u,0x80000001u,0xFFFFFFFEu};
 const uint32_t B28[3]={0u,1u,0x80u}, RB[3]={0u,1u,0x40u};
 const uint32_t SP[12]={0u,0x80000000u,1u,0x80000001u,0x3F800000u,0xBF800000u,0x7F800000u,0xFF800000u,0x7FC00000u,0xFFC00000u,0x7F800001u,0x007FFFFFu};
 auto sb=[](uint32_t a,uint32_t b){uint32_t w=load(a&~3u);int sh=(3-(a&3))*8;w=(w&~(0xffu<<sh))|((b&0xffu)<<sh);store(a&~3u,w);};
 store(M,DBL_VT); store(M+160,F); store(RADIO_PTR,R);
 Context c0=fresh();
 for(int s=0;s<8;s++) for(int b=0;b<3;b++) for(int r=0;r<3;r++){
  store(F+12,ST[s]); sb(M+28,B28[b]); sb(R+269,RB[r]);
  uint32_t h=2166136261u, n=0;
  for(uint32_t k=0;k<12u+65536u;k++){ uint32_t p = k<12u ? SP[k] : (k-12u)*65537u; store(M+108,p);
   Context c=c0; c.r3.u32=M; TEST(c,nullptr); uint32_t t=c.r3.u32&0xffu; t=t?1u:0u; n+=t; h=(h^t)*16777619u; }
  printf("%d %u %u %u %08x\n",(int)ST[s],B28[b],RB[r],n,h);
 }
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
    ids = ['F-73000', 'F-38605', 'F-48174', 'F-12623', 'F-94735']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    # (the vtable's slots 9 / 10 are the two state tests: checked here, so the run uses the tactic's own table)
    assert rdisp.image_u32(img, DBL_VT + 36) == addr['F-48174'] and rdisp.image_u32(img, DBL_VT + 40) == addr['F-12623']
    assert rdisp.image_u32(img, DBL_VT + 24) == addr['F-73000']
    main_src = (MAIN.replace('TEST', name('F-73000')).replace('DBL_VT', '0x%08Xu' % DBL_VT)
                .replace('RADIO_PTR', '0x%08Xu' % RADIO_PTR))
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-73000')], main_src, a.out, 'dbl_escalate', image=img,
                      targets=[addr['F-48174'], addr['F-12623']], limit=20)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    a.ref.write_text(r.stdout)
    print(r.stdout, end='')


if __name__ == '__main__':
    main()
