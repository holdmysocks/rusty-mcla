"""An ambient car's init draws run offline: the car init F-77422 whole and unchanged (vector mode, flat guest memory),
with its slot 5 F-60230, the base F-92604, the car MWC's seed F-37317 (with its command-line hook F-75950 reading the
image's empty parameter) and the Gaussian F-66400 with its logarithm running as they are. Stubbed (no draws in them):
the networked test F-24338 (false), the physics instance F-91199 (its call captures the car MWC after F-60230), the
instance flag F-55165, the id helpers (the peer F-78245 -> 0, the type index F-54952 -> 5, the id lookup F-10061 ->
found for the case's first k ids), the cab / trailer registration and pairing (F-70927, F-18069 -> none), the hitch
F-45988 (captured), the route copies, every virtual call (0; slot 3 of the car's object +496 writes a size).

Guest memory: the params P 0x1000 (+0 the matrix, +228 0, +248 behaviour 0, +252 spot, +260 owner 0, +264 the id,
+272, +276, +288 the seed, +292 the byte), the car 0x2000 (vtable 0x3F00, +448 type 0x3000, +456 / +472 0, +496 -> 0x3100,
+500 flags, the objects +512 and +2688 with the vtable), the type 0x3000 (+111 cab, +112 trailer), the manager S with S+0x64234 = 0x3800 (the
byte) and +0x64238 the clock; the manager pointer = S; the two shared MWCs (park_rng's words and the 8 bytes below them).

Writes <out-dir>/spawn_init_ref.txt, a case a line: "seed s2lo s2c s1lo s1c flags id292 found -> life timer wait id hour
blink blinkon tail bonus steer hitch placedlo placedc carlo carc s2lo s2c s1lo s1c lookups" (hex words; flags: 1 spot, 2
cab, 4 trailer, 8 proxy, 16 green, 32 id >= 0 given; a word starting 82 is split, 8xxx_xxxx)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build

recomp_closure.shim = lambda: recomp_leaf.shim(0x10000)
recomp_closure.EXTRA += ('#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n'
                         '#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'
                         'static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n'
                         'static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}\n'
                         'static uint32_t get(uint32_t a){uint8_t*p=image.data()+(a-0x82000000u);return uint32_t(p[0])<<24|p[1]<<16|p[2]<<8|p[3];}\n'
                         'static uint32_t snap[2],hitched,looked,found_n;\n')
_vshim = recomp_closure.vector_shim


def vector_shim():
    s = '#include <simde/x86/sse4.1.h>' + chr(10) + '#include <xmmintrin.h>' + chr(10) + _vshim()
    s = s.replace('alignas(16) static uint8_t VectorMaskL', 'typedef VReg PPCVRegister;' + chr(10) + 'alignas(16) static uint8_t VectorMaskL', 1)
    cond = 'bool eq{}, lt{}, gt{};'
    assert s.count(cond) == 1
    s = s.replace(cond, 'bool eq{}, lt{}, gt{}, so{}; void setFromMask(simde__m128 mask,int imm){int m=simde_mm_movemask_ps(mask);'
                  'lt=m==imm;gt=0;eq=m==0;so=0;}', 1)
    fp = ('struct { void disableFlushMode() {} void enableFlushMode() {} void disableFlushModeUnconditional() {}'
          ' void enableFlushModeUnconditional() {} } fpscr;')
    assert s.count(fp) == 1
    on, off = '{_mm_setcsr(_mm_getcsr()|0x8040);}', '{_mm_setcsr(_mm_getcsr()&~0x8040u);}'
    return s.replace(fp, 'struct { void disableFlushMode() %s void enableFlushMode() %s void disableFlushModeUnconditional() %s'
                     ' void enableFlushModeUnconditional() %s } fpscr;' % (off, on, off, on), 1)


recomp_closure.vector_shim = vector_shim
_run = subprocess.run
recomp_closure.subprocess.run = lambda a, **k: _run([a[0], '-msse4.1', '-mssse3'] + a[1:], **k) if a and a[0] == 'g++' else _run(a, **k)

nl = chr(10)
NL = chr(92) + 'n'


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def cases(rnd, count):
    out = []
    for c in range(count):
        seed = rnd.choice((0, 1, 0x7FFFFFFF, 0x80000000, 0xFFFFFFFF, rnd.getrandbits(31), rnd.getrandbits(31), rnd.getrandbits(32)))
        s2 = [rnd.getrandbits(32), rnd.getrandbits(32) % 0x5CDCFAA7]
        s1 = [rnd.getrandbits(32), rnd.getrandbits(32) % 0x5CDCFAA7]
        kind = rnd.choice((0, 0, 0, 2, 4))
        flags = kind | (1 if rnd.random() < 0.4 else 0) | (8 if rnd.random() < 0.1 else 0) | (16 if rnd.random() < 0.1 else 0)
        idv = 0xFFFFFFFF
        if rnd.random() < 0.15:
            flags |= 32
            idv = rnd.getrandbits(20)
        b292 = 1 if (kind and rnd.random() < 0.9) or rnd.random() < 0.05 else 0
        found = rnd.choice((0, 0, 0, 1, 2, 25))
        out.append([seed] + s2 + s1 + [flags, idv, b292, found])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    ids = ('F-77422', 'F-24338', 'F-55165', 'F-78245', 'F-54952', 'F-10061', 'F-45988')
    nm = {f: addr(a.tool, f) for f in ids}
    # the physics instance, F-92604's last helper, the registration and the pairing, the route record helpers
    extra = {k: addr(a.tool, f) for k, f in (('phys', 'F-91199'), ('tail', 'F-27899'), ('reg', 'F-70927'), ('pair', 'F-18069'),
                                              ('rinit', 'F-18347'), ('rcopy', 'F-29143'))}
    rows = cases(random.Random(77422), a.cases)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    inp = a.out_dir / 'spawn_init_inputs.bin'
    inp.write_bytes(b''.join(struct.pack('<%dI' % len(r), *r) for r in rows))
    S = (0x3800 - 0x64234) & 0xFFFFFFFF
    stubs = {
        # (the parameter parser F-75950 calls only for a set command-line seed; the image has none)
        'sub_' + addr(a.tool, 'F-53090'): 'throw std::runtime_error("seed parameter");',
        'sub_' + nm['F-24338']: 'ctx.r3.u64=0;',
        'sub_' + extra['phys']: 'snap[0]=load(ctx.r4.u32+384);snap[1]=load(ctx.r4.u32+388);',
        'sub_' + nm['F-55165']: '',
        'sub_' + extra['tail']: '',
        'sub_' + nm['F-78245']: 'ctx.r3.u64=0;',
        'sub_' + nm['F-54952']: 'ctx.r3.u64=5;',
        'sub_' + nm['F-10061']: 'looked++;ctx.r3.u64=(looked<=found_n)?0x2000:0;',
        'sub_' + extra['reg']: '',
        'sub_' + nm['F-45988']: 'hitched=1;',
        'sub_' + extra['pair']: 'ctx.r3.u64=0;',
        'sub_' + extra['rinit']: '',
        'sub_' + extra['rcopy']: '',
    }
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){' + nl +
            ' if((a>>16)!=0x82FF)throw std::runtime_error("unexpected indirect call "+std::to_string(a)+" r3 "+std::to_string(ctx.r3.u32));uint32_t k=(a&0xFFFF)/4;' + nl +
            ' if(k==3){putf(ctx.r4.u32,1.75f);putf(ctx.r4.u32+4,4.5f);}' + nl +
            ' if(k==2&&ctx.r3.u32==0x2000+2688){snap[0]=load(0x2000+384);snap[1]=load(0x2000+388);}' + nl +
            ' ctx.r3.u64=0;}' + nl)
    body = ('FILE*fi=fopen(argv[2],"rb");std::vector<uint32_t> in;uint32_t w_;while(fread(&w_,4,1,fi)==1)in.push_back(w_);fclose(fi);' + nl +
            'for(size_t i=0;i+9<=in.size();i+=9){const uint32_t*c=in.data()+i;' + nl +
            ' memset(objects,0,0xF000);const uint32_t P=0x1000,C=0x2000,T=0x3000,VT=0x3F00,S=%uu,SP=(0x8287u<<16)-10012,R2=(0x8286u<<16)+2176;' % S + nl +
            ' for(int k=0;k<64;k++)store(VT+4*k,0x82FF0000u+4*k);' + nl +
            ' for(int k=0;k<4;k++)putf(P+20*k,1.0f);' + nl +
            ' store(P+252,(c[5]&1)?0x1400:0);store(P+264,c[6]);store(P+272,0x77);store(P+276,0x55);store(P+288,c[0]);*pointer(P+292)=uint8_t(c[7]);' + nl +
            ' store(C,VT);store(C+448,T);store(C+496,0x3100);store(C+512,VT);store(C+2688,VT);store(0x3100,VT);store(T,VT);' + nl +
            ' *pointer(C+500)=(c[5]&8)?0x20:0;*pointer(T+111)=(c[5]&2)?1:0;*pointer(T+112)=(c[5]&4)?1:0;' + nl +
            ' *pointer(0x3800)=(c[5]&16)?1:0;putf(0x3804,12.5f);' + nl +
            ' put(SP,S);put(R2,c[1]);put(R2+4,c[2]);put(R2+16,c[3]);put(R2+20,c[4]);' + nl +
            ' snap[0]=snap[1]=0xDEADBEEF;hitched=0;looked=0;found_n=c[8];' + nl +
            ' Context ctx{};ctx.r1.u32=0xFF00;ctx.r3.u32=C;ctx.r4.u32=P;' + nl +
            ' sub_' + nm['F-77422'] + '(ctx,nullptr);' + nl +
            ' const int off[]={468,432,444,476,3568,3572,3576,3580,428,3584};' + nl +
            ' for(int o:off)printf("%08x ",load(C+o));' + nl +
            ' printf("%u %08x %08x %08x %08x %08x %08x %08x %08x %u' + NL + '",hitched,snap[0],snap[1],load(C+384),load(C+388),'
            'get(R2),get(R2+4),get(R2+16),get(R2+20),looked);}' + nl)
    main_src = (disp + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + nm['F-77422']], main_src, a.out_dir, 'spawn_init', stubs=stubs, vector=True)
    r = subprocess.run([str(exe), str(a.image), str(inp)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[:2000])
    out = [x.strip() for x in r.stdout.split(nl) if x.strip()]
    assert len(out) == len(rows), (len(out), len(rows))
    # (an 8-digit word starting 82 is written in two halves, 8xxx_xxxx: it is data, not an address of the original)
    split = lambda t: t[:4] + '_' + t[4:] if len(t) == 8 and t.lower().startswith('82') else t
    lines = ['%s -> %s' % (' '.join(split('%x' % x) for x in row), ' '.join(split(t) for t in o.split())) for row, o in zip(rows, out)]
    (a.out_dir / 'spawn_init_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases,', sum(r[5] & 1 for r in rows), 'spots,', sum(o.split()[10] == '1' for o in out), 'hitched')


if __name__ == '__main__':
    main()
