"""Run the police manager's detecting-cop pick F-76033 (PoliceManager_CanAnyPoliceDetectTarget's body, and the law update
F-52240's free-cop pick) offline (static recompilation, native, flat guest memory) for the port's `police.rs::first_detecting`
(test detect_pick_matches_the_original).

F-76033(mgr, target, need_check, need_free, group): over the manager's units (+4 array, u16 count +8) in order, skipping empty
slots: with need_check the unit's +1072 (allowed to check law breaking) must be set; with need_free the unit must be in no
tactic (F-72241 returns 0); its +872 must equal group; then the sight test F-94676(unit, target, tune +188, tune +220, 1).
The first unit passing all returns; none -> 0. Here F-72241 and F-94676 are stubs answering from the case's bit masks, so
the run shows the order and the tests, not the sight test itself (F-94676 is its own row).

Cases: 1 - 5 units, an empty slot by mask, +1072 / +872 in {0, 1, 2}, the tactic and sight masks, need_check / need_free / group
over every combination; 3,000 cases from a fixed seed. Output, one line a case:
  <n> <empty mask> <check bytes> <group bytes> <tactic mask> <sees mask> <need_check> <need_free> <group> <result index or -1>
"""
import argparse
import random
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t M=0x40800000u, A=0x40800100u, U=0x40810000u, TG=0x40A00000u;
 while(in>>tok){ if(tok!="case") break; uint32_t n,em,ck,gp,tm,sm,nc,nf,g; in>>n>>em>>ck>>gp>>tm>>sm>>nc>>nf>>g; rt_reset();
  store(M+4,A); store16(M+8,(uint16_t)n);
  for(uint32_t i=0;i<n;i++){ uint32_t u=U+0x1000u*i; store(A+4*i,((em>>i)&1)?0u:u);
   (*pointer(u+1072)=(uint8_t)((ck>>(2*i))&3)); (*pointer(u+872)=(uint8_t)((gp>>(2*i))&3)); store(u+16,i); }
  store(0x40C00000u,tm); store(0x40C00004u,sm);
  Context c=fresh(); c.r3.u32=M; c.r4.u32=TG; c.r5.u32=nc; c.r6.u32=nf; c.r7.u32=g; PICK(c,nullptr);
  int r = c.r3.u32 ? (int)load(c.r3.u32+16) : -1;
  printf("%u %u %u %u %u %u %u %u %u %d\n",n,em,ck,gp,tm,sm,nc,nf,g,r);}
 printf("end\n");
}
'''


def cases():
    g = random.Random(76033)
    out = []
    for _ in range(3000):
        n = g.randint(1, 5)
        m = (1 << n) - 1
        em = g.randint(0, m) if g.random() < 0.3 else 0
        ck = sum(g.choice([0, 1, 1, 1, 2]) << (2 * i) for i in range(n))
        gp = sum(g.choice([0, 1, 1, 1, 2]) << (2 * i) for i in range(n))
        out.append((n, em, ck, gp, g.randint(0, m), g.randint(0, m), g.randint(0, 1), g.randint(0, 1), g.choice([0, 1, 1, 2])))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-76033', 'F-72241', 'F-94676']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    idx = 'load(ctx.r%d.u32+16)'
    stubs = {
        # in a tactic: a non-zero tactic pointer (the unit is r4)
        name('F-72241'): 'ctx.r3.u64=(load(0x40C00000u)>>%s)&1;' % (idx % 4),
        # sees (the unit is r3)
        name('F-94676'): 'ctx.r3.u64=(load(0x40C00004u)>>%s)&1;' % (idx % 3),
    }
    exe = rdisp.build(rdisp.index(a.recomp), [name('F-76033')], MAIN.replace('PICK', name('F-76033')), a.out, 'detect_pick', image=img,
                      stubs=stubs, limit=50)
    cf = a.out / 'detect_cases.txt'
    cf.write_text(''.join('case ' + ' '.join(map(str, c)) + '\n' for c in cases()) + 'stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    a.ref.write_text(r.stdout[:-len('end\n')])
    print('cases', r.stdout.count('\n') - 1, '->', a.ref)


if __name__ == '__main__':
    main()
