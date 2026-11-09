"""player_speed_recording.py <recording.txt> <game.dat> <out.txt>: a drive recorded in the original with the speed-mix
recorder (a private copy of work/recomp-trace whose sst_audio recorder writes, at every player audio entity update F-99947,
an "SPD" line: the frame time, the camera mode +1744, the mixer inputs +1748 / +1752, the car's speed +224, the entity's
rpm +200, the engine's +76 / +68 and the linear level +4 of each of the +1732 controllers) -> the test file of
sndobj::tests::speed_mix_matches_the_recording: the 24 MANAGER_PLAYER_SPEED records of game.dat ("r", as
player_speed_ref.txt) and one "S" line per update (m mode, a / b inputs, d dt, v speed, p rpm, i / x engine +76 / +68,
then the 24 levels), all floats as their bits in hex.
Recordings: work/audio2/spdrec (run.py, cap/s3.txt: the R8 in free roam, 1,350 updates, 0 - 141 mph)."""
import struct
import sys

sys.path.insert(0, __file__.rsplit('\\', 1)[0].rsplit('/', 1)[0])
from player_speed_reference import gamedat  # noqa: E402


def main():
    rec, dat, out = sys.argv[1:4]
    t = gamedat(dat)
    lines = []
    for k in range((len(t) - 11) // 28):
        b = 11 + 28 * k
        cat, a0, a1, b0, b1 = struct.unpack_from('>IIIII', t, b)
        m0, m1 = struct.unpack_from('>HH', t, b + 20)
        rate = struct.unpack_from('>I', t, b + 24)[0]
        lines.append('r h%d a%08x a%08x b%08x b%08x m%d m%d t%08x' % (cat, a0, a1, b0, b1, m0, m1, rate))
    n, nrec = 0, len(lines)
    for l in open(rec):
        if not l.startswith('SPD '):
            continue
        head, lv = l.split('|')
        kv = dict(w.split('=') for w in head.split()[1:])
        assert int(kv['n']) == nrec == len(lv.split())
        lines.append('S m%s a%s b%s d%s v%s p%s i%s x%s L %s' % (kv['mode'], kv['a'], kv['b'], kv['dt'], kv['v'], kv['rpm'],
                                                              kv['e76'], kv['e68'], ' '.join(lv.split())))
        n += 1
    open(out, 'w', newline='\n').write('\n'.join(lines) + '\n')
    print(n, 'updates,', nrec, 'records ->', out)


if __name__ == '__main__':
    main()
