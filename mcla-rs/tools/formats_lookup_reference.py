"""Archive, resource-header, evolution and blend-target lookups run offline: the generated routines unchanged on a flat guest
memory (the recomp closure, scalar shim), what they call through a vtable answered by host bodies.

  find   F-82183  archive FindEntry(path): the real tables of contents of the disc's archives in guest memory (entries as the
                  loader keeps them: 16 bytes of numbers), every path the dictionary names plus odd spellings and misses
  hash   F-58929  the name hash the lookups use (seed, quote, signed bytes); called with its own arguments
  info   F-30036  GetResourceInfo(name, &flags): the archive's find (vtable slot 36) answered with a case's entry (null or words
                  2 and 3), the stream layer (open / read 12 bytes / close, vtable slots 2 / 7 / 13) by a host body that
                  hands a case's header words; F-46450 (the header check) runs inside it
  evo    F-95887  evolution index by name: the table built by the builder routine (F-94093: its two parser calls F-75015 / F-29871 stubbed, they
                  fill other fields; the allocator through the vtable answered by a bump pointer) from a group's names, then
                  looked up with the binary search routine F-26240 (its error paths F-52205 / F-37557 stubbed) for every query name
  blend  F-75109  blend target lookup by a u16 key: the real resource files' hash tables in guest memory (virtual block at
                  0x50000000), every key 0..65535

Reads <in-dir>/toc_*.txt (dump_real_tocs), evo_groups.txt (dump_evo_groups), blend_files.txt (dump_blend_files) written by
the ignored tests of crates/formats/src/lookup_tests.rs; writes <testdata>/rpf_lookup_ref.txt, formats_lookup_ref.txt and, in
<out-dir>, the big archive's reference."""
import argparse
import random
import re
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build
from recomp_leaf import shim as _shim


def no_addr(text):
    """A word that reads like an address of the original gets a leading h (the citation check; the tests strip it)."""
    return re.sub(r'(?<![0-9A-Za-z])82[0-9a-fA-F]{6}(?![0-9A-Za-z])', lambda m: 'h' + m.group(0), text)

M = 0xFFFFFFFF


def at_hash(bs: bytes, seed=0) -> int:
    """The original's name hash, written from its instructions (a quoted name is hashed inside the quotes, bytes are signed,
    A-Z lowered, backslash to slash)."""
    h = seed & M
    i = 0
    q = len(bs) > 0 and bs[0] == 34
    if q:
        i = 1
    while i < len(bs) and bs[i] != 0 and not (q and bs[i] == 34):
        c = bs[i]
        i += 1
        if 65 <= c <= 90:
            c += 32
        elif c == 92:
            c = 47
        h = (h + (c - 256 if c >= 128 else c)) & M
        h = (h + (h << 10)) & M
        h ^= h >> 6
    h = (h + (h << 3)) & M
    h ^= h >> 11
    h = (h + (h << 15)) & M
    return h


def addr(tool, fid):
    return subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]


EXTRA_C = r'''
static void dispatch(Context&,const uint8_t*,uint32_t);
#define REX_CALL_INDIRECT_FUNC(a) dispatch(ctx,base,a)
static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}
'''

