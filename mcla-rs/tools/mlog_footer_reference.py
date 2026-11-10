"""The mission log's footer prompt F-49278 run offline as generated: the list's name (+20) "Navmap_MLog_ActiveMissions_Tutorials"
leaves the prompt as it is; otherwise the nav map (NM) byte +376 (the map opened in a race), a list not named
"Navmap_MLog_ActiveMissions", a selection (vt+576) not below the count (+124), no target record for the row's open objective
(F-96100 stubbed: it hands the row's word on as the id; the target manager's get F-49590 stubbed: it answers the case's record) or
a record with +8 set, or NM +375 clear (navMap.SetTargetSelectionEnabled) give prompt 44; else 42 when the record is the map's
current target (NM +616, its vt+564) and 41 when not. The prompt call F-40654 is stubbed: it records its id.
Line:  <list 0 tutorials / 1 active / 2 completed> <+376> <+375> <sel> <count> <record 0 none / 1 shown / 2 +8 set>
       <target 0 none / 1 the record / 2 another / 3 an object answering none> -> <prompt id, 0 not called>
Usage: mlog_footer_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_dispatch as rd

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]);
 const uint32_t T=0x40010000,V=0x40011000,NM=0x40012000,W=0x40012800,TG=0x40012900,UI=0x40013000,R=0x40014000,REC=0x40015000,
   E2=0x40015100,ITEMS=0x40016000,NAMES[3]={NAME_TUT,NAME_ACT,NAME_DONE};
 uint32_t l,a,b,s,n,r,t;
 while(in>>l>>a>>b>>s>>n>>r>>t){rt_reset();
  for(uint32_t x=T;x<0x40016100;x+=4)store(x,0);
  store(T,V);store(V+576,0x7F000010u);store(T+20,NAMES[l]);store(T+120,ITEMS);store(T+124,n<<16);store(R+8,s);
  for(uint32_t k=0;k<16;k++)store(ITEMS+4*k,100+k);
  store(G_NM,NM);store(G_UI,UI);store(UI+48,0x40017000u);
  store(NM+372,b);store(NM+376,a<<24);
  store(REC+8,(r==2?1u:0u)<<24);store(R+12,r?REC:0);
  store(W+564,0x7F000020u);store(TG,W);store(NM+616,t?TG:0);store(R+16,t==1?REC:(t==2?E2:0));
  Context cx=fresh();cx.r3.u32=T;sub_FOOTER(cx,nullptr);
  printf("%u\n",load(R));
 }
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
    addr = lambda fid: 'sub_' + subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]
    footer = addr('F-49278')
    img = rd.load_image(a.image)
    # (the two names the footer compares with, as it loads them: lis / addi pairs; the completed list's name found in the image)
    tut, act = ((-32247 << 16) - 14636) & 0xFFFFFFFF, ((-32247 << 16) - 14664) & 0xFFFFFFFF
    for v, s in ((tut, b'Navmap_MLog_ActiveMissions_Tutorials\0'), (act, b'Navmap_MLog_ActiveMissions\0')):
        assert img[v - rd.IMAGE_BASE:][:len(s)] == s
    done = img.find(b'\0Navmap_MLog_CompletedMissions\0')
    assert done > 0
    done += rd.IMAGE_BASE + 1
    # (the globals: the nav map, the UI manager - its +48 the prompt movie - and the target manager)
    g_nm, g_ui = ((-32121 << 16) - 10220) & 0xFFFFFFFF, ((-32121 << 16) - 10236) & 0xFFFFFFFF
    idx = rd.index(a.recomp)
    objective, get, prompt = addr('F-96100'), addr('F-49590'), addr('F-40654')
    main_src = (MAIN.replace('sub_FOOTER(', footer + '(').replace('NAME_TUT', hex(tut) + 'u').replace('NAME_ACT', hex(act) + 'u')
                .replace('NAME_DONE', hex(done) + 'u').replace('G_NM', hex(g_nm) + 'u').replace('G_UI', hex(g_ui) + 'u'))
    exe = rd.build(idx, [footer], main_src, a.work, 'mlog_footer', image=img,
                   stubs={objective: '', get: 'ctx.r3.u32=load(0x4001400Cu);',
                          prompt: 'if(ctx.r3.u32!=0x40017000u)throw std::runtime_error("prompt movie");store(0x40014000u,ctx.r4.u32);'},
                   fakes={0x7F000010: 'ctx.r3.u32=load(0x40014008u);', 0x7F000020: 'ctx.r3.u32=load(0x40014010u);'})
    rnd = random.Random(49278)
    lines = [f'{l} {p} {q} {s} {n} {r} {t}' for l in range(3) for p in (0, 1) for q in (0, 1) for (s, n) in ((0, 3), (2, 3), (3, 3), (0, 0))
             for r in range(3) for t in range(4)]
    while len(lines) < 1000:
        n = rnd.randint(0, 16)
        lines.append(f'{rnd.randint(0, 2)} {rnd.randint(0, 1)} {rnd.randint(0, 1)} {rnd.randint(0, 18)} {n} {rnd.randint(0, 2)} {rnd.randint(0, 3)}')
    cases = a.work / 'mlog_footer_cases.txt'
    cases.write_text('\n'.join(lines) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    rows = p.stdout.splitlines()
    assert len(rows) == len(lines), (len(rows), len(lines))
    a.out.write_text(''.join(l + ' -> ' + r + '\n' for l, r in zip(lines, rows)))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
