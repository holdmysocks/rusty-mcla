"""Run the livery job's surface copy F-60873 offline as generated - the XDK's XGCopySurface as the job body F-42143 calls it:
linear A8R8G8B8 (0x18280086) -> DXT5 (0x1A200054), alpha threshold 0.0, no rectangle, no flags - on test images, and record
its blocks, for the port's `livery_bc3::encode_image` (crates/game/src/testdata/livery_bc3_ref.txt; test
livery_bc3::tests::blocks_match_the_original).

The copy reaches its block encoder F-45145 (BC3: colour F-39916 with F-11276 / F-85341, alpha F-71349 with F-18905) through
the image library's virtual calls; the five targets below are those it calls. Its two vector helpers that the shared shim
lacks (vcfux, vctuxs) are the recomp's own definitions (third_party/rexglue-sdk/include/rex/ppc/intrinsics.h), vperm as the
solver harness has it.

Images: 32 x 32, seeded: noise, gradients, flat blocks, two- and three-colour stripes with soft alpha edges (a vinyl's
shapes), near-grey noise, channel extremes, transparent fields with a few texels. Output per image:
  img <w> <h>
  <h lines: one image row each, its w ARGB words as one unbroken hex string (so no pixel reads as a code address)>
  <the blocks: 16 bytes each in the PC's order (the original's bytes swapped back per 16 bits), hex, one block per line>
"""
import argparse
import random
import re
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd
import solver_reference as _sr

