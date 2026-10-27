"""The driver gestures' round-robin pickers (F-80898 neutral, F-40322 left, F-82819 right, F-95452 talk left, F-59643 talk right) run
offline on the original's code: the clip lookup they end in (F-99427, which would search the animation pack) is an empty stub, so the
name pointer it was given is read back from r4. The tables and their strings come from the executable image.
Lines: `GN kind human counter -> name new_counter`   kind 0 neutral (human 0/1), 1 left, 2 right, 3 talk left, 4 talk right
Usage: people_gesture_reference.py --recomp D:/MCLA-static --image IMG --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_dispatch as rd

IDS = ['F-80898', 'F-40322', 'F-82819', 'F-95452', 'F-59643']
LOOKUP = 'F-99427'

MAIN = r'''
static std::string cstr(uint32_t a){std::string s;for(;;){uint8_t c=*pointer(a++);if(!c)break;s+=char(c);}return s;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);int kind,human;uint32_t counter;
 while(in>>kind>>human>>counter){rt_reset();
  store(0x1000,counter);Context c=fresh();c.r3.u32=0x2000;c.r4.u32=0x1000;c.r5.u32=human;c.r4.u32=0x1000;
  switch(kind){case 0:sub_G0(c,nullptr);break;case 1:sub_G1(c,nullptr);break;case 2:sub_G2(c,nullptr);break;case 3:sub_G3(c,nullptr);break;default:sub_G4(c,nullptr);}
  printf("%s %u\n",cstr(c.r4.u32).c_str(),load(0x1000));}
}
'''


def fid_addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    fns = ['sub_' + fid_addr(a.tool, f) for f in IDS]
    lookup = 'sub_' + fid_addr(a.tool, LOOKUP)
    src = MAIN
    for k, n in enumerate(fns):
        src = src.replace(f'sub_G{k}', n)
    exe = rd.build(idx, fns, src, a.work, 'people_gesture', image=img, stubs=[lookup])
    rnd = random.Random(20261029)
    cases = []
    for kind in range(5):
        for human in ((0, 1) if kind == 0 else (0,)):
            for ctr in list(range(12)) + [rnd.randrange(0, 5000) for _ in range(10)] + [2**20 + 3, 2**31 - 2]:
                cases.append((kind, human, ctr))
    cf = a.work / 'people_gesture_cases.txt'
    cf.write_text('\n'.join(f'{k} {h} {c}' for k, h, c in cases) + '\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    out = [f'GN {k} {h} {c} -> {o}' for (k, h, c), o in zip(cases, r.stdout.splitlines())]
    a.out.write_text('\n'.join(out) + '\n')
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
