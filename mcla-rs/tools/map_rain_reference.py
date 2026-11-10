"""The rain tuning loader F-55174 run offline as generated (cameras2-map4), written to crates/game/src/testdata/map_rain_ref.txt for
the port's tests. recomp_closure imports tools/memcap.py, so every process of this script (g++, the harness) runs in a 4 GB job.
The loader is run unchanged with the original's own tokenizer (its constructor F-18827, the class whose vtable the loader calls
through: token reads, key checks F-29310, number reads F-48331 and the C runtime's number conversion all run as generated). Not run: the
file open (the asset search path push / pop F-12484, F-25937, the open F-56701 answers the stream record with the text placed in
its buffer, the close F-10154), the stream refill F-19996 (answers end of file) and the C runtime's per-thread data F-62587 (answers none).
The rain object starts as a sentinel word per field, so a key the file does not give shows as the sentinel (the loader has no
default of its own except the two slices, which it stores on a camera-side object).
Each line: "RAIN <label> <input, % escaped> -> <the 29 fields in the loader's order, f32 bits> <slice near> <slice far>" where a field
the loader did not write shows as its sentinel word C0DE0000 + offset. Inputs: the shipped file and variants of it (keys moved, left out,
repeated, odd number spellings, comments, line ends).
Usage: map_rain_reference.py --recomp D:/MCLA-static --image IMG --work DIR --tune raintuning.xml --out FILE [--tool mcla-tool]"""
import argparse
import re
import subprocess
from pathlib import Path
import recomp_dispatch as rd
import recomp_closure as rc

# the rain object's fields in the order the loader reads the keys (key -> byte offset)
KEYS = [('NearSlice', None), ('FarSlice', None), ('SplashSizeScale', 568), ('RainOpacity', 8), ('SplashOpacity', 16), ('WindX', 720),
        ('WindY', 724), ('WindZ', 728), ('WindRandomRangeX', 736), ('WindRandomRangeY', 740), ('WindRandomRangeZ', 744),
        ('Rainthreshold', 596), ('Rainstrength', 564), ('Streakthickness', 600), ('Streaklength', 604), ('EmitterCamdistance', 684),
        ('EmitterPosX', 672), ('EmitterPosY', 676), ('EmitterPosZ', 680), ('EmitterPosRangeX', 688), ('EmitterPosRangeY', 692),
        ('EmitterPosRangeZ', 696), ('RainOpacityNight', 12), ('SplashOpacityNight', 20), ('SplashOffset', 592), ('RainAmbientRed', 608),
        ('RainAmbientGreen', 612), ('RainAmbientBlue', 616), ('RainSpotLightRed', 528), ('RainSpotLightGreen', 532), ('RainSpotLightBlue', 536)]

