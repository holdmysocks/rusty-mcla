"""The steady stopping acceleration F-89809 run offline: the generated code unchanged (f1 time, f2 from, f3 speed,
f4 to, f5 end speed -> f1). Writes <out-dir>/stop_rate_ref.txt: "time from v to v_end -> a" (hex float bits)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = addr(a.tool, 'F-89809')
    rnd = random.Random(3)
    sp = (0.0, -0.0, 1e-6, 1e-7, 1.0, -1.0, 0.5, 2.0, float('nan'), float('inf'))
    cases = []
    for _ in range(6000):
        c = [rnd.uniform(-0.5, 5), rnd.uniform(-50, 50), rnd.uniform(-30, 30), rnd.uniform(-50, 50), rnd.uniform(-30, 30)]
        for k in range(5):
            if rnd.random() < 0.08:
                c[k] = rnd.choice(sp)
        if rnd.random() < 0.1:
            c[4] = -c[2]
        if rnd.random() < 0.1:
            c[3] = c[1]
        cases.append(c)
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % fb(x) for x in c) for c in cases)
    body = ('static const uint32_t C[][5]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){Context ctx{};ctx.r1.u32=0x2F00;double*F[5]={&ctx.f1.f64,&ctx.f2.f64,&ctx.f3.f64,&ctx.f4.f64,&ctx.f5.f64};' +
            'for(int k=0;k<5;k++){float x;uint32_t u=c[k];memcpy(&x,&u,4);*F[k]=x;}' +
            ('sub_%s(ctx,nullptr);float r=float(ctx.f1.f64);uint32_t u;memcpy(&u,&r,4);printf("%%08x%s",u);}' % (f, NL)) + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    exe = build(index(a.recomp), ['sub_' + f], main_src, a.out_dir, 'stop_rate')
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'stop_rate_ref.txt').write_text(nl.join('%s -> %s' % (' '.join('%08x' % fb(x) for x in c), o) for c, o in zip(cases, out)) + nl)
    print(len(cases), 'cases')


if __name__ == '__main__':
    main()
