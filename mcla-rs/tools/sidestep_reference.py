"""The walkers' sidestep run offline: F-26402 whole (the 12 contact slots, the sidestep probe F-29590 with F-41334, the
probe query's ctor F-13499, the guest's sin / cos, and the slide start F-83234 with the evade clip pick F-37443 and
F-32060 inside it) - the generated code whole and unchanged in vector mode, on flat guest memory, with the SDK's flush
mode (the same build as car_dodge_reference.py, whose patches this imports). Also F-29590 called on its own.
Stubbed, as inputs of the case, and recorded in order (tag, words): the walker's forward F-50349 (a getter, not
recorded), the swept ball
F-27454 (t3: from, to, radius, an object left out; type 640, mask 0x381E, byte 7, the walker's instance and the
slot's object checked), the mover sample F-21043 (t6), the speech F-39461 (t7), the temp rails F-99671 (t8) and the
path F-50795 (t9); the walker's matrix F-33765.
Writes <out-dir>/sidestep_ref.txt: per case the input words, '->', the outputs (mode 3: the result, the avoidance part's
words, the two MWCs; mode 4: the result and the direction) and the record. Words as hex; a word whose top byte is 0x82
as two 16-bit halves hhhh_llll."""
import argparse
import math
import random
from pathlib import Path
import car_dodge_reference as cdr
from car_dodge_reference import addr, fb, f32, word
from recomp_closure import index, build, run

NSWEEP = 40
NENT = 6
ENTW = 11  # type, dir, audio, clip present, duration, move at t xyz, move at the end xyz
NSLOT = 12
SLW = 8  # object present, archetype present, type flags, n xyz, n.w, depth
P = {}
_n = 0
for name, w in (('mode', 1), ('p', 3), ('r', 1), ('u', 3), ('rows', 9), ('female', 1), ('flags', 1), ('mdir', 3),
                 ('clip', 1), ('time', 1), ('len', 1), ('yaw', 1), ('stretch', 1), ('t', 1), ('stop', 3), ('from', 3),
                 ('to', 3), ('mwc', 4), ('fwd', 3), ('sw', NSWEEP), ('nent', 1), ('ent', ENTW * NENT),
                 ('slot', SLW * NSLOT), ('x', 5)):
    P[name] = _n
    _n += w
