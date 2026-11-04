"""Execute the original view frustum builder on prepared matrices.

Runs the unchanged generated F-53768 with the matrix product it calls, F-96281, on a viewport object whose two matrices
(+448 and +384) come from the cases file, after the static initialisers of the vector constants it reads (the same set
gjk_reference.py runs). It forms the product of the two matrices, takes the six planes from its columns (+768 near, +784
left, +800 right, +816 far, +832 top, +848 bottom) and scales each by the reciprocal square root of its normal's length:
the estimate (vrsqrtefp, an exact 1 / sqrt in the recomp) with one Newton-Raphson step (1.0 and 0.5), an infinite estimate
replaced by 0 (docs/reference/engine/frustum_rule.md has the constants' places).

Cases file: one case per line, 32 hexadecimal float32 bit patterns, the matrix at +448 then the one at +384, row by row
(text after a '#' is the case's label).

CSV out: label, the product (16, row by row, as F-96281 writes it), the six planes as stored (24, in the order of
the object: near, left, right, far, top, bottom), all as hexadecimal float32 bit patterns. The constants the
initialisers produced are written to <out>/<name>_trace.txt (CONSTS: their addresses, given by the doc).
"""
import argparse
from pathlib import Path
import re
import subprocess
from recomp_vector import vector_shim
from gjk_reference import find, EXTRA

IDS = ['F-53768', 'F-96281',
       # static initialisers of the vector constants (BSS in the image), as gjk_reference.py
       'F-50173', 'F-33402', 'F-26199', 'F-55749', 'F-97349', 'F-70580', 'F-79721', 'F-64228', 'F-24192', 'F-92824']

HARNESS = r'''
enum { OBJ=0x10000, PROD=0x10800, STACK=0x1F000 };
static void dispatch(Context&,const uint8_t*,uint32_t) { throw std::runtime_error("unexpected indirect call"); }
int main(int argc,char **argv) {
    if(argc<3) return 2;
    std::ifstream bin(argv[1],std::ios::binary);
    image.assign(std::istreambuf_iterator<char>(bin),{});
    if(image.size()!=0x9e0000) return 3;
    Context globals{}; globals.r1.u32=STACK;
    INITCALLS
    std::fprintf(stderr,"constants:");
    for(uint32_t a:{CONSTS})
        std::fprintf(stderr," %08x=%08x,%08x,%08x,%08x",a,load(a),load(a+4),load(a+8),load(a+12));
    std::fputc('\n',stderr);
    std::ifstream cases(argv[2]);
    std::string line; int index=0;
    std::puts("label,product(16),near(4),left(4),right(4),far(4),top(4),bottom(4)");
    while(std::getline(cases,line)) {
        std::string label=std::to_string(index); size_t hash=line.find('#');
        if(hash!=std::string::npos) { label=line.substr(hash+1); line=line.substr(0,hash);
            while(!label.empty() && label[0]==' ') label.erase(0,1); }
        std::istringstream in(line); uint32_t w[32]; int n=0; std::string tok;
        while(n<32 && in>>tok) w[n++]=uint32_t(std::stoul(tok,nullptr,16));
        if(n==0) continue;
        if(n!=32) throw std::runtime_error("a case needs 32 values");
        std::memset(objects,0,sizeof(objects));
        for(int i=0;i<16;i++) { store(OBJ+448+4*i,w[i]); store(OBJ+384+4*i,w[16+i]); }
        // the product alone, as the builder forms it (out, +448, +384)
        { Context c{}; c.r1.u32=STACK; c.r3.u32=PROD; c.r4.u32=OBJ+448; c.r5.u32=OBJ+384; PRODUCT(c,nullptr); }
        Context ctx{}; ctx.r1.u32=STACK; ctx.r3.u32=OBJ;
        BUILDER(ctx,nullptr);
        std::printf("%s",label.c_str());
        for(int i=0;i<16;i++) std::printf(",%08x",load(PROD+4*i));
        for(int i=0;i<24;i++) std::printf(",%08x",load(OBJ+768+4*i));
        std::putchar('\n');
        index++;
    }
}
'''


