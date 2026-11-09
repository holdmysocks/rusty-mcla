"""The cheat gate F-68910 run offline as generated, over every combination of the seven cheat bytes it can read
(the cheat block's +0 nitro, -9 never damage out, +1 specials, +2 top down view, +4 no police, +5, +6 no fines) and
the race editor's three flags (its object, the word the gate reads, at 0x4000: +156 overhead view, +13 editing mode;
the byte of its test drive). Writes <out-dir>/cheat_gate_ref.txt: "b0 a7 b1 b2 b4 b5 b6 e156 ebyte e13 -> r"."""
import argparse
import subprocess
from pathlib import Path
from recomp_closure import index, build, run

CHEAT = (0x8289 << 16) - 6736  # the cheat block, as the gate's lis / offset (docs/CAREER_RACES.md)
EDITOR_PTR = (0x8287 << 16) - 6148  # the race editor object's word
EDITOR_BYTE = (0x8287 << 16) - 6144  # its test drive byte


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = subprocess.run([a.tool, 'registry', 'addr', 'F-68910'], capture_output=True, text=True, check=True).stdout.split()[2][4:]
    nl = chr(10)
    offs = [0, -9, 1, 2, 4, 5, 6]
    put = ''.join('*pointer(0x%X)=(k>>%d)&1;' % (CHEAT + o, i) for i, o in enumerate(offs))
    body = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(int k=0;k<1024;k++){memset(objects,0,sizeof(objects));' + put +
            'store(0x%X,0x4000);*pointer(0x4000+156)=(k>>7)&1;*pointer(0x%X)=(k>>8)&1;*pointer(0x4000+13)=(k>>9)&1;' % (EDITOR_PTR, EDITOR_BYTE) + nl +
            'Context ctx{};ctx.r1.u32=0x7F00;sub_' + f + '(ctx,nullptr);printf("%d' + chr(92) + 'n",(int)(ctx.r3.u32&0xFF));}' + nl + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + f], body, a.out_dir, 'cheat_gate', scratch_end=0x8000)
    out = run(exe, a.image).split(nl)[:1024]
    assert len(out) == 1024
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'cheat_gate_ref.txt').write_text(''.join(' '.join(str((k >> b) & 1) for b in range(10)) + ' -> ' + o + nl for k, o in enumerate(out)))
    print(1024, 'cases', sum(o == '1' for o in out), 'gated')


if __name__ == '__main__':
    main()
