"""The steering part's rescale F-52244 run offline on the original's code: per case the part's +1136 (scale), +1160 / +1164
(the steering pair) and the new scale in f1; out: the three words after.
Usage: steer_rescale_reference.py <out-dir> [--tool mcla-tool] [--recomp D:/MCLA-static] [--image original-image.bin] [-n 2000]
Writes <out-dir>/steer_rescale_ref.txt: "scale mid out s -> scale mid out" (float bits, hex)."""
import argparse, random, struct, subprocess, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import memcap
import recomp_dispatch as rd


def fw(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('out', type=Path)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--recomp', type=Path, default=Path('D:/MCLA-static'))
    ap.add_argument('--image', type=Path, default=Path('D:/MCLA/work/2026-10-10/original-image.bin'))
    ap.add_argument('-n', type=int, default=2000)
    a = ap.parse_args()
    memcap.apply()
    work = a.out.resolve()
    work.mkdir(parents=True, exist_ok=True)
    fn = 'sub_%08X' % int(subprocess.run([a.tool, 'registry', 'addr', 'F-52244'], capture_output=True, text=True, check=True).stdout.split()[1], 16)
    rnd = random.Random(52244)
    special = [0.0, -0.0, 1.0, 0.101, 0.999, 1e-30, float('inf'), float('nan'), 0.5, 2.0]
    def v():
        return rnd.choice(special) if rnd.random() < 0.2 else rnd.uniform(-3, 3)
    cases = []
    while len(cases) < a.n:
        c = [fw(v()) for _ in range(4)]
        if any(('%08x' % w).startswith('82') for w in c):
            continue
        cases.append(c)
    rows = ',\n'.join('{%s}' % ','.join('0x%08Xu' % w for w in c) for c in cases)
    main_src = r'''
static const uint32_t J[][4]={
''' + rows + r'''
};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 const uint32_t D=0x40800000u;
 for(auto&j:J){ rt_reset(); store(D+1136,j[0]); store(D+1160,j[1]); store(D+1164,j[2]);
  Context c=fresh(); c.r3.u64=D; float s; memcpy(&s,&j[3],4); c.f1.f64=s; FN(c,nullptr);
  printf("%08x %08x %08x\n",load(D+1136),load(D+1160),load(D+1164)); }
}
'''.replace('FN', fn)
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    exe = rd.build(idx, [fn], main_src, work, 'steer_rescale', image=img)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    out = r.stdout.splitlines()
    assert len(out) == len(cases)
    (work / 'steer_rescale_ref.txt').write_text(''.join(' '.join('%08x' % w for w in c) + ' -> ' + o + '\n' for c, o in zip(cases, out)))
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
