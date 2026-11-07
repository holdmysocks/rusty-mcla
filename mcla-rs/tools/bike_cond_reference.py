"""The bike rider machine's transition tests (mcMotorcycleAnimMachine) run offline on the original's code: the
machine is built by its own Init F-49795 (the car's too: the expression table by vtable slot 2 - the bike's symbol table F-35793 - and
the graph by slot 3 - the bike graph builder F-71745), the tune by its constructor F-40144 (mcAnimMachineConstantData), then for random
contexts every transition of every state is tested with the original's test F-15065 (the condition binding, or the RPN string through
the expression evaluator). Stubbed, as inputs of the case: the clip's time remaining, the overlay's time remaining, the phase (0) and
IsCutscenePlaying.
Usage: bike_cond_reference.py <out-dir> --symtab <pairs> --graph <graph_table.txt> [--tool mcla-tool] [--recomp D:/MCLA-static]
Writes <out-dir>/ride_cond_ref.txt: "w0 .. w13 -> bits" (hex words: ctx +4 gesture, +8 flags, +12 flags2, +20 goal, +28 smoothed
steering, +48 bytes, +92 throttle, +96 speed, +124 shift, +148 timer A, +152 timer B; the clip's remaining, the overlay's remaining,
cutscene; one 0/1 per transition in build order) and ride_graph_raw.txt (per state its transition objects)."""
import argparse, random, struct, subprocess, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import recomp_dispatch as rd
import recomp_closure as rc
rc.EXTRA += '''#include <atomic>
alignas(16) static uint8_t VectorShiftTableL[]={0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x00,
0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01,
0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02,
0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03,
0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04,
0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05,
0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06,
0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07,
0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08,
0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09,
0x19, 0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A,
0x1A, 0x19, 0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B,
0x1B, 0x1A, 0x19, 0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C,
0x1C, 0x1B, 0x1A, 0x19, 0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D,
0x1D, 0x1C, 0x1B, 0x1A, 0x19, 0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E,
0x1E, 0x1D, 0x1C, 0x1B, 0x1A, 0x19, 0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F};
namespace rex { namespace ppc {
static inline simde__m128i simde_mm_perm_epi8_(simde__m128i a, simde__m128i b, simde__m128i c) {
 simde__m128i d = simde_mm_set1_epi8(0xF); simde__m128i e = simde_mm_sub_epi8(d, simde_mm_and_si128(c, d));
 return simde_mm_blendv_epi8(simde_mm_shuffle_epi8(a, e), simde_mm_shuffle_epi8(b, e), simde_mm_slli_epi32(c, 3)); } } }
'''



def addr(tool, fid):
    return int(subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1], 16)


MAIN = r"""
static std::string gs(uint32_t a){std::string s; for(;;a++){uint8_t c=*pointer(a); if(!c)break; s+=(char)c;} return s;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 rt_reset(); const uint32_t M=0x40800000,CTX=0x40810000,DRV=0x40820000,TUNE=0x40830000;
 store(M,VTu);
 {Context c=fresh(); c.r3.u64=TUNE; TUNECTOR(c,nullptr);}
 {Context c=fresh(); c.r3.u64=M; c.r4.u64=DRV; c.r5.u64=M; c.r6.u64=CTX; INIT(c,nullptr);}
 uint32_t ns=*pointer(M+8)<<8|*pointer(M+9), sa=load(M+4);
 if(argc>3){ for(uint32_t s=0;s<ns;s++){ uint32_t st=load(sa+4*s); uint32_t nt=*pointer(st+104)<<8|*pointer(st+105), ta=load(st+100);
   printf("%s",gs(load(st+4)).c_str()); for(uint32_t t=0;t<nt;t++) printf("|%s",gs(load(load(load(ta+4*t)+68)+4)).c_str()); printf("\n"); }
   printf("tune48 %08x\n",load(TUNE+48)); return 0; }
 std::ifstream in(argv[2]); uint32_t w;
 for(;;){
  for(int i=0;i<46;i++){ if(!(in>>std::hex>>w)) return 0; store((i<43?CTX+4*i:CASE+4*(i-43)),w);} store(CTX+16,TUNE);
  for(uint32_t s=0;s<ns;s++){ uint32_t st=load(sa+4*s); uint32_t nt=*pointer(st+104)<<8|*pointer(st+105), ta=load(st+100);
   for(uint32_t t=0;t<nt;t++){ Context c=fresh(); c.r3.u64=load(ta+4*t); TEST(c,nullptr); putchar('0'+(c.r3.u32&0xff?1:0)); } }
  putchar(10); }
}
"""

