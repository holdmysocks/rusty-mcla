"""Offline reference runs for the SAVE gaps of 2026-10-28 (session save-gaps2): unchanged functions of the original cut
out of the static recompilation (with the closure of their callees, `recomp_closure.py`), run on prepared inputs.

  pink   the career's pink slip profiles by location: Add F-58746, Load F-53651, Remove F-85350, GetNum F-54647,
         GetUID F-50697, Get, GetType, GetName (the F-ids of the others: `mcla-tool registry id <addr>`), on the location
         map the career script fills (mcCareer_RegisterLocation). Stubs: the set-up loader (sets the record's +8104 word
         and its name at +8109), the record copy (the whole 8176 bytes; the comparison reads +8104 and +8109 only),
         memcpy, the log call.

Writes crates/game/src/testdata/save_gaps_<part>_ref.txt ("input -> output" lines).
Run: python save_gaps_reference.py [pink ...] (g++ of WinLibs on PATH).
"""
import random
import re
import subprocess
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent))
from recomp_closure import index, build as _build

RECOMP = Path('D:/MCLA-static')
IMAGE = Path('D:/MCLA/work/2026-10-10/original-image.bin')
OUT = Path('D:/MCLA/work/save-gaps2/out')
DATA = Path(__file__).parent.parent / 'crates/game/src/testdata'


TOOL = str(Path(__file__).parent.parent / 'target/release/mcla-tool.exe')
_ADDR = {}


