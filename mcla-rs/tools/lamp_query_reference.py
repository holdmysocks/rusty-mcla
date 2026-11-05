"""The night lamp grid's area query (F-79329) run offline: unchanged generated code on flat guest memory.

The grid object as the builder leaves it (512 x 640 cells, the scale / bias floats the lamp cell test checks, both grids
of words at +140 / +204, m = 256 in +0) with words made from a hash of the cell index, so a cell's four lamps are
(a0, a1, b0, b1) = ((h >> 7) & 63, (h >> 13) & 63, (g >> 7) & 63, (g >> 13) & 63), h = i x 2654435761, g = (i + 12345)
x 2246822519 (u32): few distinct ids, so the distinct test and the capacity are hit. The list (r4): +0 the u16 array,
+4 the count, +6 the capacity. Writes <out-dir>/lamp_query_ref.txt:
    "<x bits> <z bits> <half bits> <cap> <count> <id> ..."   (floats as hex bits)
"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ad = addr(a.tool, 'F-79329')
    fb = lambda x: struct.unpack('>I', struct.pack('>f', x))[0]
    rnd = random.Random(11)
    wild = [float('nan'), float('inf'), -float('inf'), 1e30, -1e30, 0.0, -0.0]
    cases = []
    for k in range(3000):
        x = rnd.uniform(-4500, 3800) if rnd.random() < 0.95 else rnd.choice(wild)
        z = rnd.uniform(-3200, 2900) if rnd.random() < 0.95 else rnd.choice(wild)
        if k % 3 == 0:
            # (on a cell edge, +- the half size, +- 1 ulp)
            h = 4.0
            kx, kz = rnd.randint(0, 512), rnd.randint(0, 640)
            x = (kx / 512 - 0.54857594) * 6320.0 + rnd.choice((-h, h))
            z = (kz / 640 - 0.5282554) * 4463.75 + rnd.choice((-h, h))
            xb, zb = fb(x) + rnd.choice((-1, 0, 1)) & 0xFFFFFFFF, fb(z) + rnd.choice((-1, 0, 1)) & 0xFFFFFFFF
        else:
            xb, zb = fb(x), fb(z)
        half = 4.0 if k % 2 else rnd.choice([0.0, 0.5, 4.0, 13.0, 30.0, 60.0, -2.0])
        cap = 32 if k % 2 else rnd.choice([0, 1, 2, 4, 32, 64])
        cases.append((xb, zb, fb(half), cap))
    W, H, GA = 512, 640, 0x10000
    GB = GA + 4 * W * H
    end = GB + 4 * W * H
    nl, NL = chr(10), chr(92) + 'n'
    rows = nl.join('{0x%08x,0x%08x,0x%08x,%d},' % c for c in cases)
    body = ('static const uint32_t C[][4]={' + nl + rows + nl + '};' + nl +
            'store(0x1000,256);store(0x1008,%d);store(0x100c,%d);store(0x108c,%d);store(0x10cc,%d);' % (W, H, GA, GB) +
            'store(0x10f0,0x%08x);store(0x10f8,0x%08x);store(0x1100,0x%08x);store(0x1108,0x%08x);' % (fb(1 / 6320.0), fb(1 / 4463.75), fb(-0.54857594), fb(-0.5282554)) +
            'for(uint32_t i=0;i<%d;i++){uint32_t h=i*2654435761u,g=(i+12345u)*2246822519u;' % (W * H) +
            'store(%d+4*i,(((h>>7)&63)<<17)|(((h>>13)&63)<<1));store(%d+4*i,(((g>>7)&63)<<17)|(((g>>13)&63)<<1));}' % (GA, GB) + nl +
            'for(auto&c:C){store(0x1300,0x1400);store16(0x1304,0);store16(0x1306,c[3]);store(0x1200,c[0]);store(0x1204,0);store(0x1208,c[1]);' +
            'float hf;std::memcpy(&hf,&c[2],4);Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1300;ctx.r5.u32=0x1200;ctx.f1.f64=hf;ctx.r1.u32=0x1F00;' +
            'sub_%s(ctx,nullptr);int n=load16(0x1304);printf("%%d",n);for(int j=0;j<n;j++)printf(" %%d",load16(0x1400+2*j));printf("%s");}' % (ad, NL) + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    plain = recomp_closure.shim
    # (the flat scratch area widened to hold both grids)
    recomp_closure.shim = lambda *_: plain().replace('objects[8192]', 'objects[%d]' % (end - 0x1000)).replace('a<12288', 'a<%d' % end)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + ad], main_src, a.out_dir, 'lamp_query', vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases)
    lines = ['%08x %08x %08x %d %s' % (c + (o,)) for c, o in zip(cases, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'lamp_query_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
