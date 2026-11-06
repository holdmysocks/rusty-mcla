"""The five specialAbility tune objects' static constructors run offline (zone F-32980 and its four twins): the original's code,
unchanged, with its closure (base tune constructor, sub-object constructor), on the image's static objects. The 52 words of each
object (208 bytes) after the constructor are the defaults a tune file that leaves a key out gets.
Objects: zone 8288DA60, agro 8288D790, roar 8288DB30, pulse 8288D890, grip 8288D990 (208 bytes each).
The CRT atexit registration (F-17496: registers the static destructor) is stubbed: it writes no field of the object.
Output: one line per ability: its name then the 52 words in hex.
Usage: pu_ability_tune_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE"""
import argparse
import subprocess
from pathlib import Path
from recomp_closure import index, build, run

# (the type objects' places: data words, split by '_')
CTORS = [('zone', 'F-32980', 0x8288_DA60), ('agro', 'F-39086', 0x8288_D790), ('roar', 'F-50608', 0x8288_DB30),
         ('pulse', 'F-99712', 0x8288_D890), ('grip', 'F-94769', 0x8288_D990)]
ATEXIT = 'F-17496'


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
    idx = index(a.recomp)
    lines = []
    for n, fid, obj in CTORS:
        fn = addr(a.tool, fid)
        main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                    'image.assign(std::istreambuf_iterator<char>(bin),{});\n'
                    ' std::memset(objects,0,sizeof(objects));Context ctx{};ctx.r1.u32=0x2800;\n'
                    ' %s(ctx,nullptr);for(int i=0;i<52;i++)std::printf("%%s%%08x",i?" ":"",load(0x%x+4*i));std::puts("");}\n' % (fn, obj))
        exe = build(idx, [fn], main_src, a.work, 'pu_abil_' + n, vector=True, flags=['-msse4.1'], stubs=[addr(a.tool, ATEXIT)])
        out = run(exe, a.image).strip()
        assert len(out.split()) == 52, out
        lines.append(n + ' ' + out)
        print(lines[-1])
    a.out.write_text('\n'.join(lines) + '\n')


if __name__ == '__main__':
    main()
