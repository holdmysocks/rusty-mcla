"""The sound tree's durations and play offsets run offline: the original's total duration F-53189 with every type's duration
method of the table F-25376 (dispatched by the type byte through the table F-53189 reads), the predelay / start offset rule
F-90115, the Looping start split F-39812 and the Sequential start lookup F-71973, the generated code whole and unchanged,
on random sound trees in flat guest memory. Stubbed, as inputs of the case: the stream slot lookup F-20252 (a slot whose
loaded byte +72 is the case's) and the bank's longest wave F-46175 (the case's value; that rule is the whole-ms rule of
F-13862, checked by itself). Variables are absent (+12 .. +100 pointers 0) and the parent predelay +96 is 0.
Memory: the sound pool's bucket 0 with its sound array, 256 bytes a sound (the pool's own table and
stride words are set in the image), all in the runner's arena (see the constants below).
Writes <out-dir>/sound_duration_ref.txt: per case its tree (node: type tot_pd tot_so tri0 ni tri6 p1..p5 / children), the
root's own predelay, start offset, parent offset and percent flag, then '->' the root's total and flag, the play offsets
+80 +84, the Looping split (+196, the child's +108) and the Sequential start (index, offset)."""
import argparse
import random
import re
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd
from recomp_closure import index, run

# L: the stubbed longest wave of the case
recomp_closure.EXTRA += 'static uint32_t L;' + chr(10)

POOL = 0x40100000
ARRAY = 0x40200000
STRIDE = 0x100
SLOT = 0x40300000
OUT = 0x40310000
STACK = 0x40F00000


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def lis_addi(text, reg):
    """the address a function builds with lis rX,hi / addi rY,rX,lo where rY = reg"""
    m = re.search(r'// lis (r\d+),(-?\d+)\s*\n(?:.*\n){0,3}?\s*// addi %s,\1,(-?\d+)' % reg, text)
    return ((int(m.group(2)) << 16) + int(m.group(3))) & 0xffffffff


def gen(rnd, depth):
    """one random node and its subtree: [type, tot_pd, tot_so, tri0, ni, tri6, p1..p5], children"""
    leaf = depth >= 3 or rnd.random() < 0.3
    t = 12 if leaf else rnd.choice((1, 2, 3, 5, 6, 7, 8, 9, 10, 13, 14, 22, 25, 12))
    tri0 = rnd.choice((2, 2, 0, 1))
    pct = tri0 == 1
    pd = rnd.choice((0, 0, 10, 250))
    so = rnd.choice((0, 25, 50, 100, 150)) if pct else rnd.choice((0, 0, 5, 50, 100, 1500, 99999))
    if depth > 0 and rnd.random() < 0.6:
        pd, so = 0, 0
    ni = rnd.choice((0, 0, 0x10, 0x08, 0x18))
    tri6 = 0
    p = [0, 0, 0, 0, 0]
    kids = []
    if t == 12:
        p[0] = rnd.choice((0, 1, 300, 1631, 12345, 0xffffffff, rnd.randrange(1, 60000)))
        p[1] = int(rnd.random() < 0.3)
    elif t == 1:
        p[0] = rnd.choice((-1, 1, 2, 4, 16, 0))
        kids = [gen(rnd, depth + 1)]
    elif t == 2:
        p = [rnd.choice((0, 10, 500)), rnd.choice((0, 20, 300)), rnd.choice((-1, 0, 100, 2000)), rnd.choice((-1, 0, 250)),
             rnd.choice((0, 50, 100))]
        kids = [gen(rnd, depth + 1)]
    elif t in (3, 10):
        kids = [gen(rnd, depth + 1), gen(rnd, depth + 1)]
    elif t in (5, 14, 22):
        kids = [gen(rnd, depth + 1)]
    elif t in (6, 25):
        kids = [gen(rnd, depth + 1)] if rnd.random() < 0.85 else []
    elif t == 7:
        p[0] = int(rnd.random() < 0.1)
        kids = [gen(rnd, depth + 1) for _ in range(rnd.randrange(1, 5))]
    elif t == 8:
        tri6 = rnd.choice((0, 1))
        p = [rnd.choice((0, 0, 5000)), 0, int(rnd.random() < 0.8), int(rnd.random() < 0.8), rnd.choice((-1, 0, 74123, 120000))]
    elif t == 9:
        p = [rnd.choice((-1, 0, 1, 3, 8)), rnd.choice((0, 50, 400)), 0, 0, 0]
        kids = [gen(rnd, depth + 1)] if rnd.random() < 0.8 else []
    elif t == 13:
        kids = [gen(rnd, depth + 1) for _ in range(rnd.randrange(1, 5))]
        p[0] = len(kids)
    return [t, pd, so, tri0, ni, tri6] + p, kids


