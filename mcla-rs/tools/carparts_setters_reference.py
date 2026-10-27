"""Run three one-instruction-group car-part accessors of the original offline and print what they write / return.

F-50602 (damage map full reset: +2150 = 1, +2154 = 0), F-74993 (repair request: +2149 = 1), F-45964 (options byte at +1228).
Object filled with 0xAA first; prints the offsets whose byte changed (with the new value) or the returned r3.
"""
import argparse
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = index(a.recomp)
    reset, repair, opt = (addr(a.tool, f) for f in ['F-50602', 'F-74993', 'F-45964'])
    body = ''
    for name, fn in (('reset', reset), ('repair', repair)):
        body += ('{memset(objects,0xAA,sizeof(objects));Context c{};c.r3.u32=0x1000;c.r1.u32=0x2800;sub_%s(c,nullptr);'
                 'printf("%s");for(int i=0;i<0x1000;i++)if(objects[i]!=0xAA)printf(" %%d=%%d",i,objects[i]);puts("");}\n' % (fn, name))
    for v in (0, 1, 2, 0x80, 0xff):
        body += ('{memset(objects,0,sizeof(objects));objects[0x100+1228]=%d;Context c{};c.r3.u32=0x1000;c.r4.u32=0x100;c.r1.u32=0x2800;'
                 'sub_%s(c,nullptr);printf("option %d -> %%u\\n",c.r3.u32);}\n' % (v, opt, v))
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});\n' + body + '}\n')
    exe = build(idx, [f'sub_{x}' for x in (reset, repair, opt)], main_src, a.out, 'carparts_setters')
    print(run(exe, a.image))


if __name__ == '__main__':
    main()
