"""Execute the original's power-up type constructors offline and print the fields the port models.

Each constructor runs unchanged (with its direct callees) on a zeroed object; the base constructor's two name
callees (atString init, string-table lookup) are stubbed because they only write the name fields (+60, +100).
Constructors whose closure uses vector instructions (Ice, MegaNitro, MegaDisrupter ...) do not compile here and are skipped.
Output rows are the table of `constructors_match_the_original` in crates/game/src/powerups.rs.
"""
import argparse
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def name(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2]

# registry IDs (`mcla-tool registry addr F-#####`): the base constructor, then the per-type constructors
CTORS = ['F-34625', 'F-86562', 'F-47630', 'F-88704', 'F-89556', 'F-17229', 'F-43339', 'F-41737', 'F-59527',
         'F-56106', 'F-77222', 'F-98423', 'F-44279']
STUBS = ['F-89448', 'F-11670']  # atString init, string-table lookup: they only fill the name fields


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool', help='mcla-tool, to resolve F-ids')
    a = ap.parse_args()
    idx = index(a.recomp)
    ctors = [name(a.tool, f) for f in CTORS]
    stubs = [name(a.tool, f) for f in STUBS]
    body = ''.join('''{std::memset(objects,0,sizeof(objects));Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x2400;ctx.r1.u32=0x2800;
 sub_%s(ctx,nullptr);std::printf("%s");for(int i=0;i<0x80;i++)std::printf(" %%02x",objects[i]);std::puts("");}
''' % (c[4:], c[4:]) for c in ctors)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});\n' + body + '}\n')
    exe = build(idx, ctors, main_src, a.out, 'powerup_ctor', stubs)
    for line in run(exe, a.image).splitlines():
        p = line.split()
        b = bytes(int(x, 16) for x in p[1:])
        u = lambda o: struct.unpack('>I', b[o:o + 4])[0]
        print('(%d, %d, 0x%08x, %d, 0x%08x, 0x%08x, 0x%08x, %d),' % (
            u(4), u(8), u(16), b[20], u(24), u(28), u(32), b[36]))


if __name__ == '__main__':
    main()
