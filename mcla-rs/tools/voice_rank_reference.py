"""The voice manager's ranking and pick run offline: the original's F-91203 (every voice slot in use ranked by its levels,
groups, the radix sort) and F-96767 (down the ranking: the voices to start and to stop), the generated code whole and
unchanged (but the C library memset for the original's vector one, F-29294), called as the manager's update F-46122 calls them, on random voice slots in the sound pool's buckets (the
pool's bucket count +204 and bucket table +232 set in the image).
Writes <out-dir>/voice_rank_ref.txt: per case the bucket count, budget and free voices, the slots in use (bucket, slot,
ended, state, group, flags, levels +26 +28 +30 +32), then '->' the four counts, the voices to start and the voices to stop."""
import argparse
import random
import re
import subprocess
from pathlib import Path
import recomp_dispatch as rd
from recomp_closure import index, run

SLOTS = 0x40100000
BANK = 11488
REC = 0x40200000
LA = 0x40210000
LB = 0x40211000
OUT = 0x40212000
STACK = 0x40F00000


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def lis_addi(text, reg):
    m = re.search(r'// lis (r\d+),(-?\d+)\s*\n(?:.*\n){0,3}?\s*// addi %s,\1,(-?\d+)' % reg, text)
    return ((int(m.group(2)) << 16) + int(m.group(3))) & 0xffffffff


def level(rnd):
    k = rnd.random()
    if k < 0.15:
        return 0
    if k < 0.25:
        return rnd.randrange(0x10000)
    if k < 0.35:
        return rnd.choice((0x3c00, 0x3800, 0x0400, 0x7bff))
    import struct
    b = struct.unpack('>I', struct.pack('>f', rnd.uniform(6.2e-5, 2.0)))[0]
    return ((((b >> 23) & 0x1ff) + 16) << 10 | (b >> 13) & 0x3ff | (b >> 16) & 0x8000) & 0xffff


