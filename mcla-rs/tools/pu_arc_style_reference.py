"""The lightning arc style's defaults run offline: the original's style constructor (F-97066), unchanged generated code, on a zeroed
100-byte object. Every type with lightning (Reverse, Disrupter, Stop and their megas) runs it on its ma_ and mi_ styles before the
tune file is read, so these are the values of every key a file leaves out.
Field offsets (the Reverse type's tune registration F-40053, its ma_ block at +112 and mi_ at +224: name -> offset): +0 ColorMin, +16 ColorMax, +32 EndColor
(4 floats each), +48 ArcGrowthMinMax, +56 GrowthSpeedMinMax, +64 FluxTangentRangeMinMax, +72 FluxNormalRangeMinMax,
+80 FlickerSpeedMinMax (2 floats each), +88 Thickness, +92 TexAnimRate (float), +96 Iterations (int).
Output: one line, the 25 words of the object in hex.
Usage: pu_arc_style_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import subprocess
from pathlib import Path
from recomp_closure import index, build, run

STYLE = 'F-97066'


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    name = 'sub_' + subprocess.run([a.tool, 'registry', 'addr', STYLE], capture_output=True, text=True, check=True).stdout.split()[1]
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});\n'
                ' std::memset(objects,0,sizeof(objects));Context ctx{};ctx.r3.u32=0x1000;ctx.r1.u32=0x2800;\n'
                ' %s(ctx,nullptr);for(int i=0;i<25;i++)std::printf("%%s%%08x",i?" ":"",load(0x1000+4*i));std::puts("");}\n' % name)
    exe = build(index(a.recomp), [name], main_src, a.work, 'pu_arc_style')
    out = run(exe, a.image).strip()
    assert len(out.split()) == 25, out
    a.out.write_text(out + '\n')
    print(out)


if __name__ == '__main__':
    main()
