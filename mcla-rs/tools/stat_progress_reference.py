"""A Stat condition's progress runs offline: F-13232, the generated code unchanged, on a condition at 0x1000 (+88 stat
name, +120 type string, +152 sub id, +156 target, +160 baseline); the stat manager's value call (its vtable +76, an
indirect call) returns the case's value. Cases: the type Purse / Money / Tally / purse / PURSE / Purses / empty,
values, baselines and targets round 0, small and large (signed 32-bit words). Writes <out-dir>/stat_progress_ref.txt:
"<type> <value> <base> <target> -> <result>" (signed decimal)."""
import argparse
import random
import re
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def cases():
    rnd = random.Random(13232)
    types = ['Purse', 'Money', 'Tally', 'purse', 'PURSE', 'Purses', 'Purs', '-']
    def word():
        return rnd.choice((0, 1, -1, rnd.randrange(-50, 50), rnd.randrange(0, 100000), rnd.randrange(-2**31, 2**31),
                           2**31 - 1, -2**31))
    out = []
    for _ in range(3000):
        out.append((rnd.choice(types), word(), word(), rnd.choice((0, 1, 5, 10, 5000, rnd.randrange(0, 100000), word()))))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    f = addr(a.tool, 'F-13232')
    cs = cases()
    nl = chr(10)
    recomp_closure.EXTRA += 'static uint32_t g_v;\n#define REX_CALL_INDIRECT_FUNC(a) (ctx.r3.u64=g_v)\n'
    rows = (',' + nl).join('{"%s",%du,%du,%du}' % ('' if t == '-' else t, v & 0xFFFFFFFF, b & 0xFFFFFFFF, g & 0xFFFFFFFF) for t, v, b, g in cs)
    body = ('struct K{const char*t;uint32_t v,b,g;};static const K C[]={' + nl + rows + nl + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));g_v=c.v;'
            # the stat manager read from 0x828CCF68: a vtable with +76
            'store(0x828CCF68,0x1900);store(0x1900,0x1A00);store(0x1A00+76,0x82000000);'
            'for(int i=0;c.t[i];i++)*pointer(0x1000+120+i)=uint8_t(c.t[i]);store(0x1000+156,c.g);store(0x1000+160,c.b);'
            '{Context ctx{};ctx.r1.u32=0x1F00;ctx.r3.u32=0x1000;sub_%s(ctx,nullptr);printf("%%d' % f + chr(92) + 'n",int32_t(ctx.r3.u32));}}' + nl + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + f], body, a.out_dir, 'stat_progress')
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == len(cs)
    lines = ['%s %d %d %d -> %s' % (t, v, b, g if g < 2**31 else g - 2**32, o) for (t, v, b, g), o in zip(cs, out)]
    # (a line with a word the registry check reads as an address of the original - 8 digits from 8 - is left out)
    lines = [x for x in lines if not re.search(r'\b8[0-9A-Fa-f]{7}\b', x)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'stat_progress_ref.txt').write_text(nl.join(lines) + nl)
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
