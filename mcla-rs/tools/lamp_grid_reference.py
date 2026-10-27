"""The night lamp grid's cell lists decoded with the original's own lookup (F-25968) for every cell: the two recorded tables
(`@r3+8C`, `@r3+CC`, 327,680 words each) go into the flat image, the lookup runs once per cell with the cell centre as the
place (scale 1 / width, bias 0), and its four u16 are written as one line per cell. Output: <out>.txt, "cell e0 e1 e2 e3".
    python lamp_grid_reference.py --recomp D:/MCLA-static --image <image> --dump c3.bin --out cells.txt
"""
import argparse
import struct
import subprocess
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
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ad = addr(a.tool, 'F-25968')
    c = list(calls(a.dump))[0]
    ob = c.before[c.args[0] & 0xFFFFFFFF]
    w, h = struct.unpack_from('>II', ob, 8)
    t1 = c.before[struct.unpack_from('>I', ob, 0x8C)[0]]
    t2 = c.before[struct.unpack_from('>I', ob, 0xCC)[0]]
    a.work.mkdir(parents=True, exist_ok=True)
    (a.work / 'tables.bin').write_bytes(t1 + t2)
    n = w * h
    nl = chr(10)
    NL = chr(92) + 'n'
    T1, T2 = 0x2000, 0x2000 + 4 * n
    scratch_end = T2 + 4 * n + 0x1000
    body = ('std::ifstream tb(argv[2],std::ios::binary);std::vector<char> tv((std::istreambuf_iterator<char>(tb)),{});' + nl +
            'for(uint32_t i=0;i<2*%d;i++){uint32_t v;memcpy(&v,&tv[4*i],4);v=__builtin_bswap32(v);store(0x2000+4*i,v);}' % n + nl +
            'float sx=1.0f/%d,sz=1.0f/%d;' % (w, h) + nl +
            'for(int cz=0;cz<%d;cz++)for(int cx=0;cx<%d;cx++){' % (h, w) +
            'store(0x1000,128);store(0x1008,%d);store(0x100c,%d);store(0x108c,%d);store(0x10cc,%d);' % (w, h, T1, T2) +
            'uint32_t b;memcpy(&b,&sx,4);store(0x10f0,b);memcpy(&b,&sz,4);store(0x10f8,b);store(0x1100,0);store(0x1108,0);' +
            'float px=cx+0.5f,pz=cz+0.5f;float fx=px/%d.0f*0+px,fz=pz;' % w +
            'memcpy(&b,&fx,4);store(0x1200,b);memcpy(&b,&fz,4);store(0x1208,b);' +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1300;ctx.r5.u32=0x1200;ctx.r6.u32=0;ctx.r1.u32=0x1F00;' +
            ('sub_%s(ctx,nullptr);printf("%%u %%u %%u %%u%s",load16(0x1300),load16(0x1302),load16(0x1304),load16(0x1306));}' % (ad, NL)) + nl)
    # scale: u = x * sx, so x = cx + 0.5 gives u = (cx + 0.5) / w
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    plain = recomp_closure.vector_shim
    recomp_closure.vector_shim = lambda: plain().replace('objects[8192]', 'objects[%d]' % (scratch_end - 0x1000)).replace('a<12288', 'a<%d' % scratch_end)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + ad], main_src, a.work, 'lamp_grid', vector=True, scratch_end=scratch_end)
    out = subprocess.run([str(exe), str(a.image), str(a.work / 'tables.bin')], capture_output=True, text=True, check=True).stdout
    a.out.write_text(out)
    print(len(out.split(nl)), 'cells')


if __name__ == '__main__':
    main()
