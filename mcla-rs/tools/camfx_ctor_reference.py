"""The camera effects tune's constructor (camFX, F-25608) run offline: the original's code, unchanged, with its closure (the
base tune constructor and the name set), on a zeroed object. Output: the whole object, 1,216 bytes as 304 words in hex, one
line; the defaults a camera/default file that leaves a key out gets (CamFx::default, CamShake's defaults).
Usage: camfx_ctor_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --tool mcla-tool --work DIR --out FILE"""
import argparse
import subprocess
from pathlib import Path
import recomp_closure as rc
from recomp_closure import index, build, run

# (the name copy uses cr1 / cr7: give the shim's context those condition fields)
_shim = rc.shim
rc.shim = lambda end=0x3000: _shim(end).replace('Condition cr0,cr6;', 'Condition cr0,cr1,cr6,cr7;')

CTOR = 'F-25608'
# (the tune's name set: string copy with new / delete; stubbed - it writes only the name, no default)
NAME_SET = 'F-88806'
OBJ = 0x1000
WORDS = 304  # (1216 bytes: the scalars and the five shake blocks)


def addr(tool, fid):
    return 'sub_' + subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--tool', required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    a = ap.parse_args()
    fn = addr(a.tool, CTOR)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});\n'
                ' Context ctx{};ctx.r1.u32=0x3f00;ctx.r3.u32=0x%x;\n'
                ' %s(ctx,nullptr);for(int i=0;i<%d;i++)std::printf("%%s%%08x",i?" ":"",load(0x%x+4*i));std::puts("");}\n' % (OBJ, fn, WORDS, OBJ))
    exe = build(index(a.recomp), [fn], main_src, a.work, 'camfx_ctor', vector=True, scratch_end=0x4000, stubs=[addr(a.tool, NAME_SET)], flags=['-msse4.1'])
    out = run(exe, a.image).strip()
    assert len(out.split()) == WORDS, out
    a.out.write_text(out + '\n')
    print(out)


if __name__ == '__main__':
    main()
