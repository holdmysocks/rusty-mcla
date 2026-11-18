"""The rolloff curves' FindByHash F-67395 run offline (the generated code whole and unchanged): it calls the container
search F-28348 on the table r3 (+0 base, +60 last index, +64 the 8-byte entries (offset, hash) sorted by hash), stores
the answer at r5 and returns answer != 0. The table is the disc's curves.dat: every object's offset and the game's name
hash (joaat of the lower-case name), sorted by hash, as the loader keeps it.
Queries: every curve's hash, its neighbours +-1, 0, and 2,000 random keys (none with top byte 0x82).
Writes <out-dir>/curve_find_ref.txt: 'Q <hash> -> <r3> <name of the object at *r5, or ->' (hex hash)."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_dispatch as rd
from recomp_closure import index, run
from category_lookup_reference import objects, joaat

TBL = 0x40100000
BASE = 0x40200000
OUT = 0x40110000
STACK = 0x40F00000


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--curves', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = addr(a.tool, 'F-67395')
    objs = objects(a.curves.read_bytes())
    ents = sorted((joaat(n), off, n) for n, off, _ in objs)
    by_off = {off: n for _, off, n in ents}
    r = random.Random(67395)
    qs = [0]
    for h, _, _ in ents:
        qs += [h, (h + 1) & 0xffffffff, (h - 1) & 0xffffffff]
    while len(qs) < 3 * len(ents) + 2001:
        k = r.getrandbits(32)
        if k >> 24 != 0x82:
            qs.append(k)
    tbl = ','.join('%du,%du' % (off, h) for h, off, _ in ents)
    qd = ','.join('%du' % q for q in qs)
    c = r"""
static const uint32_t T[]={@TBL@};
static const uint32_t Q[]={@Q@};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
uint32_t n=sizeof(T)/8;store(@TA@u,@BASE@u);store(@TA@u+60,n-1);store(@TA@u+64,@TA@u+0x100u);
for(uint32_t i=0;i<n;i++){store(@TA@u+0x100u+8*i,T[2*i]);store(@TA@u+0x100u+8*i+4,T[2*i+1]);}
for(size_t i=0;i<sizeof(Q)/4;i++){store(@OUT@u,0x5A5A5A5Au);
 Context c{};c.r1.u32=@STACK@u;c.r3.u32=@TA@u;c.r4.u64=Q[i];c.r5.u32=@OUT@u;sub_@FN@(c,nullptr);
 printf("%08x %x %08x\n",Q[i],c.r3.u32,load(@OUT@u));}}
"""
    for k, v in (('TBL', tbl), ('Q', qd), ('TA', TBL), ('BASE', BASE), ('OUT', OUT), ('STACK', STACK), ('FN', fn)):
        c = c.replace('@%s@' % k, str(v))
    img = rd.load_image(a.image)
    exe = rd.build(index(a.recomp), ['sub_' + fn], c, a.out_dir, 'curve_find', image=img)
    out = [x.split() for x in run(exe, a.image).split(chr(10)) if x]
    assert len(out) == len(qs), (len(out), len(qs))
    lines = []
    for q, r3, p in out:
        p = int(p, 16)
        name = by_off[p - BASE] if p else '-'
        assert (p != 0) == (r3 == '1'), (q, r3, p)
        lines.append('Q %s -> %s %s' % (q, r3, name))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'curve_find_ref.txt').write_text(chr(10).join(lines) + chr(10))
    print(len(ents), 'curves,', len(qs), 'queries,', sum(1 for x in out if x[1] == '1'), 'found')


if __name__ == '__main__':
    main()
