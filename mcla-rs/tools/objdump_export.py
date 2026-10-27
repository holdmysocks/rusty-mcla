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


if __name__ == '__main__':
    {'signals': signals, 'aogrid': aogrid, 'navsidewalk': navsidewalk}[sys.argv[1]](*sys.argv[2:])