MAIN_C = r'''
static std::ifstream in_;
static std::string tok(){std::string t;in_>>t;return t;}
static uint32_t hx(const std::string&s){return (uint32_t)std::stoul(s,nullptr,16);}
static std::string unhex(const std::string&s){std::string r;if(s=="-")return r;for(size_t i=0;i+1<s.size();i+=2)r.push_back((char)std::stoul(s.substr(i,2),nullptr,16));return r;}
static void put_bytes(uint32_t a,const std::string&b){for(size_t i=0;i<b.size();i++)*pointer(a+i)=(uint8_t)b[i];*pointer(a+b.size())=0;}
static uint32_t BUMP=0x8000,FIND_RES=0,OPEN_FAIL=0,AVAIL=12,HDR[3]={0,0,0};
static void dispatch(Context&ctx,const uint8_t*base,uint32_t a){
  switch(a){
  case 0x7F000010:{uint32_t sz=ctx.r4.u32;ctx.r3.u64=BUMP;BUMP+=(sz+15)&~15u;break;}
  case 0x7F000011:break;
  case 0xCMPADDR:sub_CMP(ctx,base);break;
  case 0x7F000001:ctx.r3.u64=FIND_RES;break;
  case 0x7F000002:if(OPEN_FAIL){ctx.r3.s64=-1;}else{store(ctx.r5.u32,0);store(ctx.r5.u32+4,0x1234);ctx.r3.u64=7;}break;
  case 0x7F000003:{uint32_t buf=ctx.r6.u32;unsigned n=AVAIL<12?AVAIL:12;for(unsigned i=0;i<n;i++)*pointer(buf+i)=uint8_t(HDR[i/4]>>(24-8*(i%4)));ctx.r3.s64=n;break;}
  case 0x7F000004:break;
  default:throw std::runtime_error("unexpected indirect call");}
}
int main(int argc,char**argv){
  std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
  in_.open(argv[2]);std::string root=argv[3];
  for(;;){
    std::string t=tok();if(t=="END"||t.empty())break;
    if(t=="A"){ // archive: name N, N entries, Q M paths
      tok();int n=std::stoi(tok());
      for(int i=0;i<n;i++)for(int k=0;k<4;k++)store(0x10000+16*i+4*k,hx(tok()));
      tok();int m=std::stoi(tok());
      for(int i=0;i<m;i++){
        std::string p=unhex(tok());
        Context ctx{};ctx.r1.u32=0x2F00;store(0x1008,0x10000);store(0x1004,0);*pointer(0x1000+1129)=0;
        put_bytes(0x90000,p);ctx.r3.u32=0x1000;ctx.r4.u32=0x90000;
        sub_FIND(ctx,nullptr);
        uint32_t r=ctx.r3.u32;
        if(r==0)printf("-1\n");else if(r>=0x10000&&r<0x10000+16u*n&&((r-0x10000)%16)==0)printf("%u\n",(r-0x10000)/16);else printf("X%08x\n",r);
      }
    }else if(t=="H"){
      int m=std::stoi(tok());
      for(int i=0;i<m;i++){
        uint32_t seed=hx(tok());std::string p=unhex(tok());
        Context ctx{};ctx.r1.u32=0x2F00;put_bytes(0x90000,p);ctx.r3.u32=0x90000;ctx.r4.u32=seed;
        sub_HASH(ctx,nullptr);printf("%08x\n",ctx.r3.u32);
      }
    }else if(t=="G"){ // evolution group: k names, P m queries
      int k=std::stoi(tok());
      memset(objects,0,0x10000);BUMP=0x8000;
      store(0x1E00,0x1E10);store(0x1E10+28,0x1E80);store(0x1E80,0x1EA0);store(0x1EA0+8,0x7F000010);store(0x1EA0+12,0x7F000011);
      store(0x1000,0x4000);store16(0x1004,k);
      for(int i=0;i<k;i++){store(0x4000+4*i,0x4100+8*i);store(0x4100+8*i,0x5000+64*i);put_bytes(0x5000+64*i,unhex(tok()));}
      Context b{};b.r1.u32=0x2F00;b.r13.u32=0x1E00;b.r3.u32=0x1000;b.r4.u32=0;
      sub_BUILD(b,nullptr);
      tok();int m=std::stoi(tok());
      for(int i=0;i<m;i++){
        Context ctx{};ctx.r1.u32=0x2F00;ctx.r13.u32=0x1E00;put_bytes(0x6000,unhex(tok()));ctx.r3.u32=0x1000;ctx.r4.u32=0x6000;
        sub_EVO(ctx,nullptr);printf("%d\n",ctx.r3.s32);
      }
    }else if(t=="B"){
      int nf=std::stoi(tok());
      for(int f=0;f<nf;f++){
        std::string rel=tok();std::ifstream fi(root+"/"+rel,std::ios::binary);std::vector<uint8_t> d((std::istreambuf_iterator<char>(fi)),{});
        if(d.size()<16||memcmp(d.data(),"MCLR",4)){printf("BAD\n");continue;}
        uint32_t vs=d[12]|d[13]<<8|d[14]<<16|(uint32_t)d[15]<<24;if(vs>d.size()-16)vs=d.size()-16;
        VBLOCK.assign(d.begin()+16,d.begin()+16+vs);
        std::string out;int hits=0;bool bad=false;
        try{
          for(uint32_t key=0;key<65536;key++){
            memset(objects,0,0x2000);store16(0x1100,key);
            Context c{};c.r1.u32=0x2F00;c.r3.u32=0x50000014;c.r4.u32=0x1100;
            sub_BLEND(c,nullptr);
            if(c.r3.u32){uint32_t tp=load(c.r3.u32);char b[64];snprintf(b,64,"%u %u\n",key,load(tp+0x44));out+=b;hits++;}
          }
        }catch(std::exception&){bad=true;}
        if(bad)printf("BAD\n");else printf("F %d\n%s",hits,out.c_str());
      }
    }else if(t=="I"){
      int n=std::stoi(tok());
      for(int i=0;i<n;i++){
        uint32_t found=hx(tok()),c=hx(tok()),d=hx(tok());AVAIL=hx(tok());HDR[0]=hx(tok());HDR[1]=hx(tok());HDR[2]=hx(tok());OPEN_FAIL=hx(tok());uint32_t f0=hx(tok());
        memset(objects,0,0x2000);
        store(0x1000,0x1100);store(0x1100+144,0x7F000001);store(0x1100+8,0x7F000002);store(0x1100+28,0x7F000003);store(0x1100+52,0x7F000004);
        store(0x1300+8,c);store(0x1300+12,d);FIND_RES=found?0x1300:0;store(0x1500,f0);
        Context ctx{};ctx.r1.u32=0x2F00;ctx.r3.u32=0x1000;ctx.r4.u32=0x1600;ctx.r5.u32=0x1500;
        sub_INFO(ctx,nullptr);
        printf("%d %08x\n",ctx.r3.s32,load(0x1500));
      }
    }
  }
}
'''


