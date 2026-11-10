"""Manual gears F-31769 run offline as generated: the pad object's +4 picks its map in the table the pad maps live in (a
global array of four, filled here by the case); manual when the first words (the devices) of the map's slots 4 SHIFT_UP
(+32) and 5 SHIFT_DOWN (+40) are both non-zero.
Line:  <pad> <slot 4 device> <slot 5 device> -> <manual>
Usage: pad_manual_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import subprocess
from pathlib import Path
import recomp_dispatch as rd

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);
 const uint32_t P=0x40010000,M=0x40011000;
 uint32_t pad,u,d;
 while(in>>pad>>u>>d){rt_reset();
  for(uint32_t k=0;k<4;k++){store(G_MAPS+4*k,M+0x100*k);for(uint32_t x=0;x<0x100;x+=4)store(M+0x100*k+x,0);}
  store(P+4,pad);store(M+0x100*pad+32,u);store(M+0x100*pad+40,d);
  Context cx=fresh();cx.r3.u32=P;sub_MANUAL(cx,nullptr);printf("%u\n",cx.r3.u32);
 }
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = 'sub_' + subprocess.run([a.tool, 'registry', 'addr', 'F-31769'], capture_output=True, text=True, check=True).stdout.split()[1]
    img = rd.load_image(a.image)
    # (the table of the four pads' maps, as the function loads it: lis / addi)
    maps = ((-32121 << 16) + 17292) & 0xFFFFFFFF
    exe = rd.build(rd.index(a.recomp), [fn], MAIN.replace('sub_MANUAL(', fn + '(').replace('G_MAPS', hex(maps) + 'u'), a.work,
                   'pad_manual', image=img)
    vals = [0, 1, 2, 3, 255, 256, 65536, 4294967295]
    lines = [f'{p} {u} {d}' for p in range(4) for u in vals for d in vals]
    cases = a.work / 'pad_manual_cases.txt'
    cases.write_text('\n'.join(lines) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    rows = p.stdout.splitlines()
    assert len(rows) == len(lines), (len(rows), len(lines))
    a.out.write_text(''.join(l + ' -> ' + r + '\n' for l, r in zip(lines, rows)))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
