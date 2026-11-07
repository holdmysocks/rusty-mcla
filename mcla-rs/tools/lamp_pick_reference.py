"""Run the original's pick of a place's strongest lamps offline (F-48037, with its sort F-39252 and everything that
calls, as the generated code has them) and record the results for the port's test
(crates/game/src/render/testdata/lamp_pick_reference.txt).

F-48037 scores each candidate lamp (F-31785 fetches the lamp, F-60154 scores it; both are stubbed here: the lamp
"pointer" is its index and the score comes from the case's table, since F-60154 is checked on its own), sorts the
(score, index) pairs with the STLport introsort F-39252 ascending by score, writes the best `cap` from the top down
and fills the remaining places with the index "none" (the lamp count) and the constant score it reads from the image.
Cases: up to 40 candidates (the query's cap is 32), many equal scores (the unstable sort's order), NaN and zero;
median-of-three killers and organ pipes of 34 - 40, and 40 lists that reach the depth limit and its heap sort.

  PICK <lamps> <cap> <n> <cand idx ...> <score hex ...> -> <out n> <idx ...> <score hex ...>

python lamp_pick_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --out <build dir> --ref <file>
"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fbits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


MAXN = 40
NAN = 0x7FC00000


def cases(rnd):
    out = []
    for k in range(3000):
        n = rnd.choice((0, 1, 2, 3, 4, 5, 8, 15, 16, 17, 18, 24, 31, 32, 33, rnd.randrange(0, MAXN + 1)))
        lamps = max(n, 1) + rnd.randrange(0, 8)
        cand = rnd.sample(range(lamps), n)
        pool = rnd.choice(('few', 'many', 'zeros', 'nan'))
        vals = [rnd.uniform(0, 1) for _ in range(3)]
        sc = []
        for _ in range(lamps):
            if pool == 'few':
                sc.append(fbits(rnd.choice(vals)))
            elif pool == 'zeros':
                sc.append(fbits(rnd.choice((0.0, 0.0, 0.0, rnd.uniform(0, 1)))))
            elif pool == 'nan' and rnd.random() < 0.2:
                sc.append(NAN)
            else:
                sc.append(fbits(rnd.choice((rnd.uniform(0, 1), 1.0, vals[0]))))
        cap = rnd.choice((4, 4, 4, 1, 2, 3, 5, 6))
        out.append((lamps, cap, cand, sc))
    # median-of-three killers and organ pipes: past the depth limit 2 floor(log2 n) the range is heap sorted
    heap = []
    for n in (34, 36, 38, 40):
        for keys in (killer(n), [x // 3 for x in killer(n)], [min(i, n - i) for i in range(n)]):
            heap.append(keys)
    while sum(map(reaches_heap, heap)) < 40:  # (killers with a few keys swapped or tied, kept when they still reach it)
        k = list(killer(40))
        for _ in range(rnd.randrange(1, 4)):
            i, j = rnd.randrange(40), rnd.randrange(40)
            k[i], k[j] = (k[j], k[i]) if rnd.random() < 0.5 else (k[j], k[j])
        if reaches_heap(k):
            heap.append(k)
    for keys in heap:
        for cap in (4, 6):
            out.append((len(keys), cap, list(range(len(keys))), [fbits(float(x)) for x in keys]))
    return out


def reaches_heap(keys):
    # the introsort loop's steps (as loose::stl_sort_by) only to tell whether a range reaches the depth limit
    a, hit = list(keys), [False]

    def quick(f, l, d):
        while l - f > 16:
            if d == 0:
                hit[0] = True
                return
            d -= 1
            n = l - f
            x, y, z = a[f], a[f + n // 2], a[l - 1]
            m = (f + n // 2 if y < z else (l - 1 if x < z else f)) if x < y else (f if x < z else (l - 1 if y < z else f + n // 2))
            p, i, j = a[m], f, l
            while True:
                while a[i] < p:
                    i += 1
                j -= 1
                while p < a[j]:
                    j -= 1
                if not i < j:
                    break
                a[i], a[j] = a[j], a[i]
                i += 1
            quick(i, l, d)
            l = i
    quick(0, len(a), 2 * (len(a).bit_length() - 1))
    return hit[0]


def killer(n):
    # (Musser's: every median of three is the second smallest)
    k = [0] * n
    half = n // 2
    for i in range(1, half + 1):
        if i % 2:
            k[i - 1] = i
            k[i] = half + i
        k[half + i - 1] = 2 * i
    return k


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    pick, fetch, score = (addr(a.tool, f) for f in ('F-48037', 'F-31785', 'F-60154'))
    cs = cases(random.Random(48))
    flat = []
    for lamps, cap, cand, sc in cs:
        assert len(cand) <= MAXN and lamps <= 64
        flat += [lamps, cap, len(cand)] + cand + sc
    nl, NL = chr(10), chr(92) + 'n'
    # (F-10861, the stack probe of F-48037's alloca, only reads the pages below the stack)
    stubs = {'sub_' + fetch: 'store(ctx.r3.u32,ctx.r4.u32);',
             'sub_' + score: 'extern uint32_t SC[64];uint32_t b=SC[load(ctx.r3.u32)];float f;memcpy(&f,&b,4);ctx.f1.f64=f;',
             'sub_' + addr(a.tool, 'F-10861'): ''}
    body = ('static const uint32_t C[]={' + ','.join('0x%x' % x for x in flat) + '};' + nl +
            'uint32_t SC[64];' + nl +
            'static void st16(uint32_t a,uint32_t v){auto p=pointer(a);p[0]=uint8_t(v>>8);p[1]=uint8_t(v);}' + nl +
            'static uint32_t ld16(uint32_t a){auto p=pointer(a);return (p[0]<<8)|p[1];}' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'size_t at=0;while(at<sizeof(C)/4){uint32_t lamps=C[at++],cap=C[at++],n=C[at++];memset(objects,0,sizeof(objects));' +
            'for(uint32_t i=0;i<lamps;i++)store(0x1000+4*i,i);' +
            'for(uint32_t i=0;i<n;i++)st16(0x1200+2*i,C[at+i]);at+=n;for(uint32_t i=0;i<lamps;i++)SC[i]=C[at+i];at+=lamps;' +
            'store(0x1800,0x1000);st16(0x1804,lamps);store(0x1810,0x1200);st16(0x1814,n);' +
            'store(0x1820,0x1400);st16(0x1824,0);st16(0x1826,cap);store(0x1830,0x1500);st16(0x1834,0);' +
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1800;ctx.r4.u32=0x1840;ctx.r5.u32=0x1810;ctx.r6.u32=0x1820;ctx.r7.u32=0x1830;' +
            ('sub_%s(ctx,nullptr);' % pick) +
            'uint32_t k=ld16(0x1824);printf("%u",k);for(uint32_t i=0;i<k;i++)printf(" %u",load(0x1400+4*i));' +
            'for(uint32_t i=0;i<k;i++)printf(" %08x",load(0x1500+4*i));printf("' + NL + '");}}' + nl)
    exe = build(index(a.recomp), ['sub_' + pick], body, a.out, 'lamp_pick', stubs=stubs)
    out = run(exe, a.image).split(nl)[:len(cs)]
    assert len(out) == len(cs), (len(out), len(cs))
    lines = []
    for (lamps, cap, cand, sc), o in zip(cs, out):
        lines.append('PICK %d %d %d %s %s -> %s' % (lamps, cap, len(cand), ' '.join(map(str, cand)), ' '.join('%08x' % x for x in sc), o))
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    a.ref.write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
