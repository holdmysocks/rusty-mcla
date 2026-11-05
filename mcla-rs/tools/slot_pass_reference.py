"""The car's part-slot pass run offline: the original's F-14987, the generated code whole and unchanged, on random cars.
Each case: a skeleton of 4..24 bones with random globals, the 55 slots each with or without a part (its matrix index
+52 random, sometimes -1 or past the end) and an instance, the wide body flag, the three damage matrices of the
car-parts object (+1232 / +1296 / +1360, sometimes the identity), G+6465 and G+92. Stubs: the part lookup F-87055
hands the case's part of slot r4; the skeleton routines F-89389 (reset) and F-97374 (update) do nothing but log
the instance updated with its parent pointer; the debug print F-47121 logs the slot; F-26026 (the 13 records) does
nothing. The vector constants the game fills at start (the xyz mask 0x82860950, (0,0,0,1) at 0x82860930) are set.
Writes <out-dir>/slot_pass_ref.txt, one case a line:
  nb wide h6465 g92 r60 | the three A matrices (48 words) | events (k:b the slot's instance hung on bone b, k:I on
  the identity, Bk the bad index print) | for each bone a moved slot (2, 3, 5, 6, 40) hung on: b, its 16 words
  before, its 16 words after."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_dispatch as rd
from recomp_closure import index, run

# (the vector shim lacks the condition register's setFromMask / so of the record-form compares, written as the SDK's
# rex/ppc/context.h has them - as tools/drawn_steer_reference.py; built with -msse4.1 as the recomp's x64 build is)
_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    return s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                     'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)


recomp_closure.vector_shim = vector_shim
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)

G, X, M, R, D = 0x40100000, 0x40102000, 0x40102400, 0x40103000, 0x40104000
S, GLOB, PARTTAB, INST, PART, CAR = 0x40105000, 0x40106000, 0x40107000, 0x40108000, 0x4010A000, 0x4010B000
STACK = 0x40F00000
EV = 0x40200000                                              # the updates logged: count, then (slot, bone) pairs
IDENT = 0x827D85D0
CASES = 400
MOVED = (2, 3, 5, 6, 40)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def fbits(v):
    return struct.unpack('>I', struct.pack('>f', v))[0]


def matrix(rng, near):
    if near and rng.random() < 0.15:
        return [1.0, 0, 0, 0, 0, 1.0, 0, 0, 0, 0, 1.0, 0, 0, 0, 0, 1.0]
    m = []
    for r in range(4):
        for c in range(4):
            if c == 3:
                m.append(1.0 if r == 3 else 0.0)
            elif near:
                m.append((1.0 if r == c else 0.0) + rng.uniform(-0.2, 0.2) if r < 3 else rng.uniform(-0.3, 0.3))
            else:
                m.append(rng.uniform(-2.0, 2.0))
    return m


def cases(seed=14987):
    rng = random.Random(seed)
    out = []
    for _ in range(CASES):
        nb = rng.randint(4, 24)
        st = []
        w = lambda a, v: st.append((a, v))
        w(G + 20, X)
        w(X + 80, M)
        w(X + 132, R)
        w(X + 84, D)
        w(M + 40, rng.choice([0, 0x01000000]))
        wide = 1 if rng.random() < 0.3 else 0
        w(R + 2200, wide)                               # R+2203 the wide body flag (big endian word at +2200)
        h6465 = 1 if rng.random() < 0.2 else 0
        w(G + 6464, h6465 << 16)                             # G+6465 (G+6467 and G+6464 stay 0)
        g92 = rng.choice([0, 5, 23, 24, 99])
        w(G + 92, g92)
        r60 = rng.choice([0, 1])
        w(R + 2204, r60 << 24)                               # R+2144+60
        w(G + 6732, S)
        w(S + 8, 0)
        w(S + 16, nb)
        w(S + 20, GLOB)
        for b in range(nb):
            for i, v in enumerate(matrix(rng, False)):
                w(GLOB + 64 * b + 4 * i, fbits(v))
        As = [matrix(rng, True) for _ in range(3)]
        for j, off in enumerate((1232, 1296, 1360)):
            for i, v in enumerate(As[j]):
                w(D + off + 4 * i, fbits(v))
        for k in range(55):
            if rng.random() < 0.75:
                w(PARTTAB + 4 * k, PART + 64 * k)
                r = rng.random()
                idx = 0xFFFFFFFF if r < 0.05 else nb + rng.randint(0, 3) if r < 0.1 else rng.randrange(nb)
                w(PART + 64 * k + 52, idx)
            if rng.random() < 0.85:
                w(G + 6740 + 4 * k, INST + 64 * k)
        for i, v in enumerate(matrix(rng, False)):
            w(CAR + 4 * i, fbits(v))
        out.append((nb, wide, h6465, g92, r60, As, st))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    st = lambda k: 'sub_' + addr(a.tool, k)
    nl = chr(10)
    cs = cases()
    stubs = {
        st('F-87055'): 'ctx.r3.u32=load(%du+4*ctx.r4.u32);' % PARTTAB,
        st('F-89389'): '',
        st('F-97374'): 'if(ctx.r3.u32!=%du){uint32_t k=(ctx.r3.u32-%du)/64,p=load(ctx.r3.u32+8);'
                       'if(p==%du)printf(" %%u:I",k);else printf(" %%u:%%u",k,(p-%du)/64);'
                       'uint32_t n=load(%du);store(%du+8+8*n,k);store(%du+12+8*n,(p-%du)/64);store(%du,n+1);}'
                       % (S, INST, IDENT, GLOB, EV, EV, EV, GLOB, EV),
        st('F-47121'): 'printf(" B%u",ctx.r5.u32);',
        st('F-26026'): '',
    }
    data = []
    heads = []
    for nb, wide, h, g92, r60, As, ops in cs:
        heads.append('"%d %d %d %d %d |%s |"' % (nb, wide, h, g92, r60, ''.join(' %08x' % fbits(v) for m in As for v in m)))
        data.append('{' + ','.join('%du,%du' % o for o in ops) + '}')
    main_src = (
        ''.join('static const uint32_t C%d[]=%s;' % (i, d) + nl for i, d in enumerate(data)) +
        'static const uint32_t*CS[]={' + ','.join('C%d' % i for i in range(len(cs))) + '};' + nl +
        'static const size_t CN[]={' + ','.join('sizeof(C%d)/8' % i for i in range(len(cs))) + '};' + nl +
        'static const char*HD[]={' + ','.join(heads) + '};' + nl +
        'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
        'image.assign(std::istreambuf_iterator<char>(bin),{});setvbuf(stdout,nullptr,_IONBF,0);' + nl +
        'for(int i=0;i<3;i++)store(0x82860950u+4*i,0xFFFFFFFFu);store(0x8286095Cu,0);'
        'store(0x82860930u,0);store(0x82860934u,0);store(0x82860938u,0);store(0x8286093Cu,0x3F800000u);' + nl +
        'for(size_t c=0;c<%d;c++){std::memset(arena+0x100000,0,0x10000);store(%du,0);' % (len(cs), EV) +
        'for(size_t j=0;j<CN[c];j++)store(CS[c][2*j],CS[c][2*j+1]);'
        'uint32_t nb=load(%du+16);std::vector<uint32_t> before(nb*16);for(uint32_t j=0;j<nb*16;j++)before[j]=load(%du+4*j);' % (S, GLOB) +
        'printf("%%s",HD[c]);'
        '{Context ctx{};ctx.r1.u32=%du;ctx.r3.u32=%du;ctx.r4.u32=%du;%s(ctx,nullptr);}' % (STACK, G, CAR, st('F-14987')) +
        'printf(" |");bool done[64]={};for(uint32_t e=0;e<load(' + str(EV) + 'u);e++){uint32_t k=load(' + str(EV + 8) + 'u+8*e),b=load(' + str(EV + 12) + 'u+8*e);'
        'if((k==2||k==3||k==5||k==6||k==40)&&b<nb&&!done[b]){done[b]=true;printf(" %u",b);'
        'for(int j=0;j<16;j++)printf(" %08x",before[b*16+j]);for(int j=0;j<16;j++)printf(" %08x",load(' + str(GLOB) + 'u+64*b+4*j));}}'
        'printf("\\n");}}' + nl)
    idx = index(a.recomp)
    exe = rd.build(idx, [st('F-14987')], main_src, a.out_dir, 'slot_pass', image=rd.load_image(a.image), stubs=stubs)
    out = run(exe, a.image)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'slot_pass_ref.txt').write_text(out)
    print(out[:2000])


if __name__ == '__main__':
    main()
