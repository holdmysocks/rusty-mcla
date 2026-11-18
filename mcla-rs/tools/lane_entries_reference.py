"""A car's lane entries (AddLane F-94578, RemoveLane F-59811, the obstacle's list +20 / count +308, at most 12) and the
lanes' car lists (the rail's +96 head / +100 tail; append F-30206 at the tail, unlink F-13436) run offline: the generated code whole
on flat guest memory, nothing stubbed. Each case: 8 obstacles, 6 lanes, 160 random operations (an add only of a lane the
obstacle does not list, as F-58331 calls it; a remove of a listed lane or, sometimes, of one it does not list).

Writes <out-dir>/lane_entries_ref.txt, a case a line: the operations 'A o l' / 'R o l' separated by ',', ' -> ' each
lane's car list from head to tail as the owning obstacles, lanes separated by '/' (test `lane_entries_match_the_original`)."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build

_shim = recomp_closure.shim
# (RemoveLane's copy loop counts with ctr, which the scalar shim's Context lacks)
recomp_closure.shim = lambda *a, **k: _shim(*a, **k).replace('uint64_t lr{};', 'uint64_t lr{}; PPCRegister ctr{};', 1)

NL = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=2000)
    a = ap.parse_args()
    add, rem = (addr(a.tool, f) for f in ('F-94578', 'F-59811'))
    rnd = random.Random(59812)
    cases = []
    for _ in range(a.cases):
        held = [[] for _ in range(8)]
        ops = []
        for _ in range(160):
            o = rnd.randrange(8)
            if held[o] and rnd.random() < 0.45:
                l = rnd.choice(held[o]) if rnd.random() < 0.9 else rnd.randrange(6)
                if l in held[o]:
                    held[o].remove(l)
                ops.append(('R', o, l))
            else:
                free = [l for l in range(6) if l not in held[o]]
                if not free:
                    continue
                l = rnd.choice(free)
                if len(held[o]) < 12:
                    held[o].append(l)
                ops.append(('A', o, l))
        cases.append(ops)
    body = '#include <sstream>' + NL + r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
std::ifstream in(argv[2]);std::string line;
while(std::getline(in,line)){if(line.empty())continue;memset(objects,0,sizeof(objects));
  for(uint32_t a=0x10000;a<0x50000;a+=4)store(a,0);
  std::istringstream ss(line);char k;int o,l;
  while(ss>>k>>o>>l){Context ctx{};ctx.r1.u32=0x7F000;ctx.r3.u32=0x10000+0x200*o;ctx.r4.u32=0x40000+0x100*l;ctx.f1.f64=0.5;
    if(k=='A')sub_%s(ctx,nullptr);else sub_%s(ctx,nullptr);}
  for(int l=0;l<6;l++){if(l)printf("/");uint32_t n=load(0x40000+0x100*l+96);bool f=true;int g=0;
    while(n&&g++<200){printf(f?"%%d":" %%d",(int)((load(n+12)-0x10000)/0x200));f=false;n=load(n+4);}}
  printf("\n");}
}
''' % (add, rem)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + add, 'sub_' + rem], body, a.out_dir, 'lane_entries', vector=False, scratch_end=0x80000)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'lane_entries_in.txt'
    inp.write_text(NL.join(' '.join('%s %d %d' % op for op in ops) for ops in cases) + NL)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True, check=True)
    out = [x for x in r.stdout.split(NL) if x]
    assert len(out) == len(cases), (len(out), len(cases), r.stdout[-300:])
    lines = [','.join('%s %d %d' % op for op in ops) + ' -> ' + o.strip() for ops, o in zip(cases, out)]
    (a.out_dir / 'lane_entries_ref.txt').write_text(NL.join(lines) + NL)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