def reference_code(recomp: Path, tool: str, consts: str):
    r = subprocess.run([tool, 'registry', 'addr', *IDS], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    FUNCS = ['sub_%08X' % addr[f] for f in IDS]
    INITS = FUNCS[2:]
    gen = recomp / 'generated/default'
    sdk = recomp / 'third_party/rexglue-sdk'
    cpp = vector_shim(sdk, 0x20000)
    cpp = cpp.replace('#include <immintrin.h>', '#include <immintrin.h>\n#include <sstream>\n#include <cstdlib>')
    old = 'union alignas(16) Vector { uint8_t u8[16]; uint32_t u32[4]; float f32[4]; };'
    assert old in cpp
    cpp = cpp.replace(old, 'union alignas(16) Vector { uint8_t u8[16]; int8_t s8[16]; uint16_t u16[8]; int16_t s16[8]; '
                      'uint32_t u32[4]; int32_t s32[4]; float f32[4]; };\ntypedef Vector PPCVRegister;' + EXTRA)
    assert 'v63;' in cpp
    cpp = cpp.replace('v63;', 'v63,' + ','.join(f'v{i}' for i in range(64, 128)) + ';', 1)
    old = 'bool eq{}, lt{}, gt{};'
    assert old in cpp
    cpp = cpp.replace(old, old + ' bool so{};\n    void setFromMask(__m128 m,int imm) { int k=_mm_movemask_ps(m); lt=k==imm; gt=0; eq=k==0; so=0; }\n'
                      '    void setFromMask(__m128i m,int imm) { int k=_mm_movemask_epi8(m); lt=k==imm; gt=0; eq=k==0; so=0; }')
    header = (sdk / 'include/rex/ppc/intrinsics.h').read_text()
    mask = re.search(r'inline uint8_t VectorMaskR\[\] = \{.*?\};', header, re.S).group()
    cpp += '\n// ReXGlue SDK VectorMaskR; see its BSD 3-Clause LICENSE.\n' + mask.replace('inline uint8_t', 'alignas(16) static uint8_t') + '\n'
    for name in ['alignr_epi8', 'blend_ps', 'cmpeq_epi32', 'cmpge_ps', 'cmpgt_ps', 'cvtepi16_epi32', 'cvtepi32_ps',
                 'max_ps', 'min_ps', 'setzero_si128', 'unpackhi_epi64', 'xor_si128']:
        cpp += f'#define simde_mm_{name} _mm_{name}\n'
    bodies = [find(gen, name) for name in FUNCS]
    helpers = sorted(set(re.findall(r'\b(__(?:save|rest)\w+)\(ctx, base\)', '\n'.join(bodies))))
    bodies += [find(gen, name) for name in helpers]
    for name in FUNCS + helpers:
        cpp += f'void {name}(Context&,const uint8_t*);\n'
    cpp += 'static void dispatch(Context&,const uint8_t*,uint32_t);\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
    for body in bodies:
        cpp += body
    main = HARNESS.replace('INITCALLS', ' '.join(f'{n}(globals,nullptr);' for n in INITS))
    main = main.replace('BUILDER', FUNCS[0]).replace('PRODUCT', FUNCS[1])
    main = main.replace('CONSTS', ','.join('0x%sU' % c for c in consts.split(',') if c) or '0U')
    return cpp + main


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    for arg in ['recomp', 'image', 'out', 'cases']:
        parser.add_argument('--' + arg, type=Path, required=True)
    parser.add_argument('--compiler', required=True)
    parser.add_argument('--name', default='frustum')
    parser.add_argument('--tool', default='mcla-tool')
    parser.add_argument('--consts', default='', help='comma-separated hex addresses of constants to print to the trace')
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    exe = args.out / 'frustum_original.exe'
    path = args.out / 'frustum_original.cpp'
    path.write_text(reference_code(args.recomp, args.tool, args.consts))
    subprocess.run([args.compiler, '-std=c++17', '-O2', '-msse4.1', '-mssse3', '-ffp-contract=off', '-fmax-errors=8', '-w',
                    '-static', str(path), '-o', str(exe)], check=True)
    result = subprocess.run([str(exe), str(args.image), str(args.cases)], capture_output=True)
    (args.out / f'{args.name}_trace.txt').write_bytes(result.stderr)
    if result.returncode:
        raise SystemExit(f'the reference executable failed ({result.returncode}): {result.stderr[-400:]!r}')
    out = args.out / f'{args.name}_reference.csv'
    out.write_bytes(result.stdout)
    print(result.stdout.count(b'\n') - 1, 'cases:', out)


if __name__ == '__main__':
    main()
