"""The race editor slot's checkpoint count, run offline as generated: the setter F-84156 (city byte 1043: 205 -> the word at 0
only; 0 -> byte 1042 and the word; any other -> byte 1042 and the word cleared) and the getter F-71298 (city 205 -> the word,
else byte 1042), on every city byte with old words and bytes, the getter read back after the set.
Line:  <city> <old word> <old byte> <n> -> <word> <byte 1042> <get>   (decimal)
Usage: editor_count_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_dispatch as rd

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);
 const uint32_t S=0x40010000;
 uint32_t city,word,byte,n;
 while(in>>city>>word>>byte>>n){rt_reset();
  store(S,word);store(S+1040,(0xCDu<<24)|(0xCDu<<16)|(byte<<8)|city);
  Context cx=fresh();cx.r3.u32=S;cx.r4.u32=n;sub_SET(cx,nullptr);
  uint32_t w=load(S),b=(load(S+1040)>>8)&255;
  Context cg=fresh();cg.r3.u32=S;sub_GET(cg,nullptr);
  printf("%u %u %u\n",w,b,cg.r3.u32);
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
    addr = lambda fid: 'sub_' + subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]
    set_, get = addr('F-84156'), addr('F-71298')
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    exe = rd.build(idx, [set_, get], MAIN.replace('sub_SET(', set_ + '(').replace('sub_GET(', get + '('), a.work, 'editor_count', image=img)
    rnd = random.Random(84156)
    lines = []
    for city in range(256):
        for _ in range(6):
            word = rnd.choice([0, 0, 5, 30, 0xCDCDCDCD, rnd.randint(0, 0xFFFFFFFF)])
            if 80_000_000 <= word < 90_000_000:
                word += 100_000_000   # (no 8-digit decimal starting with 8 in tracked files: it reads as an address)
            byte = rnd.choice([0, 0xCD, rnd.randint(0, 255)])
            n = rnd.choice([0, 2, 3, 17, 30, 31, rnd.randint(0, 255)])
            lines.append(f'{city} {word} {byte} {n}')
    cases = a.work / 'editor_count_cases.txt'
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
