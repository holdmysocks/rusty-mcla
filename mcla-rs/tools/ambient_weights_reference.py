"""The ambient type weights for a district node at a clock (F-83932: the weights of one list's types and its density)
run offline: the generated code whole and unchanged (with its callees, the weight clear and the Tod lookup) on flat
guest memory holding the real distribution trees of tune/ambients (traffic and people), built as the loader lays them
out (F-83932's reads: the list +0 types / +4 count, +8 weights / +12 count, +16 total, +20 density; a node +20 parent,
+24 data; the data +4 entries (16 bytes, the Tod at +8), +8 entry count, +12 the density Tod, +20 the entries' type
indices; a Tod {rows, u16 count}, a row 12 bytes: +4 u16 start, +6 u16 end, +8 weight). The type list is the loader's
order: the tree in pre-order (a node's entries, then its ChildNames in order), each name added at its first sight.

Cases: every node of both trees at clocks on and about the rows' edges, live counts (type +8) that put the number of
types in use below, at and over 10, the cab (+111) / trailer (+112) flags, the uniform flag (r6, from the manager's byte
+248 / +249: every weight 1) and the trailer test switch (the `testtrailers` parameter's word); and synthetic chains (random rows, negative weights, an empty density
table, a negative density, index -1, a type listed twice along the chain).

Writes <out-dir>/ambient_weights_ref.txt:
  "N <tree> <node> <parent> D <nrows> <rows> E <nentries> <entries>"  rows "s e w" (w float bits), entries "<index> <nrows> <rows>"
  "T <tree> <types>"
  "C <tree> <node> <hhmm> <uniform> <test> <old density bits> L <i:live ...> F <i:flags ...> -> <i:weight ...> | <total> <density>"
  (flags 1 cab, 2 trailer; weights only the non-zero ones; all numbers hex but tree, node, parent, index)."""
import argparse
import os
import random
import re
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build

# the count register (the alloca's clear loop) lives in r2, which the code does not use
recomp_closure.EXTRA += '#define ctr r2\n'

