"""Run the original's Dispatch to Location tactic offline (static recompilation, native, flat guest memory) and record its
machine for the port's test (crates/game/src/testdata/dispatch_fsm_ref.txt); with --port, also compare every update with
the port's dispatch_enter / dispatch_ask (extracted verbatim from police.rs, built with rustc).

The tactic object is built as the game builds it: ctor F-64847 (its FSM and four states, F-68382), reset F-12392, engage
F-32468 with a request (place +32, call +48, cop +0, target +4) for a cop already loaded (+656 = 3: the base engage F-11097
begins the machine at once, F-26187), then update F-87106 with dt = 1/60 s, N times. The FSM's own code runs unchanged
(update F-95322, enter F-43677, ask F-28114, may-go F-40040, the states' tests F-64066 / F-52633 / F-21992).
World: the cop and target carry the real mcAiRacer / mcPlayer vtables (RTTI), the cop passes the tactic's cop check
(F-86113: +1076 set), the speech manager's +2256 / +2260 and form 109's +3636 are globals the case sets, arrival (F-98680)
is the case's flag, the seen test (F-94676) is false, and the states' enter actions (radio lines and routes: F-44635,
F-81858, F-52732) and WaitAtLocation's update (F-92001) are empty: the port does those apart from the machine.

  CASE <a> <b> <c>     a career speech line plays in updates [a, b); the cop has arrived from update c
  <k> <st> <next> <t>  an update where the state or the state asked for changed: state (FSM +12), asked (+16, -1 none),
                       the state's time +8 after the update (f32 bits)
  END <k> <st> <next> <t>
"""
import argparse
import random
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

N = 900
# callees that never run in these cases but do not compile in the shim (locks, vector forms) - a call would throw
NOCOMPILE = ['F-35685', 'F-75034', 'F-29294', 'F-38446', 'F-62425', 'F-29290', 'F-33229']
EMPTY = ['F-44635', 'F-81858', 'F-52732', 'F-92001']
TARGETS = ['F-61449', 'F-16149', 'F-68382', 'F-49482', 'F-43677', 'F-26187', 'F-17537', 'F-34717', 'F-47121', 'F-89292',
           'F-74264', 'F-87882', 'F-69263', 'F-95322', 'F-93891', 'F-88920', 'F-64066', 'F-22583', 'F-37973', 'F-77819',
           'F-49619', 'F-54202', 'F-59898', 'F-82856', 'F-52633', 'F-21992', 'F-40040', 'F-44635', 'F-81858', 'F-52732',
           'F-92001']
# the speech manager and the UI root (its form table at +48 +0x13F80), the globals WaitToDispatch F-64066 reads
SPEECH_GLOBAL, UI_GLOBAL = 0x8286D114, 0x8286D804

MAIN = r'''
static void sf(uint32_t a,float f){uint32_t u;memcpy(&u,&f,4);store(a,u);}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); int A,B,C; const int N=atoi(argv[3]); const uint32_t AI=strtoul(argv[4],0,16), PL=strtoul(argv[5],0,16);
 while(in>>A>>B>>C){ rt_reset(); const uint32_t M=0x40800000u, R=0x40900000u, COP=0x40A00000u, TGT=0x40A10000u;
  Context c=fresh(); c.r3.u32=M; CTOR(c,nullptr);
  c=fresh(); c.r3.u32=M; RESET(c,nullptr);
  sf(R+32,10.5f);sf(R+36,-3.25f);sf(R+40,200.f);sf(R+44,1.f); store(R+48,0x1234abcdu); store(R+0,COP); store(R+4,TGT); store(R+8,0x40A20000u); store(R+12,0x01000101u); sf(R+16,2.5f);
  store(COP,AI); store(COP+656,3u); store(COP+1076,1u); store(TGT,PL); store(TGT+656,3u);
  c=fresh(); c.r3.u32=M; c.r4.u32=R; ENGAGE(c,nullptr);
  if((int)load(load(M+160)+12)!=0||(load(M+84)>>24)) { printf("not begun at engage\n"); return 1; }
  store(SPEECH,0x40B00000u); store(UI,0x40B10000u); store(0x40B10000u+48,0x40B20000u); store(0x40B20000u+0x13F80u+68,0x40B40000u); store(0x40B40000u+4*109,0x40B50000u);
  for(int k=0;k<N;k++){ uint32_t sp=(k>=A&&k<B)?1u:0u; store(0x40B00000u+2256,sp); store(0x40B00000u+2260,sp); store(0x40C00000u,k>=C?1u:0u);
   c=fresh(); c.r3.u32=M; c.f1.f64=(double)(1.0f/60); UPDATE(c,nullptr);
   uint32_t F=load(M+160); int st=(int)load(F+12); printf("%d %d %08x\n",st,(int)load(F+16),st>=0?load(load(load(F+4)+4*st)+8):0u);}
  printf("end\n");}
}
'''


def grab(s, pat):
    i = s.index(pat); j = s.index('{', i); d = 0
    for k in range(j, len(s)):
        if s[k] == '{': d += 1
        elif s[k] == '}':
            d -= 1
            if d == 0: return s[i:k + 1]


