"""The palm test F-62185 of the plant draw run as generated on every shipped foliage type name and a few edge names (cameras2-map6),
written to crates/game/src/testdata/map_palm_ref.txt for props.rs `palm`. recomp_closure imports tools/memcap.py (4 GB job).
One line per name: "<name> <0|1>".
Usage: map_palm_reference.py --recomp D:/MCLA-static --image IMG --types FILE --work DIR --out FILE [--tool mcla-tool]
(--types: the list `dump_type_list` in props/impostor.rs writes, the name first on each line)"""
import argparse
import subprocess
from pathlib import Path
import recomp_dispatch as rd

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int i=2;i<argc;i++){rt_reset();uint32_t a=0x40100000u;const char*s=argv[i];size_t n=strlen(s);
  for(size_t k=0;k<32;k++)*pointer(a+k)=uint8_t(k<n?s[k]:0);
  Context cx=fresh();cx.r3.u32=a;sub_PALM(cx,nullptr);printf("%s %u\n",s,cx.r3.u32&255);}
}
'''
EDGE = ['palm', 'la_prop_gfx_pal', 'la_prop_gfx_Palm_x', 'xxxxxxxxxxxxxxxxxpal', 'xxxxxxxxxxxxxxxxxpa', 'xxxxxxxxxxxxpa']


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    for k in ('recomp', 'image', 'types', 'work', 'out'):
        ap.add_argument('--' + k, type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    name = 'sub_' + subprocess.run([a.tool, 'registry', 'addr', 'F-62185'], capture_output=True, text=True, check=True).stdout.split()[1]
    names = [l.split()[0] for l in a.types.read_text().splitlines() if l.strip()] + EDGE
    a.work.mkdir(parents=True, exist_ok=True)
    exe = rd.build(rd.index(a.recomp), [name], MAIN.replace('sub_PALM', name), a.work, 'map_palm', image=rd.load_image(a.image))
    p = subprocess.run([str(exe), str(a.image)] + names, capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    out = p.stdout.splitlines()
    assert len(out) == len(names), (len(out), len(names))
    a.out.write_text('\n'.join(out) + '\n')
    print(len(out), 'names')


if __name__ == '__main__':
    main()
