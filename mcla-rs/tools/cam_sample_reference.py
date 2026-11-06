"""Run the scene camera's sampler of the original offline (static recompilation, native, flat guest memory) and record its
results for the port's test (crates/game/src/testdata/cam_sample_ref.txt).

  SMP <found> <3 translation> <4 rotation> <focal> <k14> -> <16 matrix words> <lens> <k14 out>
        F-25555 as the scene camera update F-40303 calls it: the matrix starts as identity, the lens and the kind-14 value
        as 0x0badf00d (left as they are when their track is missing). The cut test F-34419 (verified on its own,
        cam_cut_reference.py) is replaced by a stub that keeps the time; the track sampler F-15435 by a stub that answers
        the four requests (type 0 kind 0 translation, type 1 kind 1 rotation, type 2 kind 10 focal length, type 2 kind 14)
        from the case's values, each found when its bit of <found> is set. The quaternion-to-rows F-40856 and the CRT's
        double atan run unchanged.
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


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = index(a.recomp)
    smp, cut, trk = (addr(a.tool, f) for f in ('F-25555', 'F-34419', 'F-15435'))
    rnd = random.Random(31)
    head = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});\n'
    # the request record: byte +4 the value type, +5 the track kind; the value goes to +16
    stub = (
        'uint32_t d=ctx.r5.u32;int ty=*pointer(d+4);int kd=*pointer(d+5);int n=int(load(0x1050));'
        'auto P=[&](int i){return load(0x1060+4*i);};'
        'int bit=-1,at=0,cnt=0;'
        'if(ty==0&&kd==0){bit=0;at=0;cnt=3;}else if(ty==1&&kd==1){bit=1;at=3;cnt=4;}'
        'else if(ty==2&&kd==10){bit=2;at=7;cnt=1;}else if(ty==2&&kd==14){bit=3;at=8;cnt=1;}'
        'if(bit<0||!(n>>bit&1)){ctx.r3.u32=0;return;}'
        'for(int i=0;i<4;i++)store(d+16+4*i,i<cnt?P(at+i):0u);ctx.r3.u32=1;')
    rows = []
    for k in range(3000):
        q = [rnd.gauss(0, 1) for _ in range(4)]
        if rnd.random() < 0.8:
            l = sum(x * x for x in q) ** 0.5
            q = [x / l for x in q]
        else:
            q = [x * rnd.choice([0.0, 0.5, 1.3]) for x in q]
        pos = [rnd.choice([rnd.uniform(-2000, 2000), rnd.uniform(-5, 5), 0.0]) for _ in range(3)]
        focal = rnd.choice([rnd.uniform(10, 200), rnd.uniform(18, 90), 35.0, 50.0, 0.0, -24.0, rnd.uniform(0.001, 1), 1e6])
        k14 = rnd.uniform(-10, 10)
        found = rnd.choice([15, 15, 15, 7, 3, 11, 1, 2, 4, 0, rnd.randrange(16)])
        rows.append([found] + [fbits(x) for x in pos + q + [focal, k14]])
    table = 'static const uint32_t CASES[][10]={' + ','.join('{' + ','.join('0x%08xu' % v for v in r) + '}' for r in rows) + '};\n'
    M, L, K = 0x1400, 0x1500, 0x1510
    body = table + head + (
        'for(size_t k=0;k<sizeof(CASES)/sizeof(CASES[0]);k++){const uint32_t*c=CASES[k];memset(objects,0,sizeof(objects));store(0x1050,c[0]);'
        'for(int i=0;i<9;i++)store(0x1060+4*i,c[1+i]);'
        'for(int i=0;i<16;i++)store(0x%x+4*i,(i%%5==0)?0x3f800000u:0u);store(0x%x,0x0badf00du);store(0x%x,0x0badf00du);'
        'Context ctx{};ctx.r1.u32=0x2800;ctx.r3.u32=0x1800;ctx.r4.u32=0x%x;ctx.r5.u32=0x%x;ctx.r6.u32=0x%x;ctx.r7.u32=0x1800;ctx.f1.f64=1.0;'
        'sub_%s(ctx,nullptr);'
        'printf("SMP");for(int i=0;i<10;i++)printf(" %%08x",c[i]);'
        'for(int i=0;i<16;i++)printf(" %%08x",load(0x%x+4*i));printf(" %%08x %%08x\\n",load(0x%x),load(0x%x));}}\n') % (
        M, L, K, M, L, K, smp[4:], M, L, K)
    a.ref.write_text(run(build(idx, [smp], body, a.out, 'camsmp', {'sub_' + cut[4:]: '', 'sub_' + trk[4:]: stub}, vector=True,
                               flags=('-msse4.1', '-mssse3')), a.image))


if __name__ == '__main__':
    main()
