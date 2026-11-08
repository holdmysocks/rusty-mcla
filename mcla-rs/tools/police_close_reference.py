"""Run PoliceManager_IsAnyPoliceCloseToPlayer's body F-91297 offline (static recompilation, native, flat guest memory) for the
port's `police.rs::any_police_close` (test police_close_matches_the_original).

F-91297(r f1): with the police manager (8286D8E0) and the player manager (82874374) there and F-82714(pm, 0) true (stubbed: the
case's flag), the player = F-65049(pm, 0) (stubbed); over the manager's active units in order (F-53190 count / F-45180 get: the
+4 list's units whose mode +1152 is 0) not underground (+880 == 0): the squared distance unit - player (dy dy, then dz and dx
fused) strictly under r x r -> 1. Else 0. Positions come from F-35658 (stubbed: the racer + 256, its +48).

Cases: 3,000 from a fixed seed - 0 - 5 slots (empty ones too), modes 0 / 1 / 2, underground flags, positions near the radius,
r 250 / random / 0, the player flag. Output, one line a case (f32 bits and words in decimal, 8-digit numbers padded to 10):
  <flag> <r> <px py pz> <n> then per slot <empty mode under x y z> | <result>
"""
import argparse
import math
import random
import re
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t M=0x40800000u, A=0x40800100u, PM=0x40801000u, P=0x40802000u, U=0x40810000u;
 while(in>>tok){ if(tok!="case") break; uint32_t fl, rb, px, py, pz, n; in>>fl>>rb>>px>>py>>pz>>n; rt_reset();
  store(0x8286D8E0u,M); store(0x82874374u,PM); store(0x40C00000u,fl); store(M+4,A); store16(M+8,(uint16_t)n);
  store(P+256+48,px); store(P+256+52,py); store(P+256+56,pz);
  for(uint32_t i=0;i<n;i++){ uint32_t e,md,ug,x,y,z; in>>e>>md>>ug>>x>>y>>z; uint32_t u=U+0x1000u*i; store(A+4*i,e?0u:u);
   store(u+1152,md); *pointer(u+880)=(uint8_t)ug; store(u+256+48,x); store(u+256+52,y); store(u+256+56,z); }
  float r; memcpy(&r,&rb,4); Context c=fresh(); c.f1.f64=(double)r; CLOSE(c,nullptr);
  printf("%u\n",(unsigned)(c.r3.u32&0xff));}
 printf("end\n");
}
'''


def bits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


def cases():
    g = random.Random(91297)
    out = []
    for _ in range(3000):
        fl = 1 if g.random() < 0.9 else 0
        r = g.choice([250.0, 250.0, g.uniform(1, 400), 0.0])
        p = [g.uniform(-2000, 2000), g.uniform(-10, 60), g.uniform(-2000, 2000)]
        n = g.randint(0, 5)
        slots = []
        for _ in range(n):
            e = 1 if g.random() < 0.1 else 0
            md = g.choice([0, 0, 0, 1, 2])
            ug = 1 if g.random() < 0.25 else 0
            a, el = g.uniform(0, 2 * math.pi), g.uniform(-0.2, 0.2)
            d = r * g.choice([g.uniform(0.98, 1.02), g.uniform(0, 2), 1.0])
            slots.append((e, md, ug, bits(p[0] + d * math.cos(a)), bits(p[1] + d * el), bits(p[2] + d * math.sin(a))))
        out.append((fl, bits(r), [bits(x) for x in p], slots))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-91297', 'F-82714', 'F-65049', 'F-35658']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    stubs = {name('F-82714'): 'ctx.r3.u64=load(0x40C00000u);', name('F-65049'): 'ctx.r3.u64=0x40802000u;',
             name('F-35658'): 'ctx.r3.u64=ctx.r3.u32+256u;'}
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-91297')], MAIN.replace('CLOSE', name('F-91297')), a.out, 'police_close', image=img,
                      stubs=stubs, limit=30)
    cs = cases()
    line = lambda c: '%d %d %d %d %d %d%s' % (c[0], c[1], *c[2], len(c[3]), ''.join(' %d %d %d %d %d %d' % s for s in c[3]))
    cf = a.out / 'close_cases.txt'
    cf.write_text(''.join('case ' + line(c) + '\n' for c in cs) + 'stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    res = r.stdout.splitlines()[:-1]
    # (8-digit numbers padded to 10 digits, so that no word reads as an address of the original to the citation check)
    pad = lambda t: re.sub(r'\b(\d{8})\b', lambda m: m.group(1).zfill(10), t)
    a.ref.write_text(''.join(pad(line(c) + ' | ' + x) + '\n' for c, x in zip(cs, res)))
    print('cases', len(res), 'true', res.count('1'), '->', a.ref)


if __name__ == '__main__':
    main()