def fn(fid):
    """The generated function of an ID (`mcla-tool registry addr`)."""
    if fid not in _ADDR:
        out = subprocess.run([TOOL, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()
        _ADDR[fid] = 'sub_' + next(w for w in out if w.startswith('sub_'))[4:].upper()
    return _ADDR[fid]


def build(idx, names, main, out, tag, stubs=(), **kw):
    return _build(idx, [ids(n) for n in names], ids(main), out, tag, stubs={ids(k): v for k, v in dict(stubs).items()}, **kw)


def ids(text):
    """C++ text with its <F-n> tokens replaced by the generated functions' names."""
    return re.sub(r'<(F-\d+)>', lambda mm: fn(mm.group(1)), text)

# mcCareer_RegisterLocation(id, key, desc, index) of the career script (game/career/0x1529a945), in its order
LOCATIONS = [(500, 0), (501, 0), (502, 0), (510, 1), (520, 2), (530, 3), (540, 4), (503, 0), (511, 1), (521, 2), (531, 3), (541, 4),
             (506, 0), (505, 0), (512, 1), (543, 4), (504, 0), (542, 4), (513, 1), (600, 0), (601, 3), (602, 3), (603, 1), (604, 1),
             (605, 0), (606, 1), (607, 1), (608, 0), (609, 3), (610, 4)]


import recomp_closure
_shim = recomp_closure.shim


SCRATCH = [0x3000]


def _shim_ctr(scratch_end=None):
    # the count register and the trap the generated code uses (a trap would be a bug in the inputs); the vector
    # mode's shim takes the scratch size from SCRATCH
    s = _shim(scratch_end or SCRATCH[0])
    assert s.count('struct Context { PPCRegister') == 1
    s = s.replace('struct Context { PPCRegister', 'struct Context { PPCRegister ctr; PPCRegister')
    assert s.count('uint16_t u16; uint8_t u8;') == 1
    s = s.replace('uint16_t u16; uint8_t u8;', 'uint16_t u16; uint8_t u8; int8_t s8; int16_t s16;')
    return s + 'template<class... A> static void ppc_trap(A...){ throw std::runtime_error("trap"); }\n'


recomp_closure.shim = _shim_ctr


PINK_MAIN = r'''
static const uint32_t C = 0x2000;          // the career (mcCareer)
static const uint32_t NODES = 0x19000, REC = 0x1D000, STR = 0x1F800;
static uint32_t nreg = 0;
static void reg(uint32_t id, uint32_t ix) {   // the location map at C+8748: buckets (+0), count (+4); node: key, value (+4), index (+264), next (+268)
    uint32_t node = NODES + 272 * nreg++;
    uint32_t nb = load16(C + 8748 + 4), b = load(C + 8748) + 4 * (id % nb);
    store(node, id); store(node + 4, id); store(node + 264, ix); store(node + 268, load(b)); store(b, node);
}
static std::string str(uint32_t a, int n) { std::string s; for (int i = 0; i < n && *pointer(a + i); i++) s += char(*pointer(a + i)); return s.empty() ? "-" : s; }
static void call(void (*f)(Context&, const uint8_t*), uint32_t r4, uint32_t r5, uint32_t r6, Context &c) {
    c = Context{}; c.r1.u32 = 0x2F000; c.r3.u32 = C; c.r4.u32 = r4; c.r5.u32 = r5; c.r6.u32 = r6; f(c, nullptr);
}
static uint32_t entry(int k) { return k < 5 ? C + 8768 + 8272 * k : C + 50128 + 8272 * (k - 5); }
static void state() {
    printf(" | %d", (int)load(C + 0x10000 + 25960));
    for (int k = 0; k < 10; k++) {
        uint32_t e = entry(k);
        printf(" %d,%s,%d,%s,%d", (int)load(e), str(e + 4, 64).c_str(), load(e + 80 + 8104) != 0, str(e + 80 + 8109, 32).c_str(), (int)load(e + 8256));
    }
}
int main(int argc, char **argv) {
    std::ifstream bin(argv[1], std::ios::binary); image.assign(std::istreambuf_iterator<char>(bin), {});
    std::ifstream in(argv[2]); std::string op;
    while (in >> op) {
        if (op == "RESET") {   // a new career: every profile empty (F-47738 on each), the map rebuilt, the counter given
            int32_t ctr; in >> ctr;
            memset(objects, 0, sizeof(objects)); nreg = 0;
            store(C + 8748, 0x18F00); store16(C + 8748 + 4, 7);
            for (auto &p : std::vector<std::pair<int,int>>{LOCS}) reg(p.first, p.second);
            Context c{};
            for (int k = 0; k < 10; k++) { c = Context{}; c.r1.u32 = 0x2F000; c.r3.u32 = entry(k); <F-47738>(c, nullptr); }
            store(C + 0x10000 + 25960, ctr);
            printf("RESET %d ->", ctr); state(); printf("\n"); continue;
        }
        int32_t loc, a, b; in >> loc >> a >> b;
        printf("%s %d %d %d ->", op.c_str(), loc, a, b);
        Context c{};
        if (op == "A") {           // AddPinkSlipProfile(loc, record "r<a>", kind b)
            memset(pointer(REC), 0, 8176); store(REC + 8104, 1); snprintf((char*)pointer(REC + 8109), 32, "r%d", a);
            call(F_ADD, loc, REC, b, c); printf(" %d", c.r3.s32);
        } else if (op == "L") {    // LoadPinkSlipProfile(loc, set-up "s<a>", kind b)
            memset(pointer(STR), 0, 64); snprintf((char*)pointer(STR), 64, "s%d", a); call(F_LOAD, loc, STR, b, c); printf(" %d", c.r3.s32);
        } else if (op == "N") { call(F_NUM, loc, 0, 0, c); printf(" %d", c.r3.s32); }
        else if (op == "U") { call(F_UID, loc, a, 0, c); printf(" %d", c.r3.s32); }
        else if (op == "T") { call(F_TYPE, loc, a, 0, c); printf(" %d", c.r3.s32); }
        else if (op == "G") {      // the record: which entry (0..4 the location's, 5..9 the lot), -1 none
            call(F_GET, loc, a, 0, c); int w = -1;
            for (int k = 0; k < 10; k++) if (c.r3.u32 == entry(k) + 80) w = k;
            if (c.r3.u32 && w < 0) w = 99; printf(" %d", w);
        } else if (op == "M") { call(F_NAME, loc, a, 0, c); printf(" %s", c.r3.u32 ? str(c.r3.u32, 64).c_str() : "(null)"); }
        else if (op == "R") { call(F_REMOVE, loc, a, 0, c); printf(" 0"); }
        state(); printf("\n");
    }
}
'''

PINK_FUNCS = {'F_ADD': 'F-58746', 'F_LOAD': 'F-53651', 'F_REMOVE': 'F-85350', 'F_NUM': 'F-54647', 'F_UID': 'F-50697',
              'F_GET': 'F-93331', 'F_TYPE': 'F-87077', 'F_NAME': 'F-42349'}


def pink():
    idx = index(RECOMP)
    main = PINK_MAIN.replace('LOCS', ','.join('{%d,%d}' % p for p in LOCATIONS))
    for k, v in PINK_FUNCS.items():
        main = main.replace(k, '<' + v + '>')
    stubs = {
        # the set-up loader: the record is there (+8104), named after the set-up (+8109)
        '<F-67050>': 'store(ctx.r3.u32+8104,1); for(int i=0;i<32;i++) *pointer(ctx.r3.u32+8109+i)=0; '
                        'strncpy((char*)pointer(ctx.r3.u32+8109),(char*)pointer(ctx.r4.u32),31);',
        '<F-58835>': 'if(ctx.r3.u32!=ctx.r4.u32) memmove(pointer(ctx.r3.u32),pointer(ctx.r4.u32),8176);',
        '<F-44611>': 'memmove(pointer(ctx.r3.u32),pointer(ctx.r4.u32),ctx.r5.u32);',
        '<F-47121>': '',
    }
    names = ['<' + v + '>' for v in PINK_FUNCS.values()] + ['<F-47738>']
    exe = build(idx, names, main, OUT, 'pink', stubs=stubs, scratch_end=0x30000)
    rnd = random.Random(20261028)
    locs = [p[0] for p in LOCATIONS] + [507, 0, 999]
    lines = []
    for _ in range(400):
        lines.append('RESET %d' % rnd.choice([0, 5, rnd.randrange(0, 1000), 0x7FFFFFFE, 0x7FFFFFFF, -3]))
        for _ in range(rnd.randrange(5, 40)):
            loc = rnd.choice(locs if rnd.random() < .2 else [503, 511, 521, 531, 541, 500, 510])
            op = rnd.choice('AALLNUTGMRRR')
            if op in 'AL':
                lines.append('%s %d %d %d' % (op, loc, rnd.randrange(0, 50), rnd.choice([0, 0, 1, 1, 2, -1])))
            elif op == 'N':
                lines.append('N %d 0 0' % loc)
            elif op == 'U':
                lines.append('U %d %d 0' % (loc, rnd.choice([-1, 0, 1, 2, 3, 4, 5, 6])))
            else:
                lines.append('%s %d %d 0' % (op, loc, rnd.choice([-1, 0, 1, 2, 3, rnd.randrange(0, 60), 0x7FFFFFFF])))
    cases = OUT / 'pink_cases.txt'
    cases.write_text('\n'.join(lines) + '\n')
    r = subprocess.run([str(exe), str(IMAGE), str(cases)], capture_output=True, text=True, check=True)
    (DATA / 'save_gaps_pink_ref.txt').write_text(r.stdout, newline='\n')
    print('pink', len(lines), len(r.stdout.splitlines()))


CTOR_MAIN = r'''
int main(int argc, char **argv) {
    std::ifstream bin(argv[1], std::ios::binary); image.assign(std::istreambuf_iterator<char>(bin), {});
    std::ifstream in(argv[2]); uint32_t lo, carry;
    while (in >> lo >> carry) {   // the game's generator (0x82860880: lo, carry) before the constructor
        memset(objects, 0xAA, sizeof(objects));
        store(0x82860880, lo); store(0x82860884, carry);
        Context c{}; c.r1.u32 = 0x7F00; c.r3.u32 = 0x1000; <F-13631>(c, nullptr);
        printf("%u %u ->", lo, carry);
        for (int i = 0; i < 8176; i++) printf("%s%02x", i % 64 ? "" : " ", *pointer(0x1000 + i));
        printf(" %u %u\n", load(0x82860880), load(0x82860884));
    }
}
'''


def ctor():
    """The record constructor F-13631 (vtables, the performance part F-?, the look F-35798 with its default blocks,
    the random plate F-92553, the interior part), on a block filled with 0xAA, for random generator states."""
    SCRATCH[0] = 0x8000
    idx = index(RECOMP)
    stubs = {'<F-44611>': 'memmove(pointer(ctx.r3.u32),pointer(ctx.r4.u32),ctx.r5.u32);',
             '<F-30807>': 'memset(pointer(ctx.r3.u32),ctx.r4.u8,ctx.r5.u32);'}
    exe = build(idx, ['<F-13631>'], CTOR_MAIN, OUT, 'ctor', stubs=stubs, vector=True)
    rnd = random.Random(2026102801)
    lines = ['%u %u' % p for p in [(1, 0x10001), (0, 0), (0xFFFFFFFF, 0xFFFFFFFF), (0x7FFFFFFF, 0x5CDCFAA6)]]
    lines += ['%u %u' % (rnd.getrandbits(32), rnd.getrandbits(32)) for _ in range(300)]
    cases = OUT / 'ctor_cases.txt'
    cases.write_text('\n'.join(lines) + '\n')
    r = subprocess.run([str(exe), str(IMAGE), str(cases)], capture_output=True, text=True, check=True)
    (DATA / 'save_gaps_ctor_ref.txt').write_text(r.stdout, newline='\n')
    print('ctor', len(lines), len(r.stdout.splitlines()))


PART_MAIN = r'''
uint32_t g_bike, g_owned;
int main(int argc, char **argv) {
    std::ifstream bin(argv[1], std::ios::binary); image.assign(std::istreambuf_iterator<char>(bin), {});
    std::ifstream in(argv[2]); uint32_t lo, carry, cat; std::string name;
    while (in >> lo >> carry >> cat >> g_bike >> g_owned >> name) {
        memset(objects, 0, sizeof(objects)); store(0x1F00, g_bike); store(0x1F04, g_owned);
        store(0x1000 + 52, 0x2000); store(0x1000 + 48, 0x1100);
        strncpy((char*)pointer(0x2000 + 8109), name.c_str(), 31);
        store(0x82860880, lo); store(0x82860884, carry);
        printf("%u %u %u %u %u %s ->", lo, carry, cat, g_bike, g_owned, name.c_str());
        Context c{}; c.r1.u32 = 0x7F00; c.r3.u32 = 0; c.r4.u32 = cat; <F-39082>(c, nullptr);
        printf(" = %d %u %u\n", c.r3.s32, load(0x82860880), load(0x82860884));
    }
}
'''


def part():
    """The free part F-39082 (the Part reward's apply F-48610): its draw, the category's part, the owned test and
    the purchase mark (both stubs that print their arguments; the owned answer from the case's mask)."""
    SCRATCH[0] = 0x8000
    idx = index(RECOMP)
    stubs = {
        '<F-65049>': 'ctx.r3.u32 = 0x1000;',
        '<F-30867>': 'ctx.r3.u32 = 7;',
        '<F-82698>': 'ctx.r3.u32 = 0x3000;',
        '<F-54299>': 'ctx.r3.u32 = load(0x1F00);',
        '<F-96345>': 'ctx.r3.u32 = strstr((char*)pointer(ctx.r3.u32), (char*)pointer(ctx.r4.u32)) ? 1 : 0;',
        '<F-10180>': 'printf(" O%d,%d", ctx.r4.s32, ctx.r5.s32); ctx.r3.u32 = (ctx.r4.u32 < 32 && (load(0x1F04) >> ctx.r4.u32 & 1)) || (ctx.r4.s32 >= 32 && (load(0x1F04) >> 31 & 1));',
        '<F-66800>': 'printf(" M%d,%d,%d", ctx.r4.s32, ctx.r5.s32, ctx.r6.s32); ctx.r3.u32 = 1;',
    }
    exe = build(idx, ['<F-39082>'], PART_MAIN, OUT, 'part', stubs=stubs, vector=True)
    rnd = random.Random(2026102802)
    names = ['vp_frd_gt_06', 'vp_cad_escalade_06_DUB', 'vp_duc_999r_06', 'vpd_chv_chevelle_70', 'vp_x_dub_lower']
    lines = []
    for _ in range(3000):
        owned = rnd.choice([0, 0, rnd.getrandbits(32)])
        lines.append('%u %u %u %u %u %s' % (rnd.getrandbits(32), rnd.getrandbits(32), rnd.choice([0, 1, 2, 3, 4, 0, 1, 2, 5, 0xffffffff]),
                                                rnd.choice([0, 0, 1]), owned, rnd.choice(names)))
    cases = OUT / 'part_cases.txt'
    cases.write_text('\n'.join(lines) + '\n')
    r = subprocess.run([str(exe), str(IMAGE), str(cases)], capture_output=True, text=True, check=True)
    (DATA / 'save_gaps_part_ref.txt').write_text(r.stdout, newline='\n')
    print('part', len(lines), len(r.stdout.splitlines()))


if __name__ == '__main__':
    for p in sys.argv[1:] or ['pink']:
        globals()[p]()
