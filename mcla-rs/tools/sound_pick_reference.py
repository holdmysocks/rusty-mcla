"""A Randomized sound's weighted pick (F-39111) run offline: the generated code whole and unchanged, with the audio rand
F-60851 (the shared state pair 0x82836800 / +4, chained from case to case). r4 = the payload (+5 history length, +6..
the history bytes), r5 = the child count, r6 = the entries (8 bytes each, the weight a float at +4); r3 the child index.
Inputs: every Randomized payload of the disc's sounds.dat (its weights, its history length) with an empty history and
with random histories, and random ones (zero, equal, tiny and huge weights, histories longer and shorter than the list).
Writes <out-dir>/sound_pick_ref.txt: 'S<lo> S<carry> h<history length> n<count> x<history byte> ... w<weight bits> ...
-> r<index> S<lo> S<carry>' (hex)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
from recomp_closure import index, run

PAY = 0x40100000
ENT = 0x40110000
STACK = 0x40F00000
STATE = 0x82836800
MAXN = 64
FIELD_SIZE = [2, 2, 2, 2, 2, 2, 2, 2, 4, 4, 2, 2, 2, 4, 4, 2, 1, 1, 4, 4, 4, 4, 4, 4]


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def disc_payloads(path):
    d = open(path, 'rb').read()
    be = lambda o: struct.unpack_from('>I', d, o)[0]
    size = be(4)
    data = d[8:8 + size]
    p = 8 + size
    p += 4 + be(p)
    count = be(p)
    p += 8
    out = []
    for _ in range(count):
        p += 1 + d[p]
        off, sz = be(p), be(p + 4)
        p += 8
        o = data[off:off + sz]
        if o[0] != 14:
            continue
        mask = struct.unpack_from('>I', o, 11)[0]
        q = 15
        for b, s in enumerate(FIELD_SIZE):
            if mask >> b & 1:
                q += s
        pl = o[q:]
        nh = pl[5]
        n = pl[6 + nh]
        w = [struct.unpack_from('>I', pl, 7 + nh + 8 * i + 4)[0] for i in range(n)]
        out.append((nh, w))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--sounds', type=Path, required=True, help="the disc's audio/X360/config/sounds.dat")
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = addr(a.tool, 'F-39111')
    r = random.Random(39111)
    f = lambda x: struct.unpack('>I', struct.pack('>f', x))[0]
    cases = []
    disc = [c for c in disc_payloads(a.sounds) if 0 < len(c[1]) <= MAXN and c[0] <= 16]
    for nh, w in disc:
        n = len(w)
        cases.append((nh, w, [0xFF] * nh))
        for _ in range(2):
            cases.append((nh, w, [r.choice((0xFF, r.randrange(n))) for _ in range(nh)]))
    ndisc = len(cases)
    for _ in range(3000):
        n = r.choice((1, 2, 3, 4, 5, 8, r.randrange(1, MAXN + 1)))
        nh = r.choice((0, 0, 1, 2, n - 1 if n > 1 else 0, n, n + 1, r.randrange(0, 17)))
        nh = min(nh, 16)
        kind = r.random()
        if kind < 0.3:
            w = [f(1.0)] * n
        elif kind < 0.4:
            w = [f(r.choice((0.0, 0.0, 1.0)))] * n if r.random() < 0.5 else [f(r.choice((0.0, 1e-6, 1.0))) for _ in range(n)]
        else:
            w = [f(r.choice((0.0, 0.5, 1.0, 2.0, 0.1, 100.0, 1e-30, 3e30, r.uniform(0, 10)))) for _ in range(n)]
        hist = [r.choice((0xFF, r.randrange(n), r.randrange(256))) for _ in range(nh)]
        cases.append((nh, w, hist))
    rows = []
    for nh, w, hist in cases:
        rows.append([nh, len(w)] + hist + [0] * (16 - nh) + w + [0] * (MAXN - len(w)))
    W = 2 + 16 + MAXN
    data = ','.join(','.join('%du' % x for x in row) for row in rows)
    c = r"""
static const uint32_t C[]={@DATA@};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
store(@STATE@u,0x12345678u);store(@STATE@u+4,0x0badf00du);
for(size_t i=0;i<sizeof(C)/4;i+=@W@){const uint32_t*w=C+i;uint32_t nh=w[0],n=w[1];
 memset(pointer(@PAY@u),0,0x40);memset(pointer(@ENT@u),0,8*@MAXN@);
 pointer(@PAY@u+5)[0]=(uint8_t)nh;for(uint32_t k=0;k<nh;k++)pointer(@PAY@u+6+k)[0]=(uint8_t)w[2+k];
 for(uint32_t k=0;k<n;k++){store(@ENT@u+8*k,0x1000u+k);store(@ENT@u+8*k+4,w[18+k]);}
 uint32_t s0=load(@STATE@u),s1=load(@STATE@u+4);
 Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=0;ctx.r4.u32=@PAY@u;ctx.r5.u32=n;ctx.r6.u32=@ENT@u;sub_@FN@(ctx,nullptr);
 printf("S%08x S%08x h%x n%x",s0,s1,nh,n);for(uint32_t k=0;k<nh;k++)printf(" x%02x",w[2+k]);
 for(uint32_t k=0;k<n;k++)printf(" w%08x",w[18+k]);
 printf(" -> r%x S%08x S%08x\n",ctx.r3.u32,load(@STATE@u),load(@STATE@u+4));}}
"""
    for k, v in (('DATA', data), ('PAY', PAY), ('ENT', ENT), ('STACK', STACK), ('STATE', STATE), ('FN', fn), ('W', W), ('MAXN', MAXN)):
        c = c.replace('@%s@' % k, str(v))
    img = rd.load_image(a.image)
    exe = rd.build(index(a.recomp), ['sub_' + fn], c, a.out_dir, 'sound_pick', image=img)
    out = run(exe, a.image)
    lines = [x for x in out.split(chr(10)) if x]
    assert len(lines) == len(cases), (len(lines), len(cases))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'sound_pick_ref.txt').write_text(chr(10).join(lines) + chr(10))
    print(len(lines), 'cases,', ndisc, 'of them from the disc payloads')


if __name__ == '__main__':
    main()