PORT_MAIN = r'''
fn main() { let t = std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap(); let n: i64 = std::env::args().nth(2).unwrap().parse().unwrap();
  let dt = 1.0f32 / 60.0;
  for l in t.lines() { let v: Vec<i64> = l.split_whitespace().map(|x| x.parse().unwrap()).collect();
    let mut d = Dispatch { cop: 1, st: 0, t: 0.0, loc: Vec3::default(), call: String::new(), crumbs: Vec::new(), crumb_t: 1.0, seen_t: 0.0, trail: false, routed: false, arrived: false, next: None };
    for k in 0..n {
      dispatch_enter(&mut d);
      d.arrived = k >= v[2];
      dispatch_ask(&mut d, 3.0, k >= v[0] && k < v[1]);
      d.t += dt;
      println!("{} {} {:08x}", d.st, d.next.map_or(-1, |x| x as i32), d.t.to_bits());
    }
    println!("end");
  } }
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--port', type=Path, help='police.rs: compare every update with the port')
    a = ap.parse_args()
    ids = TARGETS + NOCOMPILE + EMPTY + ['F-64847', 'F-12392', 'F-32468', 'F-87106', 'F-98680', 'F-94676']
    r = subprocess.run([a.tool, 'registry', 'addr', *sorted(set(ids))], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    ai, pl = rdisp.vtables_of(img, 'mcAiRacer')[0][0], rdisp.vtables_of(img, 'mcPlayer')[0][0]
    speech, ui = SPEECH_GLOBAL, UI_GLOBAL
    stubs = {name(f): 'throw std::runtime_error("unexpected call %s");' % f for f in NOCOMPILE}
    stubs.update({name(f): '' for f in EMPTY})
    stubs[name('F-98680')] = 'ctx.r3.u64=load(0x40C00000u);'
    stubs[name('F-94676')] = 'ctx.r3.u64=0;'
    main_src = (MAIN.replace('CTOR', name('F-64847')).replace('RESET', name('F-12392')).replace('ENGAGE', name('F-32468'))
                .replace('UPDATE', name('F-87106')).replace('SPEECH', '0x%08Xu' % speech).replace('UI', '0x%08Xu' % ui))
    idx = rdisp.index(a.recomp)
    exe = rdisp.build(idx, [name(f) for f in ('F-64847', 'F-12392', 'F-32468', 'F-87106')], main_src, a.out, 'dispatch_fsm', image=img,
                      targets=[addr[f] for f in TARGETS], limit=600, stubs=stubs)
    rng = random.Random(7)
    cases = [(0, 0, 400), (150, 200, 400), (170, 190, 300), (181, 183, 250), (182, 184, 500), (100, 400, 450), (183, 186, 186), (0, 0, 185)]
    cases += [(s, s + rng.randrange(1, 60), rng.randrange(180, 700)) for s in (rng.randrange(150, 260) for _ in range(40))]
    cf = a.out / 'dispatch_fsm_cases.txt'
    cf.write_text(''.join(f'{x} {y} {z}\n' for x, y, z in cases))
    r = subprocess.run([str(exe), str(a.image), str(cf), str(N), '%08X' % ai, '%08X' % pl], capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    orig = [o.split('\n')[:-1] for o in r.stdout.split('end\n')[:-1]]
    out = []
    for (x, y, z), rows in zip(cases, orig):
        out.append(f'CASE {x} {y} {z}')
        prev = None
        for k, row in enumerate(rows):
            st, nx, _ = row.split()
            if (st, nx) != prev:
                out.append(f'{k} {row}')
                prev = (st, nx)
        out.append(f'END {len(rows) - 1} {rows[-1]}')
    a.ref.write_text('\n'.join(out) + '\n')
    print('cases', len(cases), 'updates', sum(map(len, orig)), '->', a.ref)
    if a.port:
        src = a.port.read_text()
        rs = ('#[derive(Clone, Copy, Debug, Default)] struct Vec3(f32, f32, f32);\n' + grab(src, '#[derive(Clone, Debug)]\nstruct Dispatch {')
              + '\n' + grab(src, 'fn dispatch_may_go(') + '\n' + grab(src, 'fn dispatch_enter(') + '\n' + grab(src, 'fn dispatch_ask(') + PORT_MAIN)
        (a.out / 'port_fsm.rs').write_text(rs)
        b = subprocess.run(['rustc', '-O', '--edition', '2021', '-A', 'warnings', str(a.out / 'port_fsm.rs'), '-o', str(a.out / 'port_fsm.exe')],
                           capture_output=True, text=True)
        if b.returncode:
            sys.exit(b.stderr[-2000:])
        p = subprocess.run([str(a.out / 'port_fsm.exe'), str(cf), str(N)], capture_output=True, text=True).stdout
        port = [o.split('\n')[:-1] for o in p.split('end\n')[:-1]]
        tot = sum(map(len, orig)); eq = sum(x == y for o, q in zip(orig, port) for x, y in zip(o, q))
        print('port: updates', tot, 'equal', eq)


if __name__ == '__main__':
    main()
