"""The cab's trailer pick F-45988 (called by the spawn init F-77422 right after the cab's hitch draw) run offline: the
generated code whole on flat guest memory, the cab's MWC (+384 / +388) from the case. Stubbed: the type lookup by name
F-54839 (records the name asked; the case says whether the type exists), and the hitch / matrix / place callees
(F-86587, F-45572, F-33765, F-10286, F-15197, F-57591, F-58860, F-59429) with scratch answers; F-81524 (the trailer's
fill) records the shade it is handed (r5, a halfword).

Writes <out-dir>/trailer_pick_ref.txt, a case a line: lo, carry, found; ' -> ' the name's index in the table (0x8282BDDC),
the shade (ffff when not reached), the MWC after (lo, carry) - lo / carry written as two halves 'xxxx_xxxx' (test
`trailer_pick_matches_the_original`)."""
import argparse
import random
import subprocess
from pathlib import Path
import traffic_select_reference  # noqa: F401  (the vector shim's patches)
import recomp_closure
from recomp_closure import index, build

NL = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    ids = ('F-45988', 'F-54839', 'F-86587', 'F-45572', 'F-33765', 'F-10286', 'F-15197', 'F-57591', 'F-58860', 'F-81524', 'F-59429')
    fn, look, ctor, hitch, mat, ang, diff, rot, place, fill, add = (addr(a.tool, f) for f in ids)
    rnd = random.Random(45988)
    cs = []
    for _ in range(a.cases):
        lo = rnd.choice((rnd.getrandbits(32), rnd.getrandbits(31), 0, 0xFFFFFFFF, 0x7FFFFFFF, 0x80000000))
        carry = rnd.choice((rnd.getrandbits(32), rnd.getrandbits(16), 0, 1))
        cs.append([lo, carry, int(rnd.random() < 0.9)])
    recomp_closure.EXTRA += 'static uint32_t NAME,SHADE,FOUND;' + NL
    vec = 'ctx.r3.u64=0x3000;'
    stubs = {'sub_' + look: 'NAME=ctx.r4.u32;ctx.r3.u64=FOUND?0x4000:0;', 'sub_' + ctor: '', 'sub_' + hitch: vec, 'sub_' + mat: vec,
             'sub_' + ang: 'ctx.f1.f64=0;', 'sub_' + diff: 'ctx.f1.f64=0;', 'sub_' + rot: '', 'sub_' + place: vec,
             'sub_' + fill: 'SHADE=load(ctx.r5.u32)>>16;', 'sub_' + add: ''}
    body = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
std::ifstream in(argv[2]);uint32_t lo,carry,found;
while(in>>std::hex>>lo>>carry>>found){memset(objects,0,sizeof(objects));NAME=0;SHADE=0xFFFF;FOUND=found;
  uint32_t C=0x10000;store(C+384,lo);store(C+388,carry);store(C+448,0x5000);store(0x5000+168,0x5100);store(0x4000+168,0x5200);
  Context ctx{};ctx.r1.u32=0x7F000;ctx.r3.u32=C;sub_%s(ctx,nullptr);
  int k=-1;for(int i=0;i<5;i++)if(NAME&&NAME==load(0x8282BDDCu+4*i))k=i;
  uint32_t a=load(C+384),b=load(C+388);
  printf("%%d %%x %%04x_%%04x %%04x_%%04x\n",k,SHADE,a>>16,a&0xFFFF,b>>16,b&0xFFFF);}
}
''' % fn
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fn], body, a.out_dir, 'trailer_pick', stubs=stubs, vector=True, scratch_end=0x80000)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'trailer_pick_in.txt'
    inp.write_text(NL.join(' '.join('%x' % x for x in c) for c in cs) + NL)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True, check=True)
    out = [x for x in r.stdout.split(NL) if x]
    assert len(out) == len(cs), (len(out), len(cs), r.stdout[-300:])
    split = lambda x: '%04x_%04x' % (x >> 16, x & 0xFFFF)
    lines = [' '.join((split(c[0]), split(c[1]), str(c[2]))) + ' -> ' + o for c, o in zip(cs, out)]
    (a.out_dir / 'trailer_pick_ref.txt').write_text(NL.join(lines) + NL)
    print(len(lines), 'cases;', sum(o.split()[1] != 'ffff' for o in out), 'shades drawn')


if __name__ == '__main__':
    main()
