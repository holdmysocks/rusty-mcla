"""The trailer's place in F-45988 (the cab's trailer, called by the spawn init F-77422) run offline: the generated code
whole on flat guest memory, with the real heading F-10286, angle difference F-15197, rotation about y F-57591 and the
hitch getter F-45572 (the type's +168 -> +204 -> +96). Stubbed: the type lookup F-54839 (always found), the request's
defaults F-86587, the cab's matrix F-33765 and place F-58860 (the case's rows), the queue F-59429; the fill F-81524
records the place it is handed (r9, a vector) and its rotation rows (r6).

Writes <out-dir>/trailer_place_ref.txt, a case a line: the cab's z row (x, z), its place (x, y, z), the cab's hitch and
the trailer's (x, y, z each) as float bits; ' -> ' the trailer's place (x, y, z) (test `trailer_place_matches_the_original`)."""
import argparse
import math
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build

NL = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    ids = ('F-45988', 'F-54839', 'F-86587', 'F-33765', 'F-58860', 'F-81524', 'F-59429')
    fn, look, ctor, mat, place, fill, add = (addr(a.tool, f) for f in ids)
    rnd = random.Random(45989)
    cab, trl = (0.0, 1.132, 1.289), (0.0, 1.174, -5.219)
    cs = []
    for k in range(a.cases):
        yaw = rnd.choice((rnd.uniform(-math.pi, math.pi), rnd.choice((0.0, math.pi, -math.pi, math.pi / 2, -math.pi / 2))))
        pitch = rnd.choice((0.0, rnd.uniform(-0.3, 0.3), rnd.uniform(-1.5, 1.5)))
        roll = rnd.choice((0.0, rnd.uniform(-0.3, 0.3)))
        # z row of R_y(yaw) R_x(pitch) R_z(roll) - only its x and z are read; the other rows are carried through
        cp, sp = math.cos(pitch), math.sin(pitch)
        z = (math.sin(yaw) * cp, -sp, math.cos(yaw) * cp)
        if rnd.random() < 0.05:
            z = (rnd.choice((0.0, -0.0)), 1.0, rnd.choice((0.0, -0.0)))
        p = tuple(rnd.uniform(-4000, 4000) if i != 1 else rnd.uniform(-50, 300) for i in range(3))
        ch, th = (cab, trl) if k % 3 else (tuple(rnd.uniform(-8, 8) for _ in range(3)), tuple(rnd.uniform(-8, 8) for _ in range(3)))
        cs.append([fb(z[0]), fb(z[1]), fb(z[2]), *map(fb, p), *map(fb, ch), *map(fb, th), fb(roll)])
    recomp_closure.EXTRA += 'static uint32_t OUT[12];' + NL
    stubs = {'sub_' + look: 'ctx.r3.u64=0x4000;', 'sub_' + ctor: '', 'sub_' + mat: 'ctx.r3.u64=0x6000;',
             'sub_' + place: 'ctx.r3.u64=0x6030;',
             'sub_' + fill: 'for(int i=0;i<3;i++)OUT[i]=load(ctx.r9.u32+4*i);for(int i=0;i<9;i++)OUT[3+i]=load(ctx.r6.u32+16*(i/3)+4*(i%3));',
             'sub_' + add: ''}
    body = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
std::ifstream in(argv[2]);uint32_t w[16];
while(in>>std::hex>>w[0]){for(int i=1;i<16;i++)in>>std::hex>>w[i];memset(objects,0,sizeof(objects));
  uint32_t C=0x10000;store(C+384,0x12345);store(C+388,1);store(C+448,0x5000);store(0x5000+168,0x5100);store(0x4000+168,0x5200);
  store(0x5100+204,0x5400);store(0x5200+204,0x5600);
  for(int i=0;i<3;i++){store(0x5460+4*i,w[6+i]);store(0x5660+4*i,w[9+i]);}
  store(0x6000,0x3F800000);store(0x6014,0x3F800000);
  store(0x6020,w[0]);store(0x6024,w[1]);store(0x6028,w[2]);
  for(int i=0;i<3;i++)store(0x6030+4*i,w[3+i]);
  Context ctx{};ctx.r1.u32=0x7F000;ctx.r3.u32=C;sub_%s(ctx,nullptr);
  for(int i=0;i<3;i++)printf("%%08x ",OUT[i]);printf("|");for(int i=3;i<12;i++)printf(" %%08x",OUT[i]);printf("\n");}
}
''' % fn
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fn], body, a.out_dir, 'trailer_place', stubs=stubs, vector=True, scratch_end=0x80000)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'trailer_place_in.txt'
    inp.write_text(NL.join(' '.join('%x' % x for x in c[:12]) + ' 0 0 0 0' for c in cs) + NL)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True, check=True)
    out = [x for x in r.stdout.split(NL) if x]
    assert len(out) == len(cs), (len(out), len(cs), r.stdout[-300:])
    lines = []
    for c, o in zip(cs, out):
        pos, rows = o.split('|')
        assert rows.split()[6:9] == ['%08x' % x for x in c[:3]], (rows, c)  # the cab's rows handed through
        lines.append(' '.join('%08x' % x for x in (c[0], c[2], *c[3:12])) + ' -> ' + pos.strip())
    (a.out_dir / 'trailer_place_ref.txt').write_text(NL.join(lines) + NL)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
