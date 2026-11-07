"""Run the cop brain's small state pieces offline and record them for the port's tests
(crates/game/src/testdata/brain_states_ref.txt; tests police::tests::go_to_arrived_matches_the_original,
brain_pieces_match_the_original).

F-16378 (r3 GoToPosition, the brain's state 8: +8 its time, +12 -> the machine, machine +20 -> the brain): the brain's mode word
+576 = 1 when the brain's near-goal time +580 > the image's 5.0, or the state time > 0.0 and HasReachedGoal F-43234 (brain +128
== 0, the aiv +300, F-65527: aiv +30784 >= +26480 + +26472); else +576 is left. Cases: +580 0, 5, 5 + 1 ulp, 9, NaN x time 0,
-0, 1 ulp, 1, NaN x route: +128 set / no aiv / not done / done x +576 before 0, 1, 7.
F-98680 (r3 a cop): its brain +876 -> +576, else 0. Cases: no brain; brain with +576 0, 1, 2, -1.
F-55899 (r4 the out vector): a splat of the image's 0.0. Two out patterns.
F-97098 (r3 the Search state, 352 bytes): +344 = -1, +336 = 0, byte +340 = 0, and the ten 32-byte records at +16 zeroed. Two fill
patterns; every word +0 .. +351 printed after.
F-55595 (r3 a brain): +48 (its car) and +560 (its machine) both set. Four cases.
F-64596 (r3 a cop, r4 a brain; the cop's old +68 is 0 here - the release through F-72605 is a stub that throws if reached): +68 = the brain; +876 =
the brain when its vt+0 type test (with the police brain's class id, image) is true, else 0. Brains: none, mcBrainPolice,
mcBrainRacing (their own vtables, RTTI).

Lines: K <+580 bits> <time bits> <route 0..3> <+576 before> <+576 after>  /  R <0 none / 1 brain> <+576> <result>  /
H <pattern> <x> <y> <z> <w bits>  /  Q <pattern> <word index> <word> (every word)  /  C <car> <machine> <result>  /
B <0 none / 1 police / 2 racing> <+68 == the brain> <+876 == the brain>
"""
import argparse
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp


def fb(f):
    return struct.unpack('>I', struct.pack('>f', f))[0]


def nb(b, d):
    return (b + d) & 0xFFFFFFFF


