"""The rubber-band manager's race-end account (RubberBandMgr, F-20820) run offline, generated code whole and unchanged: the
(delta, bound) table by the won byte (low 8 bits of r4) and the race difficulty (r5), the points +64 moved by delta while
the level +56 is below (won) / above (lost) the bound, then at >= 18 / <= -18 points the level one step toward the bound
(clamped 0..bound / bound..10, written to +56 and +60) and the points rolled over (C remainder); else the points kept.

Grid: level -3..13, points -40..40, won byte 0 / 1 / 0x100 / 0x101, difficulty -1..4 (33,048 cases).
Writes <ref>: "level points won diff -> +56 +60 +64" (signed decimal; +60 is set to 77777 before the call)."""
import argparse
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rd

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);
image.assign(std::istreambuf_iterator<char>(bin),{});const uint32_t O=0x40100000u;static const uint32_t W[4]={0u,1u,0x100u,0x101u};
for(int lv=-3;lv<=13;lv++)for(int pt=-40;pt<=40;pt++)for(int w=0;w<4;w++)for(int d=-1;d<=4;d++){
 rt_reset();store(O+56,(uint32_t)lv);store(O+60,77777u);store(O+64,(uint32_t)pt);
 Context c=fresh();c.r3.u32=O;c.r4.u32=W[w];c.r5.u32=(uint32_t)d;FN(c,nullptr);
 printf("%d %d %u %d -> %d %d %d\n",lv,pt,W[w],d,(int)load(O+56),(int)load(O+60),(int)load(O+64));}}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    r = subprocess.run([a.tool, 'registry', 'addr', 'F-20820'], capture_output=True, text=True, check=True).stdout
    fn = 'sub_' + r.split()[1].upper()
    exe = rd.build(rd.index(a.recomp), [fn], MAIN.replace('FN(', fn + '('), a.out, 'adaptive', image=rd.load_image(a.image), limit=5)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True, timeout=300)
    if r.returncode:
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])
    a.ref.write_text(r.stdout)
    print(r.stdout.count('\n'), 'cases ->', a.ref)


if __name__ == '__main__':
    main()