W = _n


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--cases', type=int, default=3000)
    a = ap.parse_args()
    ids = ('F-26402', 'F-29590', 'F-83234', 'F-50349', 'F-27454', 'F-33765', 'F-21043', 'F-39461', 'F-99671', 'F-50795')
    A = {f: addr(a.tool, f) for f in ids}
    rnd = random.Random(26402)

    def unit(v):
        l = math.sqrt(sum(x * x for x in v))
        return [x / l for x in v] if l > 0 else v

    def vec(v):
        return [fb(f32(x)) for x in v]

    steep = f32(0.98)
    cases = []
    for ci in range(a.cases):
        mode = 3 if ci < a.cases * 4 // 5 else 4
        c = [0] * W
        c[P['mode']] = mode
        pw = [rnd.uniform(-2000, 2000), rnd.uniform(-10, 40), rnd.uniform(-2000, 2000)]
        r = rnd.choice((0.3, 0.35, rnd.uniform(0.2, 0.5)))
        u = unit([rnd.uniform(-0.2, 0.2), 1.0, rnd.uniform(-0.2, 0.2)]) if rnd.random() < 0.5 else [0.0, 1.0, 0.0]
        if rnd.random() < 0.03:
            u = rnd.choice(([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]))
        h = rnd.uniform(-math.pi, math.pi)
        wa, wu, wc = [math.cos(h), 0.0, -math.sin(h)], [0.0, 1.0, 0.0], [math.sin(h), 0.0, math.cos(h)]
        c[P['p']:P['p'] + 3] = vec(pw)
        c[P['r']] = fb(f32(r))
        c[P['u']:P['u'] + 3] = vec(u)
        c[P['rows']:P['rows'] + 9] = vec(wa + wu + wc)
        c[P['female']] = rnd.getrandbits(1)
        c[P['flags']] = rnd.choice((0, 0, 0, 0, 0, 4, 2, 6)) | rnd.choice((0, 0, 1, 8, 0x40))
        c[P['mdir']:P['mdir'] + 3] = vec(unit([rnd.uniform(-1, 1), 0.0, rnd.uniform(-1, 1)]))
        nent = rnd.choice((0, 2, 4, 6))
        c[P['nent']] = nent
        with_clip = []
        for i in range(NENT):
            e = P['ent'] + ENTW * i
            pres = int(rnd.random() < 0.85)
            dur = rnd.choice((rnd.uniform(0.3, 3.0), 0.2, 1.0))
            c[e:e + ENTW] = [rnd.choice((0, 1, 2, 3)), rnd.choice((0, 1, 2)), rnd.choice((0, 1, 2)), pres, fb(f32(dur))] + \
                vec([rnd.uniform(-1, 1) for _ in range(3)]) + vec([rnd.uniform(-2, 2) for _ in range(3)])
            if pres and i < nent:
                with_clip.append(i)
        c[P['clip']] = rnd.choice(with_clip) if with_clip and rnd.random() < 0.3 else 0xFFFFFFFF
        for k in ('time', 'len', 'yaw', 'stretch', 't'):
            c[P[k]] = fb(f32(rnd.uniform(0, 1.5)))
        for k in ('stop', 'from', 'to'):
            c[P[k]:P[k] + 3] = vec([pw[i] + rnd.uniform(-2, 2) for i in range(3)])
        c[P['mwc']:P['mwc'] + 4] = [rnd.getrandbits(32) for _ in range(4)]
        c[P['fwd']:P['fwd'] + 3] = vec([-x for x in wc]) if rnd.random() < 0.8 else vec(unit([rnd.uniform(-1, 1), rnd.uniform(-0.2, 0.2), rnd.uniform(-1, 1)]))
        ph = rnd.choice((0.2, 0.5, 0.8, 1.0))
        for k in range(NSWEEP):
            c[P['sw'] + k] = int(rnd.random() < ph)
        for k in range(NSLOT):
            s = P['slot'] + SLW * k
            obj = int(rnd.random() < 0.6)
            arch = int(rnd.random() < 0.95)
            fl = rnd.choice((0, 0x2, 0x8, 0x10, 0x20, 0x1000, 0x800, 0x4, 0x1, 0x40, 0x103A, 0x200, 0x2 | 0x800, 0x4000))
            ny = rnd.choice((rnd.uniform(-0.3, 0.3), rnd.uniform(-1, 1), steep, -steep, rnd.choice((1.0, -1.0)),
                             f32(0.98 + 1e-7), rnd.uniform(0.97, 0.99)))
            hz = rnd.uniform(-math.pi, math.pi)
            sxz = math.sqrt(max(0.0, 1 - ny * ny))
            n = [sxz * math.cos(hz), ny, sxz * math.sin(hz)]
            if rnd.random() < 0.05:
                n = [0.0, 0.0, 0.0]
            depth = rnd.choice((rnd.uniform(0, 0.6), 0.0, rnd.uniform(-0.1, 0.0)))
            c[s:s + SLW] = [obj, arch, fl] + vec(n) + [fb(f32(rnd.uniform(-1, 1))), fb(f32(depth))]
        if mode == 4:
            dv = rnd.choice((unit([rnd.uniform(-1, 1), rnd.uniform(-1, 1), rnd.uniform(-1, 1)]), [0.0, 1.0, 0.0],
                             [1e-4, 0.5, -1e-4], [rnd.uniform(-3, 3), 0.0, rnd.uniform(-3, 3)]))
            c[P['x']:P['x'] + 5] = vec(dv) + [fb(f32(rnd.uniform(0.0, 1.0))), rnd.getrandbits(1)]
        cases.append(c)
    nl = chr(10)
    NL = chr(92) + 'n'
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    # walker I 0x2000 (+112 its instance, place +176, slots +2240), type 0x3400, matrix 0x3500; M 0x1000; slot objects
    # 0x19000 + 0x40 k, their archetypes 0x19800 + 0x40 k; avoid entries 0x13000, clips 0x13400, the list 0x13F00
    setup = (
        'memset(objects,0,sizeof(objects));LOG.clear();KC=c;NSW=0;memset(MC,0,sizeof(MC));'
        'auto v3=[&](uint32_t a,int k,uint32_t w){store(a,c[k]);store(a+4,c[k+1]);store(a+8,c[k+2]);store(a+12,w);};'
        'store(0x2000+448,0x3400);objects[0x3400-0x1000+109]=c[%d]?1:2;' % P['female'] +
        'store(0x2000+492,c[%d]);v3(0x2000+4768,%d,0);v3(0x2000+176,%d,0x3f800000);' % (P['r'], P['u'], P['p']) +
        'v3(0x3500,%d,0);v3(0x3510,%d,0);v3(0x3520,%d,0);v3(0x3530,%d,0x3f800000);' % (P['rows'], P['rows'] + 3, P['rows'] + 6, P['p']) +
        'store(0x1000+224,0x2000);store(0x1000+260,c[%d]);v3(0x1000+128,%d,0);' % (P['flags'], P['mdir']) +
        'store(0x1000+248,c[%d]==0xFFFFFFFFu?0:0x13000+20*c[%d]);' % (P['clip'], P['clip']) +
        'store(0x1000+240,c[%d]);store(0x1000+244,c[%d]);store(0x1000+252,c[%d]);store(0x1000+256,c[%d]);store(0x1000+236,c[%d]);'
        % (P['time'], P['len'], P['yaw'], P['stretch'], P['t']) +
        'v3(0x1000+96,%d,0);v3(0x1000+160,%d,0);v3(0x1000+176,%d,0);' % (P['stop'], P['from'], P['to']) +
        'put(0x82860880,c[%d]);put(0x82860884,c[%d]);put(0x82860890,c[%d]);put(0x82860894,c[%d]);'
        % (P['mwc'], P['mwc'] + 1, P['mwc'] + 2, P['mwc'] + 3) +
        'store(0x14000+0x64228,0x13F00);store(0x13F00+8,0x13000);store16(0x13F00+12,c[%d]);' % P['nent'] +
        'for(int i=0;i<%d;i++){const uint32_t*e=c+%d+%d*i;store(0x13000+20*i+4,e[0]);store(0x13000+20*i+8,e[1]);'
        'store(0x13000+20*i+12,e[2]);store(0x13000+20*i+16,e[3]?0x13400+32*i:0);store(0x13400+32*i+12,e[4]);}'
        % (NENT, P['ent'], ENTW) +
        'for(int k=0;k<%d;k++){const uint32_t*s=c+%d+%d*k;uint32_t sl=0x2000+2240+96*k,o=0x19000+0x40*k,ar=0x19800+0x40*k;'
        'store(sl,s[0]?o:0);store(o+4,s[1]?ar:0);store(ar+16,s[2]);store(sl+32,s[3]);store(sl+36,s[4]);store(sl+40,s[5]);'
        'store(sl+44,s[6]);store(sl+68,s[7]);}' % (NSLOT, P['slot'], SLW) +
        'store(0x16000,0x16100);'
    )
    stub = {
        A['F-50349']: ('if(ctx.r4.u32!=0x2000)err(10);uint32_t o=ctx.r3.u32;store(o,KC[%d]);store(o+4,KC[%d]);'
                       'store(o+8,KC[%d]);store(o+12,0);' % (P['fwd'], P['fwd'] + 1, P['fwd'] + 2)),
        A['F-27454']: ('uint32_t q=ctx.r3.u32;if(load(q+16)!=640||load(q+20)!=0x381E||*pointer(q+24)!=7||load(q+8)!=0x2070'
                       '||load(q+4)!=(load(q+12)?2u:1u)||ctx.r4.u32!=1)err(3);'
                       'logt(3);for(int k=0;k<3;k++)lw(load(q+32+4*k));for(int k=0;k<3;k++)lw(load(q+48+4*k));lw(load(q+64));'
                       'lw(load(q+12)?(load(q+12)-0x19000)/0x40+1:0);'
                       'if(NSW>=%d)err(11);else store(q+112,KC[%d+NSW++]?0x12000:0);' % (NSWEEP, P['sw'])),
        A['F-33765']: 'ctx.r3.u32=0x3500;',
        A['F-21043']: ('int i=(ctx.r3.u32-0x13400)/32;float t=float(ctx.f1.f64);uint32_t tb;memcpy(&tb,&t,4);logt(6);lw(i);lw(tb);'
                       'const uint32_t*e=KC+%d+%d*i;int k=MC[i]++?1:0;store(ctx.r7.u32+16,e[5+3*k]);store(ctx.r7.u32+20,e[6+3*k]);'
                       'store(ctx.r7.u32+24,e[7+3*k]);store(ctx.r7.u32+28,0);ctx.r3.u32=1;' % (P['ent'], ENTW)),
        A['F-39461']: 'if(ctx.r3.u32!=0x2000)err(7);logt(7);lw(ctx.r4.u32);',
        A['F-99671']: 'if(ctx.r3.u32!=0x17000||ctx.r4.u32||ctx.r5.u32)err(8);logt(8);',
        A['F-50795']: 'if(ctx.r3.u32!=0x17100)err(9);logt(9);',
    }
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){throw std::runtime_error("unexpected indirect call");}')
    x = P['x']
    pr = 'auto PW=[](uint32_t w){printf(" %08x",w);};auto PV=[&](uint32_t a){PW(load(a));PW(load(a+4));PW(load(a+8));};'
    out_m = ('{uint32_t o=load(0x1000+80);PW(o);}'
             'PV(0x1000+96);PV(0x1000+128);PV(0x1000+160);PV(0x1000+176);PW(load(0x1000+236));PW(load(0x1000+240));'
             'PW(load(0x1000+244));{uint32_t e=load(0x1000+248);PW(e?(e-0x13000)/20:0xFFFFFFFFu);}PW(load(0x1000+252));'
             'PW(load(0x1000+256));PW(load(0x1000+260));PW(get(0x82860880));PW(get(0x82860884));PW(get(0x82860890));'
             'PW(get(0x82860894));')
    body = ('static const uint32_t C[][%d]={' % W + nl + rows + nl + '};' + nl +
            'auto put=[](uint32_t a,uint32_t v){uint8_t*p=image.data()+(a-0x82000000u);p[0]=v>>24;p[1]=v>>16;p[2]=v>>8;p[3]=v;};'
            'auto get=[](uint32_t a){const uint8_t*p=image.data()+(a-0x82000000u);return uint32_t(p[0])<<24|p[1]<<16|p[2]<<8|p[3];};'
            'for(int k=0;k<4;k++)put(0x828609E0+4*k,0x7F800000);put(0x8286D8E4,0x14000);' + pr + nl +
            'for(auto&c:C){' + setup +
            'Context ctx{};ctx.r1.u32=0x1FF00;ctx.r13.u32=0x16000;ctx.r3.u32=0x1000;' +
            ('if(c[0]==3){ctx.r4.u32=0x17000;ctx.r5.u32=0x17100;store(0x1000+80,0xABCD);'
             'try{sub_%s(ctx,nullptr);}catch(std::exception&e){LOG+=" EXC";}' % A['F-26402']) +
            ('}else{ctx.r4.u32=c[%d+4]?0x19000:0;store(0x3700,c[%d]);store(0x3704,c[%d]);store(0x3708,c[%d]);store(0x370C,0);'
             'float d;memcpy(&d,&c[%d],4);ctx.f1.f64=d;ctx.r5.u32=0x3700;sub_%s(ctx,nullptr);}'
             % (x, x, x + 1, x + 2, x + 3, A['F-29590'])) +
            'if(c[0]==4){PW(ctx.r3.u32&0xFF);PV(0x3700);}'
            'else{PW(load(0x1000+80)==0xABCD);' + out_m + '}printf("%s' + NL + '",LOG.c_str());}' + nl)
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    stubs = {'sub_' + k: v for k, v in stub.items()}
    names = ['sub_' + A[f] for f in ('F-26402', 'F-29590', 'F-83234')]
    exe = build(idx, names, main_src, a.out_dir, 'sidestep', stubs=stubs, vector=True)
    res = [x for x in run(exe, a.image).split(nl) if x]
    assert len(res) == len(cases), (len(res), len(cases))
    bad = [i for i, o in enumerate(res) if 'ERR' in o or 'EXC' in o]
    assert not bad, (len(bad), res[bad[0]])
    fix = lambda o: ' '.join(word(int(t, 16)) if len(t) == 8 and not t.startswith('t') else t for t in o.split())
    lines = [' '.join(word(x) for x in c) + ' -> ' + fix(o) for c, o in zip(cases, res)]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'sidestep_ref.txt').write_text(nl.join(lines) + nl)
    m3 = [(c, o) for c, o in zip(cases, res) if c[0] == 3]
    slid = sum(' t8' in o for _, o in m3)
    print(len(lines), 'cases:', len(m3), 'sidesteps run (slides', slid, ', sweeps', sum(o.count(' t3') for _, o in m3),
          ', speech', sum(' t7' in o for _, o in m3), ',',
          'skipped by the flags', sum(c[P['flags']] & 6 != 0 for c, _ in m3), '),', len(cases) - len(m3), 'probes alone (found',
          sum(o.split()[0] == '00000001' for c, o in zip(cases, res) if c[0] == 4), ')')


if __name__ == '__main__':
    main()
