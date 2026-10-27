"""Small UI functions of the original run offline (ui_closure / recomp_closure): unchanged generated code on a flat guest memory.
F-#####s by address through `mcla-tool registry addr`. Writes ui_game_ref.txt, one case per line:
  PT  type sel -> word word                  power-up HUD type cell (the cell chosen by sel gets the word, the other keeps aaaaaaaa)
  PR  F-id -> icon:key icon:key ..           photo album prompt-bar lists (static constructors; the row adder is a recording stub)
  VD  F-id -> text                           the menus' dialog description getters"""
import argparse
import subprocess
from pathlib import Path
from ui_closure import index, build

PROMPTS = ['F-73259', 'F-29965', 'F-22161', 'F-44052', 'F-90804']
DESCS = ['F-84077', 'F-46420', 'F-79373', 'F-69707']
CELL, ADD_ROW = 'F-56191', 'F-54607'
REC = 'int nrec; uint32_t rec[64][2];\n'
STUB = 'extern int nrec; extern uint32_t rec[64][2];rec[nrec][0]=ctx.r4.u32;rec[nrec][1]=ctx.r5.u32;nrec++;'

MAIN = r'''
static std::string cstr(uint32_t a){std::string s;for(;;a++){char c=(char)*pointer(a);if(!c)break;s+=c;}return s;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int t=0;t<32;t++)for(int sel=0;sel<2;sel++){std::memset(objects,0,sizeof objects);store(0x1000,0x1100);store(0x1004,0x1104);
  store(0x1100,0xaaaaaaaa);store(0x1104,0xaaaaaaaa);Context c{};c.r1.u32=0x2f00;c.r3.u32=0x1000;c.r4.u32=sel;c.r5.u32=t;sub_CELL(c,nullptr);
  printf("PT %d %d -> %x %x\n",t,sel,load(0x1100),load(0x1104));}
 CALLS
}
'''
PR = r'{nrec=0;Context c{};c.r1.u32=0x2f00;sub_%s(c,nullptr);printf("PR %s ->");for(int i=0;i<nrec;i++)printf(" %%u:%%s",rec[i][0],cstr(rec[i][1]).c_str());printf("\n");}'
VD = r'{Context c{};c.r1.u32=0x2f00;sub_%s(c,nullptr);printf("VD %s -> %%s\n",cstr(c.r3.u32).c_str());}'


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = index(a.recomp)
    ad = {f: addr(a.tool, f) for f in PROMPTS + DESCS + [CELL, ADD_ROW]}
    calls = '\n'.join([PR % (ad[p], p) for p in PROMPTS] + [VD % (ad[d], d) for d in DESCS])
    src = REC + MAIN.replace('CALLS', calls).replace('sub_CELL', 'sub_' + ad[CELL])
    names = ['sub_' + ad[f] for f in [CELL] + PROMPTS + DESCS]
    exe = build(idx, names, src, a.work, 'ui_game', stubs={'sub_' + ad[ADD_ROW]: STUB}, vector=True)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True, check=True)
    a.out.write_text(r.stdout)
    print(len(r.stdout.splitlines()), 'lines')


if __name__ == '__main__':
    main()
