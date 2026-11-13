"""Tune files read by the original's own reader, offline: the class's constructor on a zeroed object, the class's member table
(its parse slot: parser +84 + 88 i, nested blocks too), then the original's text reader F-65466 (the tokenizer F-46449 over a
stream, the first token = the tune's name, the member walk F-66614 with its type switch) over the file's bytes, held whole in the
stream's buffer (the device's read is reached only at the end of the text: it returns 0). The tokenizers' vtables and the string
compare the reader calls through its global pointer are the image's own.
Output: one line per file, `<file name> <the object's first N words in hex>` (a hashed name as `#<decimal>`).
Usage: tune_read_reference.py --ctor F-x --schema F-y --words N --class camTrackCS [--need SSTurbo] --dir DIR --out FILE [--deref OFF:N]
       (--deref: the N words of the object +OFF points to follow the object's)
       [--pre OFF=HEX ...]   (words stored into the object after the constructor, before the file: what an owner writes first)"""
import argparse
import subprocess
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import memcap  # noqa: F401  (every harness process capped, 2026-11-07 rule)
import recomp_closure as rc
import recomp_dispatch as rd

# (vperm / lvsl as the SDK's intrinsics, for the stream's block copy)
rc.EXTRA += ('alignas(16) static uint8_t VectorShiftTableL[]={' +
             ',\n'.join(', '.join('0x%02X' % b for b in range(15 + k, k - 1, -1)) for k in range(16)) + '};\n'
             'namespace rex { namespace ppc {\n'
             'static inline simde__m128i simde_mm_perm_epi8_(simde__m128i a, simde__m128i b, simde__m128i c) {\n'
             ' simde__m128i d = simde_mm_set1_epi8(0xF); simde__m128i e = simde_mm_sub_epi8(d, simde_mm_and_si128(c, d));\n'
             ' return simde_mm_blendv_epi8(simde_mm_shuffle_epi8(a, e), simde_mm_shuffle_epi8(b, e), simde_mm_slli_epi32(c, 3)); } } }\n')

READER = 'F-65466'
OBJ, PAR, STREAM, DEV, DEVVT, NAME, BUF = 0x40100000, 0x40200000, 0x40300000, 0x40300100, 0x40300200, 0x40300400, 0x40400000
FAKE_READ = 0x7F000020
# (the text tokenizer's and the binary one's vtables, as the tokenizer constructor F-47045 stores them; the reader's string compare
# F-19164, reached through its global pointer)
TOKENIZER_VTABLES = [(0x82008B1C, 40), (0x8200C27C, 40)]
STRCMP = 'F-19164'

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 for(int f=2;f<argc;f++){
  std::ifstream tf(argv[f],std::ios::binary);std::vector<uint8_t> t((std::istreambuf_iterator<char>(tf)),{});
  std::memset(arena,0,sizeof(arena));rt_reset();
  for(size_t i=0;i<t.size();i++)REX_STORE_U8(BUF_+i,t[i]);
  store(STREAM_+0,DEV_);store(STREAM_+8,BUF_);store(STREAM_+28,(uint32_t)t.size());store(STREAM_+32,(uint32_t)t.size()+1);
  store(DEV_,DEVVT_);for(int i=0;i<40;i++)store(DEVVT_+4*i,DEVREAD_);
  const char*nm="tune";for(int i=0;;i++){REX_STORE_U8(NAME_+i,nm[i]);if(!nm[i])break;}
  Context c=fresh();
  CTOR_CALL
  PRE
  c=fresh();c.r3.u32=OBJ_;c.r4.u32=PAR_;SCHEMA(c,nullptr);
  c=fresh();c.r3.u32=PAR_;c.r4.u32=STREAM_;c.r5.u32=NAME_;READER_CALL(c,nullptr);
  printf("\nR %d",f-2);for(int i=0;i<WORDS_;i++)printf(" %08x",load(OBJ_+4*i));
  DEREF
  printf("\n");}}
