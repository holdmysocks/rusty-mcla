"""Transcribe straight-line scalar functions of the generated code (floating point, 16-byte vector copies, direct calls) to
Rust that does the same operation for operation: the registers f0..f31 as f64 holding single values, every operation
rounded to single as the generated code does (`double(float(...))`), the fused ones with one f64 `mul_add` then the
single rounding (the recompilation's fmadds family), the memory as `[u32; 16]` arrays (r3 -> `a`, r4 -> `b`) and a
stack frame `s`. Used for the 4 x 4 inverse (F-62799, F-25750, F-25548); nothing here is a decode, only a rewrite,
which the offline comparison then checks word for word.

    python recomp_scalar_to_rust.py --recomp D:/MCLA-static F-62799 F-25750 F-25548 > out.rs
"""
import argparse
import re
import subprocess
import sys
from pathlib import Path
from recomp_closure import index
from recomp_leaf import extract

STACK = 512     # bytes below the entry stack pointer the frame may use


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


class Sym:
    """A guest address as (base, offset): base 'a' (r3 on entry), 'b' (r4), 'sp' (r1), None = absolute."""

    def __init__(self, base, off):
        self.base, self.off = base, off

    def __add__(self, o):
        if isinstance(o, int):
            return Sym(self.base, self.off + o)
        assert self.base is None or o.base is None, 'two bases'
        return Sym(self.base or o.base, self.off + o.off)


