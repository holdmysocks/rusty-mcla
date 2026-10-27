"""The particle effects' evolution blend (dispatcher F-31783 and its four modes F-18828 / F-76095 / F-81297 / F-39561) run offline on their
generated code. Cases build the original's object tables in guest memory (group +0 evolutions, +4 count, +8 modes, +16 mask bits; evolution +4
per-list table, +8 list count; entry: signed slot bytes, +40 -> keyframe pointers; keyframe +24 -> 48-byte keys, +28 count), call the dispatcher
(r3 group, r4 list, r5 -> the 16-byte value in/out, r6 property, f1 time, r8 -> the weights) and record the value.
Output line:  EV len hexmem ptroffsets groupoff list prop t w0..w7 b0..b3 -> o0..o3     (the memory starts at guest 0x1000; the pointer
words are listed by their offset so a test can place the same tables behind its own pointers; floats as hex bits)
Usage: ptfx_evolve_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR --out FILE [--tool mcla-tool]"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_dispatch as rd

DISPATCH = 'F-31783'
BASE = 0x1000
OUTV, WGT = 0x2800, 0x2820

MAIN = r'''
static float fl(uint32_t w){float f;memcpy(&f,&w,4);return f;}
static uint32_t bi(float f){uint32_t w;memcpy(&w,&f,4);return w;}
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 int ncase=0;std::ifstream in(argv[2]);std::string hx;uint32_t len,g,list,prop,t,w[8],b[4];
 while(in>>std::dec>>len>>hx>>std::hex>>g>>list>>prop>>t){for(auto&x:w)in>>x;for(auto&x:b)in>>x;in>>std::dec;rt_reset();
  for(uint32_t i=0;i<len;i++){unsigned v;sscanf(hx.c_str()+2*i,"%2x",&v);*pointer(0x1000+i)=uint8_t(v);}
  for(int i=0;i<4;i++)store(OUTV_+4*i,b[i]);for(int i=0;i<8;i++)store(WGT_+4*i,w[i]);
  Context c=fresh();c.r3.u32=0x1000+g;c.r4.u32=list;c.r5.u32=OUTV_;c.r6.u32=prop;c.r8.u32=WGT_;c.f1.f64=double(fl(t));
  try{sub_DISP(c,nullptr);}catch(std::exception&e){fprintf(stderr,"case %d: %s\n",ncase,e.what());return 1;}ncase++;
  printf("%08x %08x %08x %08x\n",load(OUTV_),load(OUTV_+4),load(OUTV_+8),load(OUTV_+12));
 }
}
'''


def fid_addr(tool, fid):
    return 'sub_' + subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]


def b(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


class Mem:
    def __init__(self):
        self.d = bytearray(0x1800)
        self.top = 0
        self.top = 0
        self.ptrs = []

    def alloc(self, n, align=16):
        self.top = (self.top + align - 1) & ~(align - 1)
        a = self.top
        self.top += n
        assert self.top < len(self.d)
        return a

    def u32(self, off, v):
        struct.pack_into('>I', self.d, off, v & 0xFFFFFFFF)

    def ptr(self, off, target):
        self.u32(off, BASE + target)
        self.ptrs.append(off)

    def u16(self, off, v):
        struct.pack_into('>H', self.d, off, v)


def make_case(r, mode_pool):
    m = Mem()
    g = m.alloc(48)
    n_e = r.randint(1, 4)
    props = sorted(set((r.randint(0, 3), r.randint(0, 39)) for _ in range(r.randint(1, 3))))
    modes = {p: r.choice(mode_pool) for p in props}
    n_lists = 4
    # modes table
    mt = m.alloc(8 * n_lists)
    arrs = []
    for li in range(n_lists):
        a = m.alloc(40)
        arrs.append(a)
        m.ptr(mt + 8 * li, a)
    for (li, pr), md in modes.items():
        m.d[arrs[li] + pr] = md
    ev_arr = m.alloc(4 * n_e)
    m.ptr(g, ev_arr)
    m.u16(g + 4, n_e)
    m.ptr(g + 8, mt)
    for (li, pr) in props:
        idx = li * 40 + pr
        w = m.d[g + 16 + (idx >> 5) * 4: g + 16 + (idx >> 5) * 4 + 4]
        v = struct.unpack('>I', bytes(w))[0] | (1 << (idx & 31))
        m.u32(g + 16 + (idx >> 5) * 4, v)
    for ei in range(n_e):
        e = m.alloc(16)
        m.ptr(ev_arr + 4 * ei, e)
        lc = 4 if any(v != 1 for v in modes.values()) else r.randint(1, 4)       # (modes 0, 2 and 3 read every evolution's entry without a test: the data always has them; only mode 1 checks)
        m.u16(e + 8, lc)
        tbl = m.alloc(4 * lc)
        m.ptr(e + 4, tbl)
        for li in range(lc):
            mine = [pr for (l2, pr) in props if l2 == li and r.random() < 0.8]
            if not mine and r.random() < 0.5 and all(v == 1 for v in modes.values()):
                continue                                           # a null entry
            ent = m.alloc(48)
            m.ptr(tbl + 4 * li, ent)
            for k in range(40):
                m.d[ent + k] = 0xFF
            arr = m.alloc(4 * max(len(mine), 1))
            m.ptr(ent + 40, arr)
            for si, pr in enumerate(mine):
                m.d[ent + pr] = si
                kf = m.alloc(32)
                m.ptr(arr + 4 * si, kf)
                nk = r.randint(1, 4)
                keys = m.alloc(48 * nk)
                m.ptr(kf + 24, keys)
                m.u16(kf + 28, nk)
                t = 0.0
                for ki in range(nk):
                    k = keys + 48 * ki
                    m.u32(k, b(t))
                    t += r.uniform(0.05, 0.6)
                    for c in range(4):
                        m.u32(k + 16 + 4 * c, b(r.choice([r.uniform(-2, 2), r.uniform(0, 1), r.uniform(-50, 50)])))
                        m.u32(k + 32 + 4 * c, b(r.choice([r.uniform(-3, 3), 0.0, r.uniform(-0.5, 0.5)])))
    return m, g, props, n_e


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=700)
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    fn = fid_addr(a.tool, DISPATCH)
    exe = rd.build(idx, [fn], MAIN.replace('sub_DISP', fn).replace('OUTV_', hex(OUTV)).replace('WGT_', hex(WGT)), a.work, 'ptfx_evolve', image=img)
    r = random.Random(20261027)
    rows, outs_in = [], []
    for ci in range(a.cases):
        m, g, props, n_e = make_case(r, [4] if ci % 25 == 0 else [0, 1, 2, 3])
        used = m.top
        # a query: a chosen property most of the time, an unmarked one sometimes
        li, pr = r.choice(props) if r.random() < 0.85 else (r.randint(0, 3), r.randint(0, 39))
        t = r.choice([0.0, r.uniform(0, 1), r.uniform(0, 3), r.uniform(-0.2, 0.1)])
        w = [r.choice([0.0, -0.3, 1.0, r.uniform(0, 1), r.uniform(0, 1), r.uniform(0, 2)]) for _ in range(8)]
        base = [r.choice([r.uniform(-2, 2), r.uniform(0, 1)]) for _ in range(4)]
        hexmem = bytes(m.d[:used]).hex()
        head = f'{used} {hexmem} {g:x} {li:x} {pr:x} {b(t):x} ' + ' '.join(f'{b(x):x}' for x in w) + ' ' + ' '.join(f'{b(x):x}' for x in base)
        rows.append((head, m.ptrs, g, li, pr, t, w, base, used, hexmem))
    cases = a.work / 'ptfx_evolve_cases.txt'
    cases.write_text('\n'.join(x[0] for x in rows) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cases)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    outs = p.stdout.splitlines()
    assert len(outs) == len(rows)
    lines = []
    for (head, ptrs, g, li, pr, t, w, base, used, hexmem), o in zip(rows, outs):
        lines.append(f'EV {used} {hexmem} {",".join(str(x) for x in ptrs)} {g} {li} {pr} {b(t):08x} ' + ' '.join(f'{b(x):08x}' for x in w) + ' ' + ' '.join(f'{b(x):08x}' for x in base) + ' -> ' + o)
    a.out.write_text('\n'.join(lines) + '\n')
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
