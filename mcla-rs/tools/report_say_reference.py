"""Run the original's police report say offline and record which line it asks for, for the port's `police::report_line`
(crates/game/src/testdata/report_say_ref.txt; test police::tests::report_line_matches_the_original).

F-14569 (r3 a report record, r4 the listener passed in): when the name +4 is a non-empty string it draws once from the shared
generator (the MWC at image +0x860880: lo, carry; a = 0x5CDCFAA7; (lo & 0x7FFFFF) x 2^-23) and says the name when the draw
< +12; otherwise (or with no name) the first set bit group of +8 picks the line: 0x20 HIT_COP_CAR_HARD, 0x40 _SOFT,
0x80 MAJORINCIDENT, 0x200 REDLIGHTS, 0x8 RECKLESS_SIDEWALK, 0x2000 RECKLESS_WRONGSIDE, 0x7 SPEEDING; none - the name, or nothing
without a name. Listener: +16 0 DISPATCH, 1 OFFICER, else r4; speaker: +20 0 DISPATCH, 1 OFFICER, else +24. The request
F-70230 (stubbed: records its arguments, returns the case's answer) gets the queue byte +40 in r9; when it answers and +28 is
set, F-23328(+28, +32, +36) (stubbed: recorded).
F-66952 (the default report's send, vtable slot 1 of the record F-56787 builds): one draw first; < 0.25 sets +12 = 1.0; then
F-14569.

Cases: the default record (F-56787: REPORT_VIOLATION_PURSUIT, 0.2, listener 2, speaker 0) sent through F-66952, and records
sent straight to F-14569 with +12 = 1.0 (the pull-over / pull-suspect / hit-cop records) or other probabilities, with no name
or an empty name; +8 every single bit 0..15, the seven groups' pairs and random words; seeds random (lo, carry); listener /
speaker modes 0, 1, 2; the request answering 0 / 1 with +28 set or not.

Line per case: <entry 0 send / 1 say> <name 0 none / 1 empty / 2 PURSUIT> b<+8 hex> p<+12 bits> <+16> <+20> <+40> <answer>
<+28 set> g<lo> g<carry> -> <asked 0/1> <line or -> <listener> <speaker> <r9> <callback 0 / 1> g<lo after> g<carry after>
(hex words with a letter prefix; p is 0 for the send, whose record keeps the ctor's 0.2)
(listener / speaker: DISPATCH, OFFICER, ARG, SPK or the hex address).
"""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

IMG = 0x82 << 24
RNG = IMG + 0x860880            # the shared generator (lo, carry)
NAME_PURSUIT = IMG + 0x0C76CC   # "REPORT_VIOLATION_PURSUIT" (the default record's name)
DEFAULT_VT = IMG + 0x0C76EC     # the default record's vtable (slot 1 = F-66952)
ARG, SPK, CBO = 0x7F000020, 0x7F000030, 0x7F000040
EMPTY = 0x40900000              # a byte 0 in the harness's scratch

MAIN = r'''
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string tok;
 while(in>>tok){ if(tok!="case") break; uint32_t e,nm,bits,prob,ls,sp,q,ans,cb,lo,ca; in>>e>>nm>>bits>>prob>>ls>>sp>>q>>ans>>cb>>lo>>ca;
  rt_reset(); const uint32_t R=0x40800000u; for(uint32_t a=R;a<R+64;a+=4) store(a,0); store(EMPTY,0);
  for(uint32_t a=G0;a<G0+32;a+=4) store(a,0); store(G0+24,ans);
  Context c=fresh(); c.r3.u32=R; DEFCTOR(c,nullptr);
  if(e==1){ store(R+12,prob); store(R+16,ls); store(R+20,sp); }
  store(R+4, nm==0?0u:(nm==1?EMPTY:NAMEP)); store(R+8,bits); store(R+24,SPKV);
  store(R+28, cb?CBOV:0u); { uint32_t w=load(R+40); w=(w&0x00ffffffu)|(q<<24); store(R+40,w); }
  store(RNG,lo); store(RNG+4,ca);
  c=fresh(); c.r3.u32=R; c.r4.u32=ARGV; if(e==0) SEND(c,nullptr); else SAY(c,nullptr);
  printf("%u %u %08x %08x %u %u %u %u %u %u %u -> %u %08x %08x %08x %u %u %u %u\n",e,nm,bits,prob,ls,sp,q,ans,cb,lo,ca,
         load(G0),load(G0+4),load(G0+8),load(G0+12),load(G0+16),load(G0+20),load(RNG),load(RNG+4)); }
 printf("end\n");
}
'''


def fb(f):
    return struct.unpack('>I', struct.pack('>f', f))[0]


