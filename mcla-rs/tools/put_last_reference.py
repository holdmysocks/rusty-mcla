"""The results' "player put last" runs offline: F-87120, the generated code unchanged, on a results block at 0x1000
(racer count +3080, 192-byte racer records from +16: the type word at +0 (0 = the player), the place at +156). Cases:
0..16 racers, one player or none or two, places a permutation 1..n or any small words. Writes
<out-dir>/put_last_ref.txt: "<count> <type,place per racer> -> <place per racer after>"."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def cases():
    rnd = random.Random(87120)
    out = []
    for _ in range(3000):
        n = rnd.choice((0, 1, 2, 4, 8, 16, rnd.randrange(0, 17)))
        types = [rnd.choice((1, 1, 1, 2, 3)) for _ in range(n)]
        r = rnd.random()
        if n and r < 0.8:
            types[rnd.randrange(n)] = 0
        if n > 1 and 0.8 <= r < 0.9:
            for i in rnd.sample(range(n), 2):
                types[i] = 0
        if rnd.random() < 0.7:
            places = list(range(1, n + 1))
            rnd.shuffle(places)
        else:
            places = [rnd.randrange(-2, n + 3) for _ in range(n)]
        out.append((types, places))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = addr(a.tool, 'F-87120')
    cs = cases()
    nl = chr(10)
    rows = (',' + nl).join('{%d,{%s},{%s}}' % (len(t), ','.join(map(str, t)) or '0', ','.join(map(str, p)) or '0') for t, p in cs)
    body = ('struct K{int n;int t[16];int p[16];};static const K C[]={' + nl + rows + nl + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));store(0x1000+3080,c.n);'
            'for(int i=0;i<c.n;i++){store(0x1000+16+192*i,c.t[i]);store(0x1000+172+192*i,c.p[i]);}'
            '{Context ctx{};ctx.r1.u32=0x1F00;ctx.r3.u32=0x1E00;ctx.r4.u32=0x1000;sub_%s(ctx,nullptr);}' % f +
            'for(int i=0;i<c.n;i++)printf("%d ",(int)load(0x1000+172+192*i));printf("' + chr(92) + 'n");}' + nl + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + f], body, a.out_dir, 'putlast')
    out = run(exe, a.image).split(nl)
    out = [x.strip() for x in out][:len(cs)]
    assert len(out) == len(cs)
    lines = ['%d %s -> %s' % (len(t), ' '.join('%d,%d' % tp for tp in zip(t, p)) or '-', o or '-') for (t, p), o in zip(cs, out)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'put_last_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
