"""The effect colour tint F-92352 run offline on its generated code (with the evolution blend F-31783 it calls). Each case is an effect
rule in guest memory: its KFColorTint key-frame at +0x08 (keys pointer +0x20, count +0x24: 48-byte keys {time, value +16, slope +32}),
its evolution group pointer at +0x100 (or null) built as tools/ptfx_evolve_reference.py builds them (with list 0 property 0 marked in
most cases), byte +0x10d = 0 (no rule on the disc sets it); no instance (r4 = 0, as no instance sets the override). The call: r3 rule,
r4 0, r5 -> the 16-byte result, r7 -> the evolution weights, f1 the effect's loop time.
Output line:  TI len hexmem ptroffsets ruleoff t w0..w7 -> o0..o3     (memory from guest 0x1000; floats as hex bits)
Usage: ptfx_tint_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_dispatch as rd
import ptfx_evolve_reference as ev

TINT = 'F-92352'
OUTV, WGT = 0x2800, 0x2820

MAIN = r'''
static float fl(uint32_t w){float f;memcpy(&f,&w,4);return f;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 int ncase=0;std::ifstream in(argv[2]);std::string hx;uint32_t len,ro,t,w[8];
 while(in>>std::dec>>len>>hx>>std::hex>>ro>>t){for(auto&x:w)in>>x;in>>std::dec;rt_reset();
  for(uint32_t i=0;i<len;i++){unsigned v;sscanf(hx.c_str()+2*i,"%2x",&v);*pointer(0x1000+i)=uint8_t(v);}
  for(int i=0;i<4;i++)store(OUTV_+4*i,0xDEADBEEF);for(int i=0;i<8;i++)store(WGT_+4*i,w[i]);
  Context c=fresh();c.r3.u32=0x1000+ro;c.r4.u32=0;c.r5.u32=OUTV_;c.r7.u32=WGT_;c.f1.f64=double(fl(t));
  try{sub_TINT(c,nullptr);}catch(std::exception&e){fprintf(stderr,"case %d: %s\n",ncase,e.what());return 1;}ncase++;
  printf("%08x %08x %08x %08x\n",load(OUTV_),load(OUTV_+4),load(OUTV_+8),load(OUTV_+12));
 }
}
'''


def keys(m, r, nk):
    k0 = m.alloc(48 * nk)
    t = r.choice([0.0, 0.0, -0.1])
    for ki in range(nk):
        k = k0 + 48 * ki
        m.u32(k, ev.b(t))
        t += r.choice([r.uniform(0.05, 0.6), 0.0]) if ki else r.uniform(0.05, 0.6)   # (equal times now and then)
        for c in range(4):
            m.u32(k + 16 + 4 * c, ev.b(r.choice([1.0, r.uniform(0, 1), r.uniform(-2, 2)])))
            m.u32(k + 32 + 4 * c, ev.b(r.choice([0.0, r.uniform(-3, 3), r.uniform(-0.5, 0.5)])))
    return k0


def make_case(r):
    """An evolution group (list 0 property 0 marked in most cases) and a rule pointing at it."""
    m, g, props, n_e = ev.make_case(r, [4] if r.random() < 0.04 else [0, 1, 2, 3], force=[(0, 0)] if r.random() < 0.8 else [])
    rule = m.alloc(0x120)
    nk = r.randint(1, 5)
    m.ptr(rule + 0x20, keys(m, r, nk))
    m.u16(rule + 0x24, nk)
    if r.random() < 0.85:
        m.ptr(rule + 0x100, g)
    return m, rule


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=400)
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    fn = ev.fid_addr(a.tool, TINT)
    exe = rd.build(idx, [fn], MAIN.replace('sub_TINT', fn).replace('OUTV_', hex(OUTV)).replace('WGT_', hex(WGT)), a.work, 'ptfx_tint', image=img)
    r = random.Random(20261107)
    rows = []
    for _ in range(a.cases):
        m, rule = make_case(r)
        used = m.top
        t = r.choice([0.0, 1.0, r.uniform(0, 1), r.uniform(0, 1), r.uniform(-0.2, 1.5)])
        w = [r.choice([0.0, 0.0, 1.0, r.uniform(0, 1), r.uniform(0, 2), -0.3]) for _ in range(8)]
        hexmem = bytes(m.d[:used]).hex()
        rows.append((f'{used} {hexmem} {rule:x} {ev.b(t):x} ' + ' '.join(f'{ev.b(x):x}' for x in w), m.ptrs, used, hexmem, rule, t, w))
    cases = a.work / 'ptfx_tint_cases.txt'
    cases.write_text('\n'.join(x[0] for x in rows) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    outs = p.stdout.splitlines()
    assert len(outs) == len(rows)
    lines = [f'TI {used} {hexmem} {",".join(str(x) for x in ptrs)} {rule} {ev.b(t):08x} ' + ' '.join(f'{ev.b(x):08x}' for x in w) + ' -> ' + o
             for (_, ptrs, used, hexmem, rule, t, w), o in zip(rows, outs)]
    a.out.write_text('\n'.join(lines) + '\n')
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
