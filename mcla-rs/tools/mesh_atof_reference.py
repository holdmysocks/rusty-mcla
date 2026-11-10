"""The text tokenizer's number read run offline (the generated code as it is), for the port's test
(crates/game/src/testdata/mesh_atof_ref.txt).

GetFloat F-18899 takes a token of at most 32 characters and, when it starts with '-', '.' or a digit, calls the CRT's atof
F-53123 and rounds the double to single (frsp). atof (its body F-51157) skips leading blanks (none in a token) and hands
the string, its length and the locale (the global the body loads with lis / addi r30: the static C locale in the image) to
the converter F-52508 (the 12-byte long double parse F-44494, then its rounding to double F-60211); the double comes back
at +16 of its result. This tool calls the converter directly with that locale. Stubs: the kernel imports (TLS, locks,
memory; reached only on error paths) and the errno / invalid-parameter helpers (F-52205 answers a scratch word, F-37557
does nothing).

Inputs: every distinct number token of the shipped text meshes (vehicle/0xd8bee6cc/* text meshes, fx/ctf_flag,
tune/garage texcoord6), split as the tokenizer does at blanks, '/', '{' and '}'.

  <token, '_' after every 3 characters> <flags> <chars used> <double bits, decimal> <float bits, decimal>

python mesh_atof_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --cache extracted/cache --out <build dir> --ref <file> --tool <mcla-tool>
"""
import argparse
import re
import subprocess
from pathlib import Path
import recomp_closure
import recomp_leaf
from recomp_closure import index, build, run

recomp_closure.shim = lambda *_: recomp_leaf.shim(0x10000).replace('uint16_t u16; uint8_t u8;', 'uint16_t u16; int16_t s16; uint8_t u8; int8_t s8;').replace('Condition cr0,cr6;', 'Condition cr0,cr1,cr2,cr3,cr4,cr5,cr6,cr7;')



def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def locale_global(idx, body):
    """The address atof's body builds with `lis rX,hi ; addi r30,rX,lo` (its asm comments in the generated code)."""
    src = recomp_closure.extract(idx[body], body)
    m = re.search(r'// lis (r\d+),(-?\d+)\n(?:.*\n){0,3}?\s*// addi r30,\1,(-?\d+)', src)
    assert m, 'locale load not found'
    return ((int(m.group(2)) << 16) + int(m.group(3))) & 0xFFFFFFFF


def mesh_files(cache: Path):
    out = []
    for p in sorted((cache / 'vehicle' / '0xd8bee6cc').rglob('*')):
        if p.is_file() and p.read_bytes()[:12].startswith(b'Version 1'):
            out.append(p)
    out += [cache / 'fx/ctf_flag/ctf_flag.mesh', cache / 'tune/garage/texcoord6.mesh']
    return [p for p in out if p.is_file()]


def tokens(files):
    toks = set()
    for f in files:
        for w in re.split(r'[\s/{}]+', f.read_text(encoding='latin-1')):
            if w and (w[0] in '-.' or w[0].isdigit()):
                toks.add(w)
    return sorted(toks)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--cache', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    A = lambda f: 'sub_' + addr(a.tool, f)
    conv = A('F-52508')
    files = mesh_files(a.cache)
    toks = tokens(files)
    assert all(len(t) < 32 and re.fullmatch(r'[-+.0-9eE]+', t) for t in toks), [t for t in toks if len(t) >= 32]
    idx = index(a.recomp)
    locale = locale_global(idx, A('F-51157'))
    stubs = {A('F-52205'): 'ctx.r3.u64=0x2800;', A('F-37557'): ''}
    while True:
        try:
            recomp_closure.closure(idx, [conv], stubs=stubs, limit=300)
            break
        except KeyError as e:
            stubs[e.args[0]] = 'ctx.r3.u64=0;'
    nl, NL = chr(10), chr(92) + 'n'
    body = ('static const char*T[]={' + ','.join('"%s"' % t for t in toks) + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(size_t n=0;n<sizeof(T)/sizeof(T[0]);n++){memset(objects,0,sizeof(objects));size_t L=strlen(T[n]);'
            'for(size_t i=0;i<=L;i++)*pointer(0x2000+uint32_t(i))=uint8_t(T[n][i]);'
            'Context ctx{};ctx.r1.u32=0xF000;ctx.r3.u32=0x3000;ctx.r4.u32=0x2000;ctx.r5.u64=L;ctx.r8.u32=%uu;' % locale +
            conv + '(ctx,nullptr);uint64_t d=load64(0x3010);double dv;memcpy(&dv,&d,8);float fv=float(dv);uint32_t fb;memcpy(&fb,&fv,4);'
            'printf("%s %u %u %llu %u' + NL + '",T[n],load(0x3000),load(0x3004),(unsigned long long)d,fb);}}' + nl)
    out = run(build(idx, [conv], body, a.out, 'mesh_atof', stubs=stubs), a.image)
    assert out.count(nl) == len(toks), out[:600]
    a.ref.parent.mkdir(parents=True, exist_ok=True)
    # (the token written with '_' after every three characters: a mesh number's eight-digit fraction would read as an
    # address to `registry check`; the test removes the '_')
    lines = []
    for l in out.splitlines():
        t, rest = l.split(' ', 1)
        lines.append('_'.join(t[i:i + 3] for i in range(0, len(t), 3)) + ' ' + rest)
    a.ref.write_text(chr(10).join(lines) + chr(10))
    print(len(files), 'files', len(toks), 'tokens')


if __name__ == '__main__':
    main()
