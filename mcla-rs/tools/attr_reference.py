"""The tune attributes' readers, run offline: the generated code unchanged (the C library's atof / strtol inside) on flat
guest memory. An attribute is the original's node {+4 the value, +8 its type (0 the text, 1 int, 2 float, 3 bool), +9
flags}; every case here is a text attribute. Each case line of <out-dir>/attr_ref.txt is "<tag> <text as hex bytes> ->
<output words>". Sections:

  float  F-92206's conversion F-79880 (a text: float(atof(text))) -> the single's bits.
  int    F-79822 (a text: "0x" + hex by the base-16 conversion, else strtol base 10; the node made an int) -> the int,
         the node's type after.
  bool   F-83747 (a text: false when its first character is 'f', 'F' or '0') -> 0 / 1.

Texts: every attribute value of the disc's XML files (--values, one a line), and made-up numbers (decimal, exponents,
signs, leading blanks, trailing text, hex, overflow). The locale is the image's own (C); only the CRT's error report
F-37557 (a null text, not reached) and the locale getter F-52205 are stubs.

Run: PYTHONPATH=mcla-rs/tools python mcla-rs/tools/attr_reference.py --recomp D:/MCLA-static --image <original-image.bin>
--out-dir <dir> --tool <mcla-tool.exe> [--values <file>] (under the memcap job of recomp_closure)."""
import argparse
import random
import subprocess
from pathlib import Path
from recomp_closure import index, build, run
import recomp_closure
from cite_safe import cite_safe

_shim = recomp_closure.shim
recomp_closure.shim = lambda scratch_end=0x3000: _shim(scratch_end).replace('int32_t s32;', 'int32_t s32; int16_t s16; int8_t s8;', 1).replace('Condition cr0,cr6;', 'Condition cr0,cr1,cr6,cr7;', 1)
recomp_closure.EXTRA += ('static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}' + chr(10))
NL = chr(92) + 'n'
nl = chr(10)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def texts(rnd, disc):
    out = list(disc)
    for _ in range(3000):
        r = rnd.random()
        if r < 0.35:
            s = '%.*f' % (rnd.randint(0, 9), rnd.uniform(-5000, 5000))
        elif r < 0.5:
            s = repr(rnd.uniform(-1, 1))
        elif r < 0.6:
            s = '%de%d' % (rnd.randint(-99999, 99999), rnd.randint(-45, 40))
        elif r < 0.7:
            s = '%.*e' % (rnd.randint(0, 20), rnd.uniform(-1e30, 1e30))
        elif r < 0.8:
            s = str(rnd.randint(-(1 << 33), 1 << 33))
        elif r < 0.87:
            s = '0x%x' % rnd.getrandbits(rnd.choice((8, 16, 32, 36)))
        else:
            s = ''.join(rnd.choice('0123456789.-+eExX fF \t1a') for _ in range(rnd.randint(0, 12)))
        if rnd.random() < 0.1:
            s = rnd.choice((' ', '  ', '\t')) + s
        if rnd.random() < 0.1:
            s += rnd.choice(('f', ' ', 'abc', '.', 'e', '%'))
        out.append(s)
    # (random decimal strings near float rounding ties: 9 - 17 significant digits)
    for _ in range(1500):
        d = rnd.randint(9, 17)
        m = rnd.randint(10 ** (d - 1), 10 ** d - 1)
        e = rnd.randint(-d - 8, 8 - d)
        out.append('%de%d' % (m, e) if rnd.random() < 0.5 else ('%.*f' % (max(0, -e), m * 10.0 ** e) if e < 0 else str(m * 10 ** e)))
    return [s for s in out if len(s.encode('latin-1')) < 0x300]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--values', type=Path)
    a = ap.parse_args()
    f = {k: addr(a.tool, v) for k, v in dict(flt='F-79880', int='F-79822', bool='F-83747', err='F-37557', loc='F-52205').items()}
    rnd = random.Random(79822)
    disc = a.values.read_text(encoding='latin-1').split(nl) if a.values else []
    ts = texts(rnd, disc)
    secs = [('float', 'sub_%s(ctx,nullptr);printf("%%08x",fb(float(ctx.f1.f64)));' % f['flt']),
            ('int', 'sub_%s(ctx,nullptr);printf("%%08x %%02x",load(0x1000+4),uint32_t(*pointer(0x1008)));' % f['int']),
            ('bool', 'sub_%s(ctx,nullptr);printf("%%x",ctx.r3.u32&0xFF);' % f['bool'])]
    w = max(len(t.encode('latin-1')) for t in ts) + 1
    rows = nl.join('{%s},' % ','.join(str(b) for b in t.encode('latin-1') + bytes(w - len(t.encode('latin-1')))) for t in ts)
    body = 'static const uint8_t T[][%d]={%s%s%s};' % (w, nl, rows, nl)
    for tag, call in secs:
        body += ('for(auto&t:T){memset(objects,0,sizeof(objects));Context ctx{};ctx.r1.u32=0x2F00;memcpy(pointer(0x1800),t,%d);' % w +
                 'store(0x1000+4,0x1800);ctx.r3.u32=0x1000;try{' + call + '}catch(std::exception&){printf("trap");}printf("%s");fflush(stdout);}' % NL + nl)
    stubs = {'sub_' + f['err']: '', 'sub_' + f['loc']: 'ctx.r3.u64=0x1F00;', 'sub_82130588': ''}
    main_src = ('static uint32_t fb(float x){uint32_t w;memcpy(&w,&x,4);return w;}' + nl +
                'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    exe = build(index(a.recomp), sorted({'sub_' + f[k] for k in ('flt', 'int', 'bool')}), main_src, a.out_dir, 'attr', stubs=stubs)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == 3 * len(ts), (len(out), len(ts))
    lines = []
    for k, (tag, _) in enumerate(secs):
        for j, t in enumerate(ts):
            lines.append('%s %s -> %s' % (tag, t.encode('latin-1').hex() or '-', out[k * len(ts) + j].strip()))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'attr_ref.txt').write_text(cite_safe(nl.join(lines) + nl))
    print('texts', len(ts), 'disc', len(disc), 'traps', sum(1 for x in out if 'trap' in x))


if __name__ == '__main__':
    main()