def hexs(b: bytes) -> str:
    return b.hex() if b else '-'


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--in-dir', type=Path, required=True)
    ap.add_argument('--data-dir', type=Path, default=Path('D:/MCLA/extracted/cache'))
    ap.add_argument('--names', type=Path, required=True)
    ap.add_argument('--testdata', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    ffind, fhash, finfo, fevo, fbuild, fbs = (addr(a.tool, f) for f in ('F-82183', 'F-58929', 'F-30036', 'F-95887', 'F-94093', 'F-75109'))
    fcmp = addr(a.tool, 'F-57077')  # the binary search's comparison of two u32 keys (an indirect call by address)
    stub_ids = ('F-75015', 'F-29871', 'F-52205', 'F-37557')  # the builder's two parser calls, the bsearch's two error paths
    rnd = random.Random(821)
    names = [l.strip() for l in a.names.read_text(errors='replace').splitlines() if l.strip() and not l.startswith('#')]
    by_hash = {}
    for n in names:
        by_hash.setdefault(at_hash(n.encode()), n)

    inp = []
    meta = []  # parallel list of (kind, ref-prefix) per output line group

    # ---- find ----
    tocs = sorted(a.in_dir.glob('toc_*.txt'))
    arch = []  # (name, entries, queries)
    for p in tocs:
        lines = p.read_text().splitlines()
        n = int(lines[0].split()[1])
        ents = [tuple(int(x, 16) for x in l.split()[1:5]) for l in lines[1:1 + n]]
        # tree of names
        path = {0: ''}
        order = [0]
        known = {0: True}
        for i in order:
            h, b, c, d = ents[i]
            if c & 0x80000000:
                first, cnt = c & 0x7FFFFFFF, d & 0x3FFFFFFF
                if first > n or first + cnt > n:
                    continue
                for j in range(first, first + cnt):
                    nm = by_hash.get(ents[j][0])
                    if nm is not None and known.get(i):
                        path[j] = (path[i] + '/' + nm) if path[i] else nm
                        known[j] = True
                    order.append(j)
        known_paths = [(j, p_) for j, p_ in path.items() if j != 0 and known.get(j)]
        qs = []
        cap = 2500 if n < 5000 else 1200
        samp = known_paths if len(known_paths) <= cap else rnd.sample(known_paths, cap)
        for j, p_ in samp:
            qs.append(p_)
            qs.append(p_.upper())
            qs.append(p_.replace('/', '\\'))
            qs.append('/' + p_)
            qs.append('\\' + p_)
            qs.append(p_ + '/')
            qs.append('//' + p_)
            if '/' in p_:
                qs.append(p_.replace('/', '//', 1))
                qs.append(p_.rsplit('/', 1)[0] + '/zz_missing')
                qs.append(p_.replace('/', '/\\', 1))
            qs.append(p_ + '/extra')
            qs.append(p_ + '/extra/more')
            qs.append(p_[: len(p_) // 2])
        for j, p_ in rnd.sample(known_paths, min(len(known_paths), 300)):
            qs.append(p_.title())
            qs.append('x' + p_)
            qs.append(p_ + '.')
            qs.append(p_ + '\x00junk')
        qs += ['', '/', '\\', '//', '///', '//a', '.', 'a', 'zz_missing', 'a/b/c', '"x"', '"', ' ', 'ä', 'ä/b', '\x00']
        arch.append((p.stem[4:], ents, qs))
    for name, ents, qs in arch:
        inp.append('A %s %d' % (name, len(ents)))
        for e in ents:
            inp.append('%08x %08x %08x %08x' % e)
        inp.append('Q %d' % len(qs))
        inp.append('\n'.join(hexs(q.encode('latin-1' if all(ord(c) < 256 for c in q) else 'utf-8') if False else q.encode('utf-8')) for q in qs))

    # ---- hash ----
    pool = [b'"', b'\\', b'/', b'A', b'z', b'0', b'.', b' ', b'\x80', b'\xff', b'\xe4', b'_', b'-', b'Z', b'a']
    hq = []
    for _ in range(3000):
        k = rnd.choice((0, 1, 2, 3, 5, 8, 13, 30))
        s = b''.join(rnd.choice(pool) for _ in range(k))
        hq.append((rnd.choice((0, 0, 0, rnd.getrandbits(32))), s))
    for n in rnd.sample(names, min(len(names), 2000)):
        hq.append((0, n.encode()))
    for s in (b'', b'"', b'""', b'"a"', b'"a', b'a"b"', b'"a"b'):
        hq.append((0, s))
    inp.append('H %d' % len(hq))
    inp.append('\n'.join('%08x %s' % (s, hexs(b)) for s, b in hq))

    # ---- evolution groups ----
    real = []
    for l in (a.in_dir / 'evo_groups.txt').read_text().splitlines():
        g = [bytes.fromhex(x) for x in l.split()]
        if g not in real:
            real.append(g)
    # a hash collision of two spellings
    seen = {}
    coll = None
    r2 = random.Random(5)
    while coll is None:
        s = bytes(r2.choice(b'abcdefghijklmnopqrstuvwxyz') for _ in range(6))
        h = at_hash(s)
        if h in seen and seen[h] != s:
            coll = (seen[h], s)
        seen[h] = s
    words = [b'lod', b'LOD', b'speed', b'Speed', b'donut', b'fwd', b'throttle', b'Height', b'tireSize', b'alpha', b'time', b'a', b'"lod"', b'"q', b'\\x', b'/x', b'\xe4\xf6', b'\x80', b'x' * 40, coll[0], coll[1], b'']
    groups = list(real)
    for _ in range(400):
        k = rnd.choice((0, 1, 1, 2, 3, 4, 6, 9, 16, 17, 20, 30))
        groups.append([rnd.choice(words) for _ in range(k)])
    groups.append([b'abc', b'ABC'])
    groups.append([coll[0], coll[1]])
    groups.append([coll[1], coll[0]])
    gq = []
    for g in groups:
        qs = []
        for n in g:
            qs += [n, n.upper(), n.lower(), n.title(), n + b'x', n[:-1], b'x' + n, n.replace(b'/', b'\\')]
        qs += [b'', b'lod', b'LOD', b'zzz', b'"lod"', b'time', b'HEIGHT', coll[0], coll[1], b'"' + (g[0] if g else b'a') + b'"']
        qs = [q for q in qs if len(q) < 60 and b'\x00' not in q]
        gq.append(qs)
    for g, qs in zip(groups, gq):
        inp.append('G %d %s' % (len(g), ' '.join(hexs(n) for n in g)))
        inp.append('P %d' % len(qs))
        inp.append(' '.join(hexs(q) for q in qs) if qs else '')

    # ---- blend ----
    bfiles = [l.strip() for l in (a.in_dir / 'blend_files.txt').read_text().splitlines() if l.strip()]
    inp.append('B %d' % len(bfiles))
    inp.append('\n'.join(bfiles))

    # ---- resource info ----
    icases = []
    MAGIC = 0x05435352
    for _ in range(6000):
        found = rnd.choice((0, 1, 1, 1, 1))
        c = rnd.choice((rnd.getrandbits(32), rnd.getrandbits(32) & 0x7FFFF0FF, (rnd.getrandbits(19) << 12) | rnd.randrange(0, 256), rnd.getrandbits(32) | 0x80000000))
        d = rnd.choice((rnd.getrandbits(32), rnd.getrandbits(32) | 0x80000000, rnd.getrandbits(32) & 0x7FFFFFFF, 0x80000000 | rnd.getrandbits(8)))
        avail = rnd.choice((12, 12, 12, 12, 11, 0, 4, 8, 13))
        w0 = rnd.choice((MAGIC, MAGIC, MAGIC, rnd.getrandbits(32), 0x52534305, MAGIC ^ 1))
        w1, w2 = rnd.getrandbits(32), rnd.getrandbits(32)
        openfail = rnd.choice((0, 0, 0, 1))
        f0 = rnd.choice((0xA5A5A5A5, 0, M))
        icases.append((found, c, d, avail, w0, w1, w2, openfail, f0))
    inp.append('I %d' % len(icases))
    inp.append('\n'.join('%x %08x %08x %x %08x %08x %08x %x %08x' % x for x in icases))
    inp.append('END')

    out = a.out_dir
    out.mkdir(parents=True, exist_ok=True)
    (out / 'formats_inputs.txt').write_text('\n'.join(inp) + '\n')

    idx = index(a.recomp)
    stubs = {'sub_' + addr(a.tool, f): '' for f in stub_ids}
    # the guest address space: the resource's virtual block at 0x50000000 beside the scratch area and the image
    orig_shim = recomp_closure.shim

    def shim2(scratch_end=0x3000):
        s = orig_shim(scratch_end)
        s = s.replace('static uint8_t* pointer(uint32_t a) {',
                      'static std::vector<uint8_t> VBLOCK;\nstatic uint8_t* pointer(uint32_t a) {\n'
                      '    if ((a>>28)==5){ uint32_t o=a-0x50000000u; if(o+4>VBLOCK.size()) throw std::runtime_error("virtual block"); return VBLOCK.data()+o; }', 1)
        return s.replace('uint16_t u16; uint8_t u8;', 'uint16_t u16; uint8_t u8; int8_t s8; int16_t s16;', 1)

    recomp_closure.shim = shim2
    recomp_closure.EXTRA += EXTRA_C
    main_c = (MAIN_C.replace('sub_FIND', 'sub_' + ffind).replace('sub_HASH', 'sub_' + fhash).replace('sub_BUILD', 'sub_' + fbuild)
              .replace('sub_EVO', 'sub_' + fevo).replace('sub_BLEND', 'sub_' + fbs).replace('0xCMPADDR', '0x' + fcmp).replace('sub_CMP', 'sub_' + fcmp).replace('sub_INFO', 'sub_' + finfo))
    exe = build(idx, ['sub_' + x for x in (ffind, fhash, finfo, fevo, fbuild, fbs, fcmp)], main_c, out, 'formats_lookup', stubs=stubs,
                scratch_end=0x1000 + 0x200000)
    r = subprocess.run([str(exe), str(a.image), str(out / 'formats_inputs.txt'), str(a.data_dir)], capture_output=True, text=True, check=True)
    res = r.stdout.split('\n')
    pos = 0

    def take(n):
        nonlocal pos
        v = res[pos:pos + n]
        pos += n
        return v

    small, big = [], []
    bad_x = 0
    for name, ents, qs in arch:
        lines = ['A %s %d' % (name, len(ents))] + ['E %08x %08x %08x %08x' % e for e in ents]
        for q, o in zip(qs, take(len(qs))):
            if o.startswith('X'):
                bad_x += 1
            lines.append('Q %s %s' % (hexs(q.encode('utf-8')), o))
        (big if len(ents) > 5000 else small).extend(lines)
    (a.testdata / 'rpf_lookup_ref.txt').write_text(no_addr('\n'.join(small) + '\n'))
    (out / 'rpf_lookup_ref_cache.txt').write_text('\n'.join(big) + '\n')
    f = []
    for (s, b), o in zip(hq, take(len(hq))):
        f.append('H %08x %s %s' % (s, hexs(b), o))
    for g, qs in zip(groups, gq):
        f.append('G %s' % ' '.join(hexs(n) for n in g))
        for q, o in zip(qs, take(len(qs))):
            f.append('P %s %s' % (hexs(q), o))
    for bf in bfiles:
        h = take(1)[0]
        if h == 'BAD':
            f.append('B %s BAD' % bf)
            continue
        n = int(h.split()[1])
        f.append('B %s %d' % (bf, n))
        for l in take(n):
            f.append('K ' + l)
    for c, o in zip(icases, take(len(icases))):
        f.append('I %x %08x %08x %x %08x %08x %08x %x %08x -> %s' % (*c, o))
    (a.testdata / 'formats_lookup_ref.txt').write_text(no_addr('\n'.join(f) + '\n'))
    print({'archives': [(n, len(e), len(q)) for n, e, q in arch], 'hash': len(hq), 'groups': len(groups), 'blend': len(bfiles),
           'info': len(icases), 'unexpected_pointers': bad_x})


if __name__ == '__main__':
    main()