'''


def fixture_name(n):
    """A hashed file name (`0x` + 8 hex digits) as `#` + its decimal value: some hashes equal an address of the original,
    which tracked files may not hold (registry check); other names as they are."""
    return '#%d' % int(n, 16) if len(n) == 10 and n.startswith('0x') else n


def addr(tool, fid):
    return int(subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1], 16)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--recomp', type=Path, default=Path('D:/MCLA-static'))
    ap.add_argument('--image', type=Path, default=Path('D:/MCLA/work/2026-10-10/original-image.bin'))
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--ctor')
    ap.add_argument('--schema', required=True)
    ap.add_argument('--words', type=int, required=True)
    ap.add_argument('--dir', type=Path, required=True)
    ap.add_argument('--class', dest='cls', required=True, help="the class name on the file's second line")
    ap.add_argument('--need', default='', help='only files whose text has this word')
    ap.add_argument('--pre', action='append', default=[])
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--vtable', action='append', default=[], help='ADDR:N: more dispatch targets (e.g. the vtable of a member object)')
    ap.add_argument('--deref', help='OFF:N: also the N words of the object the word at +OFF points to')
    a = ap.parse_args()
    files = []
    for p in sorted(q for q in a.dir.iterdir() if q.is_file()):
        lines = p.read_text(errors='replace').splitlines()
        if len(lines) > 1 and lines[1].split()[:1] == [a.cls] and (not a.need or a.need in p.read_text(errors='replace')):
            files.append(p)
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    read = 'sub_%08X' % addr(a.tool, READER)
    schema = 'sub_%08X' % addr(a.tool, a.schema)
    names = [read, schema]
    ctor = ''
    if a.ctor:
        cf = 'sub_%08X' % addr(a.tool, a.ctor)
        names.append(cf)
        ctor = 'c.r3.u32=OBJ_;%s(c,nullptr);' % cf
    pre = ''.join('store(OBJ_+%d,0x%sU);' % (int(p.split('=')[0]), p.split('=')[1]) for p in a.pre)
    src = MAIN.replace('CTOR_CALL', ctor).replace('PRE', pre).replace('SCHEMA', schema).replace('READER_CALL', read)
    for k, v in (('OBJ_', OBJ), ('PAR_', PAR), ('STREAM_', STREAM), ('DEVVT_', DEVVT), ('DEV_', DEV), ('NAME_', NAME), ('BUF_', BUF),
                 ('DEVREAD_', FAKE_READ)):
        src = src.replace(k, hex(v) + 'u')
    src = src.replace('WORDS_', str(a.words))
    deref = ''
    if a.deref:
        off, n = (int(x) for x in a.deref.split(':'))
        deref = '{uint32_t q=load(%#xu);for(int i=0;i<%d;i++)printf(" %%08x",load(q+4*i));}' % (OBJ + off, n)
    src = src.replace('DEREF', deref)
    exe = rd.build(idx, names, src, a.work, 'tune_read_' + a.cls, image=img, vtables=TOKENIZER_VTABLES + [(int(v.split(':')[0], 16), int(v.split(':')[1])) for v in a.vtable], targets=[addr(a.tool, STRCMP)],
                   fakes={FAKE_READ: 'ctx.r3.u32=0;'})
    out = []
    for i in range(0, len(files), 40):
        r = subprocess.run([str(exe), str(a.image)] + [str(f) for f in files[i:i + 40]], capture_output=True, text=True)
        if r.returncode:
            raise SystemExit(r.stderr[-1500:] or 'exit %d' % r.returncode)
        # (only the result lines: what the game's own code prints is dropped; each line names its file by its index)
        for l in r.stdout.splitlines():
            if l.startswith('R '):
                k, rest = l[2:].split(' ', 1)
                out.append(fixture_name(files[i + int(k)].name) + ' ' + rest)
    assert len(out) == len(files), (len(out), len(files))
    a.out.write_text('\n'.join(out) + '\n')
    print(len(out), 'files')


if __name__ == '__main__':
    main()
