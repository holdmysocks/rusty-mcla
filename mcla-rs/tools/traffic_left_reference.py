"""The follower's distance left on its rail (F-83943) run offline: the generated code whole and unchanged on flat guest
memory. The rail's length F-62306 (the arc-line-arc rail's, `tcp_rail::len`) is stubbed with the case's length; the rail's and
the curve's vfuncs answer from the case: rail slot 7 (the point at a phase: written to r3, the phase checked), slot 12 /
13 (its start / end), slot 15 (its curve: +28 the last vertex index), the curve's slot 2 (the vertex by index: the case's
vertex, the index checked).

Memory: the follower 0x1000 (+152 byte, +156 the rail, +176 the phase), the rail 0x1200 (vtable 0x1300, +4 flags), the
curve 0x1400 (vtable 0x1500, +28 u16 the last index), points 0x1600 (start) / 0x1610 (end) / 0x1620 (vertex).

Writes <out-dir>/traffic_left_ref.txt, a case a line: has-rail flags byte phase length point(3) start(3) end(3) vertex(3)
last (hex words; floats as bits) '->' the distance (bits) and the index the curve was asked for (ffffffff: none)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build, run

# (the shim's register union gains the signed 16-bit view `extsh` reads)
recomp_closure.shim = lambda *_: recomp_leaf.shim(0x8000).replace('uint16_t u16; uint8_t u8;', 'uint16_t u16; int16_t s16; uint8_t u8;', 1)

recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\nstatic uint32_t C[20];static uint32_t ASKED;\n')
NL = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def bf(u):
    return struct.unpack('>f', struct.pack('>I', u & 0xFFFFFFFF))[0]


def f32(x):
    return bf(fb(x))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    fn, length = addr(a.tool, 'F-83943'), addr(a.tool, 'F-62306')
    rnd = random.Random(83943)
    cs = []
    for _ in range(a.cases):
        has = int(rnd.random() < 0.95)
        flags = rnd.choice((0, 0, 0, 0x1000, 0x800, 0x1800, rnd.randint(0, 0xFFFF)))
        byte = rnd.choice((0, 0, 0x80, 0x40, 0xC0, 0xC0, rnd.randint(0, 255)))
        phase = rnd.choice((0.0, 1.0, -0.0, rnd.uniform(0, 1), rnd.uniform(0, 1), bf(fb(1.0) - 1), 1e-30, rnd.uniform(-0.1, 1.1)))
        ln = rnd.choice((0.0, rnd.uniform(0, 200), rnd.uniform(0, 5)))
        base = [rnd.choice((0.0, rnd.uniform(-2000, 2000))) for _ in range(3)]
        pt = lambda s: [f32(base[i] + rnd.choice((0.0, rnd.uniform(-s, s)))) for i in range(3)]
        p, s, e, v = pt(150), pt(150), pt(150), pt(150)
        if rnd.random() < 0.05:
            v = list(p)
        last = rnd.choice((0, 1, 5, 31, 0x7FFF, 0x8000, 0xFFFF))
        cs.append([has, flags, byte, fb(f32(phase)), fb(f32(ln))] + [fb(x) for x in p + s + e + v] + [last])
    setup = r'''
static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){switch(a){
case 0x9007:{uint32_t ph=fbits(float(ctx.f1.f64));if(ctx.r4.u32!=0x1200||ph!=load(0x1000+176))printf("bad7 ");
  for(int j=0;j<3;j++)store(ctx.r3.u32+4*j,C[5+j]);store(ctx.r3.u32+12,0);return;}
case 0x900C: ctx.r3.u64=0x1600;return; case 0x900D: ctx.r3.u64=0x1610;return; case 0x900F: ctx.r3.u64=0x1400;return;
case 0x9102: ASKED=ctx.r4.u32;ctx.r3.u64=0x1620;return;}
printf("bad call %08x\n",a);throw std::runtime_error("call");}
'''
    body = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
std::ifstream in(argv[2]);std::string line;
while(std::getline(in,line)){{const char*s=line.c_str();char*e;int k=0;while(*s&&k<20){uint32_t v=strtoul(s,&e,16);if(e==s)break;C[k++]=v;s=e;}}
memset(objects,0,sizeof(objects));ASKED=0xFFFFFFFFu;
store(0x1000+156,C[0]?0x1200:0);*pointer(0x1000+152)=uint8_t(C[2]);store(0x1000+176,C[3]);
store(0x1200,0x1300);store(0x1204,C[1]);store(0x1300+28,0x9007);store(0x1300+48,0x900C);store(0x1300+52,0x900D);store(0x1300+60,0x900F);
store(0x1400,0x1500);store16(0x1400+28,uint16_t(C[17]));store(0x1500+8,0x9102);
for(int j=0;j<3;j++){store(0x1600+4*j,C[8+j]);store(0x1610+4*j,C[11+j]);store(0x1620+4*j,C[14+j]);}
Context ctx{};ctx.r1.u32=0x7000;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);
printf("%%x %%x\n",fbits(float(ctx.f1.f64)),ASKED);}
}
''' % fn
    stubs = {'sub_' + length: 'ctx.f1.f64=double(getf(0x1000+0x7FC));'}
    pre = 'static uint32_t fbits(float f){uint32_t u;memcpy(&u,&f,4);return u;}\n'
    # the stubbed length reads the case's length from a scratch word
    body = body.replace('store(0x1000+176,C[3]);', 'store(0x1000+176,C[3]);store(0x1000+0x7FC,C[4]);')
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fn], pre + setup + body, a.out_dir, 'traffic_left', stubs=stubs, scratch_end=0x8000)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'traffic_left_in.txt'
    inp.write_text(NL.join(' '.join('%x' % x for x in c) for c in cs) + NL)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True, check=True)
    out = [x for x in r.stdout.split(NL) if x]
    assert len(out) == len(cs), (len(out), len(cs), r.stdout[-300:])
    lines = []
    for c, o in zip(cs, out):
        line = ' '.join('%x' % x for x in c) + ' -> ' + o.strip()
        assert 'bad7' not in o and 'bad call' not in o, line
        assert not any(t.startswith('82') and len(t) == 8 for t in line.split()), line
        lines.append(line)
    (a.out_dir / 'traffic_left_ref.txt').write_text(NL.join(lines) + NL)
    print(len(lines), 'cases; lane branch', sum(not l.endswith(' ffffffff') for l in lines))


if __name__ == '__main__':
    main()
