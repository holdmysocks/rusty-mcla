"""The district lookup F-67263 runs offline: the generated code unchanged (with its compare F-40755) on a string written at
0x1000; the failure log is stubbed (it only prints). Cases: every district name and the short forms, in mixed case, the
mission files' district values, near misses and random strings. Writes <out-dir>/career_district_ref.txt:
"<hex of the string> <result>" per line."""
import argparse
import random
import re
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run
from recomp_leaf import extract

# (the compare F-40755 uses condition fields cr1 / cr5 / cr7, which the shared shim leaves out)
_shim = recomp_closure.shim
recomp_closure.shim = lambda *a: _shim(*a).replace('Condition cr0,cr6;', 'Condition cr0,cr1,cr5,cr6,cr7;', 1)


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def cases(missions: Path):
    rnd = random.Random(67263)
    names = ['Hollywood', 'Beaches', 'Hills', 'Downtown', 'SouthCentral', 'Unknown', 'All', 'SC', 'South Central']
    out = []
    for n in names:
        out += [n, n.lower(), n.upper(), n.swapcase(), n + ' ', ' ' + n, n[:-1], n + 's', '']
        for _ in range(6):
            out.append(''.join(c.upper() if rnd.random() < 0.5 else c.lower() for c in n))
    for p in sorted(missions.glob('*.xml')):
        for line in p.read_text(errors='replace').splitlines():
            if 'szDistrict' in line and '>' in line:
                out.append(line.split('>', 1)[1].split('<', 1)[0])
    al = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ @[`{_-0123456789'
    for _ in range(2000):
        out.append(''.join(rnd.choice(al) for _ in range(rnd.randrange(0, 16))))
    seen, res = set(), []
    for s in out:
        if s not in seen and len(s) < 60:
            seen.add(s)
            res.append(s)
    return res


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--missions', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    fl = addr(a.tool, 'F-67263')
    cs = cases(a.missions)
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('"%s",' % ''.join('\\x%02x' % b for b in s.encode()) for s in cs)
    body = ('static const char*C[]={' + nl + rows + nl + '};' + nl +
            'for(auto c:C){memset(objects,0,sizeof(objects));size_t n=strlen(c);for(size_t i=0;i<=n;i++)objects[i]=uint8_t(c[i]);'
            'Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);printf("%%d%s",int(ctx.r3.s32));}' % (fl, NL) + nl)
    main_src = ('int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});'
                + nl + body + '}' + nl)
    idx = index(a.recomp)
    # (every callee but the compare is the failure log: stubbed)
    callees = set(re.findall(r'\b(sub_[0-9A-F]{8})\(ctx, base\)', extract(idx['sub_' + fl], 'sub_' + fl)))
    cmp_fn = 'sub_' + addr(a.tool, 'F-40755')
    stubs = sorted(c for c in callees if c != cmp_fn)
    exe = build(idx, ['sub_' + fl], main_src, a.out_dir, 'district', stubs=stubs)
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cs)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'career_district_ref.txt').write_text(nl.join('%s %s' % (s.encode().hex() or '-', o) for s, o in zip(cs, out)) + nl)
    print(len(cs), 'cases; stubbed', stubs)


if __name__ == '__main__':
    main()
