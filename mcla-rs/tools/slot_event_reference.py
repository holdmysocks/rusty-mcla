"""A start scene's racer-slot event run offline: mcCineScript::Events_LaunchEventOnRacerSlot F-51323 (name, slot, start,
duration, third float), the generated code with the event type look-up F-49543 (the 97-name table, first entry the name
contains) and the C runtime's strstr F-96345 / upper-case F-94870. The scene name buffer (the cine manager's current
scene, short form) is written per case. Stubbed: the event node pool's take F-74772 (answers 0x3000) and give-back
F-77783 (logs "free"), the generic event's set-up F-21508 (logs type, racer, start, duration, third float), the track's
add F-92568, the racer by id F-51196 (0x5000 + 16 id for the ids the case has racers for, else 0), the error log F-47121,
the formatter F-32585 (host snprintf, one %s argument) and the upper-case's null-name error calls. The manager (the word
at 0x8286D8F0) at 0x1000 with the slot ids at +3812 + 4 slot.
Writes <out-dir>/slot_event_ref.txt: "scene|event|slot t dur ids racers -> type racer t dur f3" (- when no event is
made; "free" after it when the node was given back)."""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

_shim = recomp_closure.shim
recomp_closure.shim = lambda *a: (_shim(*a).replace('uint16_t u16; uint8_t u8;', 'uint16_t u16; int16_t s16; uint8_t u8; int8_t s8;', 1)
                                  .replace('Condition cr0,cr6;', 'Condition cr0,cr1,cr5,cr6,cr7;', 1))

SCENES = ['racestart/gen_prerace_taunt_H_01', 'racestart/cut_racestart_3x3_01', 'racestart/cut_redlight_01', 'RaceStart/taunt_01',
          'story/cut_story_hollywood_garage_intro', 'xracestartx', 'racestar', '']
EVENTS = ['gesture_neutral', 'gesture_left', 'gesture_right', 'gesture_talk_left', 'gesture_talk_right', 'gesture_listen_left',
          'gesture_listen_right', 'gesture_thm_dwn_lft', 'gesture_thm_dwn_rgt', 'gesture_copfidget', 'window_down_left',
          'window_up_right', 'playspeech', 'horn', 'bw_toggle_off', 'Gesture_Left', 'GESTURE_RIGHT', 'my_gesture_left_2',
          'gesture_right_gesture_left', 'gesture_neutral_left', 'gesture_lef', 'bogus', '']
TIMES = [-1.0, 0.0, 0.25, 0.5, 0.99999994, 1.0, 1.0000001, 1.5, 2.33333, 4.36667]


def f32(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    addr = lambda fid: subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]
    f = addr('F-51323')
    rd = lambda reg, v: 'char %s[200];for(int i=0;i<199;i++){%s[i]=*pointer(ctx.%s.u32+i);if(!%s[i])break;}%s[199]=0;' % (v, v, reg, v, v)
    stubs = {
        'F-74772': 'ctx.r3.u32=0x3000;',
        'F-77783': 'printf(" free");',
        'F-21508': 'printf("%d %d %.9g %.9g %.9g",ctx.r4.s32,ctx.r5.u32?(int)((ctx.r5.u32-0x5000)/16):-1,(float)ctx.f1.f64,(float)ctx.f2.f64,(float)ctx.f3.f64);g_made=1;',
        'F-92568': '',
        'F-51196': 'int s=ctx.r3.s32;ctx.r3.u32=(s>=0&&s<8&&((g_racers>>s)&1))?0x5000+16*s:0;',
        'F-47121': '',
        'F-32585': rd('r5', 'fm') + 'char ar[200]="";if(strstr(fm,"%s")){' + rd('r6', 'x') + 'strcpy(ar,x);}'
                   'char o[260];snprintf(o,sizeof o,fm,ar);for(int i=0;i<(int)ctx.r4.u32;i++){*pointer(ctx.r3.u32+i)=o[i];if(!o[i])break;}',
        # (the upper-case's errno / invalid-parameter calls, on its null-name path only)
        'F-52205': '',
        'F-37557': '',
    }
    stubs = {'sub_' + addr(k): v for k, v in stubs.items()}
    recomp_closure.EXTRA += 'static uint32_t g_racers,g_made;' + chr(10)
    rnd = random.Random(51323)
    cs = []
    for sc in SCENES:
        for ev in EVENTS:
            for t in rnd.sample(TIMES, 4):
                slot = rnd.choice((0, 1, 1, 0, 2, 3))
                ids = [rnd.choice((-1, 0, 1, 2, 3, 3)) for _ in range(4)]
                cs.append((sc, ev, slot, f32(t), f32(rnd.choice((0.0, 0.0, 1.4, 7.33333))), ids, rnd.choice((0xF, 0xF, 0x1, 0x2, 0xA, 0))))
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = (',' + nl).join('{"%s","%s",%d,(float)%.9g,(float)%.9g,{%d,%d,%d,%d},%du}' % (sc, ev, s, t, d, *ids, r) for sc, ev, s, t, d, ids, r in cs)
    body = ('struct K{const char*sc;const char*ev;int slot;float t,d;int ids[4];uint32_t r;};static const K C[]={' + nl + rows + nl + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'store(0x8286D8F0,0x1000);' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));for(int k=0;k<4;k++)store(0x1000+3812+4*k,c.ids[k]);g_racers=c.r;g_made=0;'
            'int l=strlen(c.sc);for(int j=0;j<=l;j++)*pointer(0x828CD3B0+j)=c.sc[j];'
            'l=strlen(c.ev);for(int j=0;j<=l;j++)*pointer(0x2000+j)=c.ev[j];'
            'Context ctx{};ctx.r1.u32=0x7F00;ctx.r3.u32=0x2000;ctx.r4.u32=c.slot;ctx.f1.f64=c.t;ctx.f2.f64=c.d;ctx.f3.f64=0.5;'
            'sub_' + f + '(ctx,nullptr);if(!g_made)printf("-");printf("' + NL + '");}' + nl + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + f], body, a.out_dir, 'slot_event', stubs=stubs, scratch_end=0x8000)
    out = [x.strip() for x in run(exe, a.image).split(nl) if x.strip()]
    assert len(out) == len(cs), (len(out), len(cs))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'slot_event_ref.txt').write_text(''.join('%s|%s|%d %.9g %.9g %s %d -> %s' % (sc, ev, s, t, d, ','.join(map(str, ids)), r, o) + nl
                                                          for (sc, ev, s, t, d, ids, r), o in zip(cs, out)))
    print(len(cs), 'cases')


if __name__ == '__main__':
    main()
