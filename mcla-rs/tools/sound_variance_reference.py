"""A sound's variance draws (F-40563) run offline: the generated code whole and unchanged, with the game's rand F-62480
(the shared state pair 0x82836800 / +4, chained from case to case). r3 = the new sound (+28 volume, +32 pitch, +10 pan,
+88 predelay, +104 start offset), r4 = the expanded header (+15 volume, +17 its variance, +19 pitch, +21, +23 pan
(0xFFFF none), +25, +27 predelay, +29, +31 start offset u32, +35 u32). Inputs: every distinct field set of the disc's
sounds.dat and random ones (zero and non-zero variances, no pan, large start offsets).
Writes <out-dir>/sound_variance_ref.txt: 'S<lo> S<carry> v<vol> v<var> p<pitch> p<var> n<pan> n<var> d<predelay> d<var>
o<start> o<var> -> q<+28 bits> q<+32> q<+10> q<+88> q<+104> S<lo> S<carry>' (hex)."""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd
from recomp_closure import index, run

SND = 0x40100000
HDR = 0x40110000
STACK = 0x40F00000
STATE = 0x82836800


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def disc_headers(path, field_size):
    d = open(path, 'rb').read()
    be = lambda o: struct.unpack_from('>I', d, o)[0]
    size = be(4)
    data = d[8:8 + size]
    p = 8 + size
    p += 4 + be(p)
    count = be(p)
    p += 8
    out = set()
    for _ in range(count):
        p += 1 + d[p]
        off, sz = be(p), be(p + 4)
        p += 8
        o = data[off:off + sz]
        mask = struct.unpack_from('>I', o, 11)[0]
        q, v = 15, [0] * 24
        for b, s in enumerate(field_size):
            if mask >> b & 1:
                v[b] = o[q] if s == 1 else struct.unpack_from('>H' if s == 2 else '>I', o, q)[0]
                q += s
        pan = v[4] if mask >> 4 & 1 else 0xFFFF
        out.add((v[0], v[1], v[2], v[3], pan, v[5], v[6], v[7], v[8], v[9]))
    return sorted(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--sounds', type=Path, required=True, help="the disc's audio/X360/config/sounds.dat")
    ap.add_argument('--audcfg', type=Path, required=True, help='crates/formats/src/audcfg.rs (its FIELD_SIZE table)')
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fn = addr(a.tool, 'F-40563')
    fsz = eval('[' + re.search(r'FIELD_SIZE: \[usize; 24\] = \[([^\]]*)\]', a.audcfg.read_text()).group(1) + ']')
    cases = disc_headers(a.sounds, fsz)
    ndisc = len(cases)
    r = random.Random(40563)
    u16 = lambda: r.choice((0, 0, 1, 100, 600, 1200, 65535, 32768, r.randrange(0, 65536)))
    var = lambda: 0 if r.random() < 0.4 else r.choice((1, 2, 50, 100, 360, 1000, 30000, r.randrange(0, 65536)))
    for _ in range(3000):
        pan = 0xFFFF if r.random() < 0.3 else r.choice((0, 90, 180, 359, 360, 720, 65000, r.randrange(0, 65536)))
        cases.append((u16(), var(), u16(), var(), pan, var(), u16(), var(),
                      r.choice((0, 0, 50, 100, 1000, 0x7FFFFFFF, 0xFFFFFFFF, r.getrandbits(32))),
                      0 if r.random() < 0.5 else r.choice((1, 10, 100, 1000, 0x3FFFFFFF, r.getrandbits(30)))))
    data = ','.join(','.join('%du' % x for x in c) for c in cases)
    c = r"""
static const uint32_t C[]={@DATA@};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);
store(@STATE@u,0x12345678u);store(@STATE@u+4,0x0badf00du);
for(size_t i=0;i<sizeof(C)/4;i+=10){const uint32_t*w=C+i;
 memset(pointer(@SND@u),0,0x100);memset(pointer(@HDR@u),0,0x40);
 for(int k=0;k<8;k++)store16(@HDR@u+15+2*k,w[k]);
 for(int k=0;k<4;k++){pointer(@HDR@u+31+k)[0]=(uint8_t)((w[8]>>(24-8*k)));pointer(@HDR@u+35+k)[0]=(uint8_t)((w[9]>>(24-8*k)));}
 uint32_t s0=load(@STATE@u),s1=load(@STATE@u+4);
 Context ctx{};ctx.r1.u32=@STACK@u;ctx.r3.u32=@SND@u;ctx.r4.u32=@HDR@u;sub_@FN@(ctx,nullptr);
 printf("S%08x S%08x",s0,s1);for(int k=0;k<10;k++)printf(" x%08x",w[k]);
 printf(" -> q%08x q%04x q%04x q%08x q%08x S%08x S%08x\n",load(@SND@u+28),load16(@SND@u+32),load16(@SND@u+10),load(@SND@u+88),
  load(@SND@u+104),load(@STATE@u),load(@STATE@u+4));}}
"""
    for k, v in (('DATA', data), ('SND', SND), ('HDR', HDR), ('STACK', STACK), ('STATE', STATE), ('FN', fn)):
        c = c.replace('@%s@' % k, str(v))
    img = rd.load_image(a.image)
    exe = rd.build(rd.index(a.recomp) if hasattr(rd, 'index') else index(a.recomp), ['sub_' + fn], c, a.out_dir, 'sound_variance', image=img)
    out = run(exe, a.image)
    lines = [x for x in out.split(chr(10)) if x]
    assert len(lines) == len(cases), (len(lines), len(cases))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'sound_variance_ref.txt').write_text(chr(10).join(lines) + chr(10))
    print(len(lines), 'cases,', ndisc, 'of them the disc field sets')


if __name__ == '__main__':
    main()
