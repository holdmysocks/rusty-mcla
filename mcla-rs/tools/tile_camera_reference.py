"""The plant impostor's tile camera (F-73098) run offline: unchanged generated code on flat guest memory, fed with the inputs of an
object-dump recording of the running original (`func <F-73098> ... args=r3,r4,r5,r6,r7 ext=@r3+A8:128,<constants>`): the camera
object (r3, 256 bytes), its box struct (the pointer at +168, min at +32 and max at +48), the back vector at r4, r5 / r6 / r7, and the
start-up constants the static image holds as zero (828609E0 828608C0 82858450 ...). The callees that only set the camera up are
stubs that print their register arguments; the vectors the function writes into the object (+0 .. +63) and the stack words it leaves
are printed too. Output <out-dir>/tile_camera_ref.txt: one line per call, "in: <hex words> -> out: <hex words>".
"""
import argparse
import struct
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
from objdump_read import calls

CONSTS = (0x828609E0, 0x828608C0, 0x827D8460, 0x827D8450)
# the camera set-up callees (stubbed), by registry ID; named for the case file
NAMES = {'F-11735': 'ortho', 'F-24300': 'ortho2', 'F-92665': 'scale'}
STUBS = ('F-86283', 'F-92665', 'F-97951', 'F-37236', 'F-11735', 'F-24300')


def addr(tool, fid):
    return int(subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:], 16)


