"""A walker's physics bound as the original makes it, run offline on its code: F-37628 (new(224), the capsule's ctor
F-25220, F-77704 with radius 0.3 and length one step under 1.2 - it calls the capsule's extents, vtable slot 40 F-31102 -,
then slot 6 F-76267 with the offset (0, 0.9, 0): F-67839 sets the centroid +48 and byte +5, and slot 40 again). Only the
allocator is the harness's own bump allocator; the start-up initialisers run first (F-67839's xyz mask is one of the
vector constants they fill).

Out: the bound's words +0 .. +60 (type byte +4, byte +5, +8 the radius, +12, +16 .. +28 the box's max, +32 .. +44 its
min, +48 .. +60 the centroid), +128 .. +140 (the radius splat), +144 .. +156 (the length splat) and +208 (the material
word slot 12 F-60139 answers). Writes <out-dir>/walker_bound_ref.txt. Test `camprobe::tests::walker_bound_matches_the_original`."""
import argparse
import re
import subprocess
from pathlib import Path
import memcap  # (every harness process capped at 4 GB)
import recomp_dispatch as rd
import level_query_reference  # (its vector shim and -msse4.1 for the capsule vtable's closure)

_run = subprocess.run
# (the start-up initialisers that fill the vector constants the image holds as zero - F-67839's xyz mask among them)
INITS = ['F-50173', 'F-33402', 'F-26199', 'F-55749', 'F-97349', 'F-70580', 'F-79721', 'F-64228', 'F-24192', 'F-92824']


def addr(tool, fid):
    return _run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


MAIN = r'''
static const uint32_t STK=0x40DF0000;
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 rt_reset(); { INITCALLS } store(0x40000200u+32,load(0x40000200u+28));
 Context c=fresh(); c.r1.u64=STK;
 try{ sub_F_(c,nullptr); }catch(std::exception&e){ printf("EXC %s\n",e.what()); return 1; }
 uint32_t b=c.r3.u32; if(!b){ printf("EXC null\n"); return 1; }
 printf("%08x",b);
 for(uint32_t o=0;o<64;o+=4) printf(" %08x",load(b+o));
 for(uint32_t o=128;o<160;o+=4) printf(" %08x",load(b+o));
 printf(" %08x\n",load(b+208)); }
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    work = a.out_dir / 'build'
    work.mkdir(parents=True, exist_ok=True)
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    fn = addr(a.tool, 'F-37628')
    inits = [addr(a.tool, f) for f in INITS]
    main_src = MAIN.replace('sub_F_', 'sub_' + fn).replace('INITCALLS', ' '.join('{Context g=fresh(); sub_%s(g,nullptr);}' % x for x in inits))
    exe = rd.build(idx, ['sub_' + fn] + ['sub_' + x for x in inits], main_src, work, 'walker_bound', image=img, vtables=[(0x8207FF34, 48)])
    r = _run([str(exe), str(a.image)], capture_output=True, text=True)
    out = [x for x in r.stdout.split('\n') if x]
    assert len(out) == 1 and 'EXC' not in out[0], (out, r.stderr[-800:])
    w = out[0].split()
    assert len(w) == 1 + 16 + 8 + 1, len(w)  # (the address, +0 .. +60, +128 .. +156, +208: the reader's layout)
    text = ' '.join(w[1:]) + chr(10)
    (a.out_dir / 'walker_bound_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', text))
    print(text)


if __name__ == '__main__':
    main()
