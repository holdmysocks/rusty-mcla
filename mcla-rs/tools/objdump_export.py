"""Turns object-dump recordings (objdump_read.py) into case files for port tests.

    python objdump_export.py signals <dump.bin> <out.txt>

signals: the junction arm tests F-23734 F-23525 F-59460: green (C38), yellow (C98), red (D10). One line per call:
    "<g|y|r> <current arm> <timer f32 bits> <arm> <allsignalsgreen byte> <result>"
"""
import struct
import subprocess
import sys
from objdump_read import calls

TOOL = 'mcla-tool'


def addr(fid):
    """Function address of an F-##### id (unpublished data of the registry)."""
    out = subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout
    return int(out.split()[2][4:], 16)


SIG = {'F-23734': 'g', 'F-23525': 'y', 'F-59460': 'r'}


def signals(dump, out):
    lines = []
    for fid, k in SIG.items():
        for c in calls(dump, addr(fid)):
            j = c.r3 & 0xFFFFFFFF
            cur, tm = c.u32(j + 160), c.u32(j + 164)
            ext = [b for a, b in c.before.items() if len(b) == 1]
            if cur is None or tm is None or not ext:
                continue
            lines.append('%s %d %08x %d %d %d' % (k, struct.unpack('>i', struct.pack('>I', cur))[0], tm, c.args[1] & 0xFFFFFFFF,
                                                  ext[0][0], c.ret & 0xFF))
    open(out, 'w').write('\n'.join(lines) + '\n')
    print(len(lines), 'cases')


def aogrid(dump, out):
    """F-39732: the grid text parse (r3 = object): the object after it, +0 .. +55 as big-endian words (hex)."""
    lines = []
    for c in calls(dump, addr('F-39732')):
        a = c.args[0] & 0xFFFFFFFF
        lines.append(' '.join('%08x' % c.u32(a + o, after=True) for o in range(0, 56, 4)))
    open(out, 'w').write(chr(10).join(lines) + chr(10))
    print(len(lines), 'objects')


def navsidewalk(dump, out):
    """F-27305: nav polygon filter (r4 = polygon or 0): "<flags or - for null> <result>"."""
    lines = set()
    n = 0
    for c in calls(dump, addr('F-27305')):
        poly = c.args[1] & 0xFFFFFFFF
        if poly == 0:
            lines.add('- %d' % (c.ret & 0xFF))
            continue
        fl = c.u32(poly + 8)
        if fl is None:
            continue
        n += 1
        lines.add('%08x %d' % (fl, c.ret & 0xFF))
    open(out, 'w').write(chr(10).join(sorted(lines)) + chr(10))
    print(len(lines), 'distinct cases of', n)


def hcut(v):
    """The writer's half float: cut, not rounded."""
    import numpy as np
    v = np.float32(v)
    if np.isnan(v) or abs(v) < 6.103515625e-05:
        return 0
    b = int(np.float32(np.clip(v, -65504, 65504)).view(np.uint32))
    return ((((b >> 23) & 0x1ff) + 16) << 10 | (b >> 13) & 0x3ff | (b >> 16) & 0x8000) & 0xffff


def lamptile(raw, cells, out, windows='-2000,-1500,-700,-500;300,-2300,900,-1700'):
    """F-58353 / F-12750's cell lists: for some windows (x0,z0,x1,z1; ';' between) the lamps of the open-city build that can reach them
    (table rows 6363 .. 21536 of the original, as `raw` = MCLA_LAMP_DUMP=<file> '.raw', lamp i = row 6363 + i, within the window
    widened by the longest reach) and the original's lists of the cells inside whose four lamps are all those rows (`cells` =
    lamp_grid_reference.py output). Lines: "L <row> <12 float bits>" and "C <cell> <row0> .. <row3>" (rows 0 = none)."""
    import numpy as np
    R = np.frombuffer(open(raw, 'rb').read(), dtype='<f4').reshape(-1, 12)
    O = np.array([list(map(int, l.split())) for l in open(cells) if l.strip()], dtype=np.int64)
    lo, hi = 6363, 6363 + 15174
    reach = float(np.sqrt(100 * R[:15174, 9].max()))
    lines = []
    gw, gh, org, span = 512, 640, (-3467.0, -2358.0), (6320.0, 4463.75)
    for win in windows.split(';'):
        x0, z0, x1, z1 = map(float, win.split(','))
        pick = [i for i in range(15174) if x0 - reach - 20 <= R[i][0] <= x1 + reach + 20 and z0 - reach - 20 <= R[i][2] <= z1 + reach + 20]
        lines += ['L %d %s' % (6363 + i, ' '.join('%08x' % x for x in R[i].view(np.uint32))) for i in pick]
        n = 0
        for cz in range(gh):
            for cx in range(gw):
                x = org[0] + (cx + .5) * span[0] / gw
                z = org[1] + (cz + .5) * span[1] / gh
                if not (x0 <= x <= x1 and z0 <= z <= z1):
                    continue
                o = O[cz * gw + cx]
                if (o > 0).any() and all(v == 0 or lo <= v < hi for v in o):
                    lines.append('C %d %s' % (cz * gw + cx, ' '.join(str(v) for v in o)))
                    n += 1
        print(win, len(pick), 'lamps', n, 'cells')
    open(out, 'w').write(chr(10).join(lines) + chr(10))


