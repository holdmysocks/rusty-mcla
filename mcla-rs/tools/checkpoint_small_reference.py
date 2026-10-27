"""Run small checkpoint functions of the original offline (static recompilation, native, flat guest memory) and print
what they write. Records (one per line, hex words are big-endian guest words):
  CTOR  <232 bytes>        the mcCP constructor's object bytes 0..231 (unwritten bytes stay aa; base constructor stubbed)
  REG   <type> <name> <offset> <r7> <r8>   the member registration, one line per registered member (registrar = recording stub)
  DIR   <flag> <string>    the tune folder name for the dev flag 0 / 1
  LOOK  <x> <z> <flag byte> <3 words>      Checkpoint_SetArrowLookatPosition: input bits, +331 and the vector at +144
  LPOS  <in 4 words> <out 4 words>         light set-position wrapper (out = what the light manager's setter is handed)
  LCOL  <in 4 words> <out 4 words>         light set-colour wrapper
Test: `checkpoints::tests::small_functions_match_the_original` (crates/game/tests/data/checkpoint_small_reference.txt).
"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run

HEAD = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});\n'


def bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    nm = lambda f: subprocess.run([a.tool, 'registry', 'addr', f], capture_output=True, text=True, check=True).stdout.split()[2]
    ctor, reg, dirn, look, lpos, lcol = (nm(f) for f in ('F-26586', 'F-15093', 'F-19793', 'F-27552', 'F-63326', 'F-39031'))
    base_ctor, registrar, thunk_pos, thunk_col = (nm(f) for f in ('F-97730', 'F-16967', 'F-75275', 'F-91173'))
    idx = index(a.recomp)
    rnd = random.Random(7)
    vals = [0.0, -0.0, 1.5, -2.0, 1e-30, 1e9, float('nan')]
    # --- run 1: constructor, registration, look-at, folder name
    look_src = ''.join(
        '{std::memset(objects,0,sizeof(objects));store(0x1008,0x1100);store(0x1100,0x1200);store(0x1104,0x%08x);store(0x1108,0x%08x);\n'
        ' Context ctx{};ctx.r3.u32=0x1000;ctx.r1.u32=0x2800;sub_%s(ctx,nullptr);\n'
        ' std::printf("LOOK %08x %08x %%02x",*pointer(0x1200+331));for(int i=0;i<3;i++)std::printf(" %%08x",load(0x1200+144+4*i));std::putchar(10);}\n'
        % (bits(x), bits(z), look[4:], bits(x), bits(z)) for x in vals for z in vals)
    body = HEAD + '''
 {std::memset(objects,0xAA,sizeof(objects));Context ctx{};ctx.r3.u32=0x1000;ctx.r1.u32=0x2800;sub_%(ctor)s(ctx,nullptr);
  std::printf("CTOR");for(int i=0;i<232;i++)std::printf(" %%02x",objects[i]);std::putchar(10);}
 {std::memset(objects,0,sizeof(objects));Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1800;ctx.r1.u32=0x2800;sub_%(reg)s(ctx,nullptr);}
 %(look)s
 for(int flag=0;flag<2;flag++){store(0x8286EF98,flag);Context ctx{};ctx.r1.u32=0x2800;sub_%(dirn)s(ctx,nullptr);
  std::printf("DIR %%d %%s",flag,(const char*)pointer(ctx.r3.u32));std::putchar(10);}
}
''' % dict(ctor=ctor[4:], reg=reg[4:], look=look_src, dirn=dirn[4:])
    stubs = {registrar: 'std::printf("REG %d %s %u %u %u",int(ctx.r4.u32),(const char*)pointer(ctx.r5.u32),ctx.r6.u32-0x1000,'
                        'ctx.r7.u32,ctx.r8.u32);std::putchar(10);',
             base_ctor: ''}
    out = [run(build(idx, [ctor, reg, look, dirn], body, a.out, 'cp_small', stubs, vector=True), a.image)]
    # --- runs 2 and 3: the light wrappers; the light manager's setter (a thunk) records the vector it is handed
    vec_cases = [[rnd.uniform(-1000, 1000) for _ in range(4)] for _ in range(300)] + [[0.0] * 4, [1.0, 2.0, 3.0, 4.0]]
    for tag, fn, thunk in (('LPOS', lpos, thunk_pos), ('LCOL', lcol, thunk_col)):
        src = ''.join(
            '{std::memset(objects,0,sizeof(objects));store(0x1300,0x%08x);store(0x1304,0x%08x);store(0x1308,0x%08x);store(0x130c,0x%08x);\n'
            ' std::printf("%s");for(int i=0;i<4;i++)std::printf(" %%08x",load(0x1300+4*i));\n'
            ' Context ctx{};ctx.r5.u32=0x1300;ctx.r1.u32=0x2800;sub_%s(ctx,nullptr);}\n'
            % (*[bits(v) for v in c], tag, fn[4:]) for c in vec_cases)
        stub = {thunk: 'for(int i=0;i<4;i++)std::printf(" %08x",load(ctx.r5.u32+4*i));std::putchar(10);'}
        out.append(run(build(idx, [fn], HEAD + src + '}\n', a.out, 'cp_' + tag.lower(), stub, vector=True), a.image))
    a.ref.write_text(''.join(out))


if __name__ == '__main__':
    main()
