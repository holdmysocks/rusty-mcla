"""A prop entry's composite bound as the original makes it when a prop is hit or a piece breaks off, run offline on its code.
The entry's fill (F-52007's composite copy, traffic_drive_rule.md 9.19) gives the entry a new composite with the source's
part pointers, its part matrices copied (the type composites' current and last matrices are one array, so the copy's are
too) and its box table; every part of a group that left is set to null (the part setter F-13753, with the release F-48011 of the shared
part bound stubbed: it only counts that bound's references); then the mass-properties step runs the centroid F-64537 (the
mean of the present parts' centroids, set through the composite's slot 6) and the extents F-65996 (the parts' boxes F-32566
with the radius). Here: every composite prop type of resources/prop/0x0e66973c.r131 laid as capsule_dispatch_reference.py lays
it (the composite with its real vtable), a copy made as the fill makes it, the case's parts nulled by the real setter, then
the two real routines.

A case: the type's index and a mask of the parts to null (bit c: part c). Types with one part get only mask 0; others mask 0,
each single part, and random masks (never all parts). Out: the composite's +8, +12, +16 .. +24, +32 .. +40, +48 .. +56 and
bytes +5 / +6. Writes <out-dir>/prop_entry_bound_ref.txt. Test `camprobe::tests::prop_entry_bound_matches_the_original`."""
import argparse
import random
import re
import struct
import subprocess
from pathlib import Path
import memcap  # (every harness process capped at 4 GB)
import recomp_dispatch as rd
import level_query_reference as lq

_run = subprocess.run
INITS = ['F-50173', 'F-33402', 'F-26199', 'F-55749', 'F-97349', 'F-70580', 'F-79721', 'F-64228', 'F-24192', 'F-92824']
VT_COMPOSITE = 0x82081344     # the composite bound's vtable (its ctor stores it)
COPY = 0x40C00000

MAIN = r'''
static const uint32_t STK=0x40DF0000, COPY=COPY_;
static std::vector<uint8_t> blob;
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 {std::ifstream b(argv[3],std::ios::binary); blob.assign(std::istreambuf_iterator<char>(b),{});}
 FILE*f=fopen(argv[2],"r"); unsigned h[2];
 while(fscanf(f,"%x %x",&h[0],&h[1])==2){
  rt_reset(); { INITCALLS } store(0x40000200u+32,load(0x40000200u+28));
  memcpy(pointer(RESP_),blob.data(),blob.size());
  uint32_t src=h[0]; if(src<RESP_||src>=RESP_+blob.size()){fprintf(stderr,"bad case file: %x\n",src); return 2;}
  store(src,VTCOMP_);
  unsigned n=load16(src+146), cnt=load16(src+144);
  if(n>64||cnt>64||n>cnt){fprintf(stderr,"bad composite: %u %u\n",n,cnt); return 2;}
  // (the fill's copy: the composite's words, its own part pointers, matrices - current and last one array - and boxes)
  for(uint32_t o=0;o<160;o+=4) store(COPY+o,load(src+o));
  uint32_t P=COPY+0x100, M=COPY+0x400, B=COPY+0x1800;
  for(unsigned k=0;k<cnt;k++){ store(P+4*k,load(load(src+128)+4*k));
   for(int j=0;j<64;j+=4) store(M+64*k+j,load(load(src+132)+64*k+j));
   for(int j=0;j<32;j+=4) store(B+32*k+j,load(src+140)?load(load(src+140)+32*k+j):0); }
  store(COPY+128,P); store(COPY+132,M); store(COPY+136,M); store(COPY+140,B);
  for(unsigned k=0;k<cnt;k++) if(h[1]>>k&1){ Context c=fresh(); c.r1.u64=STK; c.r3.u64=COPY; c.r4.u64=k; c.r5.u64=0; c.r6.u64=1; sub_SET_(c,nullptr); }
  try{ {Context c=fresh(); c.r1.u64=STK; c.r3.u64=COPY; sub_CEN_(c,nullptr);} {Context c=fresh(); c.r1.u64=STK; c.r3.u64=COPY; c.r4.u64=0; sub_EXT_(c,nullptr);} }
  catch(std::exception&e){ printf("EXC %s\n",e.what()); continue; }
  for(int o: {8,12,16,20,24,32,36,40,48,52,56}) printf("%08x ",load(COPY+o));
  printf("%02x %02x\n",*pointer(COPY+5),*pointer(COPY+6)); } }
'''


