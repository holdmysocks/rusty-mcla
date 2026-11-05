"""Run the original's Double Pursuit tactic offline (static recompilation, native, flat guest memory) and record its AlongSide
machine for the port's test (crates/game/src/testdata/pursuit_dbl_fsm_ref.txt); with --port, also compare every update with
the port's squad_machine (arrow = false; extracted verbatim from police.rs, built with rustc).

The tactic object is built as the game builds it: ctor F-74544 (its FSM F-74935: AlongSide F-23100, Arrest F-74792, Search),
reset F-92837, the two cops and the target set as the engage sets them (+76, +32, active +20), begin F-39789 (the FSM started
in AlongSide: its enter F-86874 with F-80770), then update F-40655 with dt = 1/60 s, N times; the FSM's code and AlongSide's
update F-41674 run unchanged. World as tools/squad_fsm_reference.py (the same flags; the formation is F-21534, two slots);
the Double Pursuit has no Halt, so the cop-within / moving / fast flags must not change its machine.

  CASE <hex>               one hex digit per update: 8 formation sends, 4 a cop within, 2 the target moves, 1 fast
  <k> <st> <next> <lock>   an update where the state (FSM +12: 0 AlongSide) or the asked state (+16) changed, every 50th, and
                           AlongSide's lock (+16 of the state, f32 bits) after the update
  END <k> <st> <next> <lock>
"""
import argparse
import random
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

N = 1200
CASES = 30
NOCOMPILE = ['F-35685', 'F-75034', 'F-29294', 'F-38446', 'F-62425', 'F-29290', 'F-33229', 'F-62311']
ACTIONS = ['F-42951', 'F-80249', 'F-99390', 'F-30098', 'F-90304', 'F-37230', 'F-54152', 'F-23825', 'F-14569', 'F-55001', 'F-77791',
           'F-39048', 'F-48012', 'F-10965', 'F-24671', 'F-19124', 'F-82234', 'F-32902', 'F-72181']
TARGETS = ['F-87882', 'F-54202', 'F-77220', 'F-49619', 'F-47121', 'F-74264', 'F-74492', 'F-24596', 'F-37056', 'F-82134', 'F-29906',
           'F-33778', 'F-94002']
