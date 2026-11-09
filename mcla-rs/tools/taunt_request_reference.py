"""The pre-race taunt request run offline: CineScript_RequestPreraceTauntScenarioAudio F-56020, the generated code with
the C runtime's upper-case and strstr, on scene names (as the table writes them, in other cases, made-up ones). Stubbed:
the formatter F-32585 (host snprintf, one %s argument), the racer by id F-51196 (answers 0x5000 + 16 id for the ids the
case has racers for, else 0) and the scenario prepare F-44767 (logs its name and the two racers), and the upper-case's
null-name error calls F-52205 / F-37557. The manager (the word
at 0x8286D8F0) at 0x1000 with the slot ids at +3812 / +3816. Writes <out-dir>/taunt_request_ref.txt: "name|slot0
slot1 racers -> scenario A B" (A, B the ids of the racers handed over, or - when nothing is requested)."""
import argparse
import random
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

_shim = recomp_closure.shim
# (strstr's extsb reads a register's low byte signed; the C runtime's compares use cr1, cr5, cr7)
recomp_closure.shim = lambda *a: (_shim(*a).replace('uint16_t u16; uint8_t u8;', 'uint16_t u16; int16_t s16; uint8_t u8; int8_t s8;', 1)
                                  .replace('Condition cr0,cr6;', 'Condition cr0,cr1,cr5,cr6,cr7;', 1))

NAMES = ['game/CineScripts/generated/RaceStart/sc_gen_prerace_taunt_HP_01_generated',
         'game/CineScripts/generated/RaceStart/sc_gen_prerace_taunt_PH_01_generated',
         'game/CineScripts/generated/RaceStart/sc_gen_prerace_taunt_H_01_generated',
         'game/cinescripts/generated/racestart/gen_prerace_taunt_hp_02_generated',
         'game/cinescripts/generated/racestart/gen_prerace_taunt_ph_02_generated',
         'game/cinescripts/generated/racestart/gen_prerace_grid_2x4_01_generated',
         'racestart/sc_gen_prerace_taunt_hp_01', 'racestart/gen_prerace_taunt_PH_02', 'racestart/gen_prerace_taunt_H_01',
         'racestart/gen_prerace_starter_2x4_01', 'racestart/sc_gen_prerace_grid_1x1_01', 'TAUNT', 'taunt', 'Taunt_hp', 'taunt_ph',
         'taunt_php', 'taunt_hph', 'ph_taunt', 'hp_taunt', 'xhpx/taunt', 'phtaunthp', 'TAUN_HP', 'tauntP_H', 'gen_prerace_taunt_h_02',
         'gen_prerace_taunt_mild_01', '']


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    addr = lambda fid: subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]
    f = addr('F-56020')
    rd = lambda reg, v: 'char %s[200];for(int i=0;i<199;i++){%s[i]=*pointer(ctx.%s.u32+i);if(!%s[i])break;}%s[199]=0;' % (v, v, reg, v, v)
    stubs = {
        'F-32585': rd('r5', 'fm') + 'char ar[200]="";if(strstr(fm,"%s")){' + rd('r6', 'x') + 'strcpy(ar,x);}'
                   'char o[260];snprintf(o,sizeof o,fm,ar);for(int i=0;i<(int)ctx.r4.u32;i++){*pointer(ctx.r3.u32+i)=o[i];if(!o[i])break;}',
        'F-51196': 'int s=ctx.r3.s32;ctx.r3.u32=(s>=0&&s<8&&((g_racers>>s)&1))?0x5000+16*s:0;',
        # (the C runtime upper-case's errno / invalid-parameter calls, on its null-name path only)
        'F-52205': '',
        'F-37557': '',
        'F-44767': rd('r3', 'n') + 'printf("%s %d %d",n,(int)((ctx.r5.u32-0x5000)/16),(int)((ctx.r4.u32-0x5000)/16));g_req=1;',
    }
    stubs = {'sub_' + addr(k): v for k, v in stubs.items()}
    recomp_closure.EXTRA += 'static uint32_t g_racers,g_req;' + chr(10)
    rnd = random.Random(56020)
    cs = []
    for n in NAMES:
        for _ in range(12):
            s0, s1 = rnd.choice((-1, 0, 1, 2, 3)), rnd.choice((-1, 0, 1, 2, 3))
            cs.append((n, s0, s1, rnd.choice((0xF, 0xF, 0xF, 0x1, 0x2, 0x5, 0xA, 0))))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = (',' + nl).join('{"%s",%d,%d,%du}' % c for c in cs)
    body = ('struct K{const char*n;int s0,s1;uint32_t r;};static const K C[]={' + nl + rows + nl + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'store(0x8286D8F0,0x1000);' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));store(0x1000+3812,c.s0);store(0x1000+3816,c.s1);g_racers=c.r;g_req=0;'
            'int l=strlen(c.n);for(int j=0;j<=l;j++)*pointer(0x2000+j)=c.n[j];'
            'Context ctx{};ctx.r1.u32=0x7F00;ctx.r3.u32=0x2000;sub_' + f + '(ctx,nullptr);if(!g_req)printf("-");printf("' + NL + '");}' + nl + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + f], body, a.out_dir, 'taunt_request', stubs=stubs, scratch_end=0x8000)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cs)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'taunt_request_ref.txt').write_text(''.join('%s|%d %d %d -> %s' % (n, s0, s1, r, o) + nl for (n, s0, s1, r), o in zip(cs, out)))
    print(len(cs), 'cases')


if __name__ == '__main__':
    main()
