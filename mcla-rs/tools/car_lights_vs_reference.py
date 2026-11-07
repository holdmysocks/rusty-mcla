"""The character vertex programs' car-light outputs from the emulated microcode (xemu): for each case the program's own
world place (o2.xyz) and world normal (o0.xyz), the car's constants c209 .. c214, and what the program exports as its
vertex lights (o5 / o7) and the floor term (o3.w = h / 0.66). Written for the port's GPU test (city.wgsl ch_vlights).
Usage: python tools/car_lights_vs_reference.py crates/game/src/testdata/car_lights_vs_ref.txt 100
Needs the microcode emulator and the programs' listings of work/lighting-chars/dis (xemu.py, dis_*.txt; made from the disc's
characters/*.fxc with xsh.py - docs/reference/lighting/character_programs_rule.md). Used by the test
drivers::cabin_tests::car_lights_per_vertex_match_the_programs."""
import sys, random, math
sys.path.insert(0, r"D:\MCLA\work\lighting-chars\dis")
import xemu
D = r"D:\MCLA\work\lighting-chars\dis\dis_%s.txt"
# (effect, program index, vertex-light output, arm (cone 0.9 / gain 10 / squared falloff), skinned)
PROGS = [("Character_skin", 7, "o5", 0, True), ("Character_skin_arm", 7, "o5", 1, True),
         ("Character_skin_normalmap", 7, "o7", 0, True), ("Character", 7, "o5", 0, False),
         ("Character_skin_arm", 6, "o5", 1, False), ("Character_normalmap", 7, "o7", 0, False)]
import re
ADDR = re.compile(r"(?<![0-9A-Za-z])82[0-9a-fA-F]{6}(?![0-9A-Za-z])")
def num(x):
    """%.9g, with more digits where the digits would read as an address of the original (registry check)"""
    for d in (9, 10, 11, 12):
        t = "%.*g" % (d, x)
        if not ADDR.search(t): return t
    raise ValueError(x)
out = sys.argv[1]; n = int(sys.argv[2]) if len(sys.argv) > 2 else 500
R = random.Random(42)
def unit(v):
    l = math.sqrt(sum(x * x for x in v)); return [x / l for x in v]
lines = ["# effect prog arm | c209 c210 c211 c212 c213 c214 (4 each) | pw n (3 each) | vlights h (from the emulated listing)"]
stats = []
for name, idx, vo, arm, skinned in PROGS:
    p = xemu.load(D % name, idx)
    nz = [0, 0]
    for t in range(n):
        rv = lambda s=1.0: [R.uniform(-s, s) for _ in range(4)]
        c = {"c%d" % i: rv() for i in range(0, 256)}
        vf = {a: rv() for a in range(0, 40)}
        if skinned:   # (the palette first: the car's constants below take their registers back; pw / n are the program's own)
            M = [rv(), rv(), rv()]
            for k in range(64, 64 + 3 * 80): c["c%d" % k] = M[(k - 64) % 3]
        # gWorld a turn and a place near the car (the drivers' gWorld is a translation; any turn is the same program)
        for k in range(4): c["c%d" % k] = rv(1.0)
        c["c3"] = [R.uniform(-2, 2), R.uniform(-2, 2), R.uniform(-2, 2), 1.0]
        c["c209"] = [R.uniform(-2.5, 2.5) for _ in range(3)] + [R.uniform(-1, 1)]
        c["c210"] = [R.uniform(0, 1) for _ in range(3)] + [R.uniform(-1, 1)]
        c["c211"] = unit([R.uniform(-1, 1) for _ in range(3)]) + [R.uniform(-1, 1)]
        c["c212"] = [R.uniform(-2.5, 2.5) for _ in range(3)] + [1.0]
        c["c213"] = [R.uniform(0, 1.5) for _ in range(3)] + [1.0]
        c["c214"] = unit([R.uniform(-0.3, 0.3), 1.0, R.uniform(-0.3, 0.3)]) + [R.uniform(-2, 2)]
        # (first run for the vertex's world place; then most cases put the lights within reach of it, the gauge's
        # cone near the direction to the vertex - else nearly every case is dark)
        pw0 = xemu.run(p, {}, c, None, lambda a: vf[a]).out["o2"][:3]
        if t % 4:
            c["c209"] = [pw0[i] + R.uniform(-1.2, 1.2) for i in range(3)] + [c["c209"][3]]
            c["c212"] = [pw0[i] + R.uniform(-1.2, 1.2) for i in range(3)] + [1.0]
            c["c211"] = unit([pw0[i] - c["c209"][i] + R.uniform(-0.4, 0.4) for i in range(3)]) + [c["c211"][3]]
        S = xemu.run(p, {}, c, None, lambda a: vf[a])
        o = S.out
        pw, nn, vl, h = o["o2"][:3], o["o0"][:3], o[vo][:3], o["o3"][3] * 0.66
        # the gauge / neon point at the vertex itself: no direction (the program divides by 0); not a drawn case
        if min(math.dist(pw, c["c209"][:3]), math.dist(pw, c["c212"][:3])) < 1e-3: continue
        if any(x != x or abs(x) == float("inf") for x in vl + [h]): continue
        nz[0] += any(abs(x) > 1e-6 for x in vl); nz[1] += 0 < h < 1
        vals = sum((c["c%d" % k] for k in range(209, 215)), []) + pw + nn + vl + [h]
        lines.append("%s %d %d %s" % (name, idx, arm, " ".join(num(x) for x in vals)))
    stats.append((name, idx, nz))
open(out, "w").write("\n".join(lines) + "\n")
for s in stats: print(s)
print(len(lines) - 1, "cases")
