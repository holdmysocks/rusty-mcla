"""Run small car-part functions of the original offline and record their results for the port's tests.

Unchanged generated code (all four are leaves) on a flat guest memory:
  F-76270 flap drawn angle, F-15658 flap last-frame angle, F-63094 damage piece hit, F-38353 vinyl half-float pair unpack.
Writes into --out-dir the reference files the tests of crates/game read:
  flap_drawn.txt   "reversed locked angle prev_angle drawn drawn_prev" (f32 bits, hex)
  pieces_hit.txt   "kind amount px py pz cx cy cz radius hit scrape hit_min scrape_min hit' scrape'" (f32 bits, hex)
  half_original.bin  u32 BE for each u16 h: the float the original unpacks from (h)
"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = index(a.recomp)
    drawn, prev, hit, half = (addr(a.tool, f) for f in ['F-76270', 'F-15658', 'F-63094', 'F-38353'])
    rnd = random.Random(7)
    angles = [0.0, -0.0, 0.2, -0.2, 0.15, -0.15, 1.0, -1.0, 1e-30, float('inf'), 3.5e-45]
    body = ''
    for rev in (0, 1, 2):
        for lock in (0, 1, 7):
            for ang in angles + [rnd.uniform(-1, 1) for _ in range(12)]:
                pa = fbits(rnd.uniform(-1, 1))
                body += ('{memset(objects,0,sizeof(objects));objects[188]=%d;objects[189]=%d;store(0x1000+180,0x%08x);store(0x1000+184,0x%08x);'
                         'Context a{};a.r3.u32=0x1000;a.r1.u32=0x2800;sub_%s(a,nullptr);float d=float(a.f1.f64);uint32_t db;memcpy(&db,&d,4);'
                         'Context b{};b.r3.u32=0x1000;b.r1.u32=0x2800;sub_%s(b,nullptr);float e=float(b.f1.f64);uint32_t eb;memcpy(&eb,&e,4);'
                         'printf("F %%d %%d %%08x %%08x %%08x %%08x\\n",%d,%d,0x%08x,0x%08x,db,eb);}\n'
                         % (rev, lock, fbits(ang), pa, drawn, prev, rev, lock, fbits(ang), pa))
    for n in range(3000):
        c = [rnd.uniform(-3, 3) for _ in range(3)]
        d = [rnd.uniform(-1, 1) * (1 if n % 3 == 0 else 0.01) for _ in range(3)]
        p = [c[i] + d[i] for i in range(3)]
        f32 = lambda x: struct.unpack('>f', struct.pack('>f', x))[0]
        dist = sum(f32(f32(p[i]) - f32(c[i])) ** 2 for i in range(3)) ** 0.5
        rad = [dist, f32(dist) * (1 + 2e-7), f32(dist) * (1 - 2e-7), rnd.uniform(0, 2), 0.0, 1.0][n % 6]
        t0 = rnd.choice([0.0, 0.01, 0.5, rnd.uniform(0, 3)])
        t1 = rnd.choice([0.0, 0.01, 0.5, rnd.uniform(0, 3)])
        amt = rnd.choice([t0, t1, 0.0, rnd.uniform(0, 3), rnd.uniform(0, 100)])
        kind = rnd.choice([0, 1, 1, 2, 255])
        v = [fbits(x) for x in p + c + [rad, rnd.uniform(0, 2), rnd.uniform(0, 2), t0, t1]]
        body += ('{memset(objects,0,sizeof(objects));Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1400;ctx.r6.u32=%d;ctx.r1.u32=0x2800;'
                 'static const uint32_t v[11]={%s};'
                 'store(0x1400,v[0]);store(0x1404,v[1]);store(0x1408,v[2]);store(0x1020,v[3]);store(0x1024,v[4]);store(0x1028,v[5]);'
                 'store(0x1030,v[6]);store(0x1034,v[7]);store(0x1038,v[8]);store(0x103c,v[9]);store(0x1040,v[10]);'
                 'ctx.f1.f64=double(bf(0x%08x));sub_%s(ctx,nullptr);'
                 'printf("P %%d %%08x",%d,0x%08x);for(int i=0;i<11;i++)printf(" %%08x",v[i]);printf(" %%08x %%08x\\n",load(0x1034),load(0x1038));}\n'
                 % (kind, ','.join('0x%x' % x for x in v), fbits(amt), hit, kind, fbits(amt)))
    main_src = ('static float bf(uint32_t u){float f;memcpy(&f,&u,4);return f;}\n'
                'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});\n' + body +
                'for(uint32_t h=0;h<65536;h++){std::memset(objects,0,sizeof(objects));Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1400;ctx.r1.u32=0x2800;'
                'store16(0x1400,h);store16(0x1402,h);sub_%s(ctx,nullptr);printf("H %%04x %%08x %%08x\\n",h,load(0x1000),load(0x1004));}\n}\n' % half)
    exe = build(idx, [f'sub_{x}' for x in (drawn, prev, hit, half)], main_src, a.out, 'carparts_small')
    flaps, pieces, hb = [], [], bytearray(65536 * 4)
    for line in run(exe, a.image).splitlines():
        t = line.split()
        if t[0] == 'F':
            flaps.append(' '.join(t[1:]))
        elif t[0] == 'P':
            pieces.append(' '.join(t[1:]))
        else:
            h = int(t[1], 16)
            assert t[2] == t[3]
            hb[h * 4:h * 4 + 4] = bytes.fromhex(t[2])
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'flap_drawn.txt').write_text('\n'.join(flaps) + '\n')
    (a.out_dir / 'pieces_hit.txt').write_text('\n'.join(pieces) + '\n')
    (a.out_dir / 'half_original.bin').write_bytes(bytes(hb))
    print(len(flaps), 'flap cases', len(pieces), 'piece cases')


if __name__ == '__main__':
    main()
