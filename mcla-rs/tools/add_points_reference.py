"""Run the police's AddPoints (F-19359) whole offline and record it for the port's test
(crates/game/src/testdata/add_points_ref.txt; test police::tests::add_points_matches_the_original).

F-19359 (r3 the manager, r4 the target's index, r5 the points, r6 the reason): the target = F-45642 (the race camera object's racer
list, the image's global pointer: +264 the head, +864 the next; the player-present check F-37636 through the camera's +132 / +8 slot),
its tactic = F-59036's lookup (the manager's tactic pointers +44, count u16 +48: the first active +20 whose target +32 is it).
A tactic whose may-escalate byte +28 is 0 drops the points. The cap: tune +0 iMaxLawBreakingScore; a type (vt+56) 1 tactic
(Pull Over): tune +12 iPursuitLawBreakScore - 1 unless +92 == 2 (the bullhorn sounded) and vt+24; types 2 / 3: iMax - 1 unless vt+24.
score (manager +104 + index x 68) = the sum clamped to 0 .. cap. The log call F-47121 is a stub.

The tactics are the five classes on their own vtables (slots 6 vt+24, 9 vt+36, 10 vt+40, 14 vt+56 from the image): Pull Over's
F-98647 (+112 bullhorn, base F-38605, state 3 Ticket), Pursuit's vt+24 (state != 0, the radio's busy byte, base; arrest state 4,
search state 6), Double Pursuit's F-73000 (radio, base; arrest 1, search 2), Dispatch and Arrow Trap (types 0 / 4: no cap).

Cases: 143 tactic set-ups (class x +28 x +92 x +108 x +112 x FSM state x radio; a tactic on another racer; no tactics; the index
past the list) x 15 (score, points) pairs x 2 tunes (1000 / 600, 900 / 400).
Line: L <class> <+28> <+92> <+108 bits> <+112 bits> <state> <radio> <max> <pursuit> <score> <points> <new score>
"""
import argparse
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

IMG = 0x82 << 24
TUNE = IMG + 0x82CAC8          # the police tune block
CAM_PTR = IMG + 0x874374       # the race camera object's global pointer (F-45642)
RADIO_PTR = IMG + 0x90A0B0     # the radio object (its busy byte +269)
VT = [IMG + 0x0C7D64, IMG + 0x0C8B54, IMG + 0x0C8704, IMG + 0x0C8514, IMG + 0x0C81BC]   # Dispatch, PullOver, Pursuit, Dbl, ArrowTrap


def fb(f):
    return struct.unpack('>I', struct.pack('>f', f))[0]