def cases():
    g = random.Random(14569)
    seed = lambda: (g.getrandbits(32), g.getrandbits(32) % 0x5CDCFAA7)
    words = [1 << b for b in range(16)] + [0]
    groups = [0x20, 0x40, 0x80, 0x200, 0x8, 0x2000, 0x7]
    words += [a | b for i, a in enumerate(groups) for b in groups[i + 1:]]
    words += [g.getrandbits(32) for _ in range(20)] + [g.getrandbits(16) for _ in range(20)]
    out = []
    # the default record through its send: 12 seeds a word, plus seeds whose two draws sit at 0.25 / 0.2
    for w in words:
        for _ in range(12):
            out.append((0, 2, w, 0, 0, 0, 0, 1, 0) + seed())
    # straight to the say: probability 1.0 (the other three records), 0.2, 0.0, 0.5; no name / an empty name
    for w in words:
        for p in (1.0, 0.2, 0.0, 0.5):
            for _ in range(3):
                out.append((1, 2, w, fb(p), 2, 0, 0, 1, 0) + seed())
        for nm in (0, 1):
            out.append((1, nm, w, fb(1.0), 2, 0, 0, 1, 0) + seed())
    # the listener / speaker / queue byte / answer / callback
    for ls in (0, 1, 2):
        for sp in (0, 1, 2):
            for q in (0, 1):
                for ans in (0, 1):
                    for cb in (0, 1):
                        out.append((1, 2, 0x40, fb(1.0), ls, sp, q, ans, cb) + seed())
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ids = ['F-14569', 'F-66952', 'F-56787', 'F-70230', 'F-23328']
    r = subprocess.run([a.tool, 'registry', 'addr', *ids], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    assert rdisp.image_u32(img, DEFAULT_VT + 4) == addr['F-66952']
    assert img[NAME_PURSUIT - IMG:NAME_PURSUIT - IMG + 25] == b'REPORT_VIOLATION_PURSUIT\0'
    stubs = {name('F-70230'): 'store(G0,1); store(G0+4,ctx.r4.u32); store(G0+8,ctx.r5.u32); store(G0+12,ctx.r6.u32); store(G0+16,ctx.r9.u32&0xff); ctx.r3.u64=load(G0+24);',
             name('F-23328'): 'store(G0+20,(ctx.r3.u32==CBOV)?1u:2u);'}
    stubs = {k: v.replace('CBOV', '0x%08Xu' % CBO).replace('G0', '0x40A00000u') for k, v in stubs.items()}
    src = (MAIN.replace('DEFCTOR', name('F-56787')).replace('SEND', name('F-66952')).replace('SAY', name('F-14569'))
           .replace('NAMEP', '0x%08Xu' % NAME_PURSUIT).replace('EMPTY', '0x%08Xu' % EMPTY).replace('SPKV', '0x%08Xu' % SPK)
           .replace('CBOV', '0x%08Xu' % CBO).replace('ARGV', '0x%08Xu' % ARG).replace('RNG', '0x%08Xu' % RNG).replace('G0', '0x40A00000u'))
    exe = rdisp.build(rdisp.index(a.recomp), [name(f) for f in ids[:3]], src, a.out, 'report_say', image=img, stubs=stubs)
    cs = cases()
    assert len(cs) < 5000
    cf = a.out / 'report_cases.txt'
    cf.write_text(''.join('case ' + ' '.join(str(v) for v in c) + '\n' for c in cs) + 'stop\n')
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True, timeout=120)
    if r.returncode or not r.stdout.endswith('end\n'):
        sys.exit(r.stdout[-800:] + r.stderr[-1500:])

    def label(u):
        if u == ARG:
            return 'ARG'
        if u == SPK:
            return 'SPK'
        if IMG <= u < IMG + len(img):
            s = img[u - IMG:img.index(b'\0', u - IMG)]
            return s.decode()
        return '%08x' % u
    rows = []
    for l in r.stdout.split('\n')[:-2]:
        lhs, rhs = l.split(' -> ')
        ask, r4, r5, r6, r9, cb, lo, ca = rhs.split()
        line = label(int(r4, 16)) if ask == '1' else '-'
        e, nm, bits, prob, ls, sp, q, ans, cb0, lo0, ca0 = lhs.split()
        # (every word carries a letter prefix, so no number in the file reads as a code address)
        rows.append('%s %s b%s p%s %s %s %s %s %s g%08x g%08x -> %s %s %s %s %s %s g%08x g%08x' % (
            e, nm, bits, prob, ls, sp, q, ans, cb0, int(lo0), int(ca0), ask, line, label(int(r5, 16)) if ask == '1' else '-',
            label(int(r6, 16)) if ask == '1' else '-', r9, cb, int(lo), int(ca)))
    a.ref.write_text('\n'.join(rows) + '\n')
    print('cases', len(rows), '->', a.ref)


if __name__ == '__main__':
    main()
