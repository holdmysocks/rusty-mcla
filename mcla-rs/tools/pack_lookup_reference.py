"""A driver pack's clip lookup F-99427 (with its compare F-40755) run offline on the original's code over made-up holders:
per case a holder of up to 12 name entries (+780: entry pointers, +1548 count; an entry's +0 string and +4 length; the clip
words +8 + 4i, some 0) and 8 names looked up (case variants, the A-Z boundaries, duplicates, empty names).
Usage: pack_lookup_reference.py <out-dir> [--tool mcla-tool] [--recomp D:/MCLA-static] [--image original-image.bin] [-n 400]
Writes <out-dir>/pack_lookup_ref.txt, per case "<entries> | <queries> -> <answers>": an entry "len:name:clip" (name '-' = no
string, clip 0 = none), a query its name, an answer the clip word returned (0 = none)."""
import argparse, random, subprocess, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import memcap
import recomp_dispatch as rd

POOL = ['drv_idle_low', 'DRV_IDLE_LOW', 'Drv_Idle_Low', 'drv_win', 'drv_win_02', 'drv_wi', 'a@b', 'a`b', 'A[B', 'a{b', 'aZb', 'azb',
        'drv_gest_disapnt', 'DRV_PAN_SWEEP_OVERLAY', 'drv_pan_sweep_overlay', 'x', 'X']


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('out', type=Path)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--recomp', type=Path, default=Path('D:/MCLA-static'))
    ap.add_argument('--image', type=Path, default=Path('D:/MCLA/work/2026-10-10/original-image.bin'))
    ap.add_argument('-n', type=int, default=400)
    a = ap.parse_args()
    memcap.apply()
    work = a.out.resolve()
    work.mkdir(parents=True, exist_ok=True)
    fn = 'sub_%08X' % int(subprocess.run([a.tool, 'registry', 'addr', 'F-99427'], capture_output=True, text=True, check=True).stdout.split()[1], 16)
    rnd = random.Random(99427)
    cases = []
    for k in range(a.n):
        ents = []
        for i in range(rnd.randint(0, 12)):
            name = rnd.choice(POOL + ['', None])
            ln = 0 if name == '' or rnd.random() < 0.05 else rnd.randint(1, 30)
            clip = 0 if rnd.random() < 0.15 else 0x100 + i
            ents.append((ln, name, clip))
        qs = [rnd.choice(POOL + [''] + [e[1] for e in ents if e[1]]) for _ in range(8)]
        cases.append((ents, qs))
    # the C side: strings as literals copied into the arena
    def cstr(x):
        return '"' + x.replace('\\', '\\\\').replace('"', '\\\\"') + '"'
    body = []
    for ents, qs in cases:
        e = ','.join('{%d,%s,%du}' % (ln, cstr(n) if n is not None else 'nullptr', c) for ln, n, c in ents)
        q = ','.join(cstr(x) for x in qs)
        body.append('{%d,{%s},{%s}}' % (len(ents), e, q))
    main_src = r'''
struct E{int len;const char*name;uint32_t clip;};
struct C{int n;E e[12];const char*q[8];};
static const C J[]={
''' + ',\n'.join(body) + r'''
};
static uint32_t put(uint32_t&at,const char*s){uint32_t a=at;size_t n=strlen(s);for(size_t i=0;i<=n;i++)*pointer(at++)=uint8_t(s[i]);at=(at+3)&~3u;return a;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 const uint32_t H=0x40800000u, EN=0x40810000u; uint32_t S=0x40820000u;
 for(auto&j:J){ rt_reset(); S=0x40820000u;
  for(uint32_t x=H;x<H+0x700;x++) *pointer(x)=0;
  store(H+1548,j.n);
  for(int i=0;i<j.n;i++){ uint32_t e=EN+16*i; store(e,j.e[i].name?put(S,j.e[i].name):0); store16(e+4,uint16_t(j.e[i].len));
   store(H+780+4*i,e); store(H+8+4*i,j.e[i].clip); }
  for(int k=0;k<8;k++){ uint32_t q=put(S,j.q[k]); Context c=fresh(); c.r3.u64=H; c.r4.u64=q; c.r5.u64=0; FN(c,nullptr);
   printf("%s%x",k?" ":"",c.r3.u32); }
  printf("\n"); }
}
'''.replace('FN', fn)
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    exe = rd.build(idx, [fn], main_src, work, 'pack_lookup', image=img)
    r = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    out = r.stdout.splitlines()
    assert len(out) == len(cases)
    with open(work / 'pack_lookup_ref.txt', 'w', newline='\n') as f:
        for (ents, qs), o in zip(cases, out):
            es = ' '.join('%d:%s:%x' % (ln, '-' if n is None else n, c) for ln, n, c in ents)
            f.write('%s | %s -> %s\n' % (es, ' '.join(q if q else '""' for q in qs), o))
    print(len(out), 'cases')


if __name__ == '__main__':
    main()
