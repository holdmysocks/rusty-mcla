"""The career's completion statistic runs offline: F-20532, the generated code unchanged, on a mission manager at 0x1000
(all missions: +28 array, +32 u16 count; completed: +44 array, +48 u16 count; the result float +76), the missions at
0x2000.. (+685 always hidden, +693 repeatable; 0x300 bytes each). The statistic set (an indirect call through the stat
manager's vtable +48) is captured: whether it ran and its value word. Cases: 0..140 missions, any of them hidden or
repeatable, a completed list of any of them. Writes <out-dir>/percent_complete_ref.txt:
"<all count> <completed count> <hidden/repeatable bits of all, as 0/1/2/3 per mission> <completed indices> -> <set> <value> <+76 bits>"."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run


# (fctiwz needs the SDK's simde: vector mode, whose shim takes the scratch size from here)
SCRATCH = 0x2000 + 0x300 * 141
_shim = recomp_closure.shim
recomp_closure.shim = lambda *a: _shim(SCRATCH)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def cases():
    rnd = random.Random(20532)
    out = []
    for _ in range(3000):
        n = rnd.choice((0, 1, 2, 3, 7, 50, 107, 110, 120, 140, rnd.randrange(0, 141)))
        p = rnd.random() * 0.3
        flags = [(1 if rnd.random() < p else 0) | (2 if rnd.random() < p else 0) for _ in range(n)]
        if n and rnd.random() < 0.05:
            flags = [rnd.choice((1, 2, 3)) for _ in range(n)]
        k = rnd.randrange(0, n + 1) if n else 0
        done = rnd.sample(range(n), k)
        out.append((flags, done))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = addr(a.tool, 'F-20532')
    cs = cases()
    nl = chr(10)
    recomp_closure.EXTRA += ('static int g_set; static uint32_t g_val;\n'
                             '#define REX_CALL_INDIRECT_FUNC(a) do{g_set++;g_val=ctx.r6.u32;}while(0)\n')
    rows = (',' + nl).join('{%d,%d,{%s},{%s}}' % (len(fl), len(d), ','.join(map(str, fl)) or '0', ','.join(map(str, d)) or '0') for fl, d in cs)
    body = ('struct K{int n,k;int fl[140];int d[140];};static const K C[]={' + nl + rows + nl + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));g_set=0;g_val=0;'
            # the stat manager the code reads from 0x828CCF68 (lis -32115, -12440): a vtable with +48
            'store(0x828CCF68,0x1900);store(0x1900,0x1A00);store(0x1A00+48,0x82000000);'
            'store(0x1000+28,0x1100);store16(0x1000+32,c.n);store(0x1000+44,0x1500);store16(0x1000+48,c.k);'
            'for(int i=0;i<c.n;i++){uint32_t m=0x2000+0x300*i;store(0x1100+4*i,m);*pointer(m+685)=c.fl[i]&1;*pointer(m+693)=(c.fl[i]>>1)&1;}'
            'for(int i=0;i<c.k;i++)store(0x1500+4*i,0x2000+0x300*c.d[i]);'
            '{Context ctx{};ctx.r1.u32=0x1F00;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);}' % f +
            'printf("%d %u %u' + chr(92) + 'n",g_set,g_val,load(0x1000+76));}' + nl + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + f], body, a.out_dir, 'percent', vector=True)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cs)
    lines = ['%d %d %s %s -> %s' % (len(fl), len(d), ''.join(map(str, fl)) or '-', ','.join(map(str, d)) or '-', o) for (fl, d), o in zip(cs, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'percent_complete_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
