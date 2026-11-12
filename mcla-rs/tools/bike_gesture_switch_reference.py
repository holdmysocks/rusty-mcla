"""The bike rider's gesture switch enter F-32265 run offline on the original's code (generated code unchanged), with its pickers
(machine +68..+92 and +100 counters, the clip tables in the image) and the two racer tests F-90764 / F-99046 the neutral picker
reads. r3 is the machine: +24 the pack, +28 the context (+4 the gesture id), +32 the anim player, +40 the racer. The pack lookups
F-99427 (pack, name, quit-on-missing) and F-66148 (machine, name) are stubs that log the name and answer the name's own address,
or 0 for the names the case leaves out of the pack; the anim player's entry points are stubs that log their arguments.
Usage: bike_gesture_switch_reference.py <out-dir> [--tool mcla-tool] [--recomp D:/MCLA-static] [--image original-image.bin] [-n 300]
Writes <out-dir>/bike_gesture_switch_ref.txt, per case "<missing mask> <racer +656 == 3> <human> <debug> | <id> -> <call>; ... | ..."
(one group per gesture of the case's sequence, in order; the counters carry over inside a case), each call
"<F-id> <args>" with names as text and f1 as float bits."""
import argparse
import random
import subprocess
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import memcap
import recomp_dispatch as rd

# every name the switch and its pickers can look up (the mask's bit i leaves NAMES[i] out of the pack)
NAMES = ['drv_gest_disapnt', 'drv_gest_agro_lft', 'drv_gest_thm_dwn_lft', 'drv_gest_thm_up_lft', 'drv_gest_look_lft',
         'drv_gest_cas_lft', 'drv_gest_agro_rgt', 'drv_gest_thm_dwn_rgt', 'drv_gest_thm_up_rgt', 'drv_gest_look_rgt',
         'drv_gest_cas_rgt', 'drv_win', 'drv_gest_listen_lft', 'drv_gest_listen_rgt', 'drv_gest_cas_rgt_01',
         'drv_gest_mild_rgt_01', 'Drv_Idle_Low', 'drv_gest_arrest', 'drv_find_sh_04']
LOOKUPS = {'F-99427': 'K', 'F-66148': 'N'}
LOGGED = {'F-24441': 3, 'F-63058': 1, 'F-73665': 3, 'F-42193': 3, 'F-19950': 2}
TESTS = ['F-90764', 'F-99046']
SEQ = 16


def addr(tool, fid):
    return int(subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1], 16)


