"""The ambient obstacle's size (F-46284, the obstacle's slot 3) run offline: the generated code whole and unchanged in
vector mode on flat guest memory. Its own slot 7 (the radius it returns) answers 0.

Memory: the obstacle 0x1000 (vtable 0x1100, +16 the car), the car 0x2000 (+500 the flag byte, +52 the proxy holder,
+112 the car's physics instance: +4 the archetype), the proxy holder 0x2400 (+48 the owner 0x2500, its +28 the owner's
instance 0x2600), archetypes 0x3000 / 0x3100 (+12 the bound), bounds 0x3400 / 0x3500 (+16 max, +32 min), out 0x1F00.

Writes <out-dir>/traffic_size_ref.txt, a case a line: flags holder owner own-archetype owner-archetype max(3) min(3)
owner-max(3) owner-min(3) (hex words) '->' the two floats written (bits)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build

recomp_closure.shim = lambda *_: recomp_leaf.shim(0x8000)
_vshim = recomp_closure.vector_shim


def vector_shim():
    # the VMX flush mode as the host's DAZ / FTZ (as tools/traffic_select_reference.py)
    s = '#include <xmmintrin.h>' + chr(10) + _vshim()
    fl = 'void disableFlushMode() {} void enableFlushMode() {}'
    assert s.count(fl) == 1
    return s.replace(fl, 'void disableFlushMode() {_mm_setcsr(_mm_getcsr()&~0x8040u);} void enableFlushMode() {_mm_setcsr(_mm_getcsr()|0x8040u);}', 1)


recomp_closure.vector_shim = vector_shim
recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n')
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)
NL = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def bf(u):
    return struct.unpack('>f', struct.pack('>I', u & 0xFFFFFFFF))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = addr(a.tool, 'F-46284')
    rnd = random.Random(46284)
    cs = []
    for _ in range(2000):
        flags = rnd.choice((0, 0, 0x20, rnd.randint(0, 255)))
        holder, owner = rnd.choice((0, 1, 1)), rnd.choice((0, 1, 1))
        own_arch, owner_arch = rnd.choice((0, 1, 1, 1)), rnd.choice((0, 1, 1))
        box = lambda: [rnd.choice((rnd.uniform(-8, 8), 0.0, -0.0, bf(rnd.randint(1, 0x7FFFFF)))) for _ in range(3)]
        cs.append([flags, holder, owner, own_arch, owner_arch] + [fb(x) for x in box() + box() + box() + box()])
    body = r'''
static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){if(a==0x9007){ctx.f1.f64=0;return;}printf("bad call %%08x\n",a);throw std::runtime_error("call");}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
std::ifstream in(argv[2]);std::string line;
while(std::getline(in,line)){std::vector<uint32_t> w;{const char*s=line.c_str();char*e;while(*s){uint32_t v=strtoul(s,&e,16);if(e==s)break;w.push_back(v);s=e;}}
memset(objects,0,sizeof(objects));
store(0x1000,0x1100);store(0x1100+28,0x9007);store(0x1000+16,0x2000);*pointer(0x2000+500)=uint8_t(w[0]);
if(w[1]){store(0x2000+52,0x2400);if(w[2]){store(0x2400+48,0x2500);store(0x2500+28,0x2600);}}
if(w[3]){store(0x2000+112+4,0x3000);store(0x3000+12,0x3400);}
if(w[4]){store(0x2600+4,0x3100);store(0x3100+12,0x3500);}
for(int j=0;j<3;j++){store(0x3400+16+4*j,w[5+j]);store(0x3400+32+4*j,w[8+j]);store(0x3500+16+4*j,w[11+j]);store(0x3500+32+4*j,w[14+j]);}
Context ctx{};ctx.r1.u32=0x7000;ctx.r3.u32=0x1000;ctx.r4.u32=0x1F00;sub_%s(ctx,nullptr);
printf("%%x %%x\n",load(0x1F00),load(0x1F04));}
}
''' % fn
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fn], body, a.out_dir, 'traffic_size', vector=True, scratch_end=0x8000)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'traffic_size_in.txt'
    inp.write_text(NL.join(' '.join('%x' % x for x in c) for c in cs) + NL)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True, check=True)
    out = [x for x in r.stdout.split(NL) if x]
    assert len(out) == len(cs), (len(out), len(cs), r.stdout[-300:])
    lines = []
    for c, o in zip(cs, out):
        assert 'bad call' not in o, o
        line = ' '.join('%x' % x for x in c) + ' -> ' + o.strip()
        assert not any(t.startswith('82') and len(t) == 8 for t in line.split()), line
        lines.append(line)
    (a.out_dir / 'traffic_size_ref.txt').write_text(NL.join(lines) + NL)
    print(len(lines), 'cases; default 3 x 6:', sum(l.endswith('-> 40400000 40c00000') for l in lines))


if __name__ == '__main__':
    main()