COMMON = r'''
static uint32_t g_stream;
'''

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2],std::ios::binary);std::string line;
 const uint32_t R=ARENA_+0x700000,S=ARENA_+0x701000,O1=ARENA_+0x701100,O2=ARENA_+0x701200,O3=ARENA_+0x701300,BUF=ARENA_+0x800000;
 const uint32_t offs[]={OFFS_};
 while(std::getline(in,line)){
  if(line.size()>0x20000){fprintf(stderr,"bad input size\n");return 2;}
  std::string text;for(size_t i=0;i<line.size();i++){if(line[i]=='%'){text+=char(std::stoi(line.substr(i+1,2),nullptr,16));i+=2;}else text+=line[i];}
  rt_reset();store(RT_THREAD_+336,1);
  for(size_t i=0;i<text.size();i++)*pointer(BUF+i)=(uint8_t)text[i];
  *pointer(BUF+text.size())=0;
  for(uint32_t a=S;a<S+0x40;a+=4)store(a,0);
  store(S+8,BUF);store(S+24,0);store(S+28,(uint32_t)text.size());g_stream=S;
  for(uint32_t a=0;a<0x400;a+=4)store(R+a,0xC0DE0000u+a);
  store(GLOBAL_,O1);store(O1+32,O2);store(O2+4,O3);store(O3+76,0xC0DE004Cu);store(O3+80,0xC0DE0050u);
  Context cx=fresh();cx.r3.u32=R;sub_RAIN(cx,nullptr);
  for(size_t i=0;i<sizeof offs/sizeof offs[0];i++)printf(" %08x",load(R+offs[i]));
  printf(" %08x %08x\n",load(O3+76),load(O3+80));
 }
}
'''


def esc(t):
    return ''.join(c if (c.isalnum() or c in '.-_+') else '%%%02X' % ord(c) for c in t)


def variants(text):
    lines = text.replace('\r', '').split('\n')
    lines = [l for l in lines if l]
    out = [('shipped', text)]
    j = lambda ls, sep='\n': sep.join(ls) + sep
    out.append(('crlf', j(lines, '\r\n')))
    out.append(('no_final_newline', '\n'.join(lines)))
    out.append(('spaces', j([re.sub(r'\t', '   ', l) for l in lines])))
    out.append(('empty', ''))
    out.append(('only_first', lines[0] + '\n'))
    out.append(('only_last', lines[-1] + '\n'))
    out.append(('reversed', j(lines[::-1])))
    ls = lines[:]
    ls[5], ls[6] = ls[6], ls[5]
    out.append(('swap_wind_xy', j(ls)))
    out.append(('missing_windx', j([l for l in lines if not l.startswith('WindX\t')])))
    out.append(('missing_nearslice', j(lines[1:])))
    out.append(('missing_last', j(lines[:-1])))
    out.append(('duplicate_windx', j(lines[:6] + [lines[5]] + lines[6:])))
    out.append(('unknown_key_between', j(lines[:5] + ['Unknown\t1.0'] + lines[5:])))
    out.append(('unknown_first', j(['Unknown\t1.0'] + lines)))
    out.append(('lowercase_key', j([lines[0].lower()] + lines[1:])))
    out.append(('key_without_value', j(lines[:5] + ['WindX'] + lines[6:])))
    out.append(('blank_lines', j([l + '\n' for l in lines])))
    for k, v in [('exp', '1e-3'), ('exp_big', '2.5E+2'), ('negzero', '-0'), ('lead_dot', '.5'), ('trail_dot', '5.'), ('plus', '+3'), ('alpha', 'abc'),
                 ('hex', '0x10'), ('inf', 'inf'), ('nan', 'nan'), ('comma', '1,5'), ('suffix', '1.5f'), ('double_minus', '--2'),
                 ('many_digits', '0.123456789012345678'), ('tiny', '1e-45'), ('huge', '1e39'), ('int', '7'), ('minus_only', '-'),
                 ('junk_tail', '3x4'), ('dot_only', '.'), ('exp_only', 'e5'), ('long_digits', '123456789.123456789'),
                 ('plus_dot', '+.5'), ('neg_dot', '-.5'), ('exp_plus_only', '1e+'), ('two_dots', '0.1.2'), ('exp_no_digits', '3e'),
                 ('leading_zeros', '007.50'), ('round_up', '16777217'), ('double_rounding', '1.00000005960464477539062500001'),
                 ('double_rounding_down', '1.0000000596046447753906249999'), ('exp_neg_big', '1e-400'), ('exp_pos_big', '1e400'),
                 ('denormal_mid', '7.006492321624085e-46'), ('nine_nines', '9.999999999'), ('neg_exp', '-2.5e-3'), ('upper_e_neg', '5E-1'),
                 ('trailing_dot_exp', '5.e2')]:
        ls = lines[:]
        ls[5] = 'WindX\t' + v
        out.append(('value_' + k, j(ls)))
    for k, c in [('hash', '# comment'), ('semicolon', '; comment'), ('slashes', '// comment'), ('brace', '{'), ('equals', 'WindX = 3')]:
        out.append(('line_' + k, j(lines[:5] + [c] + lines[5:])))
    out.append(('quoted_value', j(lines[:5] + ['WindX\t"3.5"'] + lines[6:])))
    out.append(('quoted_key', j(lines[:5] + ['"WindX"\t3.5'] + lines[6:])))
    import random
    rnd = random.Random(55174)
    def num():
        k = rnd.randrange(8)
        sign = rnd.choice(['', '', '-'])
        if k == 0:
            return sign + '%d.%06d' % (rnd.randrange(0, 100), rnd.randrange(0, 10 ** 6))
        if k == 1:
            return sign + '%d' % rnd.randrange(0, 1000)
        if k == 2:
            return sign + '0.%0*d' % (rnd.randrange(1, 12), rnd.randrange(0, 10 ** 6)) + rnd.choice(['', '1', '5', '99'])
        if k == 3:
            return sign + '%d.%d' % (rnd.randrange(0, 10 ** 9), rnd.randrange(0, 10 ** 9))
        if k == 4:
            return sign + '%d.%03de%s%d' % (rnd.randrange(1, 10), rnd.randrange(1000), rnd.choice(['', '-', '+']), rnd.randrange(0, 12))
        if k == 5:
            return sign + '%.17g' % rnd.uniform(0, 100)
        if k == 6:
            return sign + '%.9g' % rnd.uniform(0, 1)
        return sign + '%.6f' % rnd.uniform(0, 50)
    for r in range(60):
        out.append(('random_%d' % r, j(['%s	%s' % (l.split('	')[0], num()) for l in lines])))
    out.append(('same_line', ' '.join(lines) + '\n'))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--tune', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    size = a.tune.stat().st_size
    if not 0 < size <= 0x4000:
        raise SystemExit(f'tuning file size {size} outside the expected range')
    text = a.tune.read_bytes().decode('latin-1')
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    a.work.mkdir(parents=True, exist_ok=True)

    def name(fid):
        return 'sub_' + subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]
    rc.EXTRA += COMMON
    hi = lambda x: (x << 16) & 0xFFFFFFFF
    vt = hi(-32255) + 12332          # the tokenizer vtable the loader's constructor stub F-85938 stores
    glob = hi(-32119) - 18004        # the global the loader reads the slice destination through (+32, +4)
    stubs = {name('F-12484'): '', name('F-56701'): 'ctx.r3.u32=g_stream;', name('F-10154'): '', name('F-25937'): '',
             name('F-19996'): 'ctx.r3.s64=-1;', name('F-62587'): 'ctx.r3.u64=0;'}
    offs = ','.join(str(o) for k, o in KEYS if o is not None)
    src = (MAIN.replace('sub_RAIN', name('F-55174')).replace('RT_THREAD_', '0x%08Xu' % rd.RT_THREAD).replace('ARENA_', '0x%08Xu' % rd.ARENA)
           .replace('OFFS_', offs).replace('GLOBAL_', '0x%08Xu' % glob))
    exe = rd.build(idx, [name('F-55174'), name('F-85938'), name('F-18827'), name('F-29310'), name('F-48331')], src, a.work, 'map_rain',
                   image=img, stubs=stubs, vtables=[(vt, 20)])
    vs = variants(text)
    cf = a.work / 'RAIN_cases.txt'
    cf.write_text('\n'.join(esc(t) for _, t in vs) + '\n')
    p = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    outs = p.stdout.splitlines()
    assert len(outs) == len(vs), (len(outs), len(vs))
    a.out.write_text('\n'.join(f'RAIN {lab} {esc(t)} ->{o}' for (lab, t), o in zip(vs, outs)) + '\n')
    print(len(vs), 'variants')


if __name__ == '__main__':
    main()
