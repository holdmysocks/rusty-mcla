"""Run the scene camera's cut test of the original offline (static recompilation, native, flat guest memory) and record
its results for the port's test (crates/game/src/testdata/cam_cut_ref.txt).

  CAM <t> <m> <deg> <prev> <last> <cut> <on> <n> <14 sample params> -> <t'> <prev'> <last'> <last2'> <cut'>
        F-34419 (the camera cut test of F-25555), state in the image's globals (the previous grid time, the last time and the
        one before, the last cut flag, the enable byte the scene camera update F-40303 sets around its call). The camera
        sampler (F-15435) is replaced by a recording stub that returns synthetic samples (translation = (a0 + a1 t,
        b0 + b1 t, c0 + c1 t t), rotation = (d0 + d1 t, e0 + e1 t, f0 + f1 t, g0 + g1 t), all f32, found flag n bit 0
        translation / bit 1 rotation): the rule itself (1/30 s grid, 0.05 s catch-up, the distance x change term, the
        turn limit, the state) runs unchanged; the sampler is NOT compared here.
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


PREV, LAST, LAST2, CUT, ON = 0x827EAAD8, 0x828CD530, 0x828CD534, 0x828CD52E, 0x828CD52D


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = index(a.recomp)
    cut, samp = (addr(a.tool, f) for f in ('F-34419', 'F-15435'))
    rnd = random.Random(23)
    head = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});\n'
    # the translation request has byte +4 = 0, the rotation request +4 = 1
    stub = (
        'float t=float(ctx.f1.f64);uint32_t d=ctx.r5.u32;int kind=*pointer(d+4);'
        'int n=int(load(0x1050));'
        'auto P=[&](int i){return getf(0x1060+4*i);};'
        'if(kind==0){if(!(n&1)){ctx.r3.u32=0;return;}'
        ' float x=P(0)+P(1)*t;float y=P(2)+P(3)*t;float z=P(4)+P(5)*t*t;putf(d+16,x);putf(d+20,y);putf(d+24,z);putf(d+28,0.0f);}'
        'else{if(!(n&2)){ctx.r3.u32=0;return;}'
        ' float x=P(6)+P(7)*t;float y=P(8)+P(9)*t;float z=P(10)+P(11)*t;float w=P(12)+P(13)*t;putf(d+16,x);putf(d+20,y);putf(d+24,z);putf(d+28,w);}'
        'ctx.r3.u32=1;')
    rows = []
    for k in range(4000):
        speed = rnd.choice([0.0, 1.0, 20.0, 60.0, 61.0, 100.0, 600.0, rnd.uniform(0, 120)])
        turn = rnd.choice([0.0, 0.5, 5.0, 10.4, 10.5, 20.0, rnd.uniform(0, 30)])
        par = [rnd.uniform(-50, 50), speed * rnd.choice([-1, 1]), rnd.uniform(-5, 5), rnd.uniform(-3, 3), rnd.uniform(-50, 50),
               rnd.choice([0.0, speed * 0.1, rnd.uniform(-40, 40), rnd.uniform(-4000, 4000)]),
               rnd.uniform(-0.3, 0.3), turn * 0.01 * rnd.choice([-1, 1]), rnd.uniform(-0.3, 0.3), rnd.uniform(-0.01, 0.01),
               rnd.uniform(-0.3, 0.3), rnd.uniform(-0.01, 0.01), rnd.uniform(0.3, 0.9), turn * 0.02 * rnd.choice([-1, 1])]
        t = rnd.choice([0.0, 1 / 30, 0.1, 1.0, rnd.uniform(0, 60), rnd.uniform(0, 3)])
        prev = rnd.choice([-1.0, -1.0, -2.0, -0.5, 0.0, t - 0.03, t - 0.05, t - 0.0501, t - 0.2, rnd.uniform(0, 60), t])
        m, deg = rnd.choice([(2.0, 20.0), (2.0, 20.0), (0.5, 5.0), (10.0, 90.0)])
        rows.append([fbits(t), fbits(m), fbits(deg), fbits(prev), fbits(rnd.uniform(0, 60)), rnd.choice([0, 1]),
                     rnd.choice([1, 1, 1, 0]), rnd.choice([3, 3, 3, 3, 0, 1, 2])] + [fbits(x) for x in par])
    table = 'static const uint32_t CASES[][22]={' + ','.join('{' + ','.join('0x%08xu' % v for v in r) + '}' for r in rows) + '};\n'
    body = table + head + (
        'for(size_t k=0;k<sizeof(CASES)/sizeof(CASES[0]);k++){const uint32_t*c=CASES[k];memset(objects,0,sizeof(objects));store(0x1050,c[7]);'
        'for(int i=0;i<14;i++)store(0x1060+4*i,c[8+i]);store(0x%x,c[3]);store(0x%x,c[4]);store(0x%x,0x0badf00du);'
        '*pointer(0x%x)=uint8_t(c[5]);*pointer(0x%x)=uint8_t(c[6]);'
        'Context ctx{};ctx.r4.u32=0x1400;ctx.r1.u32=0x2800;'
        'uint32_t w[3]={c[0],c[1],c[2]};float fv[3];memcpy(fv,w,12);ctx.f1.f64=fv[0];ctx.f2.f64=fv[1];ctx.f3.f64=fv[2];'
        'sub_%s(ctx,nullptr);float r=float(ctx.f1.f64);uint32_t rb;memcpy(&rb,&r,4);'
        'printf("CAM");for(int i=0;i<22;i++)printf(" %%08x",c[i]);'
        'printf(" %%08x %%08x %%08x %%08x %%d\\n",rb,load(0x%x),load(0x%x),load(0x%x),int(*pointer(0x%x)));}}\n') % (
        PREV, LAST, LAST2, CUT, ON, cut[4:], PREV, LAST, LAST2, CUT)
    a.ref.write_text(run(build(idx, [cut], body, a.out, 'camcut', {samp: stub}, vector=True), a.image))


if __name__ == '__main__':
    main()
