"""The garage's doors change run offline: the original's F-67605 (the DOORS part's slot 16 applied by F-59896), the
generated code whole and unchanged, on part values. Stubs: the player lookup F-65049 hands a player whose car record
leads to a car-parts record, the frontend sound by name F-42664 prints the name it is given, the scissor-door setter
F-98860 prints its flag.
Writes <out-dir>/door_sound_ref.txt, one value a line: the part value (hex) '->' the sound's name and the flag."""
import argparse
import subprocess
from pathlib import Path
import recomp_dispatch as rd
from recomp_closure import index, run

PLAYER = 0x40108000
CARREC = 0x40109000
STACK = 0x40F00000
VALUES = [0, 1, 2, 3, 4, 5, 7, 8, 15, 16, 255, 256, 0x7FFFFFFF, 0x80000000, 0xFFFFFFFF, 0xFFFFFFFE]


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    st = lambda k: 'sub_' + addr(a.tool, k)
    nl = chr(10)
    stubs = {
        st('F-65049'): 'ctx.r3.u32=%du;' % PLAYER,
        st('F-42664'): 'printf(" %s",(const char*)pointer(ctx.r3.u32));',
        st('F-98860'): 'printf(" %d",(int)ctx.r4.u32);',
    }
    main_src = (
        'static const uint32_t V[]={' + ','.join('%du' % v for v in VALUES) + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'store(%du+48,%du);store(%du+132,%du);' % (PLAYER, CARREC, CARREC, CARREC + 0x100) +
        'for(size_t i=0;i<sizeof(V)/4;i++){printf("%%08x ->",V[i]);'
        '{Context ctx{};ctx.r1.u32=%du;ctx.r3.u32=0;ctx.r4.u32=V[i];%s(ctx,nullptr);}' % (STACK, st('F-67605')) +
        'printf("\\n");}}' + nl)
    idx = index(a.recomp)
    exe = rd.build(idx, [st('F-67605')], main_src, a.out_dir, 'door_sound', image=rd.load_image(a.image), stubs=stubs)
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'door_sound_ref.txt').write_text(out)
    print(out)


if __name__ == '__main__':
    main()
