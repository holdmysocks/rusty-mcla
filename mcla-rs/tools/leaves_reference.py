"""Leaf routines of the vehicle physics run offline: the generated code unchanged (vector mode, the recomp's own
-msse4.1 -mssse3) on flat guest memory. Each case line of <out-dir>/leaves_ref.txt is
"<tag> <input words> -> <output words>" (hex words, memory words in the guest's big-endian reading; a word that reads
82xxxxxx is written with an h in front). Sections (the tag, the row):

  sstdef    F-99345  mcCarSSTurbo defaults (the 80 byte object prefilled with random words)
  sstreset  F-21020  mcCarSSTurbo reset
  sstfire   F-68670  fire (camera / pad / stats callees are stubs; their gates drawn from the case)
  sstupd    F-60935  the update, burning and charging halves (the search F-69277 a stub answering from the case; the lying test
                     F-75615, the library pow F-21955 and the lens approach F-90102 real; the push F-14399 a stub recording its vector)
  dmgadd    F-15289  AddDamage (the callees stubs)
  kblock    F-72162  K block of a point row
  kinv      F-52431  K block, then inverted in place (F-68939 real)
  rowzero   F-90205  row multipliers reset (the row iterator F-67464 a stub walking a list from the case)
  step      F-70556  visit step
  relvel    F-52995  visit relative velocity (step real)
  edgebis   F-23797  edge bisector (the vertex fetch F-54719 a stub answering from the case)

Where a table is needed it is at 0x2800 with every slot 0x7F0000nn; `dispatch` maps the slots it needs. The guest vectors:
ctx.vN.u32[3-k] holds lane k. No denormal is generated for a vector input (the console flushes)."""
import argparse
import random
import struct
from pathlib import Path
import vehsim_small_reference as vs  # (patches recomp_closure's shim on import)
from vehsim_small_reference import addr, fb, word, fw, NL, nl
from recomp_closure import index, build, run
from cite_safe import cite_safe
import recomp_closure

recomp_closure.EXTRA += 'static uint32_t TYPES[8];' + chr(10)

VT = 'for(int k=0;k<64;k++)store(0x2800+4*k,0x7F000000+k);'
DT = '((0x827Du<<16)|0x7508u)'
GSQ = '((0x828Fu<<16)|0xF960u)'   # the global the step adds its square to
def unfb(w):
    return struct.unpack('>f', struct.pack('>I', w))[0]


R13 = 'store(0x7C00,0x7C40);ctx.r13.u32=0x7C00;'


