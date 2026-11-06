"""The five specialAbility tune objects' static constructors run offline (zone F-32980 and its four twins): the original's code,
unchanged, with its closure (base tune constructor, sub-object constructor), on the image's static objects. The 52 words of each
object (208 bytes) after the constructor are the defaults a tune file that leaves a key out gets.
Objects: zone 8288DA60, agro 8288D790, roar 8288DB30, pulse 8288D890, grip 8288D990 (208 bytes each).
The CRT atexit registration (sub_823D9A98: registers the static destructor) is stubbed: it writes no field of the object.
Output: one line per ability: its name then the 52 words in hex.
Usage: pu_ability_tune_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE"""
import argparse
from pathlib import Path
from recomp_closure import index, build, run

CTORS = [('zone', '827AD6B8', 0x8288DA60), ('agro', '827AD850', 0x8288D790), ('roar', '827AD9E8', 0x8288DB30),
         ('pulse', '827ADB80', 0x8288D890), ('grip', '827ADD18', 0x8288D990)]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    a = ap.parse_args()
    idx = index(a.recomp)
    lines = []
    for n, addr, obj in CTORS:
        fn = 'sub_' + addr
        main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                    'image.assign(std::istreambuf_iterator<char>(bin),{});\n'
                    ' std::memset(objects,0,sizeof(objects));Context ctx{};ctx.r1.u32=0x2800;\n'
                    ' %s(ctx,nullptr);for(int i=0;i<52;i++)std::printf("%%s%%08x",i?" ":"",load(0x%x+4*i));std::puts("");}\n' % (fn, obj))
        exe = build(idx, [fn], main_src, a.work, 'pu_abil_' + n, vector=True, flags=['-msse4.1'], stubs=['sub_823D9A98'])
        out = run(exe, a.image).strip()
        assert len(out.split()) == 52, out
        lines.append(n + ' ' + out)
        print(lines[-1])
    a.out.write_text('\n'.join(lines) + '\n')


if __name__ == '__main__':
    main()
