"""The police radio manager's call-sign book run offline: the take F-40359 and the release F-94030, the generated code
whole and unchanged. The manager object: +40 -> the used bit words, +48 -> the released bit words, +56 the cursor,
+232 -> the voice table (+12 the u16 count).
Cases: 400 books of 1 - 64 voices from a random state (used / released bits within the count, cursor within it), then
60 operations each: a take, or a release of a random index (held or not).
Writes <out-dir>/callsign_book_ref.txt: 'B v<count> u<used0> u<used1> r<rel0> r<rel1> c<cursor>' for each book, then per
operation 'T i0 k<returned> u.. u.. r.. r.. c..' or 'R i<index> k0 u.. u.. r.. r.. c..' (hex, the state after it)."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_dispatch as rd
from recomp_closure import index, run

OBJ = 0x40100000
USED = 0x40110000
REL = 0x40110100
TABLE = 0x40120000
STACK = 0x40F00000
BOOKS = 400
OPS = 60


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    take, rel = (addr(a.tool, k) for k in ('F-40359', 'F-94030'))
    r = random.Random(40359)
    rows = []
    for b in range(BOOKS):
        n = 6 if b < 100 else r.randint(1, 64)
        mask = (1 << n) - 1
        used = r.getrandbits(64) & mask if r.random() < 0.7 else (0 if r.random() < 0.5 else mask)
        relb = r.getrandbits(64) & mask if r.random() < 0.5 else 0
        cur = r.randrange(n)
        ops = []
        for _ in range(OPS):
            ops += [1, 0] if r.random() < 0.6 else [2, r.randrange(n)]
        rows.append([n, used & 0xffffffff, used >> 32, relb & 0xffffffff, relb >> 32, cur] + ops)
    W = 6 + 2 * OPS
    data = ','.join(','.join('%du' % x for x in row) for row in rows)
    c = r"""
static const uint32_t C[]={@DATA@};
static void st(){printf(" u%x u%x r%x r%x c%x\n",load(@USED@u),load(@USED@u+4),load(@REL@u),load(@REL@u+4),load(@OBJ@u+56));}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
store(@OBJ@u+40,@USED@u);store(@OBJ@u+48,@REL@u);store(@OBJ@u+232,@TABLE@u);
for(size_t i=0;i<sizeof(C)/4;i+=@W@){const uint32_t*w=C+i;
 store(@TABLE@u+12,w[0]<<16|0x5a5au);
 store(@USED@u,w[1]);store(@USED@u+4,w[2]);store(@REL@u,w[3]);store(@REL@u+4,w[4]);store(@OBJ@u+56,w[5]);
 printf("B v%x",w[0]);st();
 for(int k=0;k<@OPS@;k++){uint32_t op=w[6+2*k],x=w[7+2*k];Context c{};c.r1.u32=@STACK@u;c.r3.u32=@OBJ@u;
  if(op==1){sub_@TAKE@(c,nullptr);printf("T i0 k%x",c.r3.u32);}
  else{c.r4.u64=x;sub_@REL_FN@(c,nullptr);printf("R i%x k0",x);}
  st();}}}
"""
    for k, v in (('DATA', data), ('OBJ', OBJ), ('USED', USED), ('REL', REL), ('TABLE', TABLE), ('STACK', STACK),
                 ('TAKE', take), ('REL_FN', rel), ('W', W), ('OPS', OPS)):
        c = c.replace('@%s@' % k, str(v))
    img = rd.load_image(a.image)
    exe = rd.build(index(a.recomp), ['sub_' + take, 'sub_' + rel], c, a.out_dir, 'callsign_book', image=img)
    lines = [x for x in run(exe, a.image).split(chr(10)) if x]
    assert len(lines) == BOOKS * (OPS + 1), (len(lines), BOOKS * (OPS + 1))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'callsign_book_ref.txt').write_text(chr(10).join(lines) + chr(10))
    print(len(lines) - BOOKS, 'operations,', BOOKS, 'books')


if __name__ == '__main__':
    main()
