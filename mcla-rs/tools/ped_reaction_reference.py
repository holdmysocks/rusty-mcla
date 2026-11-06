"""A walker's reaction sound picked as the original does it: F-39461 run offline, unchanged (vector mode), over random
kinds, bike bytes, type sexes and sound objects. Stubbed: the entity pool F-48911 (a free entity whose +80 is null),
the object lookup F-86504 (the case's object for the type's szAudioType string, the case's fallback object for the
AMBIENT_PED_FEMALE_01 / _MALE_01 strings; each lookup recorded T / F / M), the log F-47121, the walker's matrix F-33765
and place F-58860, F-65914, and the sound start F-21756 (records the sound word it is given). The player entity
(the global at 0x8288D740) +1945 is the case's bike byte; the override object (0x82909F84) is 0, as the game leaves it
unless a debug name is set.
Writes <out-dir>/ped_reaction_ref.txt: the case's words (kind, bike byte, female byte, type string given, own object,
fallback object, then the two objects' 34 bytes as 9 words each) '->' the lookups and the sound word, or X when the
original reads through a null object."""
import argparse
import random
from pathlib import Path
from car_dodge_reference import addr, word
import recomp_closure
from recomp_closure import index, build, run

recomp_closure.EXTRA += 'static uint32_t SOUND;static bool PLAYED;' + chr(10)

PED, TYPE, TSTR, ENT, MAT, POS, PENT, OBJ, FB = 0x10000, 0x10400, 0x10800, 0x11000, 0x11400, 0x11500, 0x12000, 0x13000, 0x13100
NOW = 6 + 18


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=4000)
    a = ap.parse_args()
    ids = ('F-39461', 'F-48911', 'F-86504', 'F-47121', 'F-33765', 'F-58860', 'F-65914', 'F-21756')
    A = {f: addr(a.tool, f) for f in ids}
    rnd = random.Random(39461)
    cases = []
    for _ in range(a.cases):
        c = [rnd.randrange(0, 4), rnd.choice((0, 0, 1, 1, 2, 0xFF)), rnd.choice((0, 0, 1, 1, 0x80)),
             rnd.random() < 0.9, rnd.random() < 0.8, rnd.random() < 0.7]
        if not (c[3] and c[4]) and not c[5]:
            c[5] = rnd.random() < 0.8  # (mostly an object to read: a null one is the original's fault)
        for _o in range(2):
            c += [rnd.getrandbits(32) for _ in range(9)]
        cases.append(c)
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    stubs = {
        'sub_' + A['F-48911']: 'ctx.r3.u64=0x%x;' % ENT,
        'sub_' + A['F-86504']: ('if(ctx.r4.u32==0x%x){LOG+="T";ctx.r3.u64=KC[4]?0x%x:0;}'
                                'else if(ctx.r4.u32==0x820C3220){LOG+="F";ctx.r3.u64=KC[5]?0x%x:0;}'
                                'else if(ctx.r4.u32==0x820C3238){LOG+="M";ctx.r3.u64=KC[5]?0x%x:0;}'
                                'else throw std::runtime_error("lookup");') % (TSTR, OBJ, FB, FB),
        'sub_' + A['F-47121']: '',
        'sub_' + A['F-33765']: 'ctx.r3.u64=0x%x;' % MAT,
        'sub_' + A['F-58860']: 'ctx.r3.u64=0x%x;' % POS,
        'sub_' + A['F-65914']: '',
        'sub_' + A['F-21756']: 'SOUND=ctx.r4.u32;PLAYED=true;',
    }
    setup = (
        'memset(objects,0,sizeof(objects));LOG.clear();KC=c;SOUND=0;PLAYED=false;'
        'store(0x%x+448,0x%x);store(0x%x+176,c[3]?0x%x:0);*pointer(0x%x+109)=uint8_t(c[2]);'
        % (PED, TYPE, TYPE, TSTR, TYPE) +
        '*pointer(0x%x+1945)=uint8_t(c[1]);put(0x8288D740,0x%x);put(0x82909F84,0);' % (PENT, PENT) +
        'for(int k=0;k<9;k++){store(0x%x+4*k,c[6+k]);store(0x%x+4*k,c[15+k]);}' % (OBJ, FB)
    )
    body = ('static const uint32_t C[][%d]={' % NOW + nl + rows + nl + '};' + nl +
            'for(auto&c:C){' + setup +
            'Context ctx{};ctx.r1.u32=0x7FF00;ctx.r13.u32=0x7E000;ctx.r3.u32=0x%x;ctx.r4.u32=c[0];' % PED +
            'bool x=false;try{sub_%s(ctx,nullptr);}catch(std::exception&e){x=true;}' % A['F-39461'] +
            'if(x)printf("%s X' + NL + '",LOG.c_str());else printf("%s %08x %d' + NL + '",LOG.c_str(),SOUND,int(PLAYED));}' + nl)
    main_src = ('static void dispatch(Context&,const uint8_t*,uint32_t){throw std::runtime_error("unexpected indirect call");}' + nl +
                'static void put(uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;}' + nl +
                'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + A['F-39461']], main_src, a.out_dir, 'ped_reaction', stubs=stubs, vector=True)
    res = [x for x in run(exe, a.image).split(nl) if x]
    assert len(res) == len(cases), (len(res), len(cases))
    lines = []
    played = crashed = 0
    for c, o in zip(cases, res):
        t = o.split()
        if t[-1] == 'X':
            crashed += 1
        else:
            assert t[-1] == '1', o
            played += 1
        out = t[0] + ' ' + ('X' if t[-1] == 'X' else word(int(t[1], 16)))
        lines.append(' '.join(word(x) for x in c) + ' -> ' + out)
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'ped_reaction_ref.txt').write_text(nl.join(lines) + nl)
    print(len(cases), 'cases;', played, 'played,', crashed, 'read a null object')


if __name__ == '__main__':
    main()