MAIN = r'''
static void clr(uint32_t a,uint32_t e){for(;a<e;a+=4)store(a,0);}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 static const uint32_t K[][4]={KCASES};
 for(auto&c:K){rt_reset();const uint32_t S=0x40800000u,MC=0x40801000u,B=0x40802000u,AV=0x40810000u;clr(S,0x40803000u);clr(AV,AV+0x8000u);
  store(S+8,c[1]);store(S+12,MC);store(MC+20,B);store(B+580,c[0]);store(B+576,c[3]);
  if(c[2]==0)store(B+128,1);
  if(c[2]>=2){store(B+300,AV);store(AV+26480,3);store(AV+26472,2);store(AV+30784,c[2]==3?5u:4u);}
  Context x=fresh();x.r3.u32=S;GOTO(x,nullptr);printf("K %08x %08x %u %d %d\n",c[0],c[1],c[2],(int32_t)c[3],(int32_t)load(B+576));}
 static const uint32_t R[][2]={RCASES};
 for(auto&c:R){rt_reset();const uint32_t C=0x40800000u,B=0x40801000u;clr(C,0x40802000u);
  if(c[0]){store(C+876,B);store(B+576,c[1]);}
  Context x=fresh();x.r3.u32=C;SUBSTATE(x,nullptr);printf("R %u %d %d\n",c[0],(int32_t)c[1],(int32_t)x.r3.u32);}
 for(uint32_t pat=0;pat<2;pat++){rt_reset();const uint32_t O=0x40800000u;for(int k=0;k<4;k++)store(O+4*k,pat?0x7fc00000u:0xa5a5a5a5u+k);
  Context x=fresh();x.r4.u32=O;HEADING(x,nullptr);printf("H %u %08x %08x %08x %08x\n",pat,load(O),load(O+4),load(O+8),load(O+12));}
 for(uint32_t pat=0;pat<2;pat++){rt_reset();const uint32_t S=0x40800000u;for(uint32_t k=0;k<88;k++)store(S+4*k,pat?0xffffffffu:0x11110000u+k);
  Context x=fresh();x.r3.u32=S;SRESET(x,nullptr);for(uint32_t k=0;k<88;k++)printf("Q %u %u %08x\n",pat,k,load(S+4*k));}
 for(uint32_t car=0;car<2;car++)for(uint32_t m=0;m<2;m++){rt_reset();const uint32_t B=0x40800000u;clr(B,0x40801000u);
  if(car)store(B+48,0x40900000u);if(m)store(B+560,0x40910000u);
  Context x=fresh();x.r3.u32=B;HASMACH(x,nullptr);printf("C %u %u %u\n",car,m,x.r3.u32&0xff);}
 for(uint32_t k=0;k<3;k++){rt_reset();const uint32_t C=0x40800000u,B=0x40801000u;clr(C,0x40802000u);clr(B,0x40802000u);
  if(k)store(B,k==1?POLICE_VT:RACING_VT);store(C+68,0);store(C+876,0x12345678u);
  Context x=fresh();x.r3.u32=C;x.r4.u32=k?B:0u;SETBRAIN(x,nullptr);
  printf("B %u %u %u\n",k,load(C+68)==(k?B:0u)?1u:0u,load(C+876)==(k?B:0u)?1u:0u);}
}
'''


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-16378', 'F-98680', 'F-55899', 'F-97098', 'F-55595', 'F-64596', 'F-95387', 'F-72605']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    pol = rdisp.vtables_of(img, 'mcBrainPolice')[0][0]
    rac = rdisp.vtables_of(img, 'mcBrainRacing')[0][0]
    assert rdisp.image_u32(img, pol) == addr['F-95387']
    kc = []
    for near in (fb(0.0), fb(5.0), nb(fb(5.0), 1), fb(9.0), 0x7FC00000):
        for t in (fb(0.0), 0x80000000, 1, fb(1.0), 0x7FC00000):
            for route in range(4):
                for prev in (0, 1, 7):
                    kc.append((near, t, route, prev))
    rc = [(0, 0)] + [(1, v & 0xFFFFFFFF) for v in (0, 1, 2, -1)]
    assert len(kc) < 1000 and len(rc) < 1000
    src = (MAIN.replace('KCASES', ','.join('{0x%08xu,0x%08xu,%uu,%uu}' % c for c in kc))
           .replace('RCASES', ','.join('{%uu,0x%08xu}' % c for c in rc)).replace('GOTO', name('F-16378'))
           .replace('SUBSTATE', name('F-98680')).replace('HEADING', name('F-55899')).replace('SRESET', name('F-97098'))
           .replace('HASMACH', name('F-55595')).replace('SETBRAIN', name('F-64596'))
           .replace('POLICE_VT', '0x%08Xu' % pol).replace('RACING_VT', '0x%08Xu' % rac))
    exe = rdisp.build(rdisp.index(a.recomp), [name(f) for f in ids[:6]], src, a.out, 'brain_states', image=img,
                      vtables=[(pol, [0]), (rac, [0])], limit=40,
                      stubs={name('F-72605'): 'throw std::runtime_error("release reached");'})
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True, timeout=120)
    if r.returncode:
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    lines = r.stdout.splitlines()
    assert len(lines) == len(kc) + len(rc) + 2 + 176 + 4 + 3, len(lines)
    a.ref.write_text('\n'.join(lines) + '\n')
    print(len(lines), 'lines ->', a.ref)


if __name__ == '__main__':
    main()
