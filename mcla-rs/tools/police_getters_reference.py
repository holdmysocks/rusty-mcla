"""Run five of the police's small getters / setters offline and record them for the port's tests
(crates/game/src/testdata/police_getters_ref.txt; tests police::tests::running_arm_matches_the_original,
default_final_speed_matches_the_original, pullover_cap_open_matches_the_original, race_chase_type_matches_the_original and
patrol_hold_matches_the_original).

F-75755 / F-93041 (r3 a junction): the arm vector while green (+164 timer < the image's 6.5) / while yellow (timer >= 6.5 and
< f32(6.5 + 2.0)): &arms[+160] (16-byte entries at +44, count u16 +48; the index taken only when 0 <= it < count), else the image's
zero vector. Cases: count 0, 1, 4 x index -1, 0, count - 1, count, 5 x timer 0, -1, 6.5 and its f32 neighbours, 8.5 and its
neighbours, 20, +inf, NaN.
F-85634 (r3 a brain state: +12 -> +20 the cop unit): the default final speed: the unit's target +680, its racer +48, the racer's
car +8; when the car's speed +224 > f32(fMovingSpeed (tune +312) x 0.447) (F-43975) that speed (F-41260), else the image's 0.0.
Cases: fMovingSpeed 1, 2.5 x no target / no racer / no car / a car at 0, the product's f32 neighbours, the product, 30, NaN.
F-98647 (r3 a pull-over tactic on its real vtable, slots 9 / 10: F-33778 FSM state +160 -> +12 == 3, F-74264 0): false while the
bullhorn timer +112 > 0, else the base test F-38605 (+28, !vt+40, !(+108 > 0), !vt+36). Cases: +28 0 / 1 x +108 0, 1, -0.5,
NaN x +112 0, 1, -1, NaN x state 0 .. 3.
F-12315 (r3 a type, r4 a racer; the manager is the image's global pointer): no manager - nothing; no racer - +204 = type; else
+208 = racer and +204 = 1 when the racer's vt+40 (is the player) is true, else 2. The racer's vt+40 is a host body returning 1 / 0.
Cases: manager 0 / 1 x type 0, 1, 2, 7 x racer none / the player / an AI racer.
F-30531 (r3 a cop unit, r4 group, r5 path): +1120 = group, +1124 = path, and when group >= 0 +1128 = tune +316 fPatrolTime.
Cases: group -1, 0, 3 x path -1, 0, 9 x fPatrolTime 5, 60 (+1128 starts at 0.5).

Line per case: G|Y <count> <index> <timer bits> <x bits> <y bits> <z bits>  /  S <fMovingSpeed bits> <0 none / 1 no racer /
2 no car / 3 car> <speed bits> <result bits>  /  P <+28> <+108 bits> <+112 bits> <state> <result>  /  Z <manager> <type>
<0 none / 1 player / 2 AI> <+204> <+208 == the racer>  /  A <group> <path> <+1120> <+1124> <+1128 bits> <fPatrolTime bits>
"""
import argparse
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

IMG = 0x82 << 24
TUNE = IMG + 0x82CAC8          # the police tune block (as in police_predicates_reference.py)
PULL_VT = IMG + 0x0C8B54       # the pull-over tactic's vtable (slot 6 = F-98647)
MGR_PTR = IMG + 0x86D8E0       # the police manager's global pointer (F-12315 reads it)
FAKE_PLAYER, FAKE_AI = 0x7F000010, 0x7F000011

def fb(f):
    return struct.unpack('>I', struct.pack('>f', f))[0]

def nb(b, d):
    return (b + d) & 0xFFFFFFFF

