"""Run the hookman icon's frame builder F-90213 offline (static recompilation built with the recomp's own -msse4.1 -mssse3, native, flat guest memory) on 4,000 camera
rows / scales / pivots / places, for the port's `hookicons::frame_exact` to be compared with (test
icon_frame_matches_the_original).

F-90213 (asm, docs/reference/hookmen/triage_small_h0.md): f = 0 - the camera's row +96 (the camera from the global word the
code reads, 0x82839704 here; the start-up constants the image holds as zero set as the running game has them, 0x828609E0 =
+inf, 0x828608C0 = 0: docs/reference/ai/raceline.md), right = f x c (c the image's constant axis), up = right x f, each made unit by vmsum3fp,
vrsqrtefp (infinity -> the image's replacement) and one Newton step; the rows are scale (slot +132) x (right, up, f), the
translation place (r5) + the scaled pivot (slot +144) along them.

Cases: rows from edge lists (axes, straight up / down, zero, tiny, huge) and random unit / non-unit vectors; scale 1.4 and
random; pivots random; places random city-sized. Output in decimal (hex bits would read as addresses), one line a case:
  c <13 input words> | <16 output words>
"""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

CAM_GLOBAL = 0x82839704
_run = subprocess.run


def _gpp(cmd, *a, **k):
    # (the recomp's own x86 flags: simde then uses the CPU's dpps for vmsum3fp, whose sum order differs from simde's
    # portable loop - built without them the reference is the portable loop's, not the recomp's)
    if cmd and cmd[0] == 'g++':
        cmd = cmd[:1] + ['-msse4.1', '-mssse3'] + cmd[1:]
    return _run(cmd, *a, **k)

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); const uint32_t CAM=0x40A00000u, SLOT=0x40A01000u, POS=0x40A02000u, OUT=0x40A03000u; rt_reset();
 uint32_t w[13];
 for(int k=0;k<4;k++){store(0x828609E0u+4*k,0x7F800000u);store(0x828608C0u+4*k,0);}
 while(true){ for(int i=0;i<13;i++) if(!(in>>w[i])) { printf("end\n"); return 0; }
  store(CAMG,CAM); for(int i=0;i<4;i++){ store(CAM+96+4*i,w[i]); store(SLOT+144+4*i,w[5+i]); store(POS+4*i,w[9+i]); }
  store(SLOT+132,w[4]); for(int i=0;i<16;i++) store(OUT+4*i,0xA5A5A5A5u);
  Context c=fresh(); c.r3.u32=SLOT; c.r4.u32=OUT; c.r5.u32=POS; FRAME(c,nullptr);
  printf("c"); for(int i=0;i<13;i++) printf(" %u",w[i]); printf(" |"); for(int i=0;i<16;i++) printf(" %u",load(OUT+4*i)); printf("\n"); }
}
'''


def bits(f):
    return struct.unpack('<I', struct.pack('<f', f))[0]


def cases(n=4000, seed=90213):
    rnd = random.Random(seed)
    edge = [(0, 0, 1), (0, 0, -1), (1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 0), (1e-20, 0, 0), (3e18, 1, 2), (0.4737, 0.0717, -0.8778)]
    out = []
    for k in range(n):
        if k < len(edge) * 4:
            r = edge[k % len(edge)]
        elif rnd.random() < 0.6:
            v = [rnd.gauss(0, 1) for _ in range(3)]
            l = sum(x * x for x in v) ** 0.5 or 1.0
            r = [x / l for x in v]
        else:
            r = [rnd.uniform(-3, 3) for _ in range(3)]
        row = [bits(x) for x in r] + [bits(rnd.choice([0.0, 1.0, rnd.uniform(-1, 1)]))]
        scale = bits(1.4 if rnd.random() < 0.5 else rnd.uniform(0.1, 4))
        piv = [bits(x) for x in (rnd.uniform(-2, 2), rnd.uniform(-2, 2), rnd.uniform(-2, 2), rnd.choice([0.0, 1.0]))]
        pos = [bits(x) for x in (rnd.uniform(-3500, 3000), rnd.uniform(-50, 120), rnd.uniform(-2400, 2200), rnd.choice([0.0, 1.0]))]
        out.append(row + [scale] + piv + pos)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, help='write the output here')
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    r = subprocess.run([a.tool, 'registry', 'addr', 'F-90213'], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = 'sub_%08X' % addr['F-90213']
    img = rdisp.load_image(a.image)
    subprocess.run = _gpp
    exe = rdisp.build(rdisp.index(a.recomp), [name], MAIN.replace('FRAME', name).replace('CAMG', '0x%08Xu' % CAM_GLOBAL), a.out,
                      'icon_frame', image=img, limit=5)
    a.out.mkdir(parents=True, exist_ok=True)
    inp = a.out / 'frames.txt'
    inp.write_text(''.join(' '.join(map(str, c)) + '\n' for c in cases()))
    subprocess.run = _run
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    if a.ref:
        a.ref.write_text(r.stdout)
    print(r.stdout[:300], end='')


if __name__ == '__main__':
    main()
