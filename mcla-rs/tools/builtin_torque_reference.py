"""The engine's built-in torque curve run offline as the original sets it up: the engine's ctor F-48129 (the golden
ratio's terms +92 / +96 from fsqrts 5, the reset it calls), then its Init F-23005 on a tune without control values
(+52 = 0: no curves built, no bisection: +68 / +72 / +76 from the rpms, +84 = 1 / (max - opt)^2, +88 = 1 / opt^3), then
the curve F-91323 at a set of engine speeds - the generated code unchanged on flat guest memory (engine at 0x1000, tune
at 0x1400).

Writes <out-dir>/builtin_torque_ref.txt, one case a line: "hp idle opt max | w0 w1 .. -> t0 t1 .." (hex float bits)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run

NW = 24


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
    ctor, init, curve = (addr(a.tool, f) for f in ('F-48129', 'F-23005', 'F-91323'))
    rnd = random.Random(17)
    K = 0.10471975803375244
    cases = []
    for i in range(1500):
        hp = rnd.choice((rnd.uniform(40, 900), 150.0, 300.0))
        idle = rnd.uniform(500, 1500)
        opt = rnd.uniform(2500, 9000)
        mx = opt + rnd.choice((rnd.uniform(100, 4000), 1000.0, 0.0 if i % 97 == 0 else 500.0))
        if i % 53 == 0:
            mx = opt - 300.0
        o, m = opt * K, mx * K
        ws = [0.0, -1.0, o, m, o * 1.0000001, m * 0.9999999, m * 1.0000001, float('nan'), 1e30]
        ws += [rnd.uniform(0, max(o, m) * 1.25) for _ in range(NW - len(ws))]
        cases.append(([hp, idle, opt, mx], ws))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % fb(x) for x in t + ws) for t, ws in cases)
    body = ('static const uint32_t C[][%d]={' % (4 + NW) + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            'for(int k=0;k<4;k++)store(0x1400+4+4*k,c[k]);' +
            ('Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);store(0x1004,0x1400);' % ctor) +
            ('ctx=Context{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);' % init) +
            'for(int k=0;k<%d;k++){ctx=Context{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;{float f;uint32_t u=c[4+k];memcpy(&f,&u,4);ctx.f1.f64=f;}' % NW +
            ('sub_%s(ctx,nullptr);float r=float(ctx.f1.f64);uint32_t u;memcpy(&u,&r,4);printf(k?" %%08x":"%%08x",u);}printf("%s");}' % (curve, NL)) + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    idx = index(a.recomp)
    # the curve build F-14839 and the control-value curve F-36517 are not reached without control values (+52 = 0,
    # +120 = 0): stubs that stop the run if they were
    trap = 'throw std::runtime_error("reached");'
    stubs = {'sub_' + addr(a.tool, f): trap for f in ('F-14839', 'F-36517')}
    exe = build(idx, ['sub_' + ctor, 'sub_' + init, 'sub_' + curve], main_src, a.out_dir, 'builtin_torque', vector=True, stubs=stubs)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = ['%s | %s -> %s' % (' '.join('%08x' % fb(x) for x in t), ' '.join('%08x' % fb(x) for x in ws), o) for (t, ws), o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'builtin_torque_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
