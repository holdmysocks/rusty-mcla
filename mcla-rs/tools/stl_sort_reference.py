"""The island lists' STLport sort run offline: F-52683 (by instance A, the key (*(p+160))[+0]) and F-18034 (by instance
B, (*(p+160))[+4]) with everything they call (introsort loop, median of three, Hoare partition, the depth limit's
partial sort = make_heap / adjust_heap / push_heap / sort_heap, the guarded and unguarded insertion sorts, the memmove)
as the generated code has them, unchanged, on flat guest memory.

Element i is a pointer to an object at 0x4000 + 0xB0 i whose +160 points at its own +168, where the A key (+168) and
the B key (+172) stand. The array is at 0x1000. Cases: random lengths 0 .. 1200 with few or many distinct keys (equal
keys show the order the unstable sort leaves them in), sorted, reversed, all equal, organ pipes and median-of-three
killers (which reach the depth limit and its heap sort).

Writes <out-dir>/stl_sort_ref.txt, one case a line: "a|b n k0 k1 .. -> i0 i1 .." (keys hex, the result as element
indices)."""
import argparse
import random
import subprocess
from pathlib import Path
from recomp_closure import index, build, run

# the library memmove F-21727 (dst r3, src r4, bytes r5) in the guarded insertion sort's block move is the C
# library's memmove on guest memory: its generated code uses condition fields the shim lacks
MEMMOVE = 'memmove(pointer(ctx.r3.u32),pointer(ctx.r4.u32),ctx.r5.u32);'  # (F-21727)
MAXN = 1200


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def killer(n):
    # the median-of-3 killer (Musser): keys that make every median the second smallest
    k = [0] * n
    half = n // 2
    for i in range(1, half + 1):
        if i % 2:
            k[i - 1] = i
            k[i] = half + i
        k[half + i - 1] = 2 * i
    return k


def cases():
    rnd = random.Random(29)
    out = []
    for _ in range(1500):
        n = rnd.choice((0, 1, 2, 3, 15, 16, 17, 18, 31, 32, 33, rnd.randrange(1, 64), rnd.randrange(1, 400), rnd.randrange(1, MAXN + 1)))
        d = rnd.choice((1, 2, 3, 5, 10, n or 1, 4 * n or 1))
        base = rnd.choice((0x40000000, 0x82000000, 0xBF000000, 0x10))
        out.append([base + 0x50 * rnd.randrange(d) for _ in range(n)])
    for n in (17, 40, 100, 500, 1000, MAXN):
        out.append(list(range(n)))
        out.append(list(range(n, 0, -1)))
        out.append([7] * n)
        out.append([min(i, n - i) for i in range(n)])
        if n % 2 == 0:
            out.append(killer(n))
            out.append([x // 3 for x in killer(n)])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fa, fb = addr(a.tool, 'F-52683'), addr(a.tool, 'F-18034')
    cs = cases()
    jobs = [(w, k) for k in cs for w in 'ab']
    for w, k in jobs:
        assert len(k) <= MAXN
    nl = chr(10)
    NL = chr(92) + 'n'
    flat = []
    for w, k in jobs:
        flat += [0 if w == 'a' else 1, len(k)] + k
    body = ('static const uint32_t C[]={' + ','.join('0x%x' % x for x in flat) + '};' + nl +
            'size_t at=0;while(at<sizeof(C)/4){uint32_t w=C[at++],n=C[at++];memset(objects,0,sizeof(objects));' +
            'for(uint32_t i=0;i<n;i++){uint32_t o=0x4000+0xB0*i;store(0x1000+4*i,o);store(o+160,o+168);store(o+168+4*w,C[at+i]);}at+=n;' +
            'Context ctx{};ctx.r1.u32=0x7FF00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1000+4*n;ctx.r5.u32=0;' +
            ('if(w)sub_%s(ctx,nullptr);else sub_%s(ctx,nullptr);' % (fb, fa)) +
            'for(uint32_t i=0;i<n;i++)printf(i?" %u":"%u",(load(0x1000+4*i)-0x4000)/0xB0);printf("' + NL + '");}' + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + fa, 'sub_' + fb], main_src, a.out_dir, 'stl_sort', scratch_end=0x80000, stubs={'sub_' + addr(a.tool, 'F-21727'): MEMMOVE})
    out = run(exe, a.image).split(nl)[:len(jobs)]
    assert len(out) == len(jobs), (len(out), len(jobs))
    lines = []
    for (w, k), o in zip(jobs, out):
        assert sorted(map(int, o.split())) == list(range(len(k))), 'not a permutation'
        lines.append('%s %d %s -> %s' % (w, len(k), ' '.join('%x' % x for x in k), o))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'stl_sort_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