MAIN_IDS = ['F-74544', 'F-92837', 'F-39789', 'F-40655']
# the Double Pursuit's FSM and its three states' vtables (data, not functions)
VTABLES = [(0x820C8514, 15), (0x820C8414, 16), (0x820C8454, 22), (0x820C8574, 22), (0x820C84BC, 22)]

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); const uint32_t AI=strtoul(argv[3],0,16), PL=strtoul(argv[4],0,16); std::string line;
 while(std::getline(in,line)){ const int N=(int)line.size(); rt_reset(); const uint32_t M=0x40800000u, TGT=0x40A10000u;
  Context c=fresh(); c.r3.u32=M; CTOR(c,nullptr);
  c=fresh(); c.r3.u32=M; RESET(c,nullptr);
  for(int i=0;i<2;i++){ uint32_t C=0x40A00000u+0x2000u*i, B=0x40A30000u+0x2000u*i; store(C,AI); store(C+656,3u); store(C+1076,1u); store(C+876,B); store(B+680,TGT); store(B+568,2u); store(load(M+76)+4*i,C);}
  store(TGT,PL); store(TGT+656,3u); store(M+32,TGT); store(M+20,(load(M+20)&0x00ffffffu)|0x01000000u);
  c=fresh(); c.r3.u32=M; BEGIN(c,nullptr);
  for(int k=0;k<N;k++){ int h=line[k]<='9'?line[k]-'0':line[k]-'a'+10; for(int b=0;b<4;b++) store(0x40C00004u+4*b,(h>>(3-b))&1);
   c=fresh(); c.r3.u32=M; c.f1.f64=(double)(1.0f/60); UPDATE(c,nullptr);
   uint32_t F=load(M+160); int st=(int)load(F+12); uint32_t S=st>=0?load(load(F+4)+4*st):0;
   printf("%d %d %08x\n",st,(int)load(F+16),st==0?load(S+16):0u);}
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
fn main() { let t = std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap(); let dt = 1.0f32 / 60.0;
  for l in t.lines() { let mut sq = Squad::default();
    for c in l.chars() { let h = c.to_digit(16).unwrap(); let f = |b: u32| h & b != 0; let send = f(8);
      squad_machine(&mut sq, &mut (), false, f(4), f(2), f(1), dt, &mut |sq: &mut Squad, _: &mut (), d: f32, _: bool| { if send || d == 0.0 { sq.slots = [Some(0), Some(1), None]; } send }, &mut |_: &mut Squad, _: &mut ()| {});
      let st = if sq.halt { 2 } else { 0 };
      println!("{} {} {:08x}", st, sq.next.map_or(-1, |h| if h { 2 } else { 0 }), if st == 0 { sq.lock.to_bits() } else { 0 });
    }
    println!("end"); } }
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
    ids = MAIN_IDS + NOCOMPILE + ACTIONS + TARGETS + ['F-21534', 'F-29443', 'F-29545', 'F-96080', 'F-33067', 'F-36806', 'F-94676']
    r = subprocess.run([a.tool, 'registry', 'addr', *sorted(set(ids))], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    ai, pl = rdisp.vtables_of(img, 'mcAiRacer')[0][0], rdisp.vtables_of(img, 'mcPlayer')[0][0]
    stubs = {name(f): 'throw std::runtime_error("unexpected call %s");' % f for f in NOCOMPILE}
    stubs.update({name(f): 'ctx.r3.u64=0x40D00000u;' for f in ACTIONS})   # a zeroed block: pointers valid, booleans false
    slots = 'uint32_t T=load(load(ctx.r3.u32+12)+20); store(T+164,0); store(T+168,1);'
    stubs[name('F-21534')] = 'if(load(0x40C00004u)||ctx.f1.f64==0.0){%s} ctx.r3.u64=load(0x40C00004u);' % slots
    stubs[name('F-29443')] = stubs[name('F-29545')] = 'ctx.r3.u64=load(0x40C00008u);'
    stubs[name('F-96080')] = 'ctx.r3.u64=load(0x40C0000Cu);'
    stubs[name('F-33067')] = 'ctx.f1.f64=load(0x40C00010u)?100.0:0.0;'
    stubs[name('F-36806')] = stubs[name('F-94676')] = 'ctx.r3.u64=0;'
    main_src = (MAIN.replace('CTOR', name(MAIN_IDS[0])).replace('RESET', name(MAIN_IDS[1])).replace('BEGIN', name(MAIN_IDS[2]))
                .replace('UPDATE', name(MAIN_IDS[3])))
    exe = rdisp.build(rdisp.index(a.recomp), [name(f) for f in MAIN_IDS], main_src, a.out, 'pursuit_dbl_fsm', image=img, vtables=VTABLES,
                      targets=[addr[f] for f in TARGETS], limit=800, stubs=stubs)
    rng = random.Random(4)
    cases = []
    for _ in range(CASES):
        w, m, f, row = 0, 1, 0, ''
        for k in range(N):
            w ^= rng.random() < 0.03
            m ^= rng.random() < 0.03
            f ^= rng.random() < 0.02
            row += '%x' % ((rng.random() < 0.05) * 8 + w * 4 + m * 2 + f)
        cases.append(row)
    cf = a.out / 'pursuit_dbl_fsm_cases.txt'
    cf.write_text('\n'.join(cases) + '\n')
    r = subprocess.run([str(exe), str(a.image), str(cf), '%08X' % ai, '%08X' % pl], capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    orig = [o.split('\n')[:-1] for o in r.stdout.split('end\n')[:-1]]
    out = []
    for case, rows in zip(cases, orig):
        out.append(f'CASE {case}')
        prev = None
        for k, row in enumerate(rows):
            st, nx, _ = row.split()
            if (st, nx) != prev or k % 50 == 0:
                out.append(f'{k} {row}')
                prev = (st, nx)
        out.append(f'END {len(rows) - 1} {rows[-1]}')
    a.ref.write_text('\n'.join(out) + '\n')
    print('cases', len(cases), 'updates', sum(map(len, orig)), 'in Halt', sum(r.startswith('2 ') for o in orig for r in o), '->', a.ref)
    if a.port:
        src = a.port.read_text()
        rs = (grab(src, '#[derive(Clone, Debug, Default)]\nstruct Squad {') + '\n' + grab(src, 'fn squad_may_go(') + '\n'
              + grab(src, 'fn squad_machine<C>(') + PORT_MAIN)
        (a.out / 'port_squad.rs').write_text(rs)
        b = subprocess.run(['rustc', '-O', '--edition', '2021', '-A', 'warnings', str(a.out / 'port_squad.rs'), '-o', str(a.out / 'port_squad.exe')],
                           capture_output=True, text=True)
        if b.returncode:
            sys.exit(b.stderr[-2000:])
        p = subprocess.run([str(a.out / 'port_squad.exe'), str(cf)], capture_output=True, text=True).stdout
        port = [o.split('\n')[:-1] for o in p.split('end\n')[:-1]]
        tot = sum(map(len, orig)); eq = sum(x == y for o, q in zip(orig, port) for x, y in zip(o, q))
        print('port: updates', tot, 'equal', eq)


if __name__ == '__main__':
    main()
