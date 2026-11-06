"""The Shield / MegaShield hit count run offline: the original's own code for a shot landing on a shielded car (F-52298), the
effect start on the own car (F-63170), unchanged generated code. The two type objects get what their constructors (Shield F-78725,
MegaShield F-20422) give them that this path reads: the vtable each stores (read from its code) and the type id +4 (5, 16); the
hit count +104 is written by neither constructor (the allocator's memory; 0 here, as the port starts it).

Two holders share the type table the game keeps (global -> object, +320 -> array of type objects), as in the game: the Shield and
the MegaShield are ONE object each, so their hit count (+104) is shared by every car that has the effect. A case is a list of
operations; after each one the tool prints holder 0 and holder 1 "has an effect" (+236 != 0) and the hit counts of the Shield and
the MegaShield objects.
  A<h><k>  the effect k (5 Shield, 16 MegaShield) starts on holder h (F-63170)
  L<h>     a shot (type 2) lands on holder h (F-52298; r5 a shooter whose +28 is a matrix, the thread block's byte +72 = 0) -
           only while holder h has an effect (a shield); otherwise the op prints "-" and is skipped
The landing's Mirror branch (the landing hook and the send-to-all) is never reached with shields only: both are stubs that throw.
Host stubs (fakes): the racer's notification object (racer +16; every slot a host no-op) and the racer's +20 is null (the
start / end's flag call on it is skipped by the code itself).
Usage: pu_shield_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_dispatch as rd

LAND, APPLY, SHIELD, MEGA = 'F-52298', 'F-63170', 'F-78725', 'F-20422'
MIRROR_HOOK, MIRROR_ALL = 'F-82026', 'F-33277'
MIRROR_STUB = 'throw std::runtime_error("mirror branch reached");'
NOTE_VT, NOTE_FN =0x7F001000, 0x7F001100   # host vtable of the racer's notification object, every slot the host no-op

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string line;
 const uint32_t G=uint32_t(-2104426496)+uint32_t(-21528);
 const uint32_t H0=0x40800000,H1=0x40800400,R0=0x40800800,R1=0x40800A00,NOTE=0x40800C00,SH=0x40801000,MS=0x40801800,
                TT=0x40802000,ARR=0x40802400,SHOOT=0x40802800,MAT=0x40802A00,EMPTY=0x40803000,NVT=0x40803800;
 while(std::getline(in,line)){ if(line.empty())continue; rt_reset(); std::memset(arena+0x800000,0,0x4000);   // rt_reset clears only the heap used
  for(int i=0;i<64;i++)store(NVT+4*i,NOTE_FN);
  store(NOTE,NVT);
  store(SH,VT_SHIELD);store(SH+4,5);store(MS,VT_MEGA);store(MS+4,16);
  store(G,TT);store(TT+320,ARR);for(int i=0;i<26;i++)store(ARR+4*i,EMPTY);store(ARR+4*5,SH);store(ARR+4*16,MS);
  uint32_t H[2]={H0,H1},R[2]={R0,R1};
  for(int h=0;h<2;h++){store(H[h]+4,R[h]);store(R[h]+16,NOTE);store(R[h]+20,0);store(H[h]+236,0);}
  store(SHOOT+28,MAT);
  std::istringstream ops(line);std::string op;bool first=true;
  while(ops>>op){int h=op[1]-'0';
   if(op[0]=='A'){int k=std::stoi(op.substr(2));Context c=fresh();c.r3.u32=H[h];c.r4.u32=k;sub_APPLY(c,nullptr);}
   else if(load(H[h]+236)==0){printf("%s-",first?"":" ; ");first=false;continue;}
   else{Context c=fresh();c.r3.u32=H[h];c.r4.u32=2;c.r5.u32=SHOOT;c.r6.u32=0;sub_LAND(c,nullptr);}
   printf("%s%d %d %u %u",first?"":" ; ",load(H0+236)!=0,load(H1+236)!=0,load(SH+104),load(MS+104));first=false;}
  printf("\n");}
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    addr = lambda f: 'sub_' + subprocess.run([a.tool, 'registry', 'addr', f], capture_output=True, text=True, check=True).stdout.split()[1]
    land, apply, shield, mega = (addr(f) for f in (LAND, APPLY, SHIELD, MEGA))
    # the vtables the two constructors store (the first image pointer each stores, through its copy of `this`): their slots are
    # the dispatch targets the landing and the start reach (start +12, end +16, broken +64)
    vtables = [(rd.stored_pointers(idx, n)[0], (3, 4, 16)) for n in (shield, mega)]
    src = ('#include <sstream>\n' + MAIN.replace('sub_LAND', land).replace('sub_APPLY', apply).replace('NOTE_FN', hex(NOTE_FN) + 'u')
           .replace('VT_SHIELD', hex(vtables[0][0]) + 'u').replace('VT_MEGA', hex(vtables[1][0]) + 'u'))
    exe = rd.build(idx, [land, apply], src, a.work, 'pu_shield', image=img, vtables=vtables,
                   fakes={NOTE_FN: ''}, stubs=dict.fromkeys((addr(MIRROR_HOOK), addr(MIRROR_ALL)), MIRROR_STUB))
    rnd = random.Random(2048)
    lines = []
    for k in (5, 16):
        lines.append(f'A0{k} ' + ' '.join(['L0'] * 6))
        lines.append(f'A0{k} L0 A1{k} L1 L0 L1 L0 L1')
        lines.append(f'A0{k} A1{k} L1 L1 L1 L1 L0 L0 L0 L0 L0')
    for _ in range(300):
        ops = []
        for _ in range(rnd.randint(2, 14)):
            if rnd.random() < 0.3:
                ops.append(f'A{rnd.randint(0, 1)}{rnd.choice([5, 16])}')
            else:
                ops.append(f'L{rnd.randint(0, 1)}')
        lines.append(' '.join(ops))
    cases = a.work / 'pu_shield_cases.txt'
    cases.write_text('\n'.join(lines) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    out = p.stdout.splitlines()
    assert len(out) == len(lines), (len(out), len(lines))
    a.out.write_text('\n'.join(f'{cl} -> {o}' for cl, o in zip(lines, out)) + '\n')
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
