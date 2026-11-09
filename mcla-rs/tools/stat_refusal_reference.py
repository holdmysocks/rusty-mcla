"""The statistics manager's submit F-79933 run offline as generated, for its refusals: the record's code (four
characters at +0, the first one picked by its switch), the cheat gate F-68910 (real: its bytes set as the cheat
block's nitro or never damage out), the test-drive byte F-64975 reads, and the current race's +3152 (goal attack).
Stubbed: the current race F-27863 (0, or a race at 0x5000 with +3152 = 1), the on-line test F-24338 (0: off line), the
record look-up F-55011 = found, the code text F-24248 (host: the four characters, as its "%c%c%c%c_%d" into five
bytes), and the add F-45364 (marks the record written). Op 0 (+8 = 0). Writes <out-dir>/stat_refusal_ref.txt:
"code gate test_drive goal_attack -> written" (gate 0 off, 1 nitro, 2 never damage out)."""
import argparse
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

_shim = recomp_closure.shim
recomp_closure.shim = lambda *a: (_shim(*a).replace('uint16_t u16; uint8_t u8;', 'uint16_t u16; int16_t s16; uint8_t u8; int8_t s8;', 1)
                                  .replace('Condition cr0,cr6;', 'Condition cr0,cr1,cr5,cr6,cr7;', 1))

CHEAT = (0x8289 << 16) - 6736  # the cheat block (docs/CAREER_RACES.md)
TEST_DRIVE = (0x8291 << 16) - 29345  # F-64975's byte
EDITOR_PTR = (0x8287 << 16) - 6148  # the race editor object's word (an idle editor at 0x4000: outside the editor)
EDITOR_BYTE = (0x8287 << 16) - 6144
CODES = ['aNEW', 'aTDV', 'tTST', 'tMTC', 'tUSR', 'gCOL', 'gCOX', 'gCO', 'GCOL', 'kARS', 'kFWN', 'k$WA', 'nkDS', 'oXXX', 'vWIN',
         'xRRT', 'xCDL', 'bABC', 'zZZZ', 'sABC', 'cABC', 'dABC', 'eABC', 'fABC', 'hABC', 'iABC', 'jABC', 'lABC', 'mABC', 'pABC',
         'qABC', 'rABC', 'uABC', 'wABC', 'yABC', 'AXYZ', '1ABC']


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    addr = lambda fid: subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]
    f = addr('F-79933')
    look, add = 'F-55011', 'F-45364'
    nl = chr(10)
    recomp_closure.EXTRA += 'static int g_race, g_written;' + nl
    stubs = {
        'F-27863': 'ctx.r3.u32=g_race?0x5000:0;',
        'F-24338': 'ctx.r3.u32=0;',
        look: 'ctx.r3.u32=0x5800;',
        'F-24248': 'for(int i=0;i<4;i++)*pointer(ctx.r4.u32+i)=*pointer(ctx.r3.u32+i);*pointer(ctx.r4.u32+4)=0;ctx.r3.u32=ctx.r4.u32;',
        add: 'g_written=1;ctx.r3.u32=1;',
    }
    stubs = {'sub_' + addr(k): v for k, v in stubs.items()}
    # (not reached in these cases - the on-line players' loop, the record's creation, the other operations: fail loudly)
    for x in ('F-82714', 'F-10936', 'F-80717', 'F-42997'):
        stubs.setdefault('sub_' + addr(x), 'throw std::runtime_error("unexpected");')
    cs = [(c, g, t, r) for c in CODES for g in range(3) for t in range(2) for r in range(2)]
    rows = (',' + nl).join('{"%s",%d,%d,%d}' % c for c in cs)
    body = ('struct K{const char*c;int g,t,r;};static const K C[]={' + nl + rows + nl + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));for(int o=-9;o<8;o++)*pointer(0x%X+o)=0;' % CHEAT +
            'if(c.g==1)*pointer(0x%X)=1;if(c.g==2)*pointer(0x%X)=1;*pointer(0x%X)=c.t;store(0x%X,0x4000);*pointer(0x%X)=0;' % (CHEAT, CHEAT - 9, TEST_DRIVE, EDITOR_PTR, EDITOR_BYTE) + nl +
            'g_race=c.r;store(0x5000+3152,1);g_written=0;for(int i=0;i<4;i++)*pointer(0x3000+i)=c.c[i];*pointer(0x3000+8)=0;' + nl +
            'Context ctx{};ctx.r1.u32=0x7F00;ctx.r3.u32=0x6000;ctx.r4.u32=0x3000;sub_' + f + '(ctx,nullptr);printf("%d' + chr(92) + 'n",g_written);}' + nl + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + f], body, a.out_dir, 'stat_refusal', stubs=stubs, scratch_end=0x8000)
    out = run(exe, a.image).split(nl)[:len(cs)]
    assert len(out) == len(cs)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'stat_refusal_ref.txt').write_text(''.join('%s %d %d %d -> %s' % (*c, o) + nl for c, o in zip(cs, out)))
    print(len(cs), 'cases', sum(o == '1' for o in out), 'written')


if __name__ == '__main__':
    main()