def addr(tool, fid):
    return _run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--data', type=Path, default=Path('D:/MCLA/extracted/cache'))
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--seed', type=int, default=14217)
    a = ap.parse_args()
    rnd = random.Random(a.seed)
    path = a.data / 'resources/prop/0x0e66973c.r131'
    blob, types = lq.load_props(path)
    # (the box tables +0x8c and the last matrices +0x88 are relocated as the copy reads them)
    data = bytearray(blob)
    d, vs, raw = lq._resource(path)
    ptr = lq._ptr(vs, len(raw))
    cases = []
    for i, t in enumerate(types):
        if t is None:
            continue
        b = t[0] - lq.RES_P
        for o in (0x88, 0x8c):
            q = ptr(struct.unpack('>I', raw[b + o:b + o + 4])[0])
            data[b + o:b + o + 4] = struct.pack('>I', lq.RES_P + q if q is not None else 0)
        cnt = struct.unpack('>H', data[b + 0x90:b + 0x92])[0]
        assert 0 < cnt <= 64, cnt
        masks = [0]
        if cnt > 1:
            masks += [1 << k for k in range(min(cnt, 6))]
            full = (1 << cnt) - 1
            masks += [m for m in (rnd.getrandbits(cnt) for _ in range(4)) if m != full]
        for m in masks:
            cases.append((i, t[0], m))
    work = a.out_dir / 'build'
    work.mkdir(parents=True, exist_ok=True)
    (work / 'props.bin').write_bytes(bytes(data))
    inp = work / 'cases.txt'
    inp.write_text(''.join('%x %x\n' % (c[1], c[2]) for c in cases))
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    inits = [addr(a.tool, f) for f in INITS]
    sett, cen, ext = (addr(a.tool, f) for f in ('F-13753', 'F-64537', 'F-65996'))
    main_src = (MAIN.replace('COPY_', '0x%08Xu' % COPY).replace('RESP_', '0x%08Xu' % lq.RES_P).replace('VTCOMP_', '0x%08Xu' % VT_COMPOSITE)
                .replace('sub_SET_', 'sub_' + sett).replace('sub_CEN_', 'sub_' + cen).replace('sub_EXT_', 'sub_' + ext)
                .replace('INITCALLS', ' '.join('{Context g=fresh(); sub_%s(g,nullptr);}' % x for x in inits)))
    stubs = {'sub_' + addr(a.tool, 'F-48011'): ''}
    exe = rd.build(idx, ['sub_' + x for x in [sett, cen, ext] + inits], main_src, work, 'prop_entry', image=img, stubs=stubs,
                   vtables=[(VT_COMPOSITE, 41)])
    r = _run([str(exe), str(a.image), str(inp), str(work / 'props.bin')], capture_output=True, text=True)
    out = [x for x in r.stdout.split('\n') if x]
    assert len(out) == len(cases), (len(out), len(cases), r.stderr[-800:])
    bad = [o for o in out if 'EXC' in o]
    assert not bad, bad[:3]
    text = ''.join('%d %x -> %s\n' % (c[0], c[2], o.strip()) for c, o in zip(cases, out))
    (a.out_dir / 'prop_entry_bound_ref.txt').write_text(re.sub(r'\b(82[0-9a-f]{2})([0-9a-f]{4})\b', r'\1_\2', text))
    print(len(cases), 'cases,', len(set(c[0] for c in cases)), 'types')


if __name__ == '__main__':
    main()