def lamprow(w1, tables, raw, out):
    """F-85100's lamp table writer. Lines "pos xyz color xyz back xyz intensity cone0 cone1 (float bits) = the Lamp struct -> A (4 x u16, hex) B C (8 hex digits each)".
    w1: the writer's recorded calls (record at r9, tables dumped after); tables + raw: the whole tables of one lamp-grid build (cap
    file of F-12750 / the lookup, `@r3+2C,6C,AC`) and the port's lamp records (MCLA_LAMP_DUMP=<file>, <file>.raw) whose lamps sit at table
    index 6363 + i in the open city; every 40th lamp whose x, z and cone halves the original's table agrees with is taken."""
    import numpy as np
    lines = []
    for c in calls(w1, addr('F-85100')):
        a0 = [b for k, b in c.before.items() if len(b) == 262144][0]
        a1 = [b for k, b in c.after.items() if len(b) == 262144][0]
        n = next(i for i in range(0, len(a0), 8) if a0[i:i + 8] != a1[i:i + 8]) // 8
        # (the two 128 KB tables: C below B in address, the object's pointers +0xAC / +0x6C)
        bk = sorted(k for k, b in c.after.items() if len(b) == 131072)
        b1, c1 = c.after[bk[1]], c.after[bk[0]]
        rec = [b for b in c.before.values() if len(b) == 128][0]
        w = struct.unpack('>32I', rec)
        lines.append('%s -> %s %s %s' % (' '.join('%08x' % (w[k] ^ (0x80000000 if k in (8, 9, 10) else 0)) for k in (0, 1, 2, 4, 5, 6, 8, 9, 10, 16, 19, 20)),
                                         ' '.join('%04x' % x for x in struct.unpack('>4H', a1[n * 8:n * 8 + 8])), b1[n * 4:n * 4 + 4].hex(), c1[n * 4:n * 4 + 4].hex()))
    cs = list(calls(tables))[0]
    ob = cs.before[cs.args[0] & 0xFFFFFFFF]
    ad = lambda k: struct.unpack_from('>I', ob, k)[0]
    A, B, C = cs.before[ad(44)], cs.before[ad(0x6c)], cs.before[ad(0xac)]
    R = np.frombuffer(open(raw, 'rb').read(), dtype='<f4').reshape(-1, 12)
    for i in range(0, 15174, 40):
        r = R[i]
        t = 6363 + i
        a = struct.unpack('>4H', A[t * 8:t * 8 + 8])
        if a[2:] != (hcut(r[10]), hcut(r[11])):
            continue  # (the port's cone differs from the original's for these lamps: not the writer's)
        lines.append('%s -> %s %s %s' % (' '.join('%08x' % x for x in r.view(np.uint32)[[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]]),
                                         ' '.join('%04x' % x for x in a), B[t * 4:t * 4 + 4].hex(), C[t * 4:t * 4 + 4].hex()))
    open(out, 'w').write(chr(10).join(lines) + chr(10))
    print(len(lines), 'lamps')


if __name__ == '__main__':
    {'signals': signals, 'aogrid': aogrid, 'navsidewalk': navsidewalk, 'lamprow': lamprow, 'lamptile': lamptile}[sys.argv[1]](*sys.argv[2:])
