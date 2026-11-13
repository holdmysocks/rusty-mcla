# perfgate.py <runs dir> <tag>...: the table of tools/perfgate.ps1's runs (docs/PERFORMANCE.md "Perf gate").
# A tag is b<k>-<scenario>-<backend>-r<n>; build k's runs of a scenario are averaged. Measured from T0 s on (the load and
# the first streaming are left out). fps = frames / time, the 1 % low = 1000 / the 99th percentile frame, the worst tick =
# the largest sim time of a frame (all ticks run in it), phases = the means of the frame's phases (MCLA_FRAMETIMES),
# GPU = the mean of the `gputimes:` lines after the first two, VRAM / private = the largest sample.
# With two builds: the change of each number, and REGRESSION where fps or the 1 % low fell by more than 5 %.
# A run on a busy machine (all processes over BUSY % CPU on average) is left out and named.
import os, re, statistics as st, sys
from collections import defaultdict

T0 = 40.0
# A run whose machine (all processes) averaged more CPU than this is left out: another program's load, not the build's.
BUSY = 35.0


def pct(v, q):
    v = sorted(v)
    return v[min(len(v) - 1, int(q * len(v)))]


def one(d, tag):
    rows = []
    for line in open(os.path.join(d, tag + '.ft.csv')):
        if line[:1].isdigit():
            r = [float(x) for x in line.split(',')]
            if r[0] >= T0:
                rows.append(r)
    o = {}
    if rows:
        ms = [r[1] for r in rows]
        o.update(fps=1000 * len(ms) / sum(ms), low1=1000 / pct(ms, 0.99), worst=max(ms), tick=max(r[3] for r in rows),
                 sim=st.mean(r[3] for r in rows), prep=st.mean(r[5] for r in rows), ui=st.mean(r[7] for r in rows),
                 passes=st.mean(r[8] for r in rows), submit=st.mean(r[9] for r in rows), draws=st.mean(r[10] for r in rows))
    res = [l.strip().split(',') for l in open(os.path.join(d, tag + '.res.csv'))][1:]
    res = [[float(x) for x in r] for r in res if len(r) == 7 and float(r[0]) >= T0]
    if res and st.mean(r[6] for r in res) > BUSY:
        raise ValueError(f'machine busy: {st.mean(r[6] for r in res):.0f} % CPU on average, left out')
    if res:
        o.update(cpu=st.mean(r[1] for r in res), gpu3d=st.mean(r[2] for r in res), vram=max(r[3] for r in res), priv=max(r[4] for r in res))
    err = open(os.path.join(d, tag + '.err'), encoding='utf-8', errors='replace').read().splitlines()
    g = [l.split('gputimes: ', 1)[1] for l in err if 'gputimes: ' in l][2:]
    if g:
        sums = defaultdict(float)
        for body in g:
            parts, tot = body.rsplit(' = ', 1)
            sums['gpu'] += float(tot.split()[0])
            for p in parts.split(', '):
                n, v = p.rsplit(' ', 1)
                sums['gpu ' + n] += float(v)
        o.update({k: v / len(g) for k, v in sums.items()})
    tt = [l for l in err if 'ticktimes:' in l]
    if tt:
        o['ticktimes'] = tt[-1].split('): ', 1)[-1]
    return o


def main():
    d, tags = sys.argv[1], sys.argv[2:]
    by = defaultdict(list)
    for t in tags:
        m = re.match(r'b(\d+)-(\w+)-(\w+)-r\d+$', t)
        try:
            by[(m.group(2), int(m.group(1)))].append(one(d, t))
        except (OSError, AttributeError, ValueError, ZeroDivisionError) as e:
            print(f'{t}: no result ({e})')
    cols = ['fps', 'low1', 'worst', 'tick', 'sim', 'prep', 'ui', 'passes', 'submit', 'draws', 'gpu', 'cpu', 'gpu3d', 'vram', 'priv']
    print('scenario build ' + ' '.join(f'{c:>7s}' for c in cols))
    scens = sorted({s for s, _ in by}, key=lambda s: ['roam', 'night', 'race', 'garage', 'hangout', 'old'].index(s) if s in ['roam', 'night', 'race', 'garage', 'hangout', 'old'] else 9)
    bad = []
    for s in scens:
        ks = sorted(k for ss, k in by if ss == s)
        mean = {}
        for k in ks:
            runs = by[(s, k)]
            mean[k] = {c: st.mean(r[c] for r in runs if c in r) for c in set().union(*runs) if c != 'ticktimes' and any(c in r for r in runs)}
            print(f'{s:8s} b{k}    ' + ' '.join(f'{mean[k].get(c, float("nan")):7.2f}' for c in cols))
        if len(ks) == 2:
            a, b = mean[ks[0]], mean[ks[1]]
            print(f'{s:8s} b2-b1 ' + ' '.join(f'{b.get(c, 0) - a.get(c, 0):+7.2f}' for c in cols))
            for c in ('fps', 'low1'):
                if c in a and c in b and b[c] < a[c] * 0.95:
                    bad.append(f'{s}: {c} {a[c]:.1f} -> {b[c]:.1f} ({(b[c] / a[c] - 1) * 100:+.1f} %)')
            gp = sorted({c for c in set(a) | set(b) if c.startswith('gpu ')}, key=lambda c: -max(a.get(c, 0), b.get(c, 0)))[:6]
            print('         GPU passes: ' + ', '.join(f'{c[4:]} {a.get(c, 0):.2f} -> {b.get(c, 0):.2f}' for c in gp))
        else:
            gp = sorted((c for c in mean[ks[0]] if c.startswith('gpu ')), key=lambda c: -mean[ks[0]][c])[:6]
            print('         GPU passes: ' + ', '.join(f'{c[4:]} {mean[ks[0]][c]:.2f}' for c in gp))
        for k in ks:
            t = [r['ticktimes'] for r in by[(s, k)] if 'ticktimes' in r]
            if t:
                print(f'         b{k} ticks: ' + ', '.join(t[-1].split(', ')[:5]))
    print('REGRESSION: ' + '; '.join(bad) if bad else 'no regression over 5 % in fps or the 1 % low')


if __name__ == '__main__':
    main()
