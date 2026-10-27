"""Run small lighting functions of the original offline (static recompilation, native, flat guest memory) and record
their results for the port's tests (crates/game/src/render/testdata/lighting_small_reference.txt).

  HALF   <f bits> <half>                         F-70220, the lamp table's half writer (cut-off float -> half)
  CSCORE <flag> <term> <lx> <lz> <px> <pz> <out>  F-47404, the lamp grid's cell score (term / d^2, d flat)
  SCORE  <flag> <pos x y z> <dir x y z> <strength> <scale> <bias> <p x y z> <out>   F-60154, a light's score at a point
  FVF    <mask> <idx> <tex> <tan> <bin>          F-16058 / F-88844 / F-36981: HasTexCoord / HasTangent / HasBinormal

python lighting_small_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file>
"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2]


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


HEAD = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});\n'
OUTF = 'float r=float(ctx.f1.f64);uint32_t rb;memcpy(&rb,&r,4);'


def table(name, rows):
    n = len(rows[0])
    return 'static const uint32_t %s[][%d]={' % (name, n) + ','.join('{' + ','.join('0x%08xu' % v for v in r) + '}' for r in rows) + '};\n'


def floats(rnd, n, lo, hi):
    return [rnd.uniform(lo, hi) for _ in range(n)]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = index(a.recomp)
    half, cscore, score, ftex, ftan, fbin = (addr(a.tool, f)[4:] for f in ('F-70220', 'F-47404', 'F-60154', 'F-16058', 'F-88844', 'F-36981'))
    rnd = random.Random(27)
    out = []
    # --- half writer
    sp = [0.0, -0.0, 6.103515625e-05, -6.103515625e-05, 6.1e-05, -6.1e-05, 65504.0, -65504.0, 65505.0, 70000.0, -1e9, 1e30,
          float('inf'), float('-inf'), float('nan'), 1.0, -1.0, 0.5, 800.3, -3467.0, 2.0 ** -14 * 1.0000001]
    vals = sp + floats(rnd, 3000, -5000, 5000) + floats(rnd, 1000, -2, 2) + [rnd.uniform(-1, 1) * 10.0 ** rnd.uniform(-8, 6) for _ in range(2000)]
    body = table('C', [[fbits(v)] for v in vals]) + HEAD + (
        'for(auto&c:C){float v;memcpy(&v,&c[0],4);Context ctx{};ctx.f1.f64=v;ctx.r3.u32=0x1100;ctx.r1.u32=0x2800;sub_%s(ctx,nullptr);'
        'printf("HALF %%08x %%04x\\n",c[0],unsigned(load16(0x1100)));}}\n') % half
    out.append(run(build(idx, ['sub_' + half], body, a.out, 'half'), a.image))
    # --- cell score
    rows = []
    for k in range(4000):
        lx, lz = rnd.uniform(-3467, 2853), rnd.uniform(-2358, 2105)
        px, pz = lx + rnd.uniform(-60, 60) * rnd.choice([1, 0.01, 1e-4]), lz + rnd.uniform(-60, 60) * rnd.choice([1, 0.01, 1e-4])
        if k % 97 == 0:
            px, pz = lx, lz
        term = rnd.choice([rnd.uniform(0, 100), rnd.uniform(0, 1e4), 0.4])
        rows.append([rnd.choice([1, 1, 1, 0]), fbits(term), fbits(lx), fbits(lz), fbits(px), fbits(pz)])
    body = table('C', rows) + HEAD + (
        'for(auto&c:C){memset(objects,0,sizeof(objects));*pointer(0x1000+20)=uint8_t(c[0]);store(0x1010,c[1]);store(0x1020,c[2]);store(0x1028,c[3]);'
        'store(0x1200,c[4]);store(0x1208,c[5]);Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1200;ctx.r1.u32=0x2800;sub_%s(ctx,nullptr);' + OUTF +
        'printf("CSCORE %%u %%08x %%08x %%08x %%08x %%08x %%08x\\n",c[0],c[1],c[2],c[3],c[4],c[5],rb);}}\n') % cscore
    out.append(run(build(idx, ['sub_' + cscore], body, a.out, 'cscore'), a.image))
    # --- light score
    rows = []
    for k in range(4000):
        pos = [rnd.uniform(-3000, 3000), rnd.uniform(-30, 80), rnd.uniform(-2000, 2000)]
        p = [pos[i] + rnd.uniform(-40, 40) * rnd.choice([1, 0.1, 1e-3]) for i in range(3)]
        if k % 101 == 0:
            p = list(pos)
        d = [rnd.uniform(-1, 1) for _ in range(3)]
        n = sum(x * x for x in d) ** 0.5
        d = [x / n for x in d]
        st = rnd.choice([500.0, 20.0, 40.0, 60.0, 70.0, rnd.uniform(0, 1000)])
        sc, bi = rnd.choice([(0.0, 1.0), (4.4696, -2.872), (rnd.uniform(0, 30), rnd.uniform(-30, 1))])
        rows.append([rnd.choice([1, 1, 1, 0])] + [fbits(x) for x in pos + d + [st, sc, bi] + p])
    body = table('C', rows) + HEAD + (
        'for(auto&c:C){memset(objects,0,sizeof(objects));for(int i=0;i<3;i++){store(0x1000+4*i,c[1+i]);store(0x1020+4*i,c[4+i]);store(0x1200+4*i,c[10+i]);}'
        'store(0x1040,c[7]);store(0x104c,c[8]);store(0x1050,c[9]);*pointer(0x1000+89)=uint8_t(c[0]);'
        'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1200;ctx.r1.u32=0x2800;sub_%s(ctx,nullptr);' + OUTF +
        'printf("SCORE");for(int i=0;i<13;i++)printf(" %%08x",c[i]);printf(" %%08x\\n",rb);}}\n') % score
    out.append(run(build(idx, ['sub_' + score], body, a.out, 'score', vector=True), a.image))
    # --- vertex format bits
    rows = [[rnd.getrandbits(32) if k % 3 else rnd.choice([0, 0xffffffff, 1 << rnd.randrange(32)]), rnd.randrange(0, 10)] for k in range(600)]
    rows = [r for r in rows if r[0] >> 24 != 0x82]  # (a mask that reads like an image address would trip the citation check)
    body = table('C', rows) + HEAD + (
        'for(auto&c:C){store(0x1000,c[0]);unsigned o[3];int j=0;for(auto f:{sub_%s,sub_%s,sub_%s}){Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=c[1];'
        'ctx.r1.u32=0x2800;f(ctx,nullptr);o[j++]=ctx.r3.u32;}printf("FVF %%08x %%u %%u %%u %%u\\n",c[0],c[1],o[0],o[1],o[2]);}}\n') % (ftex, ftan, fbin)
    out.append(run(build(idx, ['sub_' + ftex, 'sub_' + ftan, 'sub_' + fbin], body, a.out, 'fvf'), a.image))
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(''.join(out))


if __name__ == '__main__':
    main()
