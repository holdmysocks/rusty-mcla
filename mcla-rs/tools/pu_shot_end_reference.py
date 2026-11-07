"""The end of a power-up shot (F-79013) and the car-audio notification it calls (F-23785, vtable slot 21 = +84) run
offline: unchanged generated code on flat guest memory (recomp_closure, vector shim, -msse4.1).
Holder at 0x1000 (+4 its car 0x1200, +16 the shot clock, +144 the icon vector, +228 the shot's type object, +232 the
shot flag), car +16 = the audio entity 0x1300 (vtable 0x1C00: slot 21 is the real F-23785, every other slot a marker;
+1376 the shot loop), the type object 0x1100 (vtable 0x1180, markers; +4 its type index).
Stubs, logged with their arguments: F-19272 (the type -> PACKAGE_POWERUP_PLAYER entry; returns 0x1A00, where the case
put the shipped entry's 62 bytes, or 0 when the case says the lookup fails), F-31667 (the loop stop), F-55796 (the
sound play: r4 the sound hash).
Each output line: the case, then the calls in order, then the holder's +228 +232 +16 +164 +192 after.
Usage: pu_shot_end_reference.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --tool mcla-tool
       --config <audio config dir with game.dat> --work DIR --out FILE"""
import argparse
import random
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build

recomp_closure.EXTRA += '#define ctr r2\nstatic void dispatch(Context&,const uint8_t*,uint32_t);\n#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)\n'

NAMES = ["NITRO", "ICE", "REVERSE", "STEALTH", "DISRUPTOR", "SHIELD", "STOP", "PULSE", "MIRROR", "AGRO"]


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def joaat(s):
    h = 0
    for c in s.lower().encode():
        h = (h + c) & 0xffffffff
        h = (h + (h << 10)) & 0xffffffff
        h ^= h >> 6
    h = (h + (h << 3)) & 0xffffffff
    h ^= h >> 11
    return (h + (h << 15)) & 0xffffffff


def game_objects(path):
    d = path.read_bytes()
    size = struct.unpack('>I', d[4:8])[0]
    data, p = d[8:8 + size], 8 + size
    p += 4 + struct.unpack('>I', d[p:p + 4])[0]
    count = struct.unpack('>I', d[p:p + 4])[0]
    assert count < 100000, count
    p += 8
    objs = {}
    for _ in range(count):
        n = d[p]
        name = d[p + 1:p + 1 + n].decode('latin1')
        p += 1 + n
        off, sz = struct.unpack('>II', d[p:p + 8])
        p += 8
        objs[name] = data[off:off + sz]
    return objs


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--tool', required=True)
    ap.add_argument('--config', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    a = ap.parse_args()
    end, note, entry, stop, play = (addr(a.tool, f) for f in ('F-79013', 'F-23785', 'F-19272', 'F-31667', 'F-55796'))
    objs = game_objects(a.config / 'game.dat')
    entries = [objs['POWERUP_%s_PLAYER' % n] for n in NAMES]
    assert all(len(e) == 62 for e in entries)
    rnd = random.Random(20261107)
    cases = []
    for i in range(512):
        shot = i % 4 != 0
        flag = 1 if i % 8 < 6 else 0           # (a flag-0 shot is not reachable in the game; run anyway: the other branch)
        kind = rnd.randrange(21)
        loop = rnd.random() < 0.7
        found = rnd.random() < 0.9                # (the entry lookup failing: the notification does nothing)
        clock = struct.unpack('>I', struct.pack('>f', rnd.uniform(-1, 5)))[0]
        cases.append((int(shot), flag, kind, int(loop), int(found), clock, rnd.getrandbits(32), rnd.getrandbits(32)))
    nl = chr(10)
    NL = chr(92) + 'n'
    ent = ','.join('{%s}' % ','.join('%d' % b for b in e) for e in entries)
    rows = nl.join('{%s},' % ','.join('0x%08x' % x for x in c) for c in cases)
    disp = ('static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){if(a==0x%s){printf(" N:%%u",ctx.r4.u32);sub_%s(ctx,base);}'
            'else printf(" V:%%x:%%x:%%u",a,ctx.r3.u32,ctx.r4.u32);}' % (note, note))
    stubs = {'sub_' + entry: 'printf(" E:%x:%u",ctx.r3.u32,ctx.r4.u32);ctx.r3.u32=load(0x1F00)?0x1A00:0;',
             'sub_' + stop: 'printf(" S:%x:%u",ctx.r3.u32,ctx.r4.u32);',
             'sub_' + play: 'printf(" P:%x:%08x",ctx.r3.u32,ctx.r4.u32);'}
    body = ('static const uint8_t ENT[10][62]={' + ent + '};' + nl +
            'static const uint32_t C[][8]={' + nl + rows + nl + '};' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' +
            'printf("%u %u %u %u %u %08x",c[0],c[1],c[2],c[3],c[4],c[5]);' +
            'store(0x1000+4,0x1200);store(0x1200+16,0x1300);store(0x1300,0x1C00);' +
            'for(int k=0;k<40;k++)store(0x1C00+4*k,0x90000000+4*k);store(0x1C00+84,0x%s);' % note +
            'store(0x1100,0x1180);for(int k=0;k<16;k++)store(0x1180+4*k,0x91000000+4*k);store(0x1100+4,c[2]);' +
            'store(0x1000+228,c[0]?0x1100:0);pointer(0x1000+232)[0]=c[1];store(0x1000+16,c[5]);' +
            'store(0x1000+144,c[6]);store(0x1000+148,c[7]);store(0x1300+1376,c[3]?0x1E00:0);store(0x1F00,c[4]);' +
            'int b=c[2]<12?c[2]:c[2]-11;if(b>9)b=0;for(int k=0;k<62;k++)pointer(0x1A00+k)[0]=ENT[b][k];' +
            'Context ctx{};ctx.r3.u32=0x1000;ctx.r1.u32=0x2F00;sub_%s(ctx,nullptr);' % end +
            'printf(" | %08x %u %08x %08x %08x' + NL + '",load(0x1000+228),pointer(0x1000+232)[0],load(0x1000+16),load(0x1000+164),load(0x1000+192));}' + nl)
    main_src = (disp + nl + 'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);'
                'image.assign(std::istreambuf_iterator<char>(bin),{});' + nl + body + '}' + nl)
    idx = index(a.recomp)
    exe = build(idx, ['sub_' + end, 'sub_' + note], main_src, a.work, 'pu_shot_end', stubs=stubs, vector=True, flags=('-msse4.1',))
    out = subprocess.run([str(exe), str(a.image)], capture_output=True, text=True, check=True).stdout
    lines = out.splitlines()
    assert len(lines) == len(cases), len(lines)
    a.out.write_text(out.replace('\r', ''))
    print(len(lines), 'lines')


if __name__ == '__main__':
    main()
