"""The bike rider's gesture pickers run offline (generated code unchanged): neutral F-38624, left F-24572, right F-99511, talk left
F-44243, talk right F-57458, mild F-51244, win F-49380 and the ticket fidget F-80672. Each takes r3 the anim player, r4 the picker's
counter, r5 (neutral only) the respond flag; it picks a name from its table, counts and hands the name to the clip play F-99427
(stubbed: it prints the name).
Usage: bike_gesture_reference.py <out-dir> [--tool mcla-tool] [--recomp D:/MCLA-static] [--image original-image.bin]
Writes <out-dir>/bike_gesture_ref.txt: "<picker> <counter> <flag> -> <name> <counter after>"."""
import argparse
import subprocess
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import recomp_dispatch as rd

PICKERS = ['F-38624', 'F-24572', 'F-99511', 'F-44243', 'F-57458', 'F-51244', 'F-49380', 'F-80672']


def addr(tool, fid):
    return int(subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1], 16)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('out', type=Path)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--recomp', type=Path, default=Path('D:/MCLA-static'))
    ap.add_argument('--image', type=Path, default=Path('D:/MCLA/work/2026-10-10/original-image.bin'))
    a = ap.parse_args()
    work = a.out.resolve(); work.mkdir(parents=True, exist_ok=True)
    fn = {k: 'sub_%08X' % addr(a.tool, k) for k in PICKERS + ['F-99427']}
    counters = list(range(0, 23)) + [99, 1000, 1001, 65535, 65536, 1000003]
    jobs = [(p, c, f) for p in range(len(PICKERS)) for c in counters for f in ((0, 1) if p == 0 else (0,))]
    nl = chr(10)
    rows = nl.join('{%d,%d,%d},' % j for j in jobs)
    calls = nl.join('  case %d: %s(c,nullptr); break;' % (i, fn[p]) for i, p in enumerate(PICKERS))
    main_src = r'''

static const uint32_t J[][3]={
''' + rows + r'''
};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 rt_reset();
 for(auto&j:J){ store(0x40800000u,j[1]); Context c=fresh(); c.r3.u64=0x40810000u; c.r4.u64=0x40800000u; c.r5.u64=j[2];
  switch(j[0]){
''' + calls + r'''
  }
  printf(" %u\n",load(0x40800000u)); }
}
'''
    stubs = {fn['F-99427']: 'for(uint32_t a=ctx.r4.u32;*pointer(a);a++) putchar(*pointer(a));'}
    idx = rd.index(a.recomp); img = rd.load_image(a.image)
    exe = rd.build(idx, [fn[p] for p in PICKERS], main_src, work, 'bike_gesture', stubs=stubs, image=img)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    out = [x for x in r.stdout.split(nl) if x.strip()]
    assert len(out) == len(jobs), (len(out), len(jobs))
    (work / 'bike_gesture_ref.txt').write_text(''.join(f'{PICKERS[p]} {c} {f} -> {o}{nl}' for (p, c, f), o in zip(jobs, out)))
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
