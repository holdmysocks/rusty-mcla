# heapcensus.py <mcla.exe> <log> [--report N] [--frames K]: the `heapcensus:` lines of a run made with
# MCLA_HEAP_CENSUS=<s>[,<min KB>] (crates/game/src/heapcensus.rs) with each site's call stack as functions and lines.
# The offsets are from the exe's load address; addr2line (WinLibs mingw64) reads the release build's line tables
# (profile.release debug = 1). The allocator's own frames (alloc::, core::, hashbrown::, std::) are left out.
# Default: the last report; --report N the N-th (0 = the first); also the change of each site since the first report.
import re, struct, subprocess, sys

SKIP = ('alloc::', 'core::', 'hashbrown::', 'std::', '__rust', 'mcla::heapcensus', '<mcla::heapcensus', 'rust_alloc', '__rdl_', '__rg_')


def image_base(exe):
    with open(exe, 'rb') as f:
        h = f.read(4096)
    pe = struct.unpack_from('<I', h, 0x3c)[0]
    magic = struct.unpack_from('<H', h, pe + 24)[0]
    return struct.unpack_from('<Q', h, pe + 24 + 24)[0] if magic == 0x20b else struct.unpack_from('<I', h, pe + 24 + 28)[0]


def main():
    exe, log = sys.argv[1], sys.argv[2]
    k = int(sys.argv[sys.argv.index('--frames') + 1]) if '--frames' in sys.argv else 6
    reports, cur = [], None
    for l in open(log, encoding='utf-8', errors='replace'):
        m = re.search(r'heapcensus: t (\d+) s: ([\d.]+) MB live', l)
        if m:
            cur = {'t': int(m.group(1)), 'live': float(m.group(2)), 'head': l.split('heapcensus: ', 1)[1].strip(), 'sites': []}
            reports.append(cur)
            continue
        m = re.search(r'heapcensus: #\d+ site (\w+): ([\d.]+) MB live in (\d+) blocks \((\d+) made\) at ([\w,]*)', l)
        if m and cur is not None:
            cur['sites'].append((m.group(1), float(m.group(2)), int(m.group(3)), int(m.group(4)), [int(x, 16) for x in m.group(5).split(',') if x]))
    if not reports:
        sys.exit('no heapcensus lines in the log')
    r = reports[int(sys.argv[sys.argv.index('--report') + 1]) if '--report' in sys.argv else -1]
    first = {s[0]: s[1] for s in reports[0]['sites']}
    base = image_base(exe)
    offs = sorted({o for s in r['sites'] for o in s[4]})
    # (a return address points after the call: one byte back is inside it)
    out = subprocess.run(['addr2line', '-f', '-C', '-e', exe] + [hex(base + o - 1) for o in offs], capture_output=True, text=True).stdout.splitlines()
    names = {o: (out[2 * i].strip(), out[2 * i + 1].strip().replace('\\', '/').split('/src/')[-1]) for i, o in enumerate(offs) if 2 * i + 1 < len(out)}
    print(f'{r["head"]}  (first report t {reports[0]["t"]} s: {reports[0]["live"]:.1f} MB)')
    for h, mb, b, made, fr in r['sites']:
        print(f'\n{mb:8.1f} MB in {b} blocks ({made} made), {mb - first.get(h, 0):+.1f} MB since the first report  [{h}]')
        shown = 0
        for o in fr:
            fn, line = names.get(o, ('?', '?'))
            if fn.startswith(SKIP) or fn == '??':
                continue
            print(f'    {fn[:110]}  {line}')
            shown += 1
            if shown >= k:
                break


if __name__ == '__main__':
    main()
