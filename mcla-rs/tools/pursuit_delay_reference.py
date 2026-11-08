"""Run the original's Pursuit tactic offline with a Delay hold (+164) > 0 and record when its Delay ends, for the port's
`flee_hold` (crates/game/src/testdata/pursuit_delay_ref.txt; test pursuit_delay_matches_the_original).

The same build as tools/pursuit_fsm_reference.py (ctor F-55508, reset F-20606, the cop and target as the engage sets them, begin
F-89627, the tactic update F-68032 with dt = 1/60 s; the world stubs of that tool), with the hold set before the begin and a
still world (no flag set, bearing 90, speed 0), so nothing but the Delay asks a state. The Delay's enter F-10824 posts brain
command 1 when the hold is not 0 (else asks CatchUp at once); its tick F-50798 asks CatchUp when the state's time (+8) > the
hold; the FSM applies an ask at the start of the next update.

Holds: 0, whole multiples of 1/60, the f32 sums of 1/60 the FSM's time takes and their neighbours (one ulp up / down), and
2 x a random f32 second count (as SuspectFleeing scales it). Output, one line a case (all decimal):
  <hold bits> <brain +568 after begin> <first update with the ask> <first update in CatchUp> <the Delay's time bits there>
"""
import argparse
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp
import pursuit_fsm_reference as pf

N = 1400


def f32(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


def bits(x):
    return struct.unpack('<I', struct.pack('<f', x))[0]


def unbits(u):
    return struct.unpack('<f', struct.pack('<I', u))[0]


def holds():
    dt = f32(1.0 / 60)
    sums, t = [], 0.0
    for _ in range(1300):
        t = f32(t + dt)
        sums.append(t)
    out = [0.0, dt, f32(2 * dt), 0.5, 1.0, 1.5, 2.0, 3.0, 10.0]
    g = pf.Rng(12345)
    for _ in range(60):
        s = sums[g.r(0, 1150)]
        out += [s, unbits(bits(s) + 1), unbits(bits(s) - 1)]
    for _ in range(60):
        out.append(f32(f32(g.r(0, 10_000_000) / 1_000_000.0) * 2.0))
    for k in range(1, 40):
        out.append(f32(k / 60.0))
    return out


MAIN = r'''
static void sf(uint32_t a,float f){uint32_t u;memcpy(&u,&f,4);store(a,u);}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); const uint32_t AI=strtoul(argv[3],0,16), PL=strtoul(argv[4],0,16); std::string tok;
 while(in>>tok){ if(tok!="case") break; uint32_t hb; in>>hb; rt_reset(); const uint32_t M=0x40800000u, TGT=0x40A10000u, C=0x40A00000u, B=0x40A30000u, G=0x40A50000u;
  sf(TUNE+192,30.0f); sf(TUNE+212,1.5f); sf(TUNE+304,5.0f); sf(TUNE+368,10.0f);
  Context c=fresh(); c.r3.u32=M; CTOR(c,nullptr);
  c=fresh(); c.r3.u32=M; RESET(c,nullptr);
  store(C,AI); store(C+656,3u); store(C+1076,1u); store(C+876,B); store(B+680,TGT); store(B+568,0u); store(B+688,0u); store(B+708,0u); store(load(M+76),C);
  store(TGT,PL); store(TGT+656,3u); store(M+32,TGT); store(M+36,G); store(M+20,(load(M+20)&0x00ffffffu)|0x01000000u); store(M+164,hb);
  store(0x40C00000u,0); sf(0x40C00004u,90.0f); sf(0x40C00008u,0.0f); store(G+56,1u);
  c=fresh(); c.r3.u32=M; BEGIN(c,nullptr);
  uint32_t b0=load(B+568); int ka=-1, kc=-1; uint32_t tb=0;
  for(int k=0;k<NN && kc<0;k++){
   c=fresh(); c.r3.u32=M; c.f1.f64=(double)(1.0f/60); UPDATE(c,nullptr);
   uint32_t F=load(M+160); int st=(int)load(F+12); int as=(int)load(F+16);
   if(st==0){ tb=load(load(load(F+4))+8); }
   if(ka<0 && (as==1 || st==1)) ka=k;
   if(st==1) kc=k; }
  printf("%u %u %d %d %u\n",hb,b0,ka,kc,tb);}
 printf("end\n");
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
    ids = pf.MAIN_IDS + pf.NOCOMPILE + pf.WORLD + pf.TARGETS
    r = subprocess.run([a.tool, 'registry', 'addr', *sorted(set(ids))], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    ai, pl = rdisp.vtables_of(img, 'mcAiRacer')[0][0], rdisp.vtables_of(img, 'mcPlayer')[0][0]
    flag = lambda k: '((load(0x40C00000u)>>%d)&1)' % pf.FLAGS.index(k)
    fl = lambda adr: '([](uint32_t u){float f;memcpy(&f,&u,4);return f;})(load(%s))' % adr
    stubs = {name(f): 'throw std::runtime_error("unexpected call %s");' % f for f in pf.NOCOMPILE}
    stubs[name('F-29545')] = ('float r=(float)ctx.f1.f64; ctx.r3.u64= r==5.0f?%s : r==7.5f?%s : %s;'
                              % (flag('near5'), flag('near_along'), flag('near_safe')))
    for f, k in [('F-96080', 'moving'), ('F-80249', 'same_dir'), ('F-99390', 'ahead'), ('F-30098', 'on_right'), ('F-43223', 'same_facing'),
                 ('F-90575', 'all_block'), ('F-32902', 'closing'), ('F-14848', 'pick')]:
        stubs[name(f)] = 'ctx.r3.u64=%s;' % flag(k)
    stubs[name('F-90304')] = 'ctx.f1.f64=(double)%s;' % fl('0x40C00004u')
    stubs[name('F-41260')] = 'ctx.f1.f64=(double)%s;' % fl('0x40C00008u')
    stubs[name('F-14415')] = 'ctx.r3.u64=1;'
    stubs[name('F-36806')] = 'ctx.r3.u64=0;'
    stubs[name('F-63286')] = 'store(ctx.r3.u32+64,load(0x40C00008u));'
    stubs[name('F-24671')] = 'uint32_t B=load(ctx.r3.u32+876); store(B+568,ctx.r4.u32);'
    for f in ['F-77791', 'F-72181', 'F-59773', 'F-94949']:
        stubs[name(f)] = ''
    main_src = (MAIN.replace('CTOR', name('F-55508')).replace('RESET', name('F-20606')).replace('BEGIN', name('F-89627'))
                .replace('UPDATE', name('F-68032')).replace('TUNE', '0x%08Xu' % pf.TUNE).replace('NN', str(N)))
    exe = rdisp.build(rdisp.index(a.recomp), [name(f) for f in pf.MAIN_IDS], main_src, a.out, 'pursuit_delay', image=img,
                      vtables=pf.VTABLES, targets=[addr[f] for f in pf.TARGETS], limit=800, stubs=stubs)
    hs = holds()
    cf = a.out / 'delay_cases.txt'
    cf.write_text(''.join(f'case {bits(h)}\n' for h in hs) + 'stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf), '%08X' % ai, '%08X' % pl], capture_output=True, text=True)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    rows = r.stdout.split('\n')[:-2]
    a.ref.write_text('\n'.join(rows) + '\n')
    print('cases', len(rows), '->', a.ref)


if __name__ == '__main__':
    main()