# the context words the riding states' tests read, kept in the reference: +4 +8 +12 +20 +28 +48 +92 +96 +124 +148 +152
USED = [1, 2, 3, 5, 7, 12, 23, 24, 31, 37, 38]
CASE = 0x40E00000


def f2w(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('out', type=Path)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--recomp', type=Path, default=Path('D:/MCLA-static'))
    ap.add_argument('--image', type=Path, default=Path('D:/MCLA/work/2026-10-10/original-image.bin'))
    ap.add_argument('--symtab', type=Path, required=True, help='the bike symbol table, lines "name 0xaddr" (F-35793 run offline)')
    ap.add_argument('--graph', type=Path, required=True, help="the bike graph table (F-71745 run offline): its 'if fn' conditions")
    ap.add_argument('--n', type=int, default=4000)
    a = ap.parse_args()
    work = a.out.resolve(); work.mkdir(parents=True, exist_ok=True)
    ids = ('F-71745', 'F-49795', 'F-15065', 'F-40144', 'F-35793', 'F-87745', 'F-70114', 'F-96834', 'F-91845', 'F-14201', 'F-35685',
           'F-75034', 'F-39484', 'F-20307', 'F-47121', 'F-42983')
    fa = {k: addr(a.tool, k) for k in ids}
    fn = {k: 'sub_%08X' % v for k, v in fa.items()}
    syms = [int(l.split()[1], 16) for l in open(a.symtab) if l.strip()]
    conds = [int(l.split(' if fn ')[1].split()[0], 16) for l in open(a.graph) if ' if fn ' in l]
    # (the RPN evaluator F-39484 and three helpers the tests reach through pointers: F-20307, F-47121, F-42983)
    targets = sorted(set(syms + conds) | {fa['F-39484'], fa['F-20307'], fa['F-47121'], fa['F-42983']})
    # (ReadTimeRemaining F-87745, ReadTimeRemainingOverlay F-70114, ReadAnimPhase F-96834, IsCutscenePlaying F-91845; a copy
    # F-14201 and the reference count's add / release F-35685 / F-75034)
    stubs = {fn['F-87745']: f'{{uint32_t w=load({CASE:#x}u);float f;memcpy(&f,&w,4);ctx.f1.f64=f;}}',
             fn['F-70114']: f'{{uint32_t w=load({CASE + 4:#x}u);float f;memcpy(&f,&w,4);ctx.f1.f64=f;}}',
             fn['F-96834']: 'ctx.f1.f64=0.0;',
             fn['F-91845']: f'ctx.r3.u64=load({CASE + 8:#x}u);',
             fn['F-14201']: 'for(uint32_t i=0;i<ctx.r5.u32;i++) *pointer(ctx.r3.u32+i)=*pointer(ctx.r4.u32+i);',
             fn['F-35685']: 'uint32_t v=load(ctx.r3.u32)+1; store(ctx.r3.u32,v); ctx.r3.u64=v;',
             fn['F-75034']: 'uint32_t v=load(ctx.r3.u32)-1; store(ctx.r3.u32,v); ctx.r3.u64=v;'}
    idx = rd.index(a.recomp); img = rd.load_image(a.image)
    # (the bike machine's vtable: the one whose slots 2 / 3 / 4 are its symbol table, its graph builder and the Init)
    pat = b''.join(struct.pack('>I', fa[k]) for k in ('F-35793', 'F-71745', 'F-49795'))
    at = img.find(pat)
    assert at > 0 and img.find(pat, at + 1) < 0
    vt = 0x82000000 + at - 8
    main_src = (MAIN.replace('VTu', '0x%08Xu' % vt).replace('TUNECTOR', fn['F-40144']).replace('INIT', fn['F-49795'])
                .replace('TEST', fn['F-15065']).replace('CASE+', '0x%08Xu+' % CASE))
    exe = rd.build(idx, [fn['F-49795'], fn['F-15065'], fn['F-40144']], main_src, work, 'ride_cond', stubs=stubs, image=img,
                   vtables=[(vt, 5)], targets=targets)
    g = subprocess.run([str(exe), str(a.image), '-', 'graph'], capture_output=True, text=True)
    if g.returncode:
        raise SystemExit(g.stderr[-800:] or f'exit {g.returncode}')
    (work / 'ride_graph_raw.txt').write_text(g.stdout)
    rnd = random.Random(20261107)
    stop = f2w(0.15)
    stop = struct.unpack('>f', struct.pack('>f', struct.unpack('>f', struct.pack('>I', stop))[0] * 0.44704))[0]
    near = [0.0, 0.02, -0.02, 0.1, -0.1, 0.25, 0.5, -0.5, 0.75, 1.5, 15.0, 0.033, stop, stop + 0.5, 1.0, -1.0, 3.0]

    def fl():
        if rnd.random() < 0.6:
            x = rnd.choice(near)
            return x if rnd.random() < 0.5 else x + rnd.choice((-1, 1)) * rnd.choice((1e-7, 1e-6, 1e-3, 0.01))
        return rnd.uniform(-20, 20)
    cases = []
    while len(cases) < a.n:
        ctx = [f2w(fl()) for _ in range(43)]
        ctx[1] = 20                                      # +4 gesture: none (the gesture switch is not part of the riding states)
        f8 = rnd.getrandbits(32) & rnd.getrandbits(32) if rnd.random() < 0.7 else rnd.getrandbits(32)
        ctx[2] = f8 & ~((1 << 15) | (1 << 17))           # +8: no impact / damage-out rising edge (the ragdoll's, not the riding states')
        ctx[3] = rnd.getrandbits(32)                     # +12
        ctx[12] = rnd.getrandbits(32) & 0x0001FFFF       # +48 IsUpdateCulled 0, +49 IsOpponentDefined 0 / 1
        ctx[31] = rnd.choice((0, 0, 1, 0xFFFFFFFF, 2))   # +124 shift
        tail = [f2w(rnd.choice((fl(), -1.0, 0.0, 0.033, 0.25))), f2w(rnd.choice((fl(), -1.0, 0.0, 0.25, 0.033))), rnd.choice((0, 0, 0, 1))]
        # (a kept word that reads like an address of the original is drawn again: the registry check scans tracked files)
        if any(f'{(ctx + tail)[k]:08x}'.startswith('82') for k in USED + [43, 44, 45]):
            continue
        cases.append(ctx + tail)
    cf = work / 'ride_cond_cases.txt'
    cf.write_text(''.join(' '.join(f'{w:08x}' for w in c) + '\n' for c in cases))
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-800:] or f'exit {r.returncode}')
    out = r.stdout.splitlines()
    assert len(out) == len(cases), (len(out), len(cases))
    (work / 'ride_cond_ref.txt').write_text(''.join(' '.join(f'{c[i]:08x}' for i in USED + [43, 44, 45]) + f' -> {o}\n' for c, o in zip(cases, out)))
    print(len(out), 'cases', len(out[0]) if out else 0, 'transitions')


if __name__ == '__main__':
    main()
