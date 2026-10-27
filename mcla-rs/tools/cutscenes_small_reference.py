"""Run small cutscene functions of the original offline (static recompilation, native, flat guest memory) and record
their results for the port's tests (crates/game/tests/data/cutscenes_small_reference.txt).

  QANG <q0 4 words> <q1 4 words> <out>               F-94425, the angle between two rotations (pure leaf + the library acos)
  CUT  <t> <prev> <n> <sample params> -> <t'> <cut> <prev'>   F-13038, the car event's cut test. The mover sampler (F-15435) is
        replaced by a recording stub that returns synthetic samples (translation = (a0 + a1 t, b0 + b1 t, c0 + c1 t t), rotation
        = (d0 + d1 t, e0 + e1 t, f0 + f1 t, g0 + g1 t), all f32, found flag n bit 0 translation / bit 1 rotation): the cut
        rule itself (1/30 s grid, 0.05 s catch-up, 2 m, 20 deg, prev) runs unchanged; the sampler is NOT compared here.
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
    qang, cut, samp = (addr(a.tool, f) for f in ('F-94425', 'F-13038', 'F-15435'))
    rnd = random.Random(11)
    out = []
    head = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});\n'
    # --- quaternion angle
    qs = [[0, 0, 0, 1], [0, 0, 0, -1], [0, 0, 0, 1.0000001], [0.5, 0.5, 0.5, 0.5], [0, 0, 0, 0], [1, 0, 0, 0], [float('nan')] * 4]
    cases = []
    for _ in range(800):
        v = [rnd.uniform(-1, 1) for _ in range(4)]
        n = sum(x * x for x in v) ** 0.5
        q0 = [x / n for x in v]
        w = [rnd.uniform(-1, 1) * (1 if _ % 2 else 0.05) + q0[i] for i, _x in enumerate(v)]
        m = sum(x * x for x in w) ** 0.5
        cases.append((q0, [x / m for x in w]))
    for i in range(len(qs)):
        for j in range(len(qs)):
            cases.append((qs[i], qs[j]))
    body = head
    for q0, q1 in cases:
        w = [fbits(x) for x in q0 + q1]
        body += ('{' + ''.join('store(0x1100+%d,0x%08x);' % (4 * i, w[i]) for i in range(4)) +
                 ''.join('store(0x1200+%d,0x%08x);' % (4 * i, w[4 + i]) for i in range(4)) +
                 'Context ctx{};ctx.r3.u32=0x1100;ctx.r4.u32=0x1200;ctx.r1.u32=0x2800;sub_%s(ctx,nullptr);float r=float(ctx.f1.f64);uint32_t rb;memcpy(&rb,&r,4);'
                 'printf("QANG %s %%08x\\n",rb);}\n' % (qang[4:], ' '.join('%08x' % x for x in w)))
    body += '}\n'
    out.append(run(build(idx, [qang], body, a.out, 'qang'), a.image))
    # --- cut test with the sampler recorded by a stub
    stub = (
        'float t=float(ctx.f1.f64);uint32_t d=ctx.r5.u32;int kind=*pointer(d+5);'
        'int n=int(load(0x1050));'
        'auto P=[&](int i){return getf(0x1060+4*i);};'
        'if(kind==5){if(!(n&1)){ctx.r3.u32=0;return;}'
        ' float x=P(0)+P(1)*t;float y=P(2)+P(3)*t;float z=P(4)+P(5)*t*t;putf(d+16,x);putf(d+20,y);putf(d+24,z);putf(d+28,0.0f);}'
        'else{if(!(n&2)){ctx.r3.u32=0;return;}'
        ' float x=P(6)+P(7)*t;float y=P(8)+P(9)*t;float z=P(10)+P(11)*t;float w=P(12)+P(13)*t;putf(d+16,x);putf(d+20,y);putf(d+24,z);putf(d+28,w);}'
        'ctx.r3.u32=1;')
    rows = []
    for k in range(2000):
        speed = rnd.choice([0.0, 1.0, 20.0, 60.0, 61.0, 100.0, rnd.uniform(0, 120)])
        turn = rnd.choice([0.0, 0.5, 5.0, 10.4, 10.5, 20.0, rnd.uniform(0, 30)])
        par = [rnd.uniform(-50, 50), speed * rnd.choice([-1, 1]), rnd.uniform(-5, 5), rnd.uniform(-3, 3), rnd.uniform(-50, 50),
               rnd.choice([0.0, speed * 0.1, rnd.uniform(-40, 40)]),
               rnd.uniform(-0.3, 0.3), turn * 0.01 * rnd.choice([-1, 1]), rnd.uniform(-0.3, 0.3), rnd.uniform(-0.01, 0.01),
               rnd.uniform(-0.3, 0.3), rnd.uniform(-0.01, 0.01), rnd.uniform(0.3, 0.9), turn * 0.02 * rnd.choice([-1, 1])]
        t = rnd.choice([0.0, 1 / 30, 0.1, 1.0, rnd.uniform(0, 60), rnd.uniform(0, 3)])
        prev = rnd.choice([0.0, t - 0.03, t - 0.05, t - 0.0501, t - 0.2, rnd.uniform(0, 60), t])
        n = rnd.choice([3, 3, 3, 3, 0, 1, 2])
        rows.append([fbits(t), fbits(prev), n] + [fbits(x) for x in par])
    table = 'static const uint32_t CASES[][17]={' + ','.join('{' + ','.join('0x%08xu' % v for v in r) + '}' for r in rows) + '};\n'
    body = table + head + (
        'for(size_t k=0;k<sizeof(CASES)/sizeof(CASES[0]);k++){const uint32_t*c=CASES[k];memset(objects,0,sizeof(objects));store(0x1050,c[2]);'
        'for(int i=0;i<14;i++)store(0x1060+4*i,c[3+i]);store(0x1000,c[1]);store(0x1020,c[0]);'
        'Context ctx{};ctx.r3.u32=0x1010;ctx.r4.u32=0x1000;ctx.f1.f64=double(getf(0x1020));ctx.f2.f64=2.0;ctx.f3.f64=20.0;ctx.r6.u32=0x1400;ctx.r1.u32=0x2800;'
        'sub_%s(ctx,nullptr);float r=float(ctx.f1.f64);uint32_t rb;memcpy(&rb,&r,4);'
        'printf("CUT %%08x %%08x %%u",c[0],c[1],c[2]);for(int i=0;i<14;i++)printf(" %%08x",c[3+i]);'
        'printf(" %%08x %%d %%08x\\n",rb,int(*pointer(0x1010)),load(0x1000));}}\n') % cut[4:]

    out.append(run(build(idx, [cut], body, a.out, 'cut', {samp: stub}, vector=True), a.image))
    a.ref.write_text(''.join(out))


if __name__ == '__main__':
    main()
