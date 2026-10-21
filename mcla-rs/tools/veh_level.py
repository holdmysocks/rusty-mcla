"""The session's level facts of a stage-R recording, for the port's replay (env MCLA_LOOSE_LEVEL, physics/loose.rs
`level_hints`): the car's level index and instance, every logged object's instance address, the entry pool's top, the
city's instance, and per update k the level indices the broken-off pieces took, in the order they were activated.
They are session values (the level's free list, heap addresses) that a run cannot derive, and they decide which body
is A in a pair - and so the narrow phase's search (sb.0: without them the car x piece pair of update 232 is turned
round and the replay parts there; with them it is bit-equal to 332).

    python veh_level.py <ext.bin> <first driven sample> <out>

Update k is the one that writes sample `first + k + 1`; a piece activated in it shows its CALL record at that sample.
`veh_diff.py` writes the file itself for every recording with a side file."""
import struct
import sys

from veh_trace import Ext

ACTIVATE = 0x8258D3F8


def write(ext_path, first, out):
    """Write the facts to `out`; False when the side file holds no level record."""
    e = Ext(ext_path)
    if not e.at or not e.at[0].get("LEVL"):
        return False
    off = first + 1
    car = e.at[0]["LEVL"][0]["mine"]
    addr, seen = {}, {}
    for r in e.records:
        if r["tag"] == "OBJ ":
            addr.setdefault(r["index"], r["inst"])
            seen.setdefault(r["index"], r["tick"])
    # (a piece: first logged after the start, its instance from the entry pool - 0xbe7..... in these sessions, the
    # props' own instances lie far below)
    pieces = [i for i in addr if seen[i] > 0 and addr[i] >> 24 == 0xBE]
    free = {}
    for r in e.records:
        if r["tag"] == "CALL" and r["id"] == ACTIVATE and r["r4"] in pieces and seen[r["r4"]] == r["tick"]:
            got = free.setdefault(r["tick"] - off, [])
            if r["r4"] not in got:
                got.append(r["r4"])
    top = max(addr[i] for i in pieces) + 0x90 if pieces else None
    # (the city's instance: B of every recorded manifold with the city, level index 0)
    city = next((struct.unpack_from(">I", r["block"], 52)[0] for r in e.records if r["tag"] == "PAIR" and r["a"] == 0), None)
    with open(out, "w") as f:
        f.write("car %d 0x%x\n" % (car, addr[car]))
        for i in sorted(addr):
            f.write("obj %d 0x%x\n" % (i, addr[i]))
        if top:
            f.write("pool 0x%x\n" % top)
        if city:
            f.write("city 0x%x\n" % city)
        for k in sorted(free):
            f.write("free %d %s\n" % (k, " ".join(map(str, free[k]))))
    return True


if __name__ == "__main__":
    if write(sys.argv[1], int(sys.argv[2]), sys.argv[3]):
        print(open(sys.argv[3]).read())
    else:
        print("no level record")
