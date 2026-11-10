"""The weather manager's track registrations F-57705 (colour correction) and F-47246 (fog) run offline as generated
(cameras2-map4), written to crates/game/src/testdata/map_weather_ref.txt for the port's tests. recomp_closure imports tools/memcap.py,
so every process of this script (g++, the harness) runs in a 4 GB job.
Stubbed, with a recorder: the name formatter F-98741 (a host snprintf over the format and the one %s argument it is given), the
schema's member registration (answers a node whose slot receives the member address) and the member objects' virtual type call
(a fake vtable). The loops, the format strings and the suffix table are read from the executable image as the functions use them.
One line per registered member: "REG <function> <call index> <name> A<array>:<member index in it>",
followed by the suffix table "SUF <index> <suffix>" and the format strings "FMT <function> <slot> <format>".
Usage: map_weather_reference.py --recomp D:/MCLA-static --image IMG --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import subprocess
from pathlib import Path
import recomp_dispatch as rd
import recomp_closure as rc

COMMON = r'''
static int ncall;static char names[64][96];static uint32_t slots[64];
'''

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 const uint32_t FVT=0x40030000;
 uint32_t members[2][5]={{0,0,0,0,0},{0,0,0,0,0}};
 for(int which=0;which<2;which++){
  rt_reset();ncall=0;
  // every member object the function can reach (40 bytes apart, from the globals the function forms) points at the fake vtable
  const uint32_t bases[6]={BASES_};
  for(int b=0;b<6;b++)for(int i=0;i<4;i++)store(bases[b]+40*i,FVT);
  store(FVT+4,0x7F000010u);
  Context cx=fresh();cx.r3.u32=0x40031000u;
  if(which==0)sub_COLOR(cx,nullptr);else sub_FOG(cx,nullptr);
  for(int i=0;i<ncall;i++){uint32_t m=load(slots[i]+8);int k=-1;for(int b=0;b<6;b++)if(m>=bases[b]&&m<bases[b]+160){k=b;break;}
   if(k<0||(m-bases[k])%40)throw std::runtime_error("member outside the arrays");printf("REG %d %d %s A%d:%u\n",which,i,names[i],k,(m-bases[k])/40);}
 }
 // the suffix table and the format strings, as the functions read them
 const uint32_t tab=TAB_;
 for(int i=0;i<4;i++){uint32_t p=load(tab+4*i);std::string s;for(;*pointer(p);p++)s+=char(*pointer(p));printf("SUF %d %s\n",i,s.c_str());}
}
'''


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
    a.work.mkdir(parents=True, exist_ok=True)

    def name(fid):
        return 'sub_' + subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]
    rc.EXTRA += COMMON
    stubs = {
        name('F-98741'): ('{char tmp[96];std::string f;for(uint32_t p=ctx.r4.u32;*pointer(p);p++)f+=char(*pointer(p));std::string s;'
                        'for(uint32_t p=ctx.r5.u32;*pointer(p);p++)s+=char(*pointer(p));'
                        'if(f.size()<3||f.compare(f.size()-2,2,"%s")!=0||f.find("%")!=f.size()-2)throw std::runtime_error("unexpected format");'
                        'snprintf(tmp,sizeof tmp,f.c_str(),s.c_str());for(int i=0;;i++){*pointer(ctx.r3.u32+i)=tmp[i];if(!tmp[i])break;}}'),
        name('F-34516'): ('{std::string s;for(uint32_t p=ctx.r4.u32;*pointer(p);p++)s+=char(*pointer(p));strcpy(names[ncall],s.c_str());'
                         'uint32_t node=0x40032000u+32*ncall;slots[ncall]=node+16;store(node+4,node+16);ncall++;ctx.r3.u32=node;}'),
    }
    fakes = {0x7F000010: 'ctx.r3.u32=ctx.r3.u32;'}
    hi = lambda x: (x << 16) & 0xFFFFFFFF
    # the member arrays (as the two functions form them: the high half of a register plus a displacement) and the suffix table
    bases = [hi(-32120) - 11712, hi(-32120) - 11372, hi(-32120) - 11212, hi(-32120) - 10572, hi(-32120) - 10412, hi(-32120) - 10092]
    tab = hi(-32130) + 6912
    src = (MAIN.replace('sub_COLOR', name('F-57705')).replace('sub_FOG', name('F-47246'))
           .replace('BASES_', ','.join('0x%08Xu' % b for b in bases)).replace('TAB_', '0x%08Xu' % tab))
    exe = rd.build(idx, [name('F-57705'), name('F-47246')], src, a.work, 'map_weather', image=img, stubs=stubs, fakes=fakes)
    p = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    lines = p.stdout.splitlines()
    out = []
    for l in lines:
        f = l.split()
        if f[0] == 'REG':
            out.append(l)
        else:
            out.append(l)
    a.out.write_text('\n'.join(out) + '\n')
    print(len(lines), 'lines')


if __name__ == '__main__':
    main()