MAIN = r'''
static void clr(uint32_t a,uint32_t e){for(;a<e;a+=4)store(a,0);}
static void sb(uint32_t a,uint32_t b){uint32_t w=load(a&~3u);int sh=(3-(a&3))*8;w=(w&~(0xffu<<sh))|((b&0xffu)<<sh);store(a&~3u,w);}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 static const uint32_t VT[5]={VTS};
 static const uint32_t CF[][7]={CFGS};
 static const int32_t PR[][2]={PAIRS};
 static const int32_t TU[][2]={{1000,600},{900,400}};
 for(auto&t:TU)for(auto&c:CF)for(auto&p:PR){rt_reset();
  const uint32_t M=0x40800000u,L=0x40801000u,T=0x40802000u,F=0x40803000u,C=0x40804000u,R0=0x40805000u,R1=0x40806000u,RD=0x40807000u;
  clr(M,0x40808000u);
  store(TUNE+0,(uint32_t)t[0]);store(TUNE+12,(uint32_t)t[1]);
  store(CAM_PTR,C);store(C+132,1);store(C+8,1);store(C+264,R0);store(R0+864,R1);store(R1+864,0);
  store(RADIO_PTR,RD);sb(RD+269,c[6]);
  uint32_t cls=c[0];
  if(cls<=5){store(M+44,L);store(L,T);uint32_t w=load(M+48);store(M+48,(w&0xffffu)|(1u<<16));
   store(T,VT[cls==5?2:cls]);sb(T+20,1);sb(T+28,c[1]);store(T+32,cls==5?R0:R1);store(T+92,c[2]);store(T+108,c[3]);store(T+112,c[4]);
   store(T+160,F);store(F+12,c[5]);}
  uint32_t idx=cls==7?2u:1u;
  store(M+104+idx*68,(uint32_t)p[0]);
  Context x=fresh();x.r3.u32=M;x.r4.u32=idx;x.r5.u32=(uint32_t)p[1];x.r6.u32=3;ADDPOINTS(x,nullptr);
  printf("L %u %u %u %08x %08x %u %u %d %d %d %d %d\n",cls,c[1],c[2],c[3],c[4],c[5],c[6],t[0],t[1],p[0],p[1],(int32_t)load(M+104+idx*68));}
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
    ids = ['F-19359', 'F-98647', 'F-73000', 'F-47121']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    img = rdisp.load_image(a.image)
    # (each class's own table: vt+24 of Pull Over / Double Pursuit is the verified F-98647 / F-73000; vt+56 the type 0 .. 4)
    assert rdisp.image_u32(img, VT[1] + 24) == addr['F-98647'] and rdisp.image_u32(img, VT[3] + 24) == addr['F-73000']
    ty = []
    for v in VT:
        f = rdisp.image_u32(img, v + 56)
        ty.append(f)
    nan = 0x7FC00000
    cf = [(0, m, 0, 0, 0, 0, 0) for m in (0, 1)] + [(4, m, 0, 0, 0, 0, 0) for m in (0, 1)]
    cf += [(1, m, b, ar, bh, st, 0) for m in (0, 1) for b in (0, 2) for ar in (fb(0.0), fb(1.0)) for bh in (fb(0.0), fb(1.0), nan)
           for st in (0, 3)]
    cf += [(2, m, 0, ar, 0, st, rd) for m in (0, 1) for st in range(7) for rd in (0, 1) for ar in (fb(0.0), fb(1.0))]
    cf += [(3, m, 0, ar, 0, st, rd) for m in (0, 1) for st in range(4) for rd in (0, 1) for ar in (fb(0.0), fb(1.0))]
    cf += [(5, 1, 0, 0, 0, 1, 0), (6, 1, 0, 0, 0, 0, 0), (7, 1, 0, 0, 0, 0, 0)]
    pr = [(0, -10), (0, 250), (5, -10), (300, 130), (590, 10), (598, 1), (599, 1), (599, 0), (650, 10), (700, -1000), (990, 10),
          (998, 1), (999, 1), (1000, 0), (1000, -10)]
    assert len(cf) == 143 and len(cf) * len(pr) * 2 < 5000
    src = (MAIN.replace('VTS', ','.join('0x%08Xu' % v for v in VT))
           .replace('CFGS', ','.join('{%uu,%uu,%uu,0x%08xu,0x%08xu,%uu,%uu}' % c for c in cf))
           .replace('PAIRS', ','.join('{%d,%d}' % p for p in pr)).replace('ADDPOINTS', 'sub_%08X' % addr['F-19359'])
           .replace('TUNE', '0x%08Xu' % TUNE).replace('CAM_PTR', '0x%08Xu' % CAM_PTR).replace('RADIO_PTR', '0x%08Xu' % RADIO_PTR))
    exe = rdisp.build(rdisp.index(a.recomp), ['sub_%08X' % addr['F-19359']], src, a.out, 'add_points', image=img,
                      vtables=[(v, [6, 9, 10, 14]) for v in VT], stubs={'sub_%08X' % addr['F-47121']: ''}, limit=40)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True, timeout=300)
    if r.returncode:
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    lines = r.stdout.splitlines()
    assert len(lines) == len(cf) * len(pr) * 2, len(lines)
    a.ref.write_text('\n'.join(lines) + '\n')
    print(len(lines), 'cases ->', a.ref, 'types', [hex(t) for t in ty])


if __name__ == '__main__':
    main()
