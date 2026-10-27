"""The night lamp grid's cell lookup (F-25968) run offline: unchanged generated code on flat guest memory.

Both tables are filled so that the four u16 it returns encode the table index it read (m = 256 in the object's +0:
out0 = (w >> 8 & 255) * 256 + (w >> 17 & 127), out1 = (w >> 24 & 255) * 256 + (w >> 1 & 127); the word at index i is
(i & 255) << 8 | (i >> 8 & 127) << 17 | (i >> 15 & 255) << 24). So the output gives the cell the original reads.
Cases come from the object-dump recording of the lookup (cap file, `cells` = the object's grid size, scale / bias (+240 / +256) and the
position vector of each call). Writes <out-dir>/lamp_cell_ref.txt:
    "<W> <H> <scale xyz bits> <bias xyz bits> <pos xyz bits> <index>"   (floats as hex bits)
"""
import argparse
import struct
import subprocess
import sys
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run
from objdump_read import calls


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--dump', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ad = addr(a.tool, 'F-25968')
    cases = []
    for c in calls(a.dump, int(addr(a.tool, 'F-25968'), 16)):
        ob = c.before[c.args[0] & 0xFFFFFFFF]
        w, h = struct.unpack_from('>II', ob, 8)
        sc = struct.unpack_from('>4I', ob, 240)
        bi = struct.unpack_from('>4I', ob, 256)
        pos = struct.unpack_from('>4I', c.before[c.args[2] & 0xFFFFFFFF], 0)
        cases.append((w, h, sc, bi, pos))
    # more places: the recorded grid object and positions all over, around the edges and beyond, and odd floats
    import random
    rnd = random.Random(7)
    fb = lambda x: struct.unpack('>I', struct.pack('>f', x))[0]
    w, h, sc, bi, _ = cases[0]
    wild = [float('nan'), float('inf'), -float('inf'), 1e30, -1e30, 0.0, -0.0]
    for _ in range(500):
        x = rnd.uniform(-4500, 3800) if rnd.random() < 0.9 else rnd.choice(wild)
        z = rnd.uniform(-3200, 2900) if rnd.random() < 0.9 else rnd.choice(wild)
        cases.append((w, h, sc, bi, (fb(x), fb(rnd.uniform(-50, 120)), fb(z), 0)))
    # cell edges: the places where x * scale - bias times the grid size is a whole number, +- 1 ulp
    for k in range(200):
        kx, kz = rnd.randint(0, w), rnd.randint(0, h)
        x = (kx / w + struct.unpack('>f', struct.pack('>I', bi[0]))[0]) / struct.unpack('>f', struct.pack('>I', sc[0]))[0]
        z = (kz / h + struct.unpack('>f', struct.pack('>I', bi[2]))[0]) / struct.unpack('>f', struct.pack('>I', sc[2]))[0]
        for d in (-1, 0, 1):
            cases.append((w, h, sc, bi, (fb(x) + d & 0xFFFFFFFF, 0, fb(z) + rnd.choice((-1, 0, 1)) & 0xFFFFFFFF, 0)))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%d,%d,%s},' % (w, h, ','.join('0x%08x' % x for x in list(sc[:3]) + list(bi[:3]) + list(pos[:3]))) for w, h, sc, bi, pos in cases)
    body = ('static const uint32_t C[][11]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));store(0x1000,256);store(0x1008,c[0]);store(0x100c,c[1]);store(0x108c,0x2000);store(0x10cc,0x2000);' +
            'for(int k=0;k<3;k++){store(0x10f0+4*k,c[2+k]);store(0x1100+4*k,c[5+k]);store(0x1200+4*k,c[8+k]);}' +
            'for(uint32_t i=0;i<c[0]*c[1];i++)store(0x2000+4*i,((i&255)<<8)|(((i>>8)&127)<<17)|(((i>>15)&255)<<24));' +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1300;ctx.r5.u32=0x1200;ctx.r6.u32=0;ctx.r1.u32=0x1F00;' +
            ('sub_%s(ctx,nullptr);printf("%%u %%u%s",load16(0x1300),load16(0x1302));}' % (ad, NL)) + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    scratch_end = 0x2000 + 4 * max(w * h for w, h, *_ in cases) + 0x1000
    plain = recomp_closure.vector_shim
    # (vector mode ignores build's scratch_end: widen the flat scratch area here)
    recomp_closure.vector_shim = lambda: plain().replace('objects[8192]', 'objects[%d]' % (scratch_end - 0x1000)).replace('a<12288', 'a<%d' % scratch_end)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + ad], main_src, a.out_dir, 'lamp_cell', vector=True, scratch_end=0x2000 + 4 * max(w * h for w, h, *_ in cases) + 0x1000)
    out = run(exe, a.image).split(nl)
    lines = []
    for (w, h, sc, bi, pos), o in zip(cases, [x for x in out if x]):
        v = [int(t) for t in o.split()]
        i = (v[0] >> 8) | ((v[0] & 255) << 8) | (((v[1] >> 8) & 15) << 15)
        lines.append('%d %d %s %s %s %d' % (w, h, ' '.join('%08x' % x for x in sc[:3]), ' '.join('%08x' % x for x in bi[:3]),
                                              ' '.join('%08x' % x for x in pos[:3]), i))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'lamp_cell_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
