# soak.py <runs dir> <tag> [--minutes 30]: PASS / FAIL of a long free-roam run made by tools/perfgate.ps1
# (docs/PERFORMANCE.md "Release gate"). It reads the run's <tag>.ft.csv (MCLA_FRAMETIMES), <tag>.res.csv (samples every 2 s)
# and <tag>.err (the log), and checks:
#   - the game ran to the end (samples up to the last minute) and logged no panic / ERROR line;
#   - private memory and dedicated VRAM: the rise from minute 5 to the end (the mean of the first and the last two minutes)
#     at most PRIV_MB / VRAM_MB, and the slope over minutes 5 - end at most SLOPE_MB a minute (a leak grows steadily);
#   - slow ticks (`slowtick:` lines, MCLA_SLOWTICK=30) after the first minute: at most SLOW_N, none over SLOW_MAX ms;
#   - frame rate: the last five minutes within FPS_DROP of minutes 5 - 10 (a session that slows down as it goes on).
import os, re, statistics as st, sys

PRIV_MB, VRAM_MB, SLOPE_MB = 300.0, 200.0, 6.0
SLOW_N, SLOW_MAX = 10, 150.0
FPS_DROP = 0.10


def slope(xs, ys):
    mx, my = st.mean(xs), st.mean(ys)
    d = sum((x - mx) ** 2 for x in xs)
    return sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / d if d else 0.0


def main():
    d, tag = sys.argv[1], sys.argv[2]
    minutes = float(sys.argv[sys.argv.index('--minutes') + 1]) if '--minutes' in sys.argv else 30.0
    end = minutes * 60
    res = [l.strip().split(',') for l in open(os.path.join(d, tag + '.res.csv'))][1:]
    res = [[float(x) for x in r] for r in res if len(r) >= 7]
    err = open(os.path.join(d, tag + '.err'), encoding='utf-8', errors='replace').read().splitlines()
    ft = [[float(x) for x in l.split(',')] for l in open(os.path.join(d, tag + '.ft.csv')) if l[:1].isdigit()]
    fails, rows = [], []

    last = res[-1][0] if res else 0
    rows.append(f'ran {last / 60:.1f} min of {minutes:g}')
    if last < end - 90:
        fails.append(f'the game stopped at {last / 60:.1f} min')
    bad = [l for l in err if 'panicked' in l or ' ERROR ' in l]
    rows.append(f'panic / ERROR lines: {len(bad)}')
    if bad:
        fails.append(f'{len(bad)} panic / ERROR lines, first: {bad[0][:160]}')

    body = [r for r in res if r[0] >= 300]
    if len(body) > 60:
        for col, name, lim in ((4, 'private', PRIV_MB), (3, 'VRAM', VRAM_MB)):
            a = st.mean(r[col] for r in body if r[0] < 420)
            b = st.mean(r[col] for r in body if r[0] >= body[-1][0] - 120)
            k = slope([r[0] / 60 for r in body], [r[col] for r in body])
            rows.append(f'{name}: {a:.0f} MB at 5-7 min -> {b:.0f} MB at the end ({b - a:+.0f} MB), slope {k:+.1f} MB / min')
            if b - a > lim:
                fails.append(f'{name} rose {b - a:.0f} MB (limit {lim:.0f})')
            if k > SLOPE_MB:
                fails.append(f'{name} grows {k:.1f} MB / min (limit {SLOPE_MB})')
    else:
        fails.append('too few samples after minute 5')

    slow = []
    for l in err:
        m = re.search(r'\[(\d{4}-\d\d-\d\dT[\d:]+)Z .*slowtick: ([\d.]+) ms', l)
        if m:
            slow.append((m.group(1), float(m.group(2))))
    # (the first minute - loading, the first streaming - is left out by the log's own clock: the first line's time)
    t0 = None
    for l in err:
        m = re.match(r'\[(\d{4}-\d\d-\d\dT)(\d\d):(\d\d):(\d\d)Z', l)
        if m:
            t0 = int(m.group(2)) * 3600 + int(m.group(3)) * 60 + int(m.group(4))
            break

    def secs(ts):
        h, mi, s = (int(x) for x in ts.split('T')[1].split(':'))
        v = h * 3600 + mi * 60 + s
        return v - t0 + (86400 if t0 is not None and v < t0 else 0) if t0 is not None else 1e9

    late = [ms for ts, ms in slow if secs(ts) > 60]
    rows.append(f'slow ticks (> 30 ms) after the first minute: {len(late)}, worst {max(late, default=0):.0f} ms')
    if len(late) > SLOW_N:
        fails.append(f'{len(late)} slow ticks (limit {SLOW_N})')
    if late and max(late) > SLOW_MAX:
        fails.append(f'a {max(late):.0f} ms tick (limit {SLOW_MAX:.0f})')

    def fps(a, b):
        ms = [r[1] for r in ft if a <= r[0] < b]
        return 1000 * len(ms) / sum(ms) if ms else float('nan')

    f1, f2 = fps(300, 600), fps(end - 300, end)
    rows.append(f'fps: {f1:.1f} at 5-10 min, {f2:.1f} in the last 5 min')
    if not (f2 >= f1 * (1 - FPS_DROP)):
        fails.append(f'fps fell {f1:.1f} -> {f2:.1f}')

    print(f'soak {tag}:')
    for r in rows:
        print('  ' + r)
    print('SOAK PASS' if not fails else 'SOAK FAIL: ' + '; '.join(fails))
    sys.exit(1 if fails else 0)


if __name__ == '__main__':
    main()
