"""The particle key-frame evaluation run offline: F-42828 (a rule's key array at +24 / +28) and the three
evolution evaluators of the emit rule (F-87862, F-96958, F-56010: arrays at +316, +356, +396), unchanged
generated code on a flat guest memory (recomp_closure, vector mode; the override branch is stubbed and never taken: object 0).
Writes map_keyframe_ref.txt: "fn n t | out0 out1 out2 out3 (the original's) and the key table" as f32 bits."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run

NL = chr(92) + 'n'


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = index(a.recomp)
    ids = ['F-42828', 'F-87862', 'F-96958', 'F-56010']
    rnd = random.Random(5)
    names = ['sub_' + addr(a.tool, fid) for fid in ids]
    words, tables = [], []
    for k in range(len(ids)):
        for case in range(1500):
            n = rnd.choice([0, 1, 1, 2, 3, 4, 6, 9])
            ts = sorted(rnd.choice([0.0, 0.25, 0.5, 1.0, rnd.random(), rnd.uniform(-1, 2)]) for _ in range(n))
            keys = [[ts[i]] + [rnd.uniform(-2, 2) for _ in range(4)] + [rnd.uniform(-5, 5) for _ in range(4)] for i in range(n)]
            t = rnd.choice([rnd.uniform(-0.5, 1.5), 0.0, 1.0, 0.5] + ts)
            flat = [fb(x) for key in keys for x in key]
            words += [k, n, fb(t)] + flat
            tables.append(' '.join('%08x' % w for w in flat))
    calls = ''.join('case %d: sub_%s(ctx,nullptr); break;' % (k, addr(a.tool, ids[k])) for k in range(len(ids)))
    main_src = ('static float bf(uint32_t u){float f;memcpy(&f,&u,4);return f;}' + chr(10) +
                'static const uint32_t W[]={' + ','.join('0x%x' % w for w in words) + '};' + chr(10) +
                'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + chr(10) +
                'size_t p=0;while(p<sizeof(W)/4){uint32_t k=W[p],n=W[p+1],tb=W[p+2];p+=3;memset(objects,0,sizeof(objects));'
                'uint32_t koff=k==0?24:(k==1?316:(k==2?356:396)),coff=koff+4;store(0x1000+koff,0x1400);store16(0x1000+coff,n);'
                'for(uint32_t i=0;i<n;i++){store(0x1400+48*i,W[p]);for(int j=0;j<4;j++){store(0x1400+48*i+16+4*j,W[p+1+j]);store(0x1400+48*i+32+4*j,W[p+5+j]);}p+=9;}'
                'Context ctx{};if(k==0)ctx.r5.u32=0x1000;else ctx.r3.u32=0x1000;ctx.r4.u32=0x1800;ctx.r7.u32=0;ctx.r1.u32=0x2800;ctx.f1.f64=double(bf(tb));'
                'try{switch(k){' + calls + '}}catch(std::exception&e){printf("EXC %u %u %08x' + NL + '",k,n,tb);continue;}'
                'printf("%u %u %08x %08x %08x %08x %08x' + NL + '",k,n,tb,load(0x1800),load(0x1804),load(0x1808),load(0x180c));}}' + chr(10))
    exe = build(idx, names, main_src, a.out, 'map_keyframe', stubs={'sub_' + addr(a.tool, 'F-31783'): ''}, vector=True)
    out = run(exe, a.image).splitlines()
    assert len(out) == len(tables)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'map_keyframe_ref.txt').write_text(chr(10).join(o + ' | ' + t for o, t in zip(out, tables)) + chr(10))
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