def words(b):
    return struct.unpack('>%dI' % (len(b) // 4), b)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--dump', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--random', type=int, default=300, help='extra random back vectors')
    ap.add_argument('--vectors', type=Path, help='a recording whose back vectors (r4) are replayed as well (t2)')
    a = ap.parse_args()
    fa = addr(a.tool, 'F-73098')
    import random
    rnd = random.Random(5)
    fb = lambda x: struct.pack('>f', x)
    cases = []
    base = None
    for c in calls(a.dump, fa):
        ob = c.before[c.args[0] & 0xFFFFFFFF]
        ptr = struct.unpack_from('>I', ob, 168)[0]
        box = c.before.get(ptr)
        cv = c.before.get(c.args[1] & 0xFFFFFFFF)
        if box is None or cv is None or len(box) < 64:
            continue
        consts = [c.before.get(k, bytes(16)) for k in CONSTS]
        consts[3] = bytes.fromhex('3f8000003f8000003f8000003f800000')  # (image constant: not in the recording)
        cases.append((ob, box[:64], cv[:16], c.args[2] & 0xFFFFFFFF, 1 if (c.args[3] & 0xFFFFFFFF) else 0, c.args[4] & 0xFF, consts))
        base = cases[0]
    # recorded back vectors of the open city (t2) and random / edge ones, with random boxes
    vecs = []
    if a.vectors:
        for c in calls(a.vectors, fa):
            cv = c.before.get(c.args[1] & 0xFFFFFFFF)
            if cv:
                vecs.append(struct.unpack('>4f', cv[:16]))
    for _ in range(a.random):
        v = [rnd.uniform(-1, 1) for _ in range(3)]
        n = sum(x * x for x in v) ** .5
        vecs.append(tuple(x / n for x in v) + (0.0,))
    for v in ((0, 1, 0, 0), (0, -1, 0, 0), (1e-4, 1, 0, 0), (0, 0.9999999, 1e-4, 0), (0, 0, 0, 0), (1, 0, 0, 0), (0, 0, -1, 0), (3, 4, 0, 1),
              (1e-20, 1, 1e-20, 0), (0.6, 0.0, 0.8, 0), (-0.6, 0.01, -0.8, 0)):
        vecs.append(v)
    ob0, box0, cv0, _, _, _, consts0 = base
    ob0 = ob0[:148] + fb(1.0) + ob0[152:]
    for v in vecs:
        for k in range(2):
            lo = [rnd.uniform(-8, -0.5), rnd.uniform(-0.5, 1), rnd.uniform(-8, -0.5)]
            hi = [rnd.uniform(0.5, 9), rnd.uniform(2, 14), rnd.uniform(0.5, 9)]
            box = box0[:32] + b''.join(fb(x) for x in lo) + box0[44:48] + b''.join(fb(x) for x in hi) + box0[60:64]
            r5 = rnd.choice((-1, 0, 1, 2, 7))
            cases.append((ob0, box, b''.join(fb(x) for x in v), r5 & 0xFFFFFFFF, rnd.choice((0, 1)), 0, consts0))
    print(len(cases), 'cases')
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = []
    for ob, box, cv, r5, r6, r7, consts in cases:
        rows.append('{' + ','.join('0x%08x' % w for w in list(words(ob)) + list(words(box)) + list(words(cv)) + [r5, r6, r7] +
                                    [w for cb in consts for w in words(cb)]) + '},')
    ncol = 64 + 16 + 4 + 3 + 16
    dump = ''.join('printf("%%08x ",load(0x%x+4*k));' % 0 for _ in ())  # (placeholder, built below)
    stubs = {}
    names = {}
    for fid in STUBS:
        s = 'sub_%X' % addr(a.tool, fid)
        names[s[4:]] = NAMES.get(fid)
        stubs[s] = ('printf("%s %%08x %%08x %%08x f %%016llx %%016llx %%016llx %%016llx %%016llx %%016llx%s",ctx.r3.u32,ctx.r4.u32,ctx.r5.u32,'
                    '(unsigned long long)ctx.f1.u64,(unsigned long long)ctx.f2.u64,(unsigned long long)ctx.f3.u64,(unsigned long long)ctx.f4.u64,'
                    '(unsigned long long)ctx.f5.u64,(unsigned long long)ctx.f6.u64);' % (s[4:], NL)).replace('" ', '" ')
    body = ('static const uint32_t C[][%d]={' % ncol + nl + nl.join(rows) + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            'for(int k=0;k<64;k++)store(0x1000+4*k,c[k]);store(0x10A8,0x1400);' +
            'for(int k=0;k<16;k++)store(0x1400+4*k,c[64+k]);' +
            'for(int k=0;k<4;k++)store(0x1800+4*k,c[80+k]);' +
            'for(int j=0;j<4;j++)for(int k=0;k<4;k++)store(%s[j]+4*k,c[87+4*j+k]);' % '(uint32_t[]){0x828609E0,0x828608C0,0x827D8460,0x827D8450}' +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r4.u32=0x1800;ctx.r5.u64=(int64_t)(int32_t)c[84];ctx.r6.u32=c[85]?0x1C00:0;ctx.r7.u32=c[86];ctx.r1.u32=0x1F00;' +
            'sub_%X(ctx,nullptr);printf("OBJ");for(int k=0;k<20;k++)printf(" %%08x",load(0x1000+4*k));for(int k=0;k<48;k++)printf(" %%08x",load(0x1DD0+4*k));printf("%s");}' % (fa, NL) + nl)
    main_src = 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl
    idx = index(a.recomp)
    exe = build(idx, ['sub_%X' % fa], main_src, a.out_dir, 'tile_camera', stubs=stubs, vector=True)
    # (as the SDK builds the game: -msse4.1, so the dot product is the hardware instruction, not simde's portable loop)
    from recomp_closure import SIMDE
    r = subprocess.run(['g++', '-std=c++17', '-O2', '-ffp-contract=off', '-msse4.1', '-static', '-I' + str(SIMDE), str(exe.with_suffix('.cpp')), '-o', str(exe)],
                       capture_output=True, text=True)
    assert r.returncode == 0, r.stderr[:600]
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'tile_camera_raw.txt').write_text(out)
    lines, cur = [], []
    ci = 0
    for ln in out.split(nl):
        if not ln:
            continue
        if ln.startswith('OBJ'):
            ob, box, cv, r5, r6, r7, _ = cases[ci]
            ci += 1
            w = ln.split()[1:]
            f = struct.unpack('>4f', cv)
            bn = struct.unpack('>16f', box)
            bit = 1 if struct.unpack('>i', struct.pack('>I', r5))[0] < 0 else 0  # (the vectors go to +16 x bit and +16 x (bit + 2))
            res = [w[4 * bit + k] for k in range(4)] + [w[4 * (bit + 2) + k] for k in range(3)]
            lines.append('%s %s %s %d %d %d -> %s | %s' % (' '.join('%08x' % x for x in struct.unpack('>4I', cv)),
                         ' '.join('%08x' % x for x in struct.unpack('>4I', box[32:48])), ' '.join('%08x' % x for x in struct.unpack('>4I', box[48:64])),
                         struct.unpack('>i', struct.pack('>I', r5))[0], r6, struct.unpack('>f', ob[148:152])[0] != 0, ' '.join(res), ' ; '.join(cur)))
            cur = []
        else:
            t = ln.split()
            if names.get(t[0]):
                cur.append(names[t[0]] + ' ' + ' '.join(t[5:]))
    (a.out_dir / 'tile_camera_ref.txt').write_text(nl.join(lines) + nl)
    print(len(out.split(nl)), 'lines')


if __name__ == '__main__':
    main()