MAIN = r'''
static void sf(uint32_t a,uint32_t u){store(a,u);}
static void clr(uint32_t a,uint32_t e){for(;a<e;a+=4)store(a,0);}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 static const uint32_t A[][3]={ACASES};
 for(int g=0;g<2;g++)for(auto&c:A){rt_reset();const uint32_t J=0x40800000u,T=0x40801000u;clr(J,0x40802000u);
  for(uint32_t k=0;k<c[0];k++){sf(T+16*k,0x3f800000u+(k<<20));sf(T+16*k+4,0x40000000u+(k<<20));sf(T+16*k+8,0xbf800000u+(k<<20));}
  store(J+44,T);{uint32_t w=load(J+48);w=(w&0x0000ffffu)|(c[0]<<16);store(J+48,w);}store(J+160,c[1]);sf(J+164,c[2]);
  Context x=fresh();x.r3.u32=J;if(g==0)GREEN(x,nullptr);else YELLOW(x,nullptr);uint32_t v=x.r3.u32;
  printf("%c %u %d %08x %08x %08x %08x\n",g?'Y':'G',c[0],(int32_t)c[1],c[2],load(v),load(v+4),load(v+8));}
 static const uint32_t S[][3]={SCASES};
 for(auto&c:S){rt_reset();const uint32_t B=0x40800000u,X=0x40801000u,U=0x40802000u,TG=0x40803000u,R=0x40804000u,C=0x40805000u;clr(B,0x40806000u);
  store(B+12,X);store(X+20,U);sf(TUNE+312,c[0]);
  if(c[1]>=1)store(U+680,TG);if(c[1]>=2)store(TG+48,R);if(c[1]>=3){store(R+8,C);sf(C+224,c[2]);}
  Context x=fresh();x.r3.u32=B;DEFSPEED(x,nullptr);float f=float(x.f1.f64);uint32_t u;memcpy(&u,&f,4);
  printf("S %08x %u %08x %08x\n",c[0],c[1],c[2],u);}
 static const uint32_t P[][4]={PCASES};
 for(auto&c:P){rt_reset();const uint32_t O=0x40800000u,F=0x40801000u;clr(O,0x40802000u);
  store(O,PULL_VT);{uint32_t w=load(O+28);w=(w&0x00ffffffu)|(c[0]<<24);store(O+28,w);}sf(O+108,c[1]);sf(O+112,c[2]);store(O+160,F);store(F+12,c[3]);
  Context x=fresh();x.r3.u32=O;CAPOPEN(x,nullptr);printf("P %u %08x %08x %u %u\n",c[0],c[1],c[2],c[3],x.r3.u32&0xff);}
 static const uint32_t Z[][3]={ZCASES};
 for(auto&c:Z){rt_reset();const uint32_t M=0x40800000u,R=0x40801000u,V=0x40802000u;clr(M,0x40803000u);
  store(MGR_PTR,c[0]?M:0u);store(M+204,0x55u);store(M+208,0u);store(R,V);store(V+40,c[2]==1?FPLAYER:FAI);
  Context x=fresh();x.r3.u32=c[1];x.r4.u32=c[2]?R:0u;CHASETGT(x,nullptr);
  printf("Z %u %u %u %u %u\n",c[0],c[1],c[2],load(M+204),load(M+208)==R?1u:0u);}
 static const uint32_t PA[][3]={ACASES2};
 for(auto&c:PA){rt_reset();const uint32_t U=0x40800000u;clr(U,0x40801000u);sf(U+1128,0x3f000000u);sf(TUNE+316,c[2]);
  Context x=fresh();x.r3.u32=U;x.r4.u32=c[0];x.r5.u32=c[1];PATROLSET(x,nullptr);
  printf("A %d %d %d %d %08x %08x\n",(int32_t)c[0],(int32_t)c[1],(int32_t)load(U+1120),(int32_t)load(U+1124),load(U+1128),c[2]);}
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
    ids = ['F-75755', 'F-93041', 'F-85634', 'F-98647', 'F-12315', 'F-30531', 'F-33778', 'F-74264']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    # (the run uses the pull-over's own table: slot 6 is the test, slots 9 / 10 the two state tests it calls)
    assert rdisp.image_u32(img, PULL_VT + 24) == addr['F-98647']
    assert rdisp.image_u32(img, PULL_VT + 36) == addr['F-33778'] and rdisp.image_u32(img, PULL_VT + 40) == addr['F-74264']
    ac = []
    for cnt in (0, 1, 4):
        for idx in (-1, 0, cnt - 1, cnt, 5):
            for t in (fb(0.0), fb(-1.0), nb(fb(6.5), -1), fb(6.5), nb(fb(6.5), 1), nb(fb(8.5), -1), fb(8.5), nb(fb(8.5), 1),
                      fb(20.0), 0x7F800000, 0x7FC00000):
                ac.append((cnt, idx & 0xFFFFFFFF, t))
    k = struct.unpack('>f', struct.pack('>f', 0.447))[0]
    sc = []
    for ms in (1.0, 2.5):
        prod = fb(struct.unpack('>f', struct.pack('>f', ms * k))[0])
        sc += [(fb(ms), 0, 0), (fb(ms), 1, 0), (fb(ms), 2, 0)]
        for s in (fb(0.0), nb(prod, -1), prod, nb(prod, 1), fb(30.0), 0x7FC00000):
            sc.append((fb(ms), 3, s))
    pc = [(m, ar, bh, st) for m in (0, 1) for ar in (fb(0.0), fb(1.0), fb(-0.5), 0x7FC00000)
          for bh in (fb(0.0), fb(1.0), fb(-1.0), 0x7FC00000) for st in range(4)]
    zc = [(mg, ty, rc) for mg in (0, 1) for ty in (0, 1, 2, 7) for rc in (0, 1, 2)]
    pa = [(g & 0xFFFFFFFF, p & 0xFFFFFFFF, fb(pt)) for g in (-1, 0, 3) for p in (-1, 0, 9) for pt in (5.0, 60.0)]
    assert all(len(l) < 1000 for l in (ac, sc, pc, zc, pa))
    c3 = lambda l: ','.join('{0x%08xu,0x%08xu,0x%08xu}' % c for c in l)
    c4 = lambda l: ','.join('{0x%08xu,0x%08xu,0x%08xu,0x%08xu}' % c for c in l)
    src = (MAIN.replace('ACASES2', c3(pa)).replace('ACASES', c3(ac)).replace('SCASES', c3(sc)).replace('PCASES', c4(pc))
           .replace('ZCASES', c3(zc)).replace('GREEN', name('F-75755')).replace('YELLOW', name('F-93041'))
           .replace('DEFSPEED', name('F-85634')).replace('CAPOPEN', name('F-98647')).replace('CHASETGT', name('F-12315'))
           .replace('PATROLSET', name('F-30531')).replace('PULL_VT', '0x%08Xu' % PULL_VT).replace('MGR_PTR', '0x%08Xu' % MGR_PTR)
           .replace('FPLAYER', '0x%08Xu' % FAKE_PLAYER).replace('FAI', '0x%08Xu' % FAKE_AI).replace('TUNE', '0x%08Xu' % TUNE))
    exe = rdisp.build(rdisp.index(a.recomp), [name(f) for f in ids[:6]], src, a.out, 'police_getters', image=img,
                      vtables=[(PULL_VT, [9, 10])], fakes={FAKE_PLAYER: "ctx.r3.u64=1;", FAKE_AI: "ctx.r3.u64=0;"}, limit=20)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True, timeout=120)
    if r.returncode:
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    lines = r.stdout.splitlines()
    assert len(lines) == 2 * len(ac) + len(sc) + len(pc) + len(zc) + len(pa), len(lines)
    a.ref.write_text('\n'.join(lines) + '\n')
    print(len(lines), 'cases ->', a.ref)

if __name__ == '__main__':
    main()
