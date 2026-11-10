"""The occluder file reader run offline as generated (cameras2-map2): F-67870 (the loader body) with its header reader F-97059, the
per-occluder reader F-97372, the second-list reader, the array allocator F-66575 and the polygon / vertex / edge setters F-45179,
F-36691, F-10868 and the rest of its closure - AND the original's own tokenizer (the class whose vtable the reader calls through:
its token reads, numbers and keyword checks all run as generated). The only things not run are the stream underneath: the
tokenizer reads the file's bytes from the stream object's buffer (record +8 buffer, +24 position, +28 length - the whole file is
placed there) and the stream's refill function (F-19996: the refill when the buffer is used up) is stubbed to answer end of file.
recomp_closure imports tools/memcap.py, so every process of this script (g++, the harness) runs in a 4 GB job.

For every occluder the reader stored (80-byte records: +0 the bounding sphere, +48 the polygons (16 bytes each: +0 the reversed-edge
mask set at the end, +5 the vertex count, +8 / +12 the vertex and edge indices as bytes), +52 the vertices (16 bytes each),
+56 the edges (2 bytes each), +65 / +66 / +67 the edge / vertex / polygon counts) one line:
  OC <index> <hash of the stored data> <hash of the polygons' masks> <vertices> <edges> <polygons>
the hash is FNV-1a (64 bit) over 32-bit words: the sphere's four floats, the vertex count, each vertex's x y z, the edge count, each
edge's two bytes, the polygon count, per polygon its count, its vertex indices and its edge indices (as bytes). The second hash over
the polygons' masks alone. The first lines of the file are also written in clear (OCT) for the doc.
Usage: occluder_parse_reference.py --recomp D:/MCLA-static --image IMG --work DIR --file city.occluder --out FILE [--tool mcla-tool]"""
import argparse
import subprocess
from pathlib import Path
import recomp_dispatch as rd
import recomp_closure as rc

def hi(x):
    return (x << 16) & 0xFFFFFFFF


VT = hi(-32255) - 29924     # the tokenizer's vtable, as the loader that calls F-67870 forms it (lis / addi)