def flatten(node, out):
    """nodes in pre-order: (fields, child indices)"""
    i = len(out)
    out.append(None)
    f, kids = node
    out[i] = (f, [flatten(k, out) for k in kids])
    return i


def pokes(nodes):
    """(addr, width, value) writes that lay the case's sounds out as the original's objects"""
    w = []
    nv = len(nodes)
    for i, (f, kids) in enumerate(nodes):
        t, pd, so, tri0, ni, tri6, p1, p2, p3, p4, p5 = f
        a = ARRAY + i * STRIDE
        slots = list(kids)
        w += [(a + 59, 1, t), (a + 80, 4, pd), (a + 84, 4, so), (a + 112, 4, tri0 | tri6 << 12), (a + 8, 1, ni), (a + 64, 1, 0)]
        if t == 12:
            v = nv + i
            slots = [v]
            va = ARRAY + v * STRIDE
            w += [(va + 212, 4, p1 & 0xffffffff), (va + 227, 1, 0x20 if p2 else 0)]
        elif t == 1:
            w += [(a + 188, 4, p1 & 0xffffffff), (a + 196, 4, 0x77)]
        elif t == 2:
            w += [(a + 228, 2, p1), (a + 230, 2, p2), (a + 232, 1, p5), (a + 220, 4, p3 & 0xffffffff), (a + 224, 4, p4 & 0xffffffff)]
        elif t == 7:
            w += [(a + 188, 4, len(kids)), (a + 198, 1, p1)]
        elif t == 8:
            w += [(a + 200, 4, p1), (a + 217, 1, tri6), (a + 60, 2, 1 if p3 else 0), (a + 56, 1, p4)]
        elif t == 9:
            w += [(a + 196, 4, p1 & 0xffffffff), (a + 192, 4, p2)]
        elif t == 13:
            w += [(a + 164, 4, p1)]
        for k in range(8):
            w.append((a + 72 + k, 1, slots[k] if k < len(slots) else 255))
        for k in kids:
            w.append((ARRAY + k * STRIDE + 108, 4, 0))
    return w


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    total, dispatcher, play, loop_play, seq_find, slot, longest, onstop = (addr(a.tool, f) for f in (
        'F-53189', 'F-25376', 'F-90115', 'F-39812', 'F-71973', 'F-20252', 'F-46175', 'F-23434'))
    idx = index(a.recomp)
    src = lambda n: idx['sub_' + n].read_text(errors='replace')
    # the type methods in case order (type 1 .. 25), the table the total reads them from, the sound pool
    d = src(dispatcher)
    body = d[d.index('DEFINE_REX_FUNC(sub_%s)' % dispatcher):]
    body = body[:body.index('\n}\n')]
    methods = ['%08X' % (((int(hi) << 16) + int(lo)) & 0xffffffff)
               for hi, lo in re.findall(r'// lis r11,(-?\d+)\s*\n.*\n\s*// addi r3,r11,(-?\d+)', body)]
    assert len(methods) == 25, len(methods)
    t = src(total)
    table = lis_addi(t[t.index('DEFINE_REX_FUNC(sub_%s)' % total):], 'r11')
    o = src(onstop)
    pool = lis_addi(o[o.index('DEFINE_REX_FUNC(sub_%s)' % onstop):], 'r7')
    rnd = random.Random(53189)
    cases = []
    for _ in range(4000):
        nodes = []
        flatten(gen(rnd, 0), nodes)
        root = [rnd.choice((0, 10, 250)), rnd.choice((0, 0, 30, 100, 1500)), rnd.choice((0, 0, 20, 100, 5000, 99999)),
                int(rnd.random() < 0.25)]
        if root[3]:
            root[2] = rnd.choice((0, 10, 25, 50, 90, 100, 200))
        cases.append((nodes, root))
    nl = chr(10)
    rows = []
    for nodes, root in cases:
        w = pokes(nodes) + [(ARRAY + 88, 4, root[0]), (ARRAY + 104, 4, root[1]), (ARRAY + 108, 4, root[2]),
                            (ARRAY + 57, 1, root[3] << 3), (ARRAY + 96, 4, 0)]
        rows.append('%d,%d,%s,' % (len(w), nodes[0][0][0], ','.join('%du,%d,%du' % x for x in w)))
    setup = ('store(0x%x+232,%du);store(0x%x+196,%d);store(%du+11440,%du);' % (pool, POOL, pool, STRIDE, POOL, ARRAY) +
             ''.join('store(0x%x+%d,0x%su);' % (table, 4 * k, m) for k, m in enumerate(methods)))
    stubs = {
        'sub_' + slot: 'ctx.r3.u32=ctx.r3.s32==1?%du:0;' % SLOT,
        'sub_' + longest: 'ctx.r3.u32=L;',
    }
    q = 'Context ctx{};ctx.r1.u32=%du;ctx.r3.u32=%du;' % (STACK, ARRAY)
    # the stubbed longest wave: one value a case, its first Streaming node's p5 (the Rust side reads it the same way)
    lw = [next((f[10] for f, _ in nodes if f[0] == 8), 0) for nodes, _ in cases]
    main_src = (
        'static const int32_t LW[]={%s};' % ','.join(map(str, lw)) + nl +
        'static const uint32_t C[]={' + nl + nl.join(rows) + nl + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + setup + nl +
        'size_t c_=0;for(size_t i=0;i<sizeof(C)/4;c_++){uint32_t n=C[i],t=C[i+1];const uint32_t*w=C+i+2;i+=2+3*n;'
        'memset(pointer(%du),0,255*%d);memset(pointer(%du),0,0x100);memset(pointer(%du),0,0x100);L=LW[c_];*pointer(%du+72)=1;'
        % (ARRAY, STRIDE, SLOT, OUT, SLOT) +
        'for(uint32_t k=0;k<n;k++){uint32_t a=w[3*k],s=w[3*k+1],v=w[3*k+2];'
        'if(s==4)store(a,v);else if(s==2)store16(a,v);else *pointer(a)=v;}' + nl +
        # a case the original itself faults on (the Looping method's flag store through a null pointer, F-53436, when a
        # null flag reaches it - never in the game's data) prints 'fault'
        'int32_t T=0,F=0;char A[40]="- -",B[40]="- -";uint32_t P80=0,P84=0;try{' +
        '{%s ctx.r4.u32=%du;sub_%s(ctx,nullptr);T=ctx.r3.s32;F=*pointer(%du);}' % (q, OUT, total, OUT) +
        'if(t==1){%s sub_%s(ctx,nullptr);uint32_t c=%du+*pointer(%du+72)*%d;snprintf(A,40,"%%d %%u",(int32_t)load(%du+196),load(c+108));}'
        % (q, loop_play, ARRAY, ARRAY, STRIDE, ARRAY) +
        'if(t==7){%s ctx.r4.u32=%du;ctx.r5.u32=%du;sub_%s(ctx,nullptr);'
        'snprintf(B,40,"%%d %%u",(int32_t)load(%du),load(%du));}' % (q, OUT + 16, OUT + 20, seq_find, OUT + 16, OUT + 20) +
        '{%s sub_%s(ctx,nullptr);P80=load(%du+80);P84=load(%du+84);}' % (q, play, ARRAY, ARRAY) +
        '}catch(const std::exception&){printf("fault\\n");continue;}' +
        'printf("%d %d %s %s %u %u\\n",T,F,A,B,P80,P84);' +
        '}}' + nl)
    exe = rd.build(idx, ['sub_' + total, 'sub_' + play, 'sub_' + loop_play, 'sub_' + seq_find], main_src, a.out_dir,
                   'sound_duration', stubs=stubs, image=rd.load_image(a.image), targets=[int(m, 16) for m in methods])
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = []
    for (nodes, root), o in zip(cases, out):
        tree = ' '.join(','.join(str(v) for v in f) + '/' + ','.join(map(str, k)) for f, k in nodes)
        lines.append('%s | %s -> %s' % (tree, ' '.join(map(str, root)), o))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'sound_duration_ref.txt').write_text(nl.join(lines) + nl)
    ok = [o for o in out if o != 'fault']
    print(len(lines), 'cases,', len(out) - len(ok), 'faulting in the original,', sum(o.split()[0] == '-1' for o in ok), 'with an unknown total,',
          sum(o.split()[1] == '1' for o in ok), 'endless')


if __name__ == '__main__':
    main()