def cases(n):
    rnd = random.Random(3226)
    out = []
    for k in range(n):
        mask = 0 if k % 4 == 0 else sum(1 << i for i in range(len(NAMES)) if rnd.random() < 0.2)
        seq = [rnd.choice(range(22)) if rnd.random() < 0.3 else rnd.choice((0, 1, 2, 3, 4, 5, 6, 17)) for _ in range(SEQ)]
        out.append((mask, rnd.random() < 0.5, rnd.random() < 0.5, k % 7 == 0, seq))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('out', type=Path)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--recomp', type=Path, default=Path('D:/MCLA-static'))
    ap.add_argument('--image', type=Path, default=Path('D:/MCLA/work/2026-10-10/original-image.bin'))
    ap.add_argument('-n', type=int, default=300)
    a = ap.parse_args()
    memcap.apply()
    work = a.out.resolve()
    work.mkdir(parents=True, exist_ok=True)
    ids = ['F-32265'] + list(LOOKUPS) + list(LOGGED) + TESTS
    fn = {k: 'sub_%08X' % addr(a.tool, k) for k in ids}
    js = cases(a.n)
    nl = chr(10)
    rows = nl.join('{0x%08Xu,%d,%d,%d,{%s}},' % (m, r, h, d, ','.join(map(str, s))) for m, r, h, d, s in js)
    names = ','.join('"%s"' % x for x in NAMES)
    main_src = r'''
struct Case{uint32_t mask,racer,human,debug;uint32_t seq[''' + str(SEQ) + r'''];};
static const Case J[]={
''' + rows + r'''
};
static const char*NAMES[]={''' + names + r'''};
static const Case*CUR;
uint32_t G_RACER,G_HUMAN;
std::string str(uint32_t v){std::string s;if(v<0x82000000u||v>=0x84000000u)return "?";for(int n=0;n<96&&*pointer(v+n);n++)s+=char(*pointer(v+n));return s;}
uint32_t have(uint32_t v){std::string s=str(v);for(int i=0;i<(int)(sizeof NAMES/sizeof*NAMES);i++)if(s==NAMES[i])return (CUR->mask>>i&1)?0:v;
 printf(" UNLISTED:%s",s.c_str());return v;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 const uint32_t M=0x40800000u, CTX=0x40810000u, PACK=0x40820000u, P=0x40830000u, RACER=0x40840000u, MGR=0x40850000u;
 for(auto&j:J){ rt_reset(); CUR=&j; G_RACER=j.racer; G_HUMAN=j.human;
  for(uint32_t x=M;x<M+0x200;x++) *pointer(x)=0;
  for(uint32_t x=CTX;x<CTX+0x200;x++) *pointer(x)=0;
  for(uint32_t x=MGR;x<MGR+0x200;x++) *pointer(x)=0;
  store(M+24,PACK); store(M+28,CTX); store(M+32,P); store(M+40,RACER);
  store(0x8286D8F0u,MGR); store(0x828CD30Cu,j.debug);
  printf("S %08x %u %u %u\n",j.mask,j.racer,j.human,j.debug);
  for(int i=0;i<''' + str(SEQ) + r''';i++){
   store(CTX+4,j.seq[i]); printf("G %u",j.seq[i]);
   Context c=fresh(); c.r3.u64=M; ''' + fn['F-32265'] + r'''(c,nullptr);
   printf(" | %u\n",load(MGR+300)); store(MGR+300,0);
  }
  printf("E\n"); }
}
'''
    log = ('std::string str(uint32_t);printf(" ; %s",T);uint64_t r[4]={ctx.r3.u64,ctx.r4.u64,ctx.r5.u64,ctx.r6.u64};'
           'for(int i=1;i<=NREG;i++){uint32_t v=uint32_t(r[i]);if(v>=0x82000000u&&v<0x84000000u)printf(" %s",str(v).c_str());'
           'else printf(" %x",v);}'
           'float f=float(ctx.f1.f64);uint32_t b;memcpy(&b,&f,4);printf(" %08x",b);ctx.r3.u64=0;')
    stubs = {fn[k]: 'static const char*T="%s";' % k + log.replace('NREG', str(n)) for k, n in LOGGED.items()}
    for k, t in LOOKUPS.items():
        stubs[fn[k]] = ('std::string str(uint32_t);uint32_t have(uint32_t);printf(" ; %s %%s %%u",str(ctx.r4.u32).c_str(),uint32_t(ctx.r5.u32&0xff));ctx.r3.u64=have(ctx.r4.u32);' % k
                        if t == 'K' else
                        'std::string str(uint32_t);uint32_t have(uint32_t);printf(" ; %s %%s",str(ctx.r4.u32).c_str());ctx.r3.u64=have(ctx.r4.u32);' % k)
    # F-90764: the racer and its +656 == 3; F-99046: the racer's vt+40 (the human test)
    stubs[fn['F-90764']] = 'extern uint32_t G_RACER;ctx.r3.u64=G_RACER;'
    stubs[fn['F-99046']] = 'extern uint32_t G_HUMAN;ctx.r3.u64=G_HUMAN;'
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    exe = rd.build(idx, [fn['F-32265']], main_src, work, 'bike_gesture_switch', stubs=stubs, image=img)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True, encoding='latin-1')
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    if 'UNLISTED' in r.stdout:
        raise SystemExit('a lookup of a name not in NAMES: ' + r.stdout[r.stdout.index('UNLISTED'):][:80])
    lines, cur = [], None
    for x in r.stdout.split(nl):
        if x.startswith('S '):
            cur = [x[2:]]
        elif x.startswith('G '):
            body, mgr = x[2:].rsplit(' | ', 1)
            gid, _, calls = body.partition(' ; ')
            cur.append('%s -> %s%s' % (gid, calls, ' ; mgr+300 1' if mgr.strip() == '1' else ''))
        elif x == 'E':
            lines.append(' | '.join(cur))
    assert len(lines) == len(js), (len(lines), len(js))
    (work / 'bike_gesture_switch_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
