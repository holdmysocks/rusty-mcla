"""Run the original's cone mesh builders offline (the generated code as it is): the vertex fill F-78485 and the index
strip F-11042, which the mesh's constructor F-90312 calls with (rings, slices) 16 x 32 and 12 x 24 for the lamps' cones
(rage_spotlightfog, flag 0, uv scale (1, 1)), and record what they write, for the port's test
(crates/game/src/testdata/cone_mesh_reference.txt).

Stubs: the vertex buffer's create F-64660 answers 0x1C00, the writer's init answers with no lock to undo; the writer's
four setters (position, normal, tangent, texcoord) print what they get; the index buffer's create answers an object whose
lock (vtable +8) hands out 0x2000 and whose unlock (+12) does nothing, and the words written there are printed.

  CASE <rings> <slices> <flag>
  POS <index> <x y z>        NRM <index> <x y z>        TAN <index> <x y z>        UV <index> <u v>
  IDX <count> <u16 ...>

python cone_mesh_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file> --tool <mcla-tool>
"""
import argparse
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

# the index buffer's lock / unlock go through `dispatch` (defined with main); r2 stands in for the count register
recomp_closure.EXTRA += '#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


CASES = [(16, 32, 0), (12, 24, 0), (16, 32, 1), (1, 3, 0), (2, 5, 1), (5, 7, 0), (3, 1, 0)]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    verts, idxs = addr(a.tool, 'F-78485'), addr(a.tool, 'F-11042')
    # the callees of the two builders, in the order they call them: vertex buffer create, writer init, the four setters,
    # index buffer create
    vb, init, spos, snrm, stan, suv, ib = (addr(a.tool, f) for f in ('F-64660', 'F-40099', 'F-69926', 'F-28885', 'F-77969', 'F-95343', 'F-40590'))
    nl, NL = chr(10), chr(92) + 'n'
    v3 = lambda tag, ix, p: ('printf("%s %%u %%08x %%08x %%08x' % tag + NL + '",ctx.%s.u32,load(ctx.%s.u32),load(ctx.%s.u32+4),load(ctx.%s.u32+8));' % (ix, p, p, p))
    stubs = {'sub_' + vb: 'ctx.r3.u64=0x1C00;',
             'sub_' + init: 'store(ctx.r3.u32+8,0);',
             'sub_' + spos: v3('POS', 'r4', 'r5'),
             'sub_' + snrm: v3('NRM', 'r4', 'r5'),
             'sub_' + stan: v3('TAN', 'r4', 'r6'),
             'sub_' + suv: 'printf("UV %u %08x %08x' + NL + '",ctx.r4.u32,load(ctx.r6.u32),load(ctx.r6.u32+4));',
             'sub_' + ib: 'store(0x1800,0x1810);store(0x1818,0xA0000008u);store(0x181C,0xA000000Cu);ctx.r3.u64=0x1800;'}
    body = ('static const uint32_t C[][3]={' + ','.join('{%d,%d,%d}' % c for c in CASES) + '};' + nl +
            'static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){if(a==0xA0000008u)ctx.r3.u64=0x2000;else if(a==0xA000000Cu){}'
            'else{printf("bad dispatch %08x' + NL + '",a);exit(1);}}' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));printf("CASE %u %u %u' + NL + '",c[0],c[1],c[2]);' +
            # the uv scale (1, 1) at 0x1100; the mesh object at 0x1000
            'store(0x1100,0x3F800000u);store(0x1104,0x3F800000u);' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1200;ctx.r5.u32=c[0];ctx.r6.u32=c[1];ctx.r7.u32=c[2];ctx.r8.u32=0x1100;' +
            ('sub_%s(ctx,nullptr);' % verts) +
            'memset(pointer(0x2000),0xEE,0xC00);Context k{};k.r1.u32=0x2F00;k.r3.u32=0x1000;k.r4.u32=c[0];k.r5.u32=c[1];' + ('sub_%s(k,nullptr);' % idxs) +
            'uint32_t n=2*c[0]*(c[1]+2)-2;printf("IDX %u",n);for(uint32_t i=0;i<n;i++)printf(" %u",unsigned(pointer(0x2000+2*i)[0])<<8|pointer(0x2000+2*i)[1]);'
            'printf(" end %04x' + NL + '",unsigned(pointer(0x2000+2*n)[0])<<8|pointer(0x2000+2*n)[1]);}}' + nl)
    out = run(build(index(a.recomp), ['sub_' + verts, 'sub_' + idxs], body, a.out, 'cone_mesh', stubs=stubs, vector=True, flags=('-msse4.1', '-mssse3')), a.image)
    assert out.count('CASE') == len(CASES) and out.count('IDX') == len(CASES), out[:400]
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(out)
    print(len(CASES), 'cases', out.count('POS'), 'vertices')


if __name__ == '__main__':
    main()
