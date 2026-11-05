"""The ambient rider's stuck test (F-14023) run offline: the generated code whole and unchanged on flat guest memory, with
the rider's own vfuncs 21 / 20 (F-60920 path fired, F-74273 path held: behaviour slots +84 / +80 read from the rider's
real vtable in the image) running as they are. What the code asks of the world is stubbed and answers from the case:
the network test F-24338 (bit 2), the car's matrix block F-33765 (up y at +20), the next rail's junction F-41621 (bit 128)
and its green test F-78185 (bit 256), the next rail's start point (its slot 12, read through its vtable), the car's place
F-58860 and its active test F-17857 (bit 1024).

Guest memory: the tracker 0x1000 (+0 standing time, +4 backing time, +8 tries), the context 0x1100 (+0 the rider B,
+4 the manager S, +8 the car C, +12 dt), S 0x1200 (byte +0x64251 the event, bit 1), B 0x2000 (vtable from the image,
+184 the follower's reverse slot byte (bit 512 -> 0x80), +188 the rail (bit 32), +192 the next rail R 0x4000 (bit 64),
+432 nearest, +436 blocking, +468 the path 0x2800 (bit 2048; +188 bit 4096, +189 bit 8192)), C 0x3000 (+412 speed,
+420 the controller's slot, +448 the type 0x3800 (+111 bit 4, +112 bit 8), +452 B (bit 16), +3604 the partner id).

Writes <out-dir>/traffic_stuck_ref.txt, a case a line: st0 st4 tries dt bits upy slot partner nearest blocking v rx rz px
pz (hex words; floats as bits) '->' the answer and the tracker's three words after the run."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build, run

recomp_closure.shim = lambda *_: recomp_leaf.shim(0x80000)
recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n'
                         'static const uint32_t*cur;\n')

NL = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def f32(x):
    return struct.unpack('>f', struct.pack('>f', x))[0]


def bf(u):
    return struct.unpack('>f', struct.pack('>I', u))[0]


def cases(rnd, n):
    out = []
    for _ in range(n):
        # mostly a car that would count (no refusal), the refusals each a few per cent
        bits = 0
        if rnd.random() < 0.04:
            bits |= 1
        if rnd.random() < 0.04:
            bits |= 2
        if rnd.random() < 0.06:
            bits |= 4
        if rnd.random() < 0.04:
            bits |= 8
        if rnd.random() < 0.9:
            bits |= 16
        if rnd.random() < 0.8:
            bits |= 32
        if rnd.random() < 0.7:
            bits |= 64
        if rnd.random() < 0.5:
            bits |= 128
        if rnd.random() < 0.5:
            bits |= 256
        if rnd.random() < 0.3:
            bits |= 512
        if rnd.random() < 0.5:
            bits |= 1024
        if rnd.random() < 0.15:
            bits |= 2048
            bits |= rnd.choice((0, 4096, 8192, 4096 | 8192))
        st0 = rnd.choice((0.0, rnd.uniform(0, 2.2), 2.0, bf(fb(2.0) - 1), bf(fb(2.0) + 1), rnd.uniform(1.9, 2.1), rnd.uniform(0, 9)))
        st4 = rnd.choice((0.0, 0.0, 0.0, 0.0, rnd.uniform(0, 4), -0.0, bf(1)))
        tries = rnd.choice((0, 0, 1, 2, 2, 3, 4))
        dt = rnd.choice((1 / 30, 1 / 60, rnd.uniform(0, 0.1), 0.0))
        upy = rnd.choice((1.0, 1.0, 0.9, rnd.uniform(-1, 1), -0.0, 0.0, -bf(1), bf(1)))
        slot = rnd.choice((-1, 0, 0, 1, 2))
        partner = rnd.choice((-1, -1, 0, 5))
        ids = (0, 0x7100, 0x7200)
        nearest = rnd.choice(ids)
        blocking = rnd.choice(ids + (nearest, nearest))
        v = rnd.choice((0.0, 0.0, 0.0, -0.0, rnd.uniform(0, 15), bf(1)))
        px, pz = rnd.uniform(-2000, 2000), rnd.uniform(-2000, 2000)
        if rnd.random() < 0.5:
            # the next rail's start round 50 m off, at the rounding of the squared distance
            d = rnd.choice((50.0, rnd.uniform(40, 60), rnd.uniform(0, 80)))
            a = rnd.uniform(0, 6.2831853)
            import math
            rx, rz = f32(px + d * math.cos(a)), f32(pz + d * math.sin(a))
            if rnd.random() < 0.5:
                rx = bf(fb(rx) + rnd.randint(-3, 3))
        else:
            rx, rz = rnd.uniform(-2000, 2000), rnd.uniform(-2000, 2000)
        words = [fb(f32(st0)), fb(f32(st4)), tries & 0xFFFFFFFF, fb(f32(dt)), bits, fb(f32(upy)), slot & 0xFFFFFFFF,
                 partner & 0xFFFFFFFF, nearest, blocking, fb(f32(v)), fb(f32(rx)), fb(f32(rz)), fb(f32(px)), fb(f32(pz))]
        out.append(words)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=6000)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-14023')
    net, mat, junc, green, place, active = (addr(a.tool, x) for x in ('F-24338', 'F-33765', 'F-41621', 'F-78185', 'F-58860', 'F-17857'))
    fired, held = addr(a.tool, 'F-60920'), addr(a.tool, 'F-74273')
    # the rider's vtable: the word that holds its update F-87488 at slot 3 (+12)
    upd = int(addr(a.tool, 'F-87488'), 16)
    img = a.image.read_bytes()
    vt = None
    for off in range(0, len(img) - 4, 4):
        if struct.unpack_from('>I', img, off)[0] == upd and struct.unpack_from('>I', img, off + 68)[0] == int(held, 16):
            vt = (0x8200 << 16) + off - 12
            break
    assert vt is not None
    rnd = random.Random(14023)
    cs = cases(rnd, a.cases)
    stubs = {
        'sub_' + net: 'ctx.r3.u64=(cur[4]>>1)&1;',
        'sub_' + mat: 'ctx.r3.u64=0x3400;',
        'sub_' + junc: 'ctx.r3.u64=(cur[4]&128)?0x5000:0;',
        'sub_' + green: 'ctx.r3.u64=(cur[4]>>8)&1;',
        'sub_' + place: 'ctx.r3.u64=0x3500;',
        'sub_' + active: 'ctx.r3.u64=(cur[4]>>10)&1;',
    }
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){switch(a){'
            'case 0x%s: sub_%s(ctx,base);return; case 0x%s: sub_%s(ctx,base);return;'
            'case 0x9000: ctx.r3.u64=0x4100;return;}'
            'printf("bad call %%08x\\n",a);throw std::runtime_error("call");}' % (fired, fired, held, held)) + NL
    rows = NL.join('{%s},' % ','.join('0x%08xu' % x for x in c) for c in cs)
    body = ('static const uint32_t C[][15]={' + NL + rows + NL + '};' + NL +
            'for(auto&c:C){cur=c;memset(objects,0,sizeof(objects));uint32_t b=c[4];'
            'store(0x1000,c[0]);store(0x1004,c[1]);store(0x1008,c[2]);'
            'store(0x1100,0x2000);store(0x1104,0x1200);store(0x1108,0x3000);store(0x110C,c[3]);'
            '*pointer(0x1200+0x64251)=b&1;'
            'store(0x2000,0x%08xu);*pointer(0x2000+184)=(b&512)?0x80:0;store(0x2000+188,(b&32)?0x4400:0);'
            'store(0x2000+192,(b&64)?0x4000:0);store(0x2000+432,c[8]);store(0x2000+436,c[9]);'
            'store(0x2000+468,(b&2048)?0x2800:0);*pointer(0x2800+188)=(b&4096)?1:0;*pointer(0x2800+189)=(b&8192)?1:0;'
            'store(0x3000+412,c[10]);store(0x3000+420,c[6]);store(0x3000+448,0x3800);*pointer(0x3800+111)=(b&4)?1:0;'
            '*pointer(0x3800+112)=(b&8)?1:0;store(0x3000+452,(b&16)?0x2000:0);store(0x3000+3604,c[7]);'
            'store(0x3400+20,c[5]);store(0x3500,c[13]);store(0x3508,c[14]);'
            'store(0x4000,0x4800);store(0x4800+48,0x9000);store(0x4100,c[11]);store(0x4108,c[12]);'
            'Context ctx{};ctx.r1.u32=0x7F000;ctx.r3.u32=0x1000;ctx.r4.u32=0x1100;'
            'sub_%s(ctx,nullptr);printf("%%x %%x %%x %%x\\n",ctx.r3.u32&0xFF,load(0x1000),load(0x1004),load(0x1008));}' % (vt, fn)) + NL
    main_src = (disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + NL + body + '}' + NL)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fn, 'sub_' + fired, 'sub_' + held], main_src, a.out_dir, 'traffic_stuck', stubs=stubs, scratch_end=0x80000)
    out = [x for x in run(exe, a.image).split(NL) if x]
    assert len(out) == len(cs), (len(out), len(cs))
    lines = [' '.join('%x' % x for x in c) + ' -> ' + o for c, o in zip(cs, out)]
    for l in lines:
        assert not any(w.startswith('82') and len(w) == 8 for w in l.split()), l
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'traffic_stuck_ref.txt').write_text(NL.join(lines) + NL)
    print(len(lines), 'cases,', sum(o.startswith('1 ') for o in out), 'true')


if __name__ == '__main__':
    main()