def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fb(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def rows_of(text):
    out = []
    for it in re.split(r'<[Ii]tem>', text)[1:]:
        s = re.search(r'<iStartTime value="([^"]*)"', it)
        e = re.search(r'<iEndTime value="([^"]*)"', it)
        w = re.search(r'<fWeight value="([^"]*)"', it)
        if s and e and w:
            out.append((int(float(s.group(1))), int(float(e.group(1))), fb(float(w.group(1)))))
    return out


def load_tree(top):
    """[(parent, entries [(type index, rows)], density rows)] in pre-order, and the type names."""
    nodes = []

    def walk(d, parent):
        k = len(nodes)
        f = d / 'mcAmbientDistribution.xml'
        x = f.read_text(errors='replace') if f.exists() else ''
        body, _, dens = x.partition('<TodDensityArray>')
        ents = [(item.split('<')[0].strip().lower(), rows_of(item)) for item in body.split('<TypeName>')[1:]]
        nodes.append([parent, ents, rows_of(dens) if dens else []])
        td = d / 'mcAmbientDistributionTreeData.xml'
        if td.exists():
            ch = re.search(r'<ChildNames>(.*?)</ChildNames>', td.read_text(errors='replace'), re.S)
            subs = {p.name.lower(): p for p in d.iterdir() if p.is_dir()}
            for n in (re.findall(r'<Item>\s*([^<]*?)\s*</Item>', ch.group(1)) if ch else []):
                if n.lower() in subs:
                    walk(subs[n.lower()], k)
    walk(top, -1)
    names = []
    for _, ents, _ in nodes:
        for n, _ in ents:
            if n not in names:
                names.append(n)
    return [(p, [(names.index(n), r) for n, r in ents], d) for p, ents, d in nodes], names


def synth_rows(rnd, neg=False):
    rows, t = [], 0
    for _ in range(rnd.randint(0, 6)):
        s = rnd.choice((t, rnd.randint(0, 2400)))
        e = rnd.choice((0, min(2400, s + rnd.randint(0, 900)), rnd.randint(0, 2400)))
        w = rnd.choice((0.0, 1.0, 0.5, 0.03, rnd.uniform(0, 3), rnd.uniform(-1, 0) if neg else 0.25, -0.0))
        rows.append((s, e, fb(w)))
        t = e
    return rows


def synth_tree(rnd, n):
    nodes = []
    for k in range(rnd.randint(1, 5)):
        ents = [((-1 if rnd.random() < 0.1 else rnd.randrange(n)), synth_rows(rnd)) for _ in range(rnd.randint(0, 12))]
        nodes.append((k - 1, ents, [] if rnd.random() < 0.4 else synth_rows(rnd, neg=True)))
    return nodes


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--ambients', type=Path, default=Path('D:/MCLA/extracted/cache/tune/ambients'))
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    fn = addr(a.tool, 'F-83932')
    trees = []
    for d in sorted(os.listdir(a.ambients)):
        top = a.ambients / d
        if (top / 'mcAmbientDistribution.xml').exists():
            nodes, names = load_tree(top)
            if names:
                trees.append((nodes, len(names)))
    real = len(trees)
    rnd = random.Random(83932)
    for _ in range(40):
        n = rnd.randint(1, 40)
        trees.append((synth_tree(rnd, n), n))
    # the testtrailers parameter's word (the parameter object +4)
    flag = (0x8289 << 16) - 6040 + 4
    edges = sorted({0, 1, 2359, 2400, 2401, 9999} | {max(v + d, 0) for nodes, _ in trees for _, ents, dens in nodes
                                                      for rows in [dens] + [r for _, r in ents] for s, e, _ in rows
                                                      for v in (s, e) for d in (-1, 0, 1)})
    cases = []
    for c in range(a.cases):
        t = (c % real if c < 3000 else rnd.randrange(len(trees)))
        nodes, n = trees[t]
        node = rnd.randrange(len(nodes))
        hhmm = rnd.choice(edges) if rnd.random() < 0.7 else rnd.randint(0, 2400)
        k = rnd.choice((0, 1, 5, 9, 10, 10, 11, 12, 20, n))
        live = {i: rnd.choice((1, 1, 2, 7, 0x7FFFFFFF)) for i in rnd.sample(range(n), min(k, n))}
        for i in rnd.sample(range(n), min(n, rnd.randint(0, 2))):
            if i not in live:
                live[i] = rnd.choice((-1, -2))
        flags = {i: rnd.choice((1, 2, 3)) for i in range(n) if rnd.random() < 0.15}
        tree = 1 if rnd.random() < 0.06 else 0
        test = 1 if rnd.random() < 0.08 else 0
        old = fb(rnd.choice((0.7, 2.0, 0.0)))
        cases.append((t, node, hhmm, tree, test, old, live, flags))
    # guest memory: the list 0x1000, its type pointers 0x1100, weights 0x1600, the types 0x2000 + 0x80 i,
    # nodes 0xA000 + 0x20 k, node data 0xB000 + 0x20 k, entries / indices / rows from 0xC000
    words = []
    for t, node, hhmm, tree, test, old, live, flags in cases:
        nodes, n = trees[t]
        st = []
        heap = [0xC000]

        def alloc(sz):
            p = heap[0]
            heap[0] += (sz + 15) & ~15
            return p

        def tod(rows):
            p = alloc(12 * len(rows))
            for j, (s, e, w) in enumerate(rows):
                st.extend([(1, p + 12 * j + 4, s), (1, p + 12 * j + 6, e), (0, p + 12 * j + 8, w)])
            return p, len(rows)
        st += [(0, 0x1000, 0x1100), (1, 0x1004, n), (0, 0x1008, 0x1600), (1, 0x100C, n), (0, 0x1010, fb(5.5)), (0, 0x1014, old)]
        for i in range(n):
            f = flags.get(i, 0)
            st += [(0, 0x1100 + 4 * i, 0x2000 + 0x80 * i), (0, 0x1600 + 4 * i, fb(9.25)),
                   (0, 0x2000 + 0x80 * i + 8, live.get(i, 0) & 0xFFFFFFFF),
                   (2, 0x2000 + 0x80 * i + 111, f & 1), (2, 0x2000 + 0x80 * i + 112, f >> 1)]
        for k, (p, ents, dens) in enumerate(nodes):
            nd, da = 0xA000 + 0x20 * k, 0xB000 + 0x20 * k
            st += [(0, nd + 20, 0xA000 + 0x20 * p if p >= 0 else 0), (0, nd + 24, da)]
            ep, ip = alloc(16 * len(ents)), alloc(4 * len(ents))
            for j, (ix, rows) in enumerate(ents):
                rp, rn = tod(rows)
                st += [(0, ep + 16 * j + 8, rp), (1, ep + 16 * j + 12, rn), (0, ip + 4 * j, ix & 0xFFFFFFFF)]
            dp, dn = tod(dens)
            st += [(0, da + 4, ep), (1, da + 8, len(ents)), (0, da + 12, dp), (1, da + 16, dn), (0, da + 20, ip)]
        st.append((0, flag, test))
        words.append(len(st))
        for kind, ad, v in st:
            words += [kind, ad, v]
        words += [0xA000 + 0x20 * node, hhmm, tree, n]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'ambient_weights_inputs.bin'
    inp.write_bytes(b''.join(struct.pack('<I', w) for w in words))
    nl = chr(10)
    NL = chr(92) + 'n'
    body = ('FILE*fi=fopen(argv[2],"rb");std::vector<uint32_t> in;uint32_t w_;while(fread(&w_,4,1,fi)==1)in.push_back(w_);fclose(fi);' + nl +
            'size_t i=0;while(i<in.size()){uint32_t k=in[i++];' + nl +
            ' memset(objects,0,sizeof(objects));' + nl +
            ' for(uint32_t j=0;j<k;j++,i+=3){uint32_t kind=in[i],a=in[i+1],v=in[i+2];'
            'if(kind==0)store(a,v);else if(kind==1)store16(a,uint16_t(v));else *pointer(a)=uint8_t(v);}' + nl +
            ' uint32_t node=in[i],hhmm=in[i+1],tree=in[i+2],n=in[i+3];i+=4;' + nl +
            ' Context ctx{};ctx.r1.u32=0x7F000;ctx.r3.u32=0x1000;ctx.r4.u32=node;ctx.r5.u32=hhmm;ctx.r6.u32=tree;' + nl +
            ' sub_' + fn + '(ctx,nullptr);' + nl +
            ' for(uint32_t t=0;t<n;t++)printf("%08x ",load(0x1600+4*t));printf("| %08x %08x' + NL + '",load(0x1010),load(0x1014));}' + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + fn], main_src, a.out_dir, 'ambient_weights', scratch_end=0x80000)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[:2000])
    out = [x for x in r.stdout.split(nl) if x.strip()]
    assert len(out) == len(cases), (len(out), len(cases))
    rs = lambda rows: ' '.join('%x %x %08x' % r for r in rows)
    lines = []
    for t, (nodes, n) in enumerate(trees):
        lines.append('T %d %d' % (t, n))
        for k, (p, ents, dens) in enumerate(nodes):
            lines.append(('N %d %d %d D %d %s E %d %s' % (t, k, p, len(dens), rs(dens), len(ents),
                                                         ' '.join('%d %d %s' % (ix, len(r), rs(r)) for ix, r in ents))).replace('  ', ' '))
    capped = 0
    for (t, node, hhmm, tree, test, old, live, flags), o in zip(cases, out):
        ws, _, tail = o.partition('|')
        nz = ' '.join('%x:%s' % (i, w) for i, w in enumerate(ws.split()) if w != '00000000')
        lv = ' '.join('%x:%x' % (i, v & 0xFFFFFFFF) for i, v in sorted(live.items()))
        fl = ' '.join('%x:%x' % (i, v) for i, v in sorted(flags.items()))
        lines.append('C %d %d %x %d %d %08x L %s F %s -> %s | %s' % (t, node, hhmm, tree, test, old, lv, fl, nz, tail.strip()))
        capped += sum(1 for v in live.values() if 0 < v < 0x80000000) >= 10
    (a.out_dir / 'ambient_weights_ref.txt').write_text(nl.join(lines) + nl)
    print(len(cases), 'cases;', len(trees), 'trees (', real, 'real:', [len(x[0]) for x in trees[:real]], 'nodes,',
          [x[1] for x in trees[:real]], 'types);', capped, 'with 10 or more types in use;', sum(c[3] for c in cases),
          'uniform;', sum(c[4] for c in cases), 'with the trailer test')


if __name__ == '__main__':
    main()
