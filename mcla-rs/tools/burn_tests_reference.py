"""Run the two burn-out tests of the police offline (static recompilation, native, flat guest memory) for the port's
`police.rs::burn_charged` and `police.rs::burn_state_on` (test burn_tests_match_the_original).

F-25339(_, racer): car = racer +48, chassis = car +8 (either 0 -> 0); the burnout charge, f32 at chassis +580, fcmpu-greater
than 0.0 -> 1, else 0 (NaN and -0 -> 0).
F-48805 (the Racer_IsBurningOut native, r3 = the call record: +8 -> the arguments, arg 0 = the racer; +0 -> the result word):
car = racer +48, chassis = car +8 (the racer or the car 0 -> result 0); the burnout state, the byte at chassis +578 sign-extended,
greater than 0 -> 1, else 0.

Cases: the charge - the special floats (0, -0, the smallest subnormals, the smallest normal, 1, -1, the infinities, NaNs) and
2,000 random words; the null car / chassis; the state - every byte 0..255; the null racer / car (F-48805 does not test the chassis: a car without one reads address 578).
Output, one line a case:  charge <bits> <null> | <result>   /   state <byte> <null> | <result>
(<null>: 0 = all there, 1 = no car, 2 = no chassis, 3 = no racer (state only); words in decimal, 8-digit numbers padded to 10)
"""
import argparse
import random
import re
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t R=0x40800000u, CAR=0x40801000u, CH=0x40802000u, CALL=0x40803000u, ARGS=0x40803100u, RES=0x40803200u;
 while(in>>tok){ if(tok=="stop") break; uint32_t v, nl; in>>v>>nl; rt_reset();
  store(R+48, nl==1?0u:CAR); store(CAR+8, nl==2?0u:CH);
  if(tok=="charge"){ store(CH+580,v); Context c=fresh(); c.r3.u32=0; c.r4.u32=R; CHARGE(c,nullptr); printf("%u\n",(unsigned)(c.r3.u32&0xff)); }
  else { *pointer(CH+578)=(uint8_t)v; store(CALL,RES); store(CALL+8,ARGS); store(ARGS,nl==3?0u:R); store(RES,0x77777777u);
   Context c=fresh(); c.r3.u32=CALL; STATE(c,nullptr); printf("%u\n",load(RES)); } }
 printf("end\n");
}
'''


def cases():
    g = random.Random(25339)
    sp = [0x00000000, 0x80000000, 0x00000001, 0x80000001, 0x007FFFFF, 0x807FFFFF, 0x00800000, 0x80800000, 0x3F800000, 0xBF800000,
          0x7F800000, 0xFF800000, 0x7FC00000, 0xFFC00000, 0x7F800001, 0xFF800001, 0x7F7FFFFF, 0xFF7FFFFF]
    out = [('charge', w, 0) for w in sp] + [('charge', g.getrandbits(32), 0) for _ in range(2000)]
    out += [('charge', 0x3F800000, 1), ('charge', 0x3F800000, 2)]
    out += [('state', b, 0) for b in range(256)] + [('state', 1, n) for n in (1, 3)]
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-25339', 'F-48805']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-25339'), name('F-48805')],
                      MAIN.replace('CHARGE', name('F-25339')).replace('STATE', name('F-48805')), a.out, 'burn_tests', image=img, limit=10)
    cs = cases()
    cf = a.out / 'burn_cases.txt'
    cf.write_text(''.join('%s %d %d\n' % c for c in cs) + 'stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    res = r.stdout.splitlines()[:-1]
    # (8-digit numbers padded to 10 digits, so that no word reads as an address of the original to the citation check)
    pad = lambda t: re.sub(r'\b(\d{8})\b', lambda m: m.group(1).zfill(10), t)
    a.ref.write_text(''.join(pad('%s %d %d | %s' % (*c, x)) + '\n' for c, x in zip(cs, res)))
    print('cases', len(res), 'true', res.count('1'), '->', a.ref)


if __name__ == '__main__':
    main()