def build_group(a, gname, gsecs, gstubs, gdisp, results, extra=()):
    if not gsecs:
        return
    body = ''
    for tag, cs, setup, fn, nout, outc in gsecs:
        w = max(len(c) for c in cs)
        rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c + [0] * (w - len(c))) for c in cs)
        body += ('{CN=0;static const uint32_t C[][%d]={%s%s%s};' % (w, nl, rows, nl) +
                 'for(auto&c:C){memset(objects,0,sizeof(objects));Context ctx{};ctx.r1.u32=0x7F00;ncalls=0;memset(REC,0,sizeof(REC));' + setup +
                 'try{sub_%s(ctx,nullptr);}catch(std::exception&e){fprintf(stderr,"case %%d: %%s",CN,e.what());throw;}CN++;' % fn +
                 'for(int i=0;i<%d;i++)printf("%%08x ",uint32_t(%s));printf("%s");fflush(stdout);}}' % (nout, outc, NL) + nl)
    names = ({'sub_' + s[3] for s in gsecs} | set('sub_' + e for e in extra)) - set(gstubs)
    main_src = ('static int CN=0;' + nl + 'static void store8(uint32_t a,uint8_t v){*pointer(a)=v;}' + nl + 'static uint8_t load8(uint32_t a){return *pointer(a);}' + nl +
                gdisp + nl +
                'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    exe = build(index(a.recomp), sorted(names), main_src, a.out_dir, 'leaves_' + gname, stubs=gstubs, vector=True, scratch_end=0x8000, flags=('-msse4.1', '-mssse3'))
    out = [x for x in run(exe, a.image).split(nl) if x]
    assert len(out) == sum(len(s[1]) for s in gsecs), (gname, len(out), sum(len(s[1]) for s in gsecs))
    k = 0
    for tag, cs, *_ in gsecs:
        for c in cs:
            results.setdefault(tag, []).append('%s %s -> %s' % (tag, ' '.join('%08x' % x for x in c), out[k].strip()))
            k += 1


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--only', default='')
    a = ap.parse_args()
    N = lambda fid: addr(a.tool, fid)
    rows = dict(sstdef='F-99345', sstreset='F-21020', sstfire='F-68670', sstupd='F-60935', dmgadd='F-15289', kblock='F-72162', kinv='F-52431',
                rowzero='F-90205', step='F-70556', relvel='F-52995', edgebis='F-23797')
    f = {k: N(v) for k, v in rows.items()}
    rnd = random.Random(60935)
    results = {}
    only = a.only.split(',') if a.only else None
    want = lambda *tags: only is None or any(t in only for t in tags)

    wmode = [0]

    def vec4(scale=10.0, w=None):
        v = [fw(rnd, scale) for _ in range(3)]
        return v + [fw(rnd, scale) if (w is None or (w == 0 and wmode[0])) else w]

    # ================================================================== SST
    if want('sstdef', 'sstreset', 'sstfire', 'sstupd'):
        # callees, named by their ids (the registry gives the address)
        G = {k: N(v) for k, v in dict(nitro_same='F-12574', push='F-14399', answering='F-14919', find='F-69277', player_index='F-55794',
                                      player='F-35412', shake='F-68650', unk_a='F-58287', pad_gate='F-86040', stats_obj='F-65049',
                                      stat_name='F-16038', lying='F-75615', pow='F-21955', approach='F-90102').items()}
        stubs = {
            'sub_' + G['nitro_same']: 'ctx.r3.u32=SV[1];',
            'sub_' + G['push']: '{for(int k=0;k<4;k++)REC[0][k]=ctx.v1.u32[3-k];REC[1][0]++;}',
            'sub_' + G['answering']: 'ctx.r3.u32=SV[2];',
            'sub_' + G['find']: '{ncalls++;if(ncalls==1){if(SV[7]){store(ctx.r4.u32,SV[8]);store(ctx.r5.u32,SV[9]);}ctx.r3.u32=SV[7];}else ctx.r3.u32=SV[10];}',
            'sub_' + G['player_index']: 'ctx.r3.u32=SV[3];',
            'sub_' + G['player']: 'ctx.r3.u32=0x6100;',
            'sub_' + G['shake']: '{REC[2][0]=FB(ctx.f1.f64);REC[2][1]=FB(ctx.f2.f64);REC[2][2]++;}',
            'sub_' + G['unk_a']: '',
            'sub_' + G['pad_gate']: 'ctx.r3.u32=SV[4];',
            'sub_' + G['stats_obj']: 'ctx.r3.u32=0x6300;',
            'sub_' + G['stat_name']: 'ctx.r3.u32=0x6400;',
        }
        disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){switch(a){'
                'case 0x7F00000B:ctx.r3.u32=SV[5];return;case 0x7F000007:REC[3][0]++;REC[3][1]=ctx.r4.u32;return;'
                'case 0x7F00000D:ctx.f1.f64=FD(SV[0]);return;case 0x7F00000C:ctx.r3.u32=SV[6];return;case 0x7F00000A:REC[3][2]++;return;'
                'default:throw std::runtime_error("unexpected indirect call");}}')
        secs = []
        pre20 = lambda: [rnd.getrandbits(32) for _ in range(20)]
        secs.append(('sstdef', [pre20() for _ in range(1000)], 'for(int k=0;k<20;k++)store(0x1000+4*k,c[k]);ctx.r3.u32=0x1000;', f['sstdef'], 20, 'load(0x1000+4*i)'))
        secs.append(('sstreset', [pre20() for _ in range(1000)], 'for(int k=0;k<20;k++)store(0x1000+4*k,c[k]);ctx.r3.u32=0x1000;', f['sstreset'], 20, 'load(0x1000+4*i)'))

        # fire: the object (20 words), mcCar present, +12, global block byte, answering (twice), pad gate, view mode, ...
        cs = []
        for _ in range(1000):
            c = pre20()
            c[2] = 0                                   # (+8 set below)
            c += [1, rnd.choice((0, 0, 0xFFFFFFFF, 1)), rnd.choice((0, 1)), rnd.choice((0, 1, 1)), rnd.choice((0, 1)), rnd.choice((0, 1, 2, 3)),
                  rnd.choice((0, 1, 1)), rnd.choice((0, 1))]
            cs.append(c)
        secs.append(('sstfire', cs,
                     'for(int k=0;k<20;k++)store(0x1000+4*k,c[k]);store(0x1000+8,c[20]?0x1100:0);store(0x1000+12,c[21]);store(0x1100+24,0x6600);store(0x6600,0x2800);' + VT +
                     'store((0x828Cu<<16)|0xCEA0u,0x6000);store8(0x6000+6,c[22]);SV[2]=c[23];SV[4]=c[24];store(0x6100+880,0x6200);store(0x6200+80,c[25]);SV[6]=c[26];' +
                     'store(0x6300,0x2800);store((0x828Cu<<16)|0xCF68u,0x6500);store(0x6500,0x2800);store(0x6400,0);ctx.r3.u32=0x1000;SV[3]=0;' + R13,
                     f['sstfire'], 26, '(i<20?load(0x1000+4*i):i==20?REC[2][2]:i==21?REC[2][0]:i==22?REC[2][1]:i==23?REC[3][0]:i==24?REC[3][1]:REC[3][2])'))

        # update: see the module docstring in the Rust test for the case layout
        names = ['burning', 'full', 'level', 'stay', 'fov', 'dist', 'blur', 'strength', 'drafter', 'leader', 'lock', 'dt', 'exp', 'amount', 'use', 'regen', 'falloff',
                 'cfalloff', 'stayfull', 'lose', 'fout', 'fin', 'tfov', 'tdist', 'bluroff', 'blurstart', 'speed', 'throttle', 'wheels', 'contacts', 'upy', 'air',
                 'row0', 'row1', 'row2', 'row3', 'mask', 'found', 's', 'idx', 'drafting', 'count', 'entries', 'pidx', 'trail', 'player']
        I = {n: i for i, n in enumerate(names)}
        cs = []
        for k in range(2000):
            burning = 1 if k % 2 == 0 else 0
            lvl = rnd.choice((fb(rnd.uniform(0, 1)), fb(rnd.uniform(0, 1)), fb(rnd.uniform(0.9, 1.01)), 0, 0x3f800000, 0x80000000, 0x7fc00000, fb(rnd.uniform(-0.2, 1.5)), fb(1e-3)))
            hasfound = rnd.choice((0, 1, 1))
            s = rnd.choice((fb(rnd.uniform(0, 1)), fb(rnd.uniform(0, 1)), fb(rnd.uniform(0, 1)), 0x3f800000, fb(1e-4), fb(rnd.uniform(0.9, 1.0)))) if hasfound else rnd.choice((0, 0x80000000, fb(rnd.uniform(-1, 0))))
            tune = {'exp': rnd.choice((0x40400000, fb(rnd.uniform(0.2, 4)), fb(rnd.uniform(1, 3)), 0x3f800000, 0x40000000, 0)),
                    'amount': rnd.choice((0x41c80000, fb(rnd.uniform(5, 40)), fw(rnd, 30.0))), 'use': rnd.choice((0x40200000, fb(rnd.uniform(0.5, 5)), fw(rnd, 5.0))),
                    'regen': rnd.choice((0x3fc00000, fb(rnd.uniform(0.5, 3)), fw(rnd, 3.0))), 'falloff': rnd.choice((0x41700000, fb(rnd.uniform(2, 30)), fw(rnd, 20.0))),
                    'cfalloff': rnd.choice((0x3f666419, fb(rnd.uniform(0.3, 3)), fw(rnd, 3.0))), 'stayfull': rnd.choice((0x3f99999a, fb(rnd.uniform(0, 3)), fw(rnd, 3.0))),
                    'lose': rnd.choice((0x41a00000, fb(rnd.uniform(5, 40)), fw(rnd, 40.0))), 'fout': rnd.choice((0x3f000000, fb(rnd.uniform(0.2, 2)), fw(rnd, 2.0))),
                    'fin': rnd.choice((0x3f800000, fb(rnd.uniform(0.2, 2)), fw(rnd, 2.0))), 'tfov': rnd.choice((0x41700000, fb(rnd.uniform(-20, 40)), fw(rnd, 30.0))),
                    'tdist': rnd.choice((0x3f800000, fb(rnd.uniform(0.5, 1.5)), fw(rnd, 2.0))), 'bluroff': rnd.choice((0x3e800000, fb(rnd.uniform(0.1, 1)), fw(rnd, 2.0))),
                    'blurstart': rnd.choice((0x3e800000, fb(rnd.uniform(0, 1)), fw(rnd, 2.0)))}
            c = {'burning': burning, 'full': rnd.choice((0, 1)), 'level': lvl, 'stay': rnd.choice((0, fb(rnd.uniform(0, 1.5)), fb(rnd.uniform(0, 0.05)), fw(rnd, 2.0), 0x80000000)),
                 'fov': rnd.choice((0, fb(rnd.uniform(0, 15)), 0x41700000, fw(rnd, 20.0), 0x80000000)), 'dist': rnd.choice((0x3f800000, fb(rnd.uniform(0.5, 1.2)), fw(rnd, 2.0))),
                 'blur': rnd.choice((0x3f800000, fb(rnd.uniform(0, 1)), fw(rnd, 2.0))), 'strength': 0xA5A5A5A5, 'drafter': rnd.choice((0xFFFFFFFF, 0, 1, 2, 3, 5)),
                 'leader': rnd.choice((0xFFFFFFFF, 0, 1, 2, 3, 5)), 'lock': rnd.choice((0, 0, 0, 0x10, 0x08, 0x18)),
                 'dt': rnd.choice((0x3c888889, fb(1 / 30.0), fb(rnd.uniform(0.001, 0.05)), fw(rnd, 0.1))),
                 'speed': rnd.choice((fb(rnd.uniform(0, 80)), fb(rnd.uniform(10, 30)), 0x41a00000, fw(rnd, 60.0))), 'throttle': rnd.choice((0x3f800000, fb(rnd.uniform(0, 1)), 0, fw(rnd, 2.0))),
                 'wheels': rnd.choice((4, 4, 4, 2, 1, 0, 3)), 'contacts': rnd.choice((4, 4, 3, 2, 1, 0, 5)), 'upy': rnd.choice((fb(rnd.uniform(-1, 1)), fb(rnd.uniform(0.9, 1)), fb(rnd.uniform(0.09, 0.11)), 0x3dcccccd, 0)),
                 'air': rnd.choice((0, 0, fb(rnd.uniform(0.01, 3)), 0x80000000, fb(rnd.uniform(-1, 0)))),
                 'mask': 0, 'found': hasfound, 's': s, 'idx': rnd.choice((0, 1, 2, 3, 4, 0xFFFFFFFF, 5)), 'drafting': rnd.choice((0, 1)), 'count': rnd.choice((0, 1, 2, 3, 4)),
                 'entries': rnd.getrandbits(12) | (0xB6D if rnd.random() < 0.6 else 0), 'pidx': rnd.choice((0, 1, 2, 3, 0xFFFFFFFF)), 'trail': rnd.choice((0xA5A5A5A5, 0x3f800000, 0)),
                 'player': rnd.choice((0, 1))}
            c.update(tune)
            back = vec4(1.0)
            c.update({'row0': back[0], 'row1': back[1], 'row2': back[2], 'row3': back[3]})
            cs.append([c[n] for n in names])
        c_ = lambda n: 'c[%d]' % I[n]
        setup = ('store(0x1000+4,0x1100);store(0x1000+8,0x1200);store(0x1200+8,0x1300);store(0x1200+24,0x6600);store(0x6600,0x2800);' + VT +
                 'store(0x1000+60,0x5000);store(0x5000+776,%s);' % c_('trail') +
                 'store8(0x1000+40,%s);store8(0x1000+41,%s);store8(0x1000+48,0);store(0x1000+36,%s);store(0x1000+44,%s);store(0x1000+20,%s);store(0x1000+24,%s);' % (c_('burning'), c_('full'), c_('level'), c_('stay'), c_('fov'), c_('dist')) +
                 'store(0x1000+28,%s);store(0x1000+64,%s);store(0x1000+68,%s);store(0x1000+72,%s);' % (c_('blur'), c_('strength'), c_('drafter'), c_('leader')) +
                 ''.join('store(0x1100+%d,%s);' % (o, c_(n)) for o, n in ((0, 'exp'), (4, 'amount'), (8, 'use'), (12, 'regen'), (16, 'falloff'), (20, 'cfalloff'), (24, 'stayfull'),
                                                                       (28, 'lose'), (40, 'fout'), (44, 'fin'), (48, 'tfov'), (52, 'tdist'), (56, 'bluroff'), (60, 'blurstart'))) +
                 'store(0x1300,0x2800);store(0x1300+12,0x6700);store(0x1300+24,%s);store(0x1300+136,%s);store(0x1300+224,%s);store(0x1300+260,%s);store16(0x1300+576,%s);' % (c_('wheels'), c_('throttle'), c_('speed'), c_('upy'), c_('lock')) +
                 ''.join('store(0x1300+272+%d,%s);' % (4 * k, c_('row%d' % k)) for k in range(4)) + 'store(0x1300+128,0x6800);' +
                 'store(%s,%s);' % (DT, c_('dt')) +
                 'SV[0]=%s;SV[1]=%s;SV[2]=%s;SV[3]=%s;SV[5]=%s;SV[7]=%s;SV[8]=%s;SV[9]=%s;SV[10]=%s;' % (c_('air'), c_('mask'), c_('player'), c_('pidx'), c_('contacts'), c_('found'), c_('s'), c_('idx'), c_('drafting')) +
                 'store((0x8287u<<16)|0x45E4u,0x6A00);store(0x6A00+400,%s);' % c_('count') +
                 'for(int i=0;i<4;i++){uint32_t e=%s>>(3*i);if(e&1)store(0x6A00+4*i,0x6B00+0x100*i);store(0x6B00+0x100*i+48,(e&2)?0x6C00+0x100*i:0);store(0x6C00+0x100*i+8,0x6D00+0x100*i);' % c_('entries') +
                 'store(0x6D00+0x100*i+668,(e&4)?0x7000+0x40*i:0);store(0x7000+0x40*i+60,0x7100+0x40*i);}' +
                 R13 + 'ctx.r3.u32=0x1000;')
        outs = ('(i<20?load(0x1000+4*i):i==20?load(0x5000+776):i<25?REC[0][i-21]:i==25?REC[1][0]:REC[3][0])')
        secs.append(('sstupd', cs, setup, f['sstupd'], 27, outs))
        build_group(a, 'sst', secs, stubs, disp, results, extra=[G['lying'], G['pow'], G['approach']])

    # ================================================================== damage: AddDamage
    if want('dmgadd'):
        G = {k: N(v) for k, v in dict(a24338='F-24338', a65988='F-65988', a76352='F-76352', a74220='F-74220').items()}
        stubs = {
            'sub_' + G['a24338']: 'ctx.r3.u32=SV[0];',
            'sub_' + G['a65988']: 'ctx.r3.u32=SV[1];',
            'sub_' + G['a76352']: '{for(int k=0;k<8;k++)REC[0][k]=load(ctx.r4.u32+4*k);for(int k=0;k<3;k++)REC[1][k]=load(ctx.r5.u32+4*k);ncalls|=1;}',
            'sub_' + G['a74220']: '{for(int k=0;k<8;k++)REC[2][k]=load(ctx.r4.u32+4*k);for(int k=0;k<3;k++)REC[3][k]=load(ctx.r5.u32+4*k);ncalls|=2;}',
        }
        disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){switch(a){case 0x7F00000C:ctx.r3.u32=SV[2];return;'
                'default:throw std::runtime_error("unexpected indirect call");}}')
        cs = []
        for _ in range(1500):
            mx = rnd.choice((fb(rnd.uniform(5, 100)), fb(rnd.uniform(10, 30)), fb(10), 0, 0x3f800000, fw(rnd, 50.0)))
            c = [rnd.choice((0, 0, 0, 0, 1)), rnd.choice((0, 0, 0, 0, 1)), rnd.choice((fb(rnd.uniform(0, 30)), fb(rnd.uniform(0, 5)), fw(rnd, 30.0), 0x3f800000, 0)),
                 rnd.choice((0x3f800000, fb(rnd.uniform(0, 3)), fw(rnd, 3.0), 0)), rnd.choice((fb(rnd.uniform(0, 30)), fb(rnd.uniform(0, 15)), fw(rnd, 30.0), 0, 0x80000000)),
                 rnd.choice((fb(rnd.uniform(0, 3)), 0, fw(rnd, 3.0))), rnd.choice((fb(rnd.uniform(0, 3)), 0, fw(rnd, 3.0))),
                 mx, rnd.choice((fb(rnd.uniform(1, 30)), fw(rnd, 30.0))), rnd.choice((fb(rnd.uniform(1, 30)), fw(rnd, 30.0), mx)),
                 rnd.choice((0, 1, 1)), rnd.choice((0, 1)), rnd.choice((0, 1)), rnd.choice((0, 1))]
            c[10] = 1
            # (the hit is the multiplier f1 given in c[2]; sometimes the points are near or at the maximum)
            if rnd.random() < 0.4:
                c[4] = fb(unfb(mx) * rnd.uniform(0.5, 1.2))
            cs.append(c)
        secs = [('dmgadd', cs,
                 'store(0x1000+4,0x1100);store(0x1100+20,0x1200);store8(0x1200+33,c[0]);store8((0x8288u<<16)|0xE5A7u,c[1]);store(0x1000+8,0x1400);' +
                 'store(0x1000+12,c[3]);store(0x1000+40,c[4]);store(0x1000+28,c[5]);store(0x1000+36,c[6]);store(0x1400+24,c[7]);store(0x1400+36,c[8]);store(0x1400+52,c[9]);' +
                 'store(0x1100+128,c[10]?0x1300:0);store(0x1300,0x2800);store((0x8286u<<16)|0xD800u,0x6800);store(0x6800+4296,0x6900);' + VT + 'SV[0]=c[11];SV[1]=c[12];SV[2]=c[13];ctx.r3.u32=0x1000;{float x;memcpy(&x,&c[2],4);ctx.f1.f64=x;}',
                 f['dmgadd'], 22, '(i==0?load(0x1000+40):i==1?load(0x1000+28):i==2?ncalls:i<11?REC[0][i-3]:i<19?REC[2][i-11]:REC[3][i-19])')]
        build_group(a, 'dmg', secs, stubs, disp, results, extra=[])

    # ================================================================== solver rows
    if want('kblock', 'kinv', 'rowzero', 'step', 'relvel'):
        G = {k: N(v) for k, v in dict(inv='F-68939', iter='F-67464', reset_state='F-80860').items()}
        stubs = {
            'sub_' + G['iter']: '{uint32_t i=ncalls++;if(i<NW){store(ctx.r3.u32+16,TYPES[i]);ctx.r3.u32=0x3000+0x100*i;}else ctx.r3.u32=0;}',
            'sub_' + G['reset_state']: '{REC[0][REC[1][0]++]=ctx.r3.u32;}',
        }
        disp = 'static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){throw std::runtime_error("unexpected indirect call");}'

        def row_case(kind):
            wmode[0] = 1 if rnd.random() < 0.15 else 0
            c = vec4(1.0, 0) + vec4(1.0, 0)                                   # neg, bias
            c += sum((vec4(1.0, 0) for _ in range(3)), [])                    # bcol rows
            c += sum((vec4(1.0, 0) for _ in range(3)), [])                    # K rows (inverse rows for step / relvel)
            c += sum((vec4(1.0, 0) for _ in range(3)), [])                    # jcol
            c += sum((vec4(1.0, 0) for _ in range(3)), [])                    # jcol_b
            c += [rnd.choice((0, 0, 0, 1, 0xFF))]                             # the angular-only byte
            for _ in range(2):
                c += vec4(5.0, 0) + vec4(5.0, 0)                              # vel, ang
                c += sum((vec4(2.0, 0) for _ in range(3)), [])                # mass rows
                c += sum((vec4(2.0, 0) for _ in range(3)), [])                # inertia rows
            c += vec4(3.0, 0)                                                  # the global vector before
            return c
        setup_row = ('for(int k=0;k<4;k++){store(0x1000+4*k,c[k]);store(0x1000+16+4*k,c[4+k]);}for(int k=0;k<12;k++){store(0x1000+32+4*k,c[8+k]);store(0x1000+96+4*k,c[20+k]);' +
                     'store(0x1000+144+4*k,c[32+k]);store(0x1000+192+4*k,c[44+k]);}store8(0x1000+240,c[56]);store(0x1000+80,0x2000);store(0x1000+84,0x2200);' +
                     'for(int s=0;s<2;s++){uint32_t b=57+32*s;uint32_t S=s?0x2200:0x2000;for(int k=0;k<8;k++)store(S+4*k,c[b+k]);for(int k=0;k<12;k++){store(S+32+4*k,c[b+8+k]);store(S+80+4*k,c[b+20+k]);}}' +
                     'for(int k=0;k<4;k++)store(' + GSQ + '+4*k,c[121+k]);')
        # outputs: neg 0-3, K rows 4-15, A vel/ang 16-23, B 24-31, global 32-35, step vector (out buffer at 0x3000) 36-39
        out_all = ('(i<4?load(0x1000+4*i):i<16?load(0x1000+96+4*(i-4)):i<24?load(0x2000+4*(i-16)):i<32?load(0x2200+4*(i-24)):i<36?load(' + GSQ + '+4*(i-32)):load(0x4000+4*(i-36)))')
        secs = []
        if want('kblock'):
            secs.append(('kblock', [row_case(1) for _ in range(1200)], setup_row + 'ctx.r3.u32=0x1000;ctx.r4.u32=0x3000;',
                         f['kblock'], 12, 'load(0x3000+4*i)'))
        if want('kinv'):
            secs.append(('kinv', [row_case(1) for _ in range(1200)], setup_row + 'ctx.r3.u32=0x1000;', f['kinv'], 12, 'load(0x1000+96+4*i)'))
        if want('step'):
            secs.append(('step', [row_case(1) + vec4(2.0, 0) for _ in range(1200)], setup_row + 'for(int k=0;k<4;k++)store(0x3800+4*k,c[125+k]);ctx.r3.u32=0x4000;ctx.r4.u32=0x1000;ctx.r5.u32=0x3800;',
                         f['step'], 40, out_all))
        if want('relvel'):
            secs.append(('relvel', [row_case(1) for _ in range(1200)], setup_row + 'ctx.r3.u32=0x1000;', f['relvel'], 36, out_all))
        if want('rowzero'):
            cs = []
            for _ in range(1000):
                n = rnd.choice((0, 1, 2, 3, 4, 6))
                c = [n] + [rnd.choice((0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 13, 14, 0xFFFFFFFF)) for _ in range(6)] + [rnd.getrandbits(32) for _ in range(8)]   # types, list prefill
                for _ in range(6):
                    c += [rnd.getrandbits(32) for _ in range(4)] + [rnd.getrandbits(32), rnd.getrandbits(32)]    # the row's first vector, +80, +84
                cs.append(c)
            secs.append(('rowzero', cs,
                         'NW=c[0];for(int k=0;k<6;k++)TYPES[k]=c[1+k];for(int k=0;k<8;k++)store(0x2000+4*k,c[7+k]);' +
                         'for(int r=0;r<6;r++){for(int k=0;k<4;k++)store(0x3000+0x100*r+4*k,c[15+6*r+k]);store(0x3000+0x100*r+80,c[15+6*r+4]);store(0x3000+0x100*r+84,c[15+6*r+5]);}' +
                         'ctx.r3.u32=0;ctx.r4.u32=0x2000;',
                         f['rowzero'], 9 + 24 + 6 + 2, '(i<9?load(0x2000+4*i):i<33?load(0x3000+0x100*((i-9)/4)+4*((i-9)%4)):i<39?REC[0][i-33]:i==39?REC[1][0]:ctx.r3.u32)'))
        build_group(a, 'solver', secs, stubs, disp, results, extra=[G['inv']])

    # ================================================================== GJK: the edge bisector
    if want('edgebis'):
        G = {k: N(v) for k, v in dict(fetch='F-54719').items()}
        # the vertex fetch F-54719(out, mesh, index): writes the vertex into the out slot and returns it; a stub reading a table
        stubs = {'sub_' + G['fetch']: '{uint32_t idx=ctx.r5.u32;for(int k=0;k<4;k++)store(ctx.r3.u32+4*k,load(0x3000+16*(idx&7)+4*k));}'}
        disp = 'static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){throw std::runtime_error("unexpected indirect call");}'
        cs = []
        for _ in range(2000):
            sc = rnd.choice((10.0, 1.0, 100.0))
            c = [rnd.choice((0, 1, 2)), rnd.choice((0, 1, 2)), rnd.choice((0xFFFF, 0, 1, 2, 3, 0xFFFF, 5, 9, 15)), rnd.choice((0, 1, 4, 8, 12, 16))]
            c += vec4(sc, 0) + vec4(sc, 0) + vec4(sc, 0)                                                           # N, v1, v2
            c += [rnd.choice((fb(rnd.uniform(0.01, 5)), fb(rnd.uniform(0.01, 5)), fb(rnd.uniform(0.01, 5)), 0, 0x80000000, fw(rnd, 5.0)))]    # the neighbour's area (+0)
            c += [rnd.randrange(8) for _ in range(3)]                                                              # its vertex indices (+8, +6, +4)
            for _ in range(8):
                c += vec4(sc, 0)                                                                                   # vertex table
            c = [0x80000000 if (k >= 4 and k != 16 and (x & 0x7fffffff) > 0x7f800000) else x for k, x in enumerate(c)]    # (no NaN vector input)
            cs.append(c)
        secs = [('edgebis', cs,
                 'store16(0x1000+10+2*c[0],c[2]);store(0x1500+204,c[3]);store(0x1500+140,0x1800);'
                 'for(int p=0;p<16;p++){store(0x1800+16*p,c[16]);store16(0x1800+16*p+8,c[17]);store16(0x1800+16*p+6,c[18]);store16(0x1800+16*p+4,c[19]);}'
                 'for(int k=0;k<8;k++)for(int j=0;j<4;j++)store(0x3000+16*k+4*j,c[20+4*k+j]);'
                 'ctx.r3.u32=c[0];ctx.r4.u32=c[1];ctx.r5.u32=0x1000;ctx.r6.u32=0x1500;ctx.r7.u32=0x4000;ctx.r8.u32=0x4010;'
                 'for(int j=0;j<4;j++){ctx.v3.u32[3-j]=c[4+j];ctx.v1.u32[3-j]=c[8+j];ctx.v2.u32[3-j]=c[12+j];}',
                 f['edgebis'], 9, '(i==0?ctx.r3.u32:i<5?load(0x4000+4*(i-1)):load(0x4010+4*(i-5)))')]
        build_group(a, 'gjk', secs, stubs, disp, results, extra=[])

    lines = [l for tag in results for l in results[tag]]
    a.out_dir.mkdir(parents=True, exist_ok=True)
    out = a.out_dir / 'leaves_ref.txt'
    old = [l for l in (out.read_text().splitlines() if out.exists() else []) if l.split(' ')[0] not in results]
    out.write_text(cite_safe(nl.join(old + lines) + nl))
    print({t: len(v) for t, v in results.items()})


if __name__ == '__main__':
    main()
