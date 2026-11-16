# soak.py <runs dir> <tag> [--minutes 30] [--ref <runs dir> <tag>] [--vehicle <name>]: PASS / FAIL of a long free-roam run made by tools/perfgate.ps1
# (docs/PERFORMANCE.md "Release gate"). It reads the run's <tag>.ft.csv (MCLA_FRAMETIMES), <tag>.res.csv (samples every 2 s)
# and <tag>.err (the log), and checks:
#   - the game ran to the end (samples up to the last minute) and logged no panic / ERROR line;
#   - private memory and dedicated VRAM: the rise from minute 5 to the end (the mean of the first and the last two minutes)
#     at most PRIV_MB / VRAM_MB, and the slope over minutes 5 - end at most SLOPE_MB a minute (a leak grows steadily);
#     with --ref (the last release's soak, run beside it) instead of those: the level at the end at most the reference's
#     end plus PRIV_MB / VRAM_MB (v0.21.0 already grows ~40 MB / min in free roam, and a build that starts lower may
#     grow faster to the same level: docs/PERFORMANCE.md "Release gate"); private memory under PRIV_CEIL_MB at the end
#     either way;
#   - slow ticks (`slowtick:` lines, MCLA_SLOWTICK=30) after the first minute: at most SLOW_N, none over SLOW_MAX ms;
#   - frame rate: the last five minutes within FPS_DROP of minutes 5 - 10 (a session that slows down as it goes on);
#   - the car kept driving: at least MOVING licence plates made (the perf line's count: new traffic about the player) in
#     every 5 minutes after minute 5, in the run and in the --ref run. `--auto` can stick (from the default start at
#     ~12 min, both v0.21.0 and v0.22.0): such a run measures a parked car and is void (good runs >= 1,323, stuck 76 - 225);
#   - the vehicle driven: the game's `driven: player vehicle <name>` lines are printed; with --vehicle the run must have
#     driven only that one (a save's current car replaces `--car`: the v0.22.0 soak's "bike" legs drove the R8). The
#     --ref run is checked too when its build writes the line (builds before 2026-11-16 do not).
import os, re, statistics as st, sys

PRIV_MB, VRAM_MB, SLOPE_MB = 300.0, 200.0, 6.0
SLOW_N, SLOW_MAX = 10, 150.0
FPS_DROP = 0.10
MOVING = 600
PRIV_CEIL_MB = 7000.0


def slope(xs, ys):
    mx, my = st.mean(xs), st.mean(ys)
    d = sum((x - mx) ** 2 for x in xs)
    return sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / d if d else 0.0


def growth(d, tag):
    """{col: (rise from minutes 5-7 to the last two minutes, slope over minutes 5 - end, MB / min)} of a run, and its last private sample"""
    res = [l.strip().split(',') for l in open(os.path.join(d, tag + '.res.csv'))][1:]
    res = [[float(x) for x in r] for r in res if len(r) >= 7]
    body = [r for r in res if r[0] >= 300]
    if len(body) <= 60:
        return None, res
    o = {}
    for col in (4, 3):
        a = st.mean(r[col] for r in body if r[0] < 420)
        b = st.mean(r[col] for r in body if r[0] >= body[-1][0] - 120)
        o[col] = (a, b, slope([r[0] / 60 for r in body], [r[col] for r in body]))
    return o, res


def least_moving(d, tag):
    """(the fewest plates made in a 5-minute window after minute 5, its end in s) by the log's clock, or None"""
    pts, t0 = [], None
    for l in open(os.path.join(d, tag + '.err'), encoding='utf-8', errors='replace'):
        m = re.match(r'\[\d{4}-\d\d-\d\dT(\d\d):(\d\d):(\d\d)Z', l)
        if not m:
            continue
        t = int(m.group(1)) * 3600 + int(m.group(2)) * 60 + int(m.group(3))
        t0 = t if t0 is None else t0
        p = re.search(r'\((\d+) plates made', l)
        if p:
            pts.append(((t - t0) % 86400, int(p.group(1))))
    w = []
    for t, n in pts:
        before = [q for q in pts if q[0] <= t - 300]
        if t >= 600 and before:
            w.append((n - before[-1][1], t))
    return min(w) if w else None


def driven(d, tag):
    """the player vehicles the log names, in order (`driven: player vehicle <name>`)"""
    return re.findall(r'driven: player vehicle (\S+)', open(os.path.join(d, tag + '.err'), encoding='utf-8', errors='replace').read())


def main():
    d, tag = sys.argv[1], sys.argv[2]
    ref = None
    if '--ref' in sys.argv:
        i = sys.argv.index('--ref')
        ref, _ = growth(sys.argv[i + 1], sys.argv[i + 2])
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

    g, _ = growth(d, tag)
    if g:
        for col, name, lim in ((4, 'private', PRIV_MB), (3, 'VRAM', VRAM_MB)):
            a, b, k = g[col]
            rows.append(f'{name}: {a:.0f} MB at 5-7 min -> {b:.0f} MB at the end ({b - a:+.0f} MB), slope {k:+.1f} MB / min')
            if ref:
                ra, rb, rk = ref[col]
                rows.append(f'  the reference: {ra:.0f} -> {rb:.0f} MB ({rb - ra:+.0f} MB), slope {rk:+.1f} MB / min')
                if b > rb + lim:
                    fails.append(f'{name} {b:.0f} MB at the end, the reference {rb:.0f} MB (limit +{lim:.0f})')
            else:
                if b - a > lim:
                    fails.append(f'{name} rose {b - a:.0f} MB (limit {lim:.0f})')
                if k > SLOPE_MB:
                    fails.append(f'{name} grows {k:.1f} MB / min (limit {SLOPE_MB})')
        if g[4][1] > PRIV_CEIL_MB:
            fails.append(f'private {g[4][1]:.0f} MB at the end (ceiling {PRIV_CEIL_MB:.0f})')
    else:
        fails.append('too few samples after minute 5')
    if '--ref' in sys.argv and not ref:
        fails.append('the reference run has too few samples')

    for dd, tt, who in [(d, tag, 'the car')] + ([(sys.argv[sys.argv.index('--ref') + 1], sys.argv[sys.argv.index('--ref') + 2], 'the reference run\'s car')] if '--ref' in sys.argv else []):
        mv = least_moving(dd, tt)
        if mv is None:
            rows.append(f'{who}: no plate counts in the log')
            continue
        rows.append(f'{who}: at least {mv[0]} plates made in 5 min (the fewest by {mv[1] / 60:.1f} min)')
        if mv[0] < MOVING:
            fails.append(f'{who} stopped moving (only {mv[0]} plates made in the 5 min to {mv[1] / 60:.1f} min, limit {MOVING}): void, run again')

    want = sys.argv[sys.argv.index('--vehicle') + 1].lower() if '--vehicle' in sys.argv else None
    for dd, tt, who in [(d, tag, 'the run')] + ([(sys.argv[sys.argv.index('--ref') + 1], sys.argv[sys.argv.index('--ref') + 2], 'the reference run')] if '--ref' in sys.argv else []):
        names = driven(dd, tt)
        rows.append(f'{who} drove: {", ".join(dict.fromkeys(names)) if names else "not logged"}')
        if want is None:
            continue
        if not names and who == 'the run':
            fails.append(f'{who} logged no `driven:` line (an exe from before 2026-11-16?), cannot check it drove {want}')
        elif names and any(n.lower() != want for n in names):
            fails.append(f'{who} drove {", ".join(dict.fromkeys(names))}, not {want}: void, run again with a save whose car is {want}')

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