def gen(rnd):
    nb = rnd.choice((1, 1, 2))
    grouped = rnd.random() < 0.3
    dense = rnd.choice((0.05, 0.2, 0.5, 0.9))
    slots = []
    for b in range(nb):
        for i in range(96):
            if rnd.random() >= dense:
                continue
            ended = int(rnd.random() < 0.1)
            st = rnd.choice((0, 0, 1, 1, 2, 3))
            g = rnd.choice((0, 0, 1, 2, 3, 0x1234)) if grouped else 0
            fl = rnd.choice((0, 0, 0, 0x80, 0x20, 0x08, 0x01, 0x04, 0x40, 0x10, 0x25, 0xa1))
            # flag 0x20 voices with equal levels make ties for the radix sort
            h = [level(rnd) for _ in range(4)] if rnd.random() < 0.8 else [0x3c00] * 4
            slots.append((b, i, ended, st, g, fl, *h))
    budget = rnd.choice((0, 1, 5, 20, 76, 76, 200))
    free = rnd.choice((0, 1, 3, 10, 96, 96))
    return nb, budget, free, slots


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    rank, pick, memset = (addr(a.tool, f) for f in ('F-91203', 'F-96767', 'F-29294'))
    idx = index(a.recomp)
    src = lambda n: idx['sub_' + n].read_text(errors='replace')
    r = src(rank)
    r = r[r.index('DEFINE_REX_FUNC(sub_%s)' % rank):]
    r = r[:r.index('\n}\n')]
    pool = lis_addi(r, 'r5')
    p = src(pick)
    p = p[p.index('DEFINE_REX_FUNC(sub_%s)' % pick):]
    p = p[:p.index('\n}\n')]
    # the two id lists' pointers: to start (lis r8 .. lwz 5696(r8)) and to stop (lis r31 .. lwz 5700(r31))
    hi = {m.group(1): int(m.group(2)) for m in re.finditer(r'// lis (r8|r31),(-?\d+)', p)}
    la, lb = ((hi[g] << 16) + int(re.search(r'// lwz r\d+,(-?\d+)\(%s\)' % g, p).group(1)) & 0xffffffff for g in ('r8', 'r31'))
    rnd = random.Random(91203)
    cases = [gen(rnd) for _ in range(600)]
    nl = chr(10)
    rows = []
    for nb, budget, free, slots in cases:
        w = []
        for b, i, ended, st, g, fl, h26, h28, h30, h32 in slots:
            base = SLOTS + b * BANK
            rec = base + 96 + 80 * i
            ent = base + 7776 + 32 * i
            w += [(base + i, 1, 1 | rnd.choice((0, 0x08, 0x40))), (rec + 4, 4, g), (rec + 24, 1, fl), (rec + 26, 2, h26),
                  (rec + 28, 2, h28), (rec + 30, 2, h30), (rec + 32, 2, h32),
                  (ent + 8, 4, (0xfffffff8 | rnd.randrange(8)) if ended else (rnd.randrange(1 << 28) << 3 | rnd.randrange(8))),
                  (ent + 16, 4, st << 30 | rnd.randrange(1 << 30))]
        rows.append('%d,%d,%d,%d,%s,' % (len(w), nb, budget, free, ','.join('%du,%d,%du' % x for x in w)))
    q = 'Context ctx{};ctx.r1.u32=%du;' % STACK
    main_src = (
        'static const uint32_t C[]={' + nl + nl.join(rows) + nl + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' +
        'store(0x%x+232,%du);store(0x%xu,%du);store(0x%xu,%du);' % (pool, SLOTS, la, LA, lb, LB) + nl +
        'for(size_t i=0;i<sizeof(C)/4;){uint32_t n=C[i],nb=C[i+1],bu=C[i+2],fr=C[i+3];const uint32_t*w=C+i+4;i+=4+3*n;'
        'memset(pointer(%du),0,2*%d);memset(pointer(%du),0,0x3000);memset(pointer(%du),0,0x100);store(0x%x+204,nb);'
        % (SLOTS, BANK, REC, OUT, pool) +
        'for(uint32_t k=0;k<n;k++){uint32_t a=w[3*k],s=w[3*k+1],v=w[3*k+2];'
        'if(s==4)store(a,v);else if(s==2)store16(a,v);else *pointer(a)=v;}' + nl +
        # P (+0) holds the record buffer, Q (+4) the group list, the counts at +16
        'store(%du,%du);' % (OUT, REC) +
        '{%s ctx.r3.u32=%du;ctx.r4.u32=%du;ctx.r5.u32=%du;sub_%s(ctx,nullptr);}' % (q, OUT, OUT + 4, OUT + 16, rank) +
        '{%s ctx.r3.u32=load(%du);ctx.r4.u32=load(%du);ctx.r5.u32=fr;ctx.r6.u32=bu;ctx.r7.u32=%du;sub_%s(ctx,nullptr);}'
        % (q, OUT + 4, OUT, OUT + 16, pick) +
        'printf("%%u %%u %%u %%u |",load16(%du),load16(%du),load16(%du),load16(%du));' % (OUT + 16, OUT + 18, OUT + 20, OUT + 22) +
        'for(uint32_t k=0;k<load16(%du);k++)printf(" %%u",load16(%du+2*k));printf(" |");' % (OUT + 24, LA) +
        'for(uint32_t k=0;k<load16(%du);k++)printf(" %%u",load16(%du+2*k));printf("\\n");' % (OUT + 26, LB) +
        '}}' + nl)
    exe = rd.build(idx, ['sub_' + rank, 'sub_' + pick], main_src, a.out_dir, 'voice_rank', image=rd.load_image(a.image),
                   stubs={'sub_' + memset: 'memset(pointer(ctx.r3.u32),ctx.r4.u8,ctx.r5.u32);'})
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cases), (len(out), len(cases))
    lines = []
    for (nb, budget, free, slots), o in zip(cases, out):
        lines.append('%d %d %d | %s -> %s' % (nb, budget, free, ' '.join(','.join(map(str, s)) for s in slots), o))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'voice_rank_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(o.split('|')[1].strip() != '' for o in out), 'starting voices,',
          sum(o.split('|')[2].strip() != '' for o in out), 'stopping voices')


if __name__ == '__main__':
    main()
