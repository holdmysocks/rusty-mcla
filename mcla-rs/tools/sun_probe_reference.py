"""The sun glow's occlusion probe (F-63954 -> its uniform-radius wrapper -> the solid ellipsoid builder F-68903) run offline:
unchanged generated code on flat guest memory, the immediate-mode calls replaced by stubs that print what they get
(the begin's primitive and count, each vertex's x y z, the end). Writes <out-dir>/sun_probe_ref.txt:
    "case <rx> <ry> <rz> <slices>" then "B <mode> <count>" / "V <x bits> <y bits> <z bits>" / "E" lines
"""
import argparse
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--builder', required=True, help='the ellipsoid builder, as an F-id')
    ap.add_argument('--begin', required=True)
    ap.add_argument('--vertex', required=True)
    ap.add_argument('--end', required=True)
    ap.add_argument('--matrix', required=True, help='the world matrix setter it calls first')
    a = ap.parse_args()
    fn = {k: 'sub_' + addr(a.tool, getattr(a, k)) for k in ('builder', 'begin', 'vertex', 'end', 'matrix')}
    cases = [(20.0, 20.0, 20.0, 8), (1.0, 1.0, 1.0, 8), (3.0, 5.0, 7.0, 6), (1.0, 2.0, 0.5, 12), (20.0, 20.0, 20.0, 5)]
    nl, NL = chr(10), chr(92) + 'n'
    fb = lambda x: struct.unpack('>I', struct.pack('>f', x))[0]
    body = ''
    for rx, ry, rz, s in cases:
        body += ('printf("case %%g %%g %%g %%d%s",%r,%r,%r,%d);store(0x1200,0x%08x);store(0x1204,0x%08x);store(0x1208,0x%08x);'
                 '{Context ctx{};ctx.r3.u32=0x1200;ctx.r4.u32=0x1300;ctx.r5.u32=%d;ctx.r6.u32=0;ctx.r7.u32=1;ctx.r1.u32=0x2F00;%s(ctx,nullptr);}' %
                 (NL, rx, ry, rz, s, fb(rx), fb(ry), fb(rz), s, fn['builder']) + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    bits = 'auto fb=[](double d){float f=(float)d;uint32_t b;std::memcpy(&b,&f,4);return b;};'
    stubs = {
        fn['matrix']: '',
        fn['begin']: 'printf("B %%u %%u%s",ctx.r3.u32,ctx.r4.u32);' % NL,
        fn['vertex']: bits + 'printf("V %%08x %%08x %%08x%s",fb(ctx.f1.f64),fb(ctx.f2.f64),fb(ctx.f3.f64));' % NL,
        fn['end']: 'printf("E%s");' % NL,
    }
    idx = index(a.recomp)
    exe = build(idx, [fn['builder']], main_src, a.out_dir, 'sun_probe', stubs=stubs, vector=True)
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'sun_probe_ref.txt').write_text(out)
    print(out.count('V '), 'vertices')


if __name__ == '__main__':
    main()
