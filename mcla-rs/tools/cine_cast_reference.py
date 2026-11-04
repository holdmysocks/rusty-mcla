"""The scenes' generic cast, CineScript_Characters_AssignBoyToSlot (F-40835) and AssignGirlToSlot (F-49041), run offline on the
original's code: slot 0 takes the next driver set of a 7 (boy) / 3 (girl) list by its own counter, skipping a set an active
racer already wears (F-68164) at most twice; any other slot takes the next of a 10 / 6 pedestrian list by a second counter.
The four counters live in the image's data and are set from the case. Stubbed: F-68164 (answers bit k of the case's mask on
its k-th call) and F-65272 (AssignNameToSlot: records the set name and the slot).
Lines: `CC girl slot c0 c1 c2 c3 mask -> set slot c0 c1 c2 c3`  (c0 boy driver, c1 girl driver, c2 boy ped, c3 girl ped)
Usage: cine_cast_reference.py --recomp D:/MCLA-static --image IMG --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_dispatch as rd

BOY, GIRL, IN_USE, NAME = 'F-40835', 'F-49041', 'F-68164', 'F-65272'
# the four counters (driver boy / girl, pedestrian boy / girl), the cine script manager pointer
CTRS = (0x828CD444, 0x828CD448, 0x828CD44C, 0x828CD450)
MGR = 0x8286D8F0

MAIN = r'''
static std::string cstr(uint32_t a){std::string s;for(;;){uint8_t c=*pointer(a++);if(!c)break;s+=char(c);}return s;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);int girl,slot;int32_t c0,c1,c2,c3;uint32_t mask;const uint32_t ANS=0x40809000,MG=0x4080A000;
 while(in>>girl>>slot>>c0>>c1>>c2>>c3>>mask){rt_reset();
  store(CTR0,c0);store(CTR1,c1);store(CTR2,c2);store(CTR3,c3);store(MGRP,MG);
  store(ANS,mask);store(ANS+4,0);store(ANS+8,0);store(ANS+12,0xFFFFFFFF);
  Context c=fresh();c.r3.u32=uint32_t(slot);if(girl)sub_GIRL(c,nullptr);else sub_BOY(c,nullptr);
  printf("%s %d %d %d %d %d\n",cstr(load(ANS+8)).c_str(),int32_t(load(ANS+12)),int32_t(load(CTR0)),int32_t(load(CTR1)),int32_t(load(CTR2)),int32_t(load(CTR3)));}
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
    boy, girl = ('sub_' + fid_addr(a.tool, f) for f in (BOY, GIRL))
    ans = 0x40809000
    stubs = {'sub_' + fid_addr(a.tool, IN_USE): f'{{uint32_t k=load({ans + 4:#x}u);store({ans + 4:#x}u,k+1);ctx.r3.u64=(load({ans:#x}u)>>k)&1;}}',
             'sub_' + fid_addr(a.tool, NAME): f'store({ans + 8:#x}u,ctx.r3.u32);store({ans + 12:#x}u,ctx.r4.u32);'}
    src = MAIN.replace('sub_BOY', boy).replace('sub_GIRL', girl).replace('MGRP', f'{MGR:#x}u')
    for k, c in enumerate(CTRS):
        src = src.replace(f'CTR{k}', f'{c:#x}u')
    exe = rd.build(idx, [boy, girl], src, a.work, 'cine_cast', image=img, stubs=stubs)
    rnd = random.Random(20261104)
    cases = []
    for g in (0, 1):
        for slot in (0, 1, 2, 3, 5, 7, 15):
            for mask in range(8):
                for ctr in list(range(-1, 23)) + [rnd.randrange(0, 100000) for _ in range(6)] + [2**31 - 4]:
                    cs = [rnd.randrange(0, 1000) for _ in range(4)]
                    cs[(1 if g else 0) if slot == 0 else (3 if g else 2)] = ctr
                    cases.append((g, slot, *cs, mask))
    cf = a.work / 'cine_cast_cases.txt'
    cf.write_text(''.join(' '.join(map(str, c)) + '\n' for c in cases))
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    out = [f'CC {" ".join(map(str, c))} -> {o}' for c, o in zip(cases, r.stdout.splitlines())]
    a.out.write_text('\n'.join(out) + '\n')
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
