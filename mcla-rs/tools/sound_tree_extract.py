"""sound_tree_extract.py <cap.txt> <out.txt>: whole sound trees of a drive recorded in the original, one line per root
sound instance for the test sndobj::tests::trees_match_the_recorded_drive (MCLA_TREE_REC=<out.txt>).

Input: the private recorder's TD lines (every root sound at each update of F-89113, after its time load: the update, the
timer ms and the tree with each node's type, state and data pointer, children in brackets) and SK req lines (the stop
bit seen, F-22244). Names from the data pointers: sounds.dat object offsets plus the run's data base (found by voting).
Output: one line per root sound instance of a recorded drive (TD / SK lines of the private
recorder): NAME|stop index|ms of each update|waves alive after each update (-1 gone)|the node names of its trees. A wave alive = an environment sound (type 15, the
Simple's voice, which takes the Simple's slot once played) in state 2, a Streaming (8) node counted once when any child is; the TD tree is the state at the update's entry,
so the count after update k is the TD of update k+1 of the same instance. Roots holding Speech (4) or Risset (16) skipped.
The original's wait for its banks before the first play (D42) is left out; the Randomized choices are listed (the node
data of the instance's trees) so the port takes the same children."""
import struct, collections, sys, re
def objects(path=r'D:\MCLA\extracted\cache\audio\X360\config\sounds.dat'):
    d = open(path, 'rb').read()
    ver, size = struct.unpack_from('>II', d, 0); o = 8 + size
    strsize = struct.unpack_from('>I', d, o)[0]; o += 4 + strsize
    n = struct.unpack_from('>I', d, o)[0]; o += 8
    objs = {}
    for _ in range(n):
        l = d[o]; nm = d[o + 1:o + 1 + l].decode(); off, sz = struct.unpack_from('>II', d, o + 1 + l); o += 1 + l + 8
        objs[off] = nm
    return objs
def base(ptrs, objs):
    votes = collections.Counter()
    for p in list(ptrs)[:300]:
        for off in objs:
            b = p - off
            if 0x80000000 < b < 0xC0000000: votes[b] += 1
    return votes.most_common(1)[0]

objs = objects()
lines = [l for l in open(sys.argv[1], errors='replace') if l.startswith(('TD ', 'SK '))]
tok = re.compile(r'(\d+):(\d+):([0-9a-f]+)|\(|\)')
def parse(tree):
    """-> (root type, root state, root data, alive count, types seen)"""
    stack = []; root = None; alive = 0; types = set(); stream_depth = []
    nodes = []  # (depth, type, state)
    d = 0
    for m in tok.finditer(tree):
        if m.group(0) == '(': d += 1; continue
        if m.group(0) == ')': d -= 1; continue
        nodes.append((d, int(m.group(1)), int(m.group(2)), int(m.group(3), 16)))
    for i, (dd, ty, st, da) in enumerate(nodes):
        types.add(ty)
        if ty == 8:
            # its children: the following nodes deeper than it until one at its depth or above
            j = i + 1; any2 = False
            while j < len(nodes) and nodes[j][0] > dd:
                if nodes[j][1] == 15 and nodes[j][2] == 2: any2 = True
                j += 1
            alive += any2
        elif ty == 15 and st == 2:
            # not under a Streaming node
            k = i - 1; under = False; want = dd
            while k >= 0:
                if nodes[k][0] < want:
                    if nodes[k][1] == 8: under = True; break
                    want = nodes[k][0]
                k -= 1
            alive += not under
    return nodes[0][1], nodes[0][2], nodes[0][3], alive, types, {n[3] for n in nodes if n[1] != 15}
inst = {}     # snd -> current instance dict
done = []
def close(snd):
    i = inst.pop(snd, None)
    if i: done.append(i)
ptrs = set()
for l in lines:
    p = l.split(); f = dict(t.split('=', 1) for t in p[1:4] if '=' in t) if p[0] == 'TD' else dict(t.split('=') for t in p[2:])
    if p[0] == 'SK':
        snd = f['snd']
        if p[1] == 'req' and snd in inst: inst[snd].setdefault('stop', len(inst[snd]['ms']) - 1)
        # (pk / fail / tout are not kills: the hooks sit at every playing update, Prepare result >= 1, not-ready wait)
        continue
    u, ms, snd = int(f['u']), int(f['ms']), f['snd']
    rest = l.split(None, 4)[4].strip() if len(l.split(None, 4)) > 4 else '='
    cur = inst.get(snd)
    if rest != '=':
        rty, rst, rda, alive, types, datas = parse(rest)
        ptrs.add(rda)
        if cur and (rda != cur['data'] or (rst == 0 and cur['last_st'] >= 2)):
            close(snd); cur = None
        if cur is None:
            cur = inst[snd] = {'data': rda, 'ms': [], 'alive': [], 'types': set(), 'u0': u, 'st': [], 'datas': set()}
        cur['last_st'] = rst; cur['last_alive'] = -1 if rst == 3 else alive; cur['types'] |= types; cur['datas'] |= datas
    elif cur is None:
        continue
    if cur['ms']:
        cur['alive'].append(cur['last_alive'])   # the state after the previous update
    if cur['last_alive'] == -1:
        close(snd); continue
    cur['ms'].append(ms); cur['st'].append(cur['last_st'])
for s in list(inst): close(s)
b, v = base(ptrs, objs)
print('base %08x votes %d' % (b, v), file=sys.stderr)
out = []; skipped = collections.Counter(); trimmed = collections.Counter()
for i in done:
    nm = objs.get(i['data'] - b)
    if not nm: skipped['no name'] += 1; continue
    if i['types'] & {4, 16}: skipped['speech / risset'] += 1; continue
    a = i['alive'] if len(i['alive']) == len(i['ms']) else i['alive'] + [i['last_alive']]
    # D42: the original's wait for its banks (root still 0 / 1 at the next update) is left out: start at the update that played it
    st = i['st'][1:] + [i['last_st'] if i['last_alive'] != -1 else 3]
    j = next((k for k, v in enumerate(st) if v >= 2), None)
    if j is None: skipped['never played'] += 1; continue
    if j > 0: trimmed['load wait'] += 1
    ms, a, stop = i['ms'][j:], a[j:], i.get('stop', -1)
    if stop != -1 and stop < j: skipped['stopped while loading'] += 1; continue
    if stop != -1: stop -= j
    if len(ms) < 2: skipped['one update'] += 1; continue
    picks = ' '.join(sorted({objs[d - b] for d in i['datas'] if d - b in objs}))
    out.append('%s|%d|%s|%s|%s' % (nm, stop, ' '.join(map(str, ms)), ' '.join(map(str, a)), picks))
open(sys.argv[2], 'w').write('\n'.join(out) + '\n')
print(len(out), 'instances', dict(skipped), dict(trimmed), file=sys.stderr)