def translate(src, name, rs_name, image, callees):
    gpr = {i: Sym(None, 0) for i in range(32)}
    gpr[1], gpr[3], gpr[4] = Sym('sp', 0), Sym('a', 0), Sym('b', 0)
    out = [f'pub fn {rs_name}(a: &mut [u32; 16], b: &mut [u32; 16]) -> f64 {{', '    let mut s = [0u32; %d];' % (STACK // 4), '    let mut f = [0f64; 32];']
    pend_w, pend_s, ea = None, None, None

    def mem(sym, extra=0):
        o = sym.off + extra
        if sym.base in ('a', 'b'):
            assert 0 <= o < 64 and o % 4 == 0, (sym.base, o)
            return f'{sym.base}[{o // 4}]'
        if sym.base == 'sp':
            assert -STACK <= o < 0 and o % 4 == 0, o
            return f's[{(o + STACK) // 4}]'
        raise ValueError('absolute store')

    def const_word(sym, extra=0):
        o = (sym.off + extra) & 0xFFFFFFFF
        p = o - 0x82000000
        return '0x%08xu32' % int.from_bytes(image[p:p + 4], 'big')

    def word_at(sym, extra=0):
        return const_word(sym, extra) if sym.base is None else mem(sym, extra)

    for raw in src.splitlines():
        l = raw.strip()
        if not l or l.startswith('//') or l.startswith('DEFINE_REX') or l in ('{', '}') or l.startswith(('REX_FUNC_PROLOGUE', 'PPCRegister', 'uint32_t ea', 'ctx.fpscr', 'ctx.lr', 'return', 'ctx.r12', '__savefpr', '__restfpr')):
            continue
        m = re.fullmatch(r'temp\.u32 = REX_LOAD_U32\(ctx\.r(\d+)\.u32 \+ (-?\d+)\);', l)
        if m:
            pend_w = word_at(gpr[int(m[1])], int(m[2]))
            continue
        m = re.fullmatch(r'ctx\.f(\d+)\.f64 = double\(temp\.f32\);', l)
        if m:
            out.append(f'    f[{m[1]}] = f32::from_bits({pend_w}) as f64;')
            continue
        m = re.fullmatch(r'temp\.f32 = float\(ctx\.f(\d+)\.f64\);', l)
        if m:
            pend_s = f'(f[{m[1]}] as f32).to_bits()'
            continue
        m = re.fullmatch(r'REX_STORE_U32\(ctx\.r(\d+)\.u32 \+ (-?\d+), temp\.u32\);', l)
        if m:
            out.append(f'    {mem(gpr[int(m[1])], int(m[2]))} = {pend_s};')
            continue
        if re.fullmatch(r'REX_STORE_U32\((ea|ctx\.r\d+\.u32 \+ -?\d+), ctx\.r\d+\.u32\);', l) or l.startswith('REX_LOAD_U32') or re.fullmatch(r'ctx\.r12\.u64 = REX_LOAD_U32.*', l):
            continue
        m = re.fullmatch(r'ctx\.f(\d+)\.f64 = double\(float\(ctx\.f(\d+)\.f64 ([*+/-]) ctx\.f(\d+)\.f64\)\);', l)
        if m:
            out.append(f'    f[{m[1]}] = (f[{m[2]}] {m[3]} f[{m[4]}]) as f32 as f64;')
            continue
        m = re.fullmatch(r'ctx\.f(\d+)\.f64 = double\(float\(std::fma\(ctx\.f(\d+)\.f64, ctx\.f(\d+)\.f64, (-?)ctx\.f(\d+)\.f64\)\)\);', l)
        if m:
            out.append(f'    f[{m[1]}] = f[{m[2]}].mul_add(f[{m[3]}], {m[4]}f[{m[5]}]) as f32 as f64;')
            continue
        m = re.fullmatch(r'ctx\.f(\d+)\.f64 = double\(float\(-std::fma\(ctx\.f(\d+)\.f64, ctx\.f(\d+)\.f64, (-?)ctx\.f(\d+)\.f64\)\)\);', l)
        if m:
            out.append(f'    f[{m[1]}] = (-f[{m[2]}].mul_add(f[{m[3]}], {m[4]}f[{m[5]}])) as f32 as f64;')
            continue
        m = re.fullmatch(r'ctx\.f(\d+)\.u64 = ctx\.f(\d+)\.u64 \^ 0x8000000000000000;', l)
        if m:
            out.append(f'    f[{m[1]}] = -f[{m[2]}];')
            continue
        # integer address arithmetic
        m = re.fullmatch(r'ctx\.r(\d+)\.[su]64 = ctx\.r(\d+)\.[su]64 \+ (-?\d+);', l)
        if m:
            gpr[int(m[1])] = gpr[int(m[2])] + int(m[3])
            continue
        m = re.fullmatch(r'ctx\.r(\d+)\.[su]64 = ctx\.r(\d+)\.[su]64;', l)
        if m:
            gpr[int(m[1])] = gpr[int(m[2])]
            continue
        m = re.fullmatch(r'ctx\.r(\d+)\.s64 = (-?\d+);', l)
        if m:
            gpr[int(m[1])] = Sym(None, int(m[2]))
            continue
        m = re.fullmatch(r'ea = (-?\d+) \+ ctx\.r(\d+)\.u32;', l)
        if m:
            ea = gpr[int(m[2])] + int(m[1])
            continue
        m = re.fullmatch(r'ctx\.r1\.u32 = ea;', l)
        if m:
            gpr[1] = ea
            continue
        m = re.fullmatch(r'ctx\.r1\.s64 = ctx\.r1\.s64 \+ (-?\d+);', l)
        if m:
            gpr[1] = gpr[1] + int(m[1])
            continue
        m = re.fullmatch(r'ea = \(ctx\.r(\d+)\.u32(?: \+ ctx\.r(\d+)\.u32)?\) & ~0xF;', l)
        if m:
            ea = gpr[int(m[1])] + (gpr[int(m[2])] if m[2] else 0)
            assert ea.off % 16 == 0 or ea.base is None, 'unaligned vector address'
            continue
        m = re.fullmatch(r'simde_mm_store_si128\(\(simde__m128i\*\)ctx\.v(\d+)\.u8, simde_mm_shuffle_epi8\(simde_mm_load_si128\(\(simde__m128i\*\)REX_RAW_ADDR\(ea\)\), .*', l)
        if m:
            words = ', '.join(word_at(ea, 4 * k) for k in range(4))
            out.append(f'    let v{m[1]} = [{words}];')
            continue
        m = re.fullmatch(r'simde_mm_store_si128\(\(simde__m128i\*\)REX_RAW_ADDR\(ea\), simde_mm_shuffle_epi8\(simde_mm_load_si128\(\(simde__m128i\*\)ctx\.v(\d+)\.u8\), .*', l)
        if m:
            for k in range(4):
                out.append(f'    {mem(ea, 4 * k)} = v{m[1]}[{k}];')
            continue
        m = re.fullmatch(r'(sub_[0-9A-F]{8})\(ctx, base\);', l)
        if m:
            callee = callees[m[1]]
            arg = gpr[3]
            assert arg.off == 0 and arg.base in ('a', 'b'), arg.__dict__
            out.append(f'    f[1] = {callee}({arg.base}, &mut [0u32; 16]);')
            continue
        raise ValueError('cannot transcribe: ' + l)
    out += ['    f[1]', '}']
    return '\n'.join(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--names', required=True, help='comma list FID=rust_name in call order (callees first)')
    a = ap.parse_args()
    idx = index(a.recomp)
    image = a.image.read_bytes()
    callees = {}
    for item in a.names.split(','):
        fid, rs = item.split('=')
        n = 'sub_' + addr(a.tool, fid)
        src = extract(idx[n], n)
        print(f'// {fid}')
        print(translate(src, n, rs, image, callees))
        print()
        callees[n] = rs


if __name__ == '__main__':
    main()