COMMON = r'''
static uint64_t fnv(uint64_t h,uint32_t w){for(int i=0;i<4;i++){h^=(w>>(8*i))&0xFF;h*=0x100000001b3ULL;}return h;}
'''

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2],std::ios::binary);std::vector<char> file((std::istreambuf_iterator<char>(in)),{});
 if(file.empty()||file.size()>0x700000){fprintf(stderr,"bad input size %zu\n",file.size());return 2;}
 rt_reset();store(RT_THREAD_+336,1);
 const uint32_t T=ARENA_+0x700000,S=ARENA_+0x701000,NM=ARENA_+0x701100,C=ARENA_+0x702000,BUF=ARENA_+0x800000;
 for(size_t i=0;i<file.size();i++)*pointer(BUF+i)=(uint8_t)file[i];
 const char*nm="city.occluder";for(int i=0;nm[i];i++)*pointer(NM+i)=nm[i];
 store(S+8,BUF);store(S+24,0);store(S+28,(uint32_t)file.size());
 for(uint32_t a=T;a<T+0x300;a+=4)store(a,0);
 store(T,VT_);store(T+544,0);
 {Context cx=fresh();cx.r3.u32=T;cx.r4.u32=NM;cx.r5.u32=S;sub_CTOR(cx,nullptr);}
 for(uint32_t a=C;a<C+0x100;a+=4)store(a,0);
 Context cx=fresh();cx.r3.u32=T;cx.r4.u32=C+16;cx.r5.u32=C+20;cx.r6.u32=C+8;cx.r7.u32=C+12;sub_LOAD(cx,nullptr);
 uint32_t ok=cx.r3.u32&0xFF;
 uint32_t n=load(C+16),list=load(C+20);
 printf("HDR ok=%u count=%u second=%u\n",ok,n,load(C+8));
 for(uint32_t i=0;i<n;i++){
  uint32_t r=list+80*i;uint64_t h=0xcbf29ce484222325ULL,hm=0xcbf29ce484222325ULL;
  for(int k=0;k<4;k++)h=fnv(h,load(r+4*k));
  uint32_t nv=*pointer(r+66),ne=*pointer(r+65),np=*pointer(r+67);
  h=fnv(h,nv);uint32_t va=load(r+52);
  for(uint32_t v=0;v<nv;v++)for(int k=0;k<3;k++)h=fnv(h,load(va+16*v+4*k));
  h=fnv(h,ne);uint32_t ea=load(r+56);
  for(uint32_t e=0;e<ne;e++){h=fnv(h,*pointer(ea+2*e));h=fnv(h,*pointer(ea+2*e+1));}
  h=fnv(h,np);uint32_t pa=load(r+48);
  for(uint32_t p=0;p<np;p++){uint32_t q=pa+16*p;uint32_t c=*pointer(q+5);h=fnv(h,c);
   uint32_t vi=c>4?load(q+8):q+8,ei=c>4?load(q+12):q+12;
   for(uint32_t k=0;k<c;k++)h=fnv(h,*pointer(vi+k));
   for(uint32_t k=0;k<c;k++)h=fnv(h,*pointer(ei+k));
   hm=fnv(hm,load(q));}
  printf("OC %u %016llx %016llx %u %u %u\n",i,(unsigned long long)h,(unsigned long long)hm,nv,ne,np);
 }
 for(uint32_t i=0;i<n&&i<3;i++){uint32_t r=list+80*i;uint32_t nv=*pointer(r+66),np=*pointer(r+67);uint32_t va=load(r+52),pa=load(r+48);
  printf("OCT %u sphere %08x %08x %08x %08x nv=%u w0=%08x np=%u\n",i,load(r),load(r+4),load(r+8),load(r+12),nv,load(va+12),np);
  for(uint32_t p=0;p<np;p++){uint32_t q=pa+16*p;uint32_t c=*pointer(q+5);uint32_t vi=c>4?load(q+8):q+8,ei=c>4?load(q+12):q+12;
   printf("OCT  poly %u mask=%08x flag4=%u n=%u v:",p,load(q),*pointer(q+4),c);for(uint32_t k=0;k<c;k++)printf(" %u",*pointer(vi+k));printf(" e:");for(uint32_t k=0;k<c;k++)printf(" %u",*pointer(ei+k));printf("\n");}}
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--file', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    size = a.file.stat().st_size
    if not 0 < size <= 0x700000:
        raise SystemExit(f'input size {size} outside the harness buffer')
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    a.work.mkdir(parents=True, exist_ok=True)

    def addr(fid):
        return subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1]

    def name(fid):
        return 'sub_' + addr(fid)
    rc.EXTRA += COMMON
    # the stream's refill (reads one byte through the device when the buffer is used up) answers end of file
    refill, ctor, getptd = 'F-19996', 'F-18827', 'F-62587'     # the stream refill, the tokenizer constructor, the C runtime's per-thread data lookup
    # (the C runtime's per-thread data lookup answers none, so its number parsing takes the static C locale the image holds - the
    # path of a thread without its own data)
    stubs = {name(refill): 'ctx.r3.s64=-1;', name(getptd): 'ctx.r3.u64=0;'}
    src = (MAIN.replace('sub_CTOR(', name(ctor) + '(').replace('sub_LOAD(', name('F-67870') + '(').replace('RT_THREAD_', '0x%08Xu' % rd.RT_THREAD).replace('ARENA_', '0x%08Xu' % rd.ARENA)
           .replace('VT_', '0x%08Xu' % VT))
    exe = rd.build(idx, [name('F-67870'), name(ctor)], src, a.work, 'occluder_parse', image=img, stubs=stubs, vtables=[(VT, 20)])
    p = subprocess.run([str(exe), str(a.image), str(a.file)], capture_output=True, text=True)
    if p.returncode:
        raise SystemExit(p.stderr[-800:] or f'exit {p.returncode}')
    a.out.write_text(p.stdout.replace('\r', ''))
    print(p.stdout.splitlines()[0], len(p.stdout.splitlines()), 'lines')


if __name__ == '__main__':
    main()
