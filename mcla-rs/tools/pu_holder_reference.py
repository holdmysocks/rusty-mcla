"""The power-up holder's two decisions run offline: may the car take type r4 (F-37974) and may it fire (F-22431) - unchanged generated code;
the racer / holder / type-table objects are built in guest memory with the offsets the assembly shows, the vehicle's virtual 'may fire' (slot 12)
is a host stub returning the case's flag.
Holder r3: +4 racer, +136 slot 0, +140 slot 1, +228 shot, +236 effect. Racer: +8 vehicle (+664 nitrous object, its s16 at +18 is the shots), +20 -> time
object (+36 float, compared with the float at (+8 of it) +24: finished when >=). Type table: global -> object, +320 -> array of type objects, +8 mega.
Lines:  ACC <finished> <nitrous 0/1> <shots> <kind> <slot0> <slot1> <mega of kind> -> <result>
        FIRE <slot0> <shot> <effect> <vehicle flag> -> <result>
Usage: pu_holder_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_dispatch as rd

ACCEPT, CAN_FIRE = 'F-37974', 'F-22431'

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);std::string t;
 const uint32_t G=uint32_t(-2104426496)+uint32_t(-21528);
 const uint32_t H=0x40010000,RACER=0x40010200,VEH=0x40010400,NIT=0x40010600,TIMEA=0x40010800,TIMEB=0x40010900,TT=0x40010A00,ARR=0x40010C00,ENT=0x40011000,VT=0x40012000;
 while(in>>t){rt_reset();
  store(H+4,RACER);store(RACER+8,VEH);store(RACER+20,TIMEA);store(TIMEA+8,TIMEB);store(VEH,VT);store(VT+48,0x7F000010u);
  if(t=="ACC"){int fin,nit,shots,kind,s0,s1,mega;in>>fin>>nit>>shots>>kind>>s0>>s1>>mega;
   store(TIMEA+36,fin?0x3f800000u:0x3f000000u);store(TIMEB+24,0x3f800000u);   // time 1.0 or 0.5 against 1.0: finished when >=
   store(VEH+664,nit?NIT:0);store16(NIT+18,uint16_t(shots));
   store(H+136,s0);store(H+140,s1);store(G,TT);store(TT+320,ARR);for(int i=0;i<26;i++){store(ARR+4*i,ENT+16*i);store(ENT+16*i+8,i==kind?mega:25);}
   Context c=fresh();c.r3.u32=H;c.r4.u32=kind;sub_ACCEPT(c,nullptr);printf("%u\n",c.r3.u32&0xff);}
  else{int s0,shot,eff,vf;in>>s0>>shot>>eff>>vf;store(H+136,s0);store(H+228,shot?0x40020000u:0);store(H+236,eff?0x40020000u:0);store(0x40013000u,vf);
   Context c=fresh();c.r3.u32=H;sub_FIRE(c,nullptr);printf("%u\n",c.r3.u32&0xff);}
 }
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
    addr = lambda f: 'sub_' + subprocess.run([a.tool, 'registry', 'addr', f], capture_output=True, text=True, check=True).stdout.split()[1]
    acc, fire = addr(ACCEPT), addr(CAN_FIRE)
    src = 'static int rt_vf=0;\n' + MAIN.replace('sub_ACCEPT', acc).replace('sub_FIRE', fire)
    exe = rd.build(idx, [acc, fire], src, a.work, 'pu_holder', image=rd.load_image(a.image), fakes={0x7F000010: 'ctx.r3.u64=load(0x40013000u);'})
    rnd = random.Random(2029)
    kinds = list(range(24)) + [25]
    lines = []
    for fin in (0, 1):
        for nit, shots in [(0, 0), (1, 0), (1, 4), (1, 5), (1, 7)]:
            for kind in (0, 3, 25):
                for s0 in (0, 25, 3):
                    for s1 in (25, 5):
                        lines.append(f'ACC {fin} {nit} {shots} {kind} {s0} {s1} {rnd.choice([12, 25])}')
    for _ in range(4000):
        lines.append(f'ACC {rnd.randint(0, 1)} {rnd.randint(0, 1)} {rnd.randint(0, 8)} {rnd.choice(kinds)} {rnd.choice(kinds)} {rnd.choice(kinds)} {rnd.choice([12, 13, 14, 15, 16, 17, 18, 19, 20, 25])}')
    for s0 in kinds:
        for shot in (0, 1):
            for eff in (0, 1):
                for vf in (0, 1):
                    lines.append(f'FIRE {s0} {shot} {eff} {vf}')
    cases = a.work / 'pu_holder_cases.txt'
    cases.write_text('\n'.join(lines) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    out = p.stdout.split()
    assert len(out) == len(lines)
    a.out.write_text('\n'.join(f'{cl} -> {o}' for cl, o in zip(lines, out)) + '\n')
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
