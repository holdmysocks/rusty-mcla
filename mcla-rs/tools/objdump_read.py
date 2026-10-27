"""Reader of the object-dump recorder's files (work/harness-objdump, docs/FORMATS_TOOLS.md "Object-dump recorder").

File: b'OBJDUMP1', then records (little-endian host fields, guest bytes raw = big-endian as in guest memory):
    u32 func, u32 seq (call index of that function), u32 tick, u32 phase | depth << 8 (phase 0 before, 1 after),
    u32 nblocks, u32 payload bytes, u64 microseconds, u64 r1 r3..r10, f64 f1..f8, u64 lr,
    then nblocks x (u32 guest address, u32 length, bytes).
Both phases of one call carry the same block list.

    from objdump_read import read, calls
    for c in calls('dump.bin', func=<address of the function; from `mcla-tool registry addr F-#####`>):
        c.r3, c.before[addr], c.after[addr], c.ret (r3 after), c.fret (f1 after)
        c.u32(addr)  # big-endian read from the memory before the call
"""
import struct
from dataclasses import dataclass, field

HDR = struct.Struct('<6IQ9Q8dQ')


@dataclass
class Rec:
    func: int
    seq: int
    tick: int
    phase: int
    depth: int
    us: int
    gpr: tuple  # r1, r3 .. r10
    fpr: tuple  # f1 .. f8
    lr: int
    blocks: list  # [(addr, bytes)]


@dataclass
class Call:
    func: int
    seq: int
    tick: int
    depth: int
    us: int
    r1: int
    args: tuple  # r3 .. r10 before the call
    fargs: tuple  # f1 .. f8 before
    lr: int
    before: dict = field(default_factory=dict)  # guest address -> bytes
    after: dict = field(default_factory=dict)
    ret: int = None  # r3 after
    fret: float = None  # f1 after
    ret_regs: tuple = None  # r3 .. r10 after
    fret_regs: tuple = None

    @property
    def r3(self): return self.args[0]

    def u32(self, addr, after=False):
        """Big-endian word at a guest address inside a dumped block (None if not covered)."""
        for a, b in (self.after if after else self.before).items():
            if a <= addr and addr + 4 <= a + len(b):
                return struct.unpack_from('>I', b, addr - a)[0]
        return None

    def mem(self, after=False):
        return self.after if after else self.before


def read(path):
    data = open(path, 'rb').read()
    if data[:8] != b'OBJDUMP1':
        raise ValueError('not an objdump file')
    p = 8
    while p + HDR.size <= len(data):
        h = HDR.unpack_from(data, p)
        p += HDR.size
        func, seq, tick, pd, nb, payload = h[:6]
        blocks = []
        end = p + payload
        if end > len(data):
            break  # truncated tail (recorder killed mid-write)
        for _ in range(nb):
            a, n = struct.unpack_from('<II', data, p)
            blocks.append((a, bytes(data[p + 8:p + 8 + n])))
            p += 8 + n
        yield Rec(func, seq, tick, pd & 0xFF, pd >> 8, h[6], h[7:16], h[16:24], h[24], blocks)


def calls(path, func=None):
    """Pair the before / after records of each call (nesting by depth and thread order of the file)."""
    stack = {}
    for r in read(path):
        if func is not None and r.func != func:
            continue
        key = (r.func, r.seq)
        if r.phase == 0:
            c = Call(r.func, r.seq, r.tick, r.depth, r.us, r.gpr[0], tuple(r.gpr[1:]), tuple(r.fpr), r.lr)
            c.before = dict(r.blocks)
            stack[key] = c
        else:
            c = stack.pop(key, None)
            if c is None:
                continue
            c.after = dict(r.blocks)
            c.ret, c.fret = r.gpr[1], r.fpr[0]
            c.ret_regs, c.fret_regs = tuple(r.gpr[1:]), tuple(r.fpr)
            yield c


def image(call, after=False, size=0x100000):
    """Sparse guest memory of one call as {address: bytes}; for the closure runner's flat image use flat()."""
    return dict(call.mem(after))


def flat(call, base, length, after=False):
    """bytearray of guest [base, base + length) filled from the dumped blocks (0 where not recorded)."""
    out = bytearray(length)
    for a, b in call.mem(after).items():
        lo, hi = max(a, base), min(a + len(b), base + length)
        if lo < hi:
            out[lo - base:hi - base] = b[lo - a:hi - a]
    return out


def summary(path):
    n = {}
    for c in calls(path):
        n[c.func] = n.get(c.func, 0) + 1
    return n


if __name__ == '__main__':
    import sys
    for f, k in sorted(summary(sys.argv[1]).items()):
        print('%08X %d calls' % (f, k))