TARGET_IDS = ['F-86787', 'F-20496', 'F-45145', 'F-89260', 'F-57185']

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok; const uint32_t S=0x40800000u, D=0x40C00000u;
 while(in>>tok){ if(tok!="img") break; uint32_t w,h; in>>std::dec>>w>>h;
  rt_reset();
  for(uint32_t i=0;i<w*h;i++){ uint32_t px; in>>std::hex>>px; store(S+4*i,px); }
  for(uint32_t i=0;i<w*h;i+=4) store(D+i,0);
  Context c=fresh();
  c.r3.u64=D; c.r4.u64=(w/4)*16; c.r5.u64=w; c.r6.u64=h; c.r7.u64=0x1A200054u; c.r8.u64=0; c.r9.u64=S; c.r10.u64=w*4;
  store(c.r1.u32+84,0x18280086u); store(c.r1.u32+92,0); store(c.r1.u32+100,0); c.f1.f64=0.0;
  COPY(c,nullptr);
  printf("ret %08x\n",c.r3.u32);
  for(uint32_t i=0;i<(w/4)*(h/4)*4;i++) printf("%08x%s",load(D+4*i),(i%4==3)?"\n":"");
 }
 printf("end\n");
}
'''


def shim_extra(recomp: Path) -> str:
    src = (recomp / 'third_party/rexglue-sdk/include/rex/ppc/intrinsics.h').read_text()

    def grab(sig):
        i = src.index(sig)
        j = src.index('{', i)
        d = 0
        for k in range(j, len(src)):
            d += {'{': 1, '}': -1}.get(src[k], 0)
            if d == 0:
                return 'static ' + src[i:k + 1]
    return ('#include <simde/x86/avx.h>\nnamespace rex { namespace ppc {\n' + grab('inline simde__m128 simde_mm_cvtepu32_ps_(') + '\n' +
            grab('inline simde__m128i simde_mm_vctuxs(') + '\n} }\n')


def images(seed=20261112):
    rnd = random.Random(seed)
    W = H = 32
    out = []

    def argb(a, r, g, b):
        cl = lambda v: max(0, min(255, int(round(v))))
        return cl(a) << 24 | cl(r) << 16 | cl(g) << 8 | cl(b)
    # noise
    out.append([rnd.getrandbits(32) for _ in range(W * H)])
    out.append([0xff000000 | rnd.getrandbits(24) for _ in range(W * H)])
    # gradients
    for _ in range(3):
        c = [[rnd.uniform(-200, 455) for _ in range(3)] for _ in range(4)]
        out.append([argb(255 if rnd.random() < 0.5 else 40 + 200 * x / W, *(c[0][k] + (c[1][k] - c[0][k]) * x / W + (c[2][k] - c[0][k]) * y / H for k in range(3)))
                    for y in range(H) for x in range(W)])
    # flat 4x4 or 8x8 blocks of one colour, opaque and not
    for _ in range(2):
        cells = {}
        img = []
        for y in range(H):
            for x in range(W):
                k = (x // 8, y // 4)
                if k not in cells:
                    cells[k] = rnd.choice([0xff000000, 0xffffffff, 0x00000000]) if rnd.random() < 0.2 else (rnd.choice([255, 255, 128, 0]) << 24 | rnd.getrandbits(24))
                img.append(cells[k])
        out.append(img)
    # stripes / shapes with soft edges (a vinyl over the paint)
    for _ in range(4):
        cols = [(rnd.randrange(256), rnd.randrange(256), rnd.randrange(256)) for _ in range(3)]
        ang = rnd.uniform(0, 6.283)
        import math
        dx, dy = math.cos(ang), math.sin(ang)
        width = rnd.uniform(2, 9)
        img = []
        for y in range(H):
            for x in range(W):
                t = (x * dx + y * dy) / width
                band = int(math.floor(t)) % 3
                edge = abs(t - math.floor(t) - 0.5) * 2
                a = 255 if edge < 0.7 else max(0, 255 * (1 - (edge - 0.7) / 0.3))
                if rnd.random() < 0.5:
                    a = 255
                c = cols[band]
                img.append(argb(a, *c))
        out.append(img)
    # near grey noise
    out.append([argb(255, *(g + rnd.randrange(-3, 4) for _ in range(3))) for g in (rnd.randrange(256) for _ in range(W * H))])
    # extremes
    out.append([argb(*(rnd.choice([0, 1, 254, 255]) for _ in range(4))) for _ in range(W * H)])
    # transparent with a few texels
    out.append([rnd.getrandbits(32) if rnd.random() < 0.06 else rnd.getrandbits(24) for _ in range(W * H)])
    # alpha gradients over flat colour
    out.append([argb(8 * (x % 32) * (y % 4 + 1) / 4, 200, 30, 90) for y in range(H) for x in range(W)])
    return W, H, out


def pc_order(words):
    b = b''.join(int(w, 16).to_bytes(4, 'big') for w in words)
    return bytes(b[i ^ 1] for i in range(len(b)))


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-60873'] + TARGET_IDS
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    assert len(addr) == len(ids), addr
    vs = rd.vector_shim
    extra = shim_extra(a.recomp)
    rd.vector_shim = lambda *x, **k: vs(*x, **k) + _sr.EXTRA2 + extra
    name = 'sub_%08X' % addr['F-60873']
    exe = rd.build(rd.index(a.recomp), [name], MAIN.replace('COPY', name), a.out, 'livery_bc3', image=rd.load_image(a.image),
                   targets=[addr[i] for i in TARGET_IDS], limit=2000)
    W, H, imgs = images()
    cf = a.out / 'livery_bc3_cases.txt'
    cf.write_text(''.join('img %d %d\n%s\n' % (W, H, ' '.join('%08x' % p for p in img)) for img in imgs) + 'stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True, timeout=600)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    parts = r.stdout.split('ret ')[1:]
    assert len(parts) == len(imgs), len(parts)
    rows = []
    blocks = 0
    for img, part in zip(imgs, parts):
        lines = part.strip().split('\n')
        assert lines[0] == '00000000', lines[0]
        words = re.findall(r'[0-9a-f]{8}', ''.join(l for l in lines[1:] if l != 'end'))
        pc = pc_order(words)
        rows.append('img %d %d\n%s' % (W, H, ''.join(''.join('%08x' % p for p in img[y * W:(y + 1) * W]) + '\n' for y in range(H))))
        rows.append(''.join(pc[i:i + 16].hex() + '\n' for i in range(0, len(pc), 16)))
        blocks += len(pc) // 16
    a.ref.write_text(''.join(rows))
    print('images', len(imgs), 'blocks', blocks, '->', a.ref)


if __name__ == '__main__':
    main()
