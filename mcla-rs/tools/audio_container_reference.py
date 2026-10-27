"""The sound config container's lookups run offline (unchanged generated code): by hash (F-17925 -> F-28348, a binary search of the 8-byte entries
(offset, hash) sorted by hash, answering base + offset or 0 for key 0 / absent) and by name (F-86504 -> F-73434: the game's string hash of the name,
F-58929, then the same search). Engine object r3: the container at +700: +0 base, +60 last index, +64 table. Objects are 8 bytes at base + 8 i.
Line:  CNT <n> <names..> | <queries> -> results (object index, or -1 for 0)      queries: H<hash> (by hash) or N<name> (by name)
Usage: audio_container_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import re
import subprocess
from pathlib import Path
import recomp_dispatch as rd

IDS = {'BYHASH': 'F-17925', 'BYNAME': 'F-86504'}

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);int n;const uint32_t ENG=0x40010000,CONT=ENG+700,BASE=0x40020000,TBL=0x40011000,STR=0x40012000;
 while(in>>n){rt_reset();store(CONT,BASE);store(CONT+60,uint32_t(n-1));store(CONT+64,TBL);
  for(int i=0;i<n;i++){uint32_t off,h;in>>std::hex>>off>>h>>std::dec;store(TBL+8*i,off);store(TBL+8*i+4,h);}
  int q;in>>q;
  for(int i=0;i<q;i++){std::string s;in>>s;Context c=fresh();c.r3.u32=ENG;
   if(s[0]=='H'){c.r4.u32=std::stoul(s.substr(1),nullptr,16);sub_BYHASH(c,nullptr);}
   else{std::string nm=s.substr(1);for(size_t k=0;k<=nm.size();k++)*pointer(STR+k)=k<nm.size()?uint8_t(nm[k]):0;c.r4.u32=STR;sub_BYNAME(c,nullptr);}
   printf(" %d",c.r3.u32?int((c.r3.u32-BASE)/8):-1);}
  printf("\n");}
}
'''


def joaat(s: str) -> int:
    """Jenkins one-at-a-time, lower-cased (the game's string hash for names without '\' or quotes)."""
    h = 0
    for ch in s.lower():
        h = (h + ord(ch)) & 0xFFFFFFFF
        h = (h + (h << 10)) & 0xFFFFFFFF
        h ^= h >> 6
    h = (h + (h << 3)) & 0xFFFFFFFF
    h ^= h >> 11
    h = (h + (h << 15)) & 0xFFFFFFFF
    return h


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    addr = {k: 'sub_' + subprocess.run([a.tool, 'registry', 'addr', v], capture_output=True, text=True, check=True).stdout.split()[1] for k, v in IDS.items()}
    src = MAIN
    for k, v in addr.items():
        src = src.replace('sub_' + k, v)
    exe = rd.build(idx, list(addr.values()), src, a.work, 'audio_container', image=rd.load_image(a.image))
    rnd = random.Random(2034)
    alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_'
    cases = []
    for _ in range(250):
        n = rnd.choice([1, 2, 3, 5, 8, 13, 40, 100])
        names = set()
        while len(names) < n:
            names.add(''.join(rnd.choice(alphabet) for _ in range(rnd.randint(1, 24))))
        names = list(names)
        rnd.shuffle(names)                       # object i = names[i], at offset 8 i
        ents = sorted((joaat(nm), 8 * i) for i, nm in enumerate(names))
        if len({h for h, _ in ents}) != n:
            continue
        qs = [f'H{joaat(nm):08x}' for nm in names[:12]] + [f'N{nm}' for nm in names[:12]] + [f'N{nm.swapcase()}' for nm in names[:4]]
        qs += ['H00000000', f'H{rnd.getrandbits(32):08x}', f'H{ents[0][0] - 1 & 0xFFFFFFFF:08x}', f'H{ents[-1][0] + 1 & 0xFFFFFFFF:08x}', 'N' + ''.join(rnd.choice(alphabet) for _ in range(9))]
        cases.append((names, ents, qs))
    lines = [f'{len(ents)} ' + ' '.join(f'{o:x} {h:x}' for h, o in ents) + f' {len(qs)} ' + ' '.join(qs) for names, ents, qs in cases]
    cf = a.work / 'audio_container_cases.txt'
    cf.write_text('\n'.join(lines) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    rows = p.stdout.splitlines()
    assert len(rows) == len(cases)
    out = []
    for (names, ents, qs), row in zip(cases, rows):
        out.append(f'CNT {len(names)} ' + ' '.join(names) + ' | ' + ' '.join(qs) + ' ->' + row)
    keep = [l for l in out if not re.search(r'82[0-9a-f]{6}', l)]
    a.out.write_text(chr(10).join(keep) + chr(10))
    print(len(keep), 'containers')


if __name__ == '__main__':
    main()
