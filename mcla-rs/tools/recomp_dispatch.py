"""recomp_closure plus virtual calls and kernel-import stubs, so closures that call through vtables or into the C runtime can run offline.

Use it like recomp_closure (same `index`, `closure`, `build`, `run`), with more arguments to `build`:

    import recomp_dispatch as rd
    idx = rd.index(Path('D:/MCLA-static'))
    img = rd.load_image(Path('work/2026-10-10/original-image.bin'))
    vt = rd.vtables_of(img, 'swfSHAPE@rage')[0][0]            # RTTI name -> vtable address (docs/UI.md "RTTI -> vtables")
    exe = rd.build(idx, ['sub_xxxxxxxx'], MAIN, work, 'tag', image=img, vtables=[(vt, 8)])   # the first 8 slots are dispatch targets
    # MAIN builds its objects with store(addr, vtable) and calls rt_reset(); fresh() gives a Context with r1 and r13 set.

What it adds (all documented in docs/FORMATS_TOOLS.md, "Offline closure runner: virtual calls and kernel imports"):

* Virtual / indirect calls. The generated code ends every `bctrl` in REX_CALL_INDIRECT_FUNC(target). Here that is a switch over a
  table of known targets: the functions named in `targets=[addr..]`, the slots of `vtables=[(vtable, nslots)]` (read from the
  executable image: the real vtables are in it, an object only needs its vptr stored) and `fakes={addr: body}` (host bodies at
  addresses outside the image, e.g. the allocator below). Each target's own closure is compiled in. A target not in the table throws
  `unresolved indirect call 0x........` at run time, which names the address to add.
* Kernel imports: KERNEL below gives every import that closures were seen to reach a body with a fixed, documented behaviour
  (single thread, no locks, a bump allocator, a TLS table, ...). An import without a body is compiled as a throw
  (`unmodelled import X`), so a case that really reaches one fails loudly instead of returning a made-up value.
* Guest memory beyond recomp_leaf's: a 16 MB zeroed arena at 0x40000000 (the allocator's heap, never reused; `free` is a no-op),
  the thread block r13 points at, with the allocator object the game's operator new / delete call through (fake vtable, slot 2 =
  allocate(size, align), slot 3 = free), and the shim additions the UI closures needed (signed byte / half views, ctr, all cr fields).
"""
import re
import struct
from pathlib import Path
import recomp_closure as rc
from recomp_leaf import extract, shim as _shim

IMAGE_BASE = 0x82000000
ARENA = 0x40000000
ARENA_SIZE = 0x1000000
RT_THREAD = ARENA + 0x100      # what r13 points at: [0] -> RT_BLOCK
RT_BLOCK = ARENA + 0x200       # [28] = allocator object
RT_ALLOC = ARENA + 0x300       # allocator object: [0] = RT_ALLOC_VT
RT_ALLOC_VT = ARENA + 0x400    # vtable: slot 2 = allocate, slot 3 = free
FAKE_ALLOC, FAKE_FREE = 0x7F000002, 0x7F000003
HEAP_START = ARENA + 0x1000


def index(recomp: Path) -> dict:
    return rc.index(recomp)


def load_image(path: Path) -> bytes:
    return Path(path).read_bytes()


def image_u32(img: bytes, a: int) -> int:
    return struct.unpack_from('>I', img, a - IMAGE_BASE)[0]


def vtables_of(img: bytes, cls: str):
    """RTTI lookup: class name as in the type descriptor without `.?AV` and `@@` (`swfSHAPE@rage`) -> [(vtable address, this offset)].
    type descriptor (`.?AV<cls>@@`, 8 bytes after its start) -> complete object locators (+12 holds the descriptor address) ->
    vtables (vtable[-1] holds the locator)."""
    out = []
    name = b'.?AV' + cls.encode() + b'@@\0'
    p = img.find(name)
    while p >= 0:
        td = IMAGE_BASE + p - 8
        pat = struct.pack('>I', td)
        q = img.find(pat)
        while q >= 0:
            col = IMAGE_BASE + q - 12
            if q >= 12 and struct.unpack_from('>I', img, q - 12)[0] == 0:
                off = struct.unpack_from('>I', img, q - 8)[0]
                cpat = struct.pack('>I', col)
                r = img.find(cpat)
                while r >= 0:
                    out.append((IMAGE_BASE + r + 4, off))
                    r = img.find(cpat, r + 1)
            q = img.find(pat, q + 1)
        p = img.find(name, p + 1)
    return sorted(set(out))


def stored_pointers(idx, name, base_reg=None):
    """Constants a (straight-line) function stores to memory that lie in the image and are word aligned - the vtable pointers a
    constructor stores, the function pointers a table initialiser stores. A tiny symbolic pass over the generated text
    (`lis` / `li` / `addi` forming the constants), so a tool can find a class's vtable without a raw address in it
    (docs/FORMATS_TOOLS.md). `base_reg` None: all stores, returned as {order: value}; 3 (`this`): {offset from r3: value}."""
    regs, out = {}, {}
    for line in extract(idx[name], name).splitlines():
        s = line.strip()
        m = re.match(r'ctx\.r(\d+)\.[su]64 = (-?\d+);', s)
        if m:
            regs[int(m.group(1))] = int(m.group(2)) & 0xFFFFFFFF
            continue
        m = re.match(r'ctx\.r(\d+)\.[su]64 = ctx\.r(\d+)\.[su]64 \+ (-?\d+);', s)
        if m:
            src = regs.get(int(m.group(2)))
            if src is None:
                regs.pop(int(m.group(1)), None)
            else:
                regs[int(m.group(1))] = (src + int(m.group(3))) & 0xFFFFFFFF
            continue
        m = re.match(r'REX_STORE_U32\(ctx\.r(\d+)\.u32 \+ (-?\d+), ctx\.r(\d+)\.u32\);', s)
        if m:
            v = regs.get(int(m.group(3)), 0)
            if v >= IMAGE_BASE and v & 3 == 0 and (base_reg is None or int(m.group(1)) == base_reg):
                out[len(out) if base_reg is None else int(m.group(2))] = v
            continue
        m = re.match(r'ctx\.r(\d+)\.', s)       # any other write to a register forgets it
        if m and ' = ' in s:
            regs.pop(int(m.group(1)), None)
    return out


# --- kernel imports -------------------------------------------------------------------------------------------------------
# name -> C++ body (Context &ctx, const uint8_t *base). Arguments in r3..; the result in r3. Anything not listed is compiled as
# a throw. "single thread" bodies: the closure runs on one thread, so locks and IRQL changes have no effect on values.
KERNEL = {
    # locks and interrupt levels: single thread, nothing to wait for
    '__imp__RtlEnterCriticalSection': '',
    '__imp__RtlLeaveCriticalSection': '',
    '__imp__RtlInitializeCriticalSection': '',
    '__imp__KfAcquireSpinLock': 'ctx.r3.u64=0;',
    '__imp__KfReleaseSpinLock': '',
    '__imp__KeRaiseIrqlToDpcLevel': 'ctx.r3.u64=0;',
    '__imp__KfLowerIrql': '',
    '__imp__KeAcquireSpinLockAtRaisedIrql': '',
    '__imp__KeReleaseSpinLockFromRaisedIrql': '',
    '__imp__KeGetCurrentProcessType': 'ctx.r3.u64=1;',        # user process
    # thread local storage: one table, indexes as the game's own TlsAlloc'd words (zero in the image: index 0), values start at 0
    '__imp__KeTlsGetValue': 'ctx.r3.u64=rt_tls[ctx.r3.u32&63];',
    '__imp__KeTlsSetValue': 'rt_tls[ctx.r3.u32&63]=ctx.r4.u32;ctx.r3.u64=1;',
    # virtual memory: a bump allocation in the arena, size rounded up to 4 KB pages, zeroed (the arena is never reused); r3 -> base
    # (in/out), r4 -> size (in/out), r5 type, r6 protection
    '__imp__NtAllocateVirtualMemory':
        'uint32_t want=load(ctx.r4.u32);uint32_t pg=(want+0xFFF)&~0xFFFu;uint32_t at=load(ctx.r3.u32);'
        'if(!at)at=rt_alloc(pg,0x1000);store(ctx.r3.u32,at);store(ctx.r4.u32,pg);ctx.r3.u64=0;',
    '__imp__NtFreeVirtualMemory': 'ctx.r3.u64=0;',
    '__imp__RtlCompareMemoryUlong':   # (source, length, pattern): the number of bytes equal to the pattern, in whole words
        'uint32_t n=0;for(uint32_t i=0;i+4<=ctx.r4.u32;i+=4){if(load(ctx.r3.u32+i)!=ctx.r5.u32)break;n+=4;}ctx.r3.u64=n;',
    '__imp__RtlInitAnsiString':       # (ANSI_STRING *dst, const char *src): Length, MaximumLength (incl. the NUL), Buffer
        'uint32_t n=0;if(ctx.r4.u32)while(*pointer(ctx.r4.u32+n))n++;'
        'store16(ctx.r3.u32,n);store16(ctx.r3.u32+2,ctx.r4.u32?n+1:0);store(ctx.r3.u32+4,ctx.r4.u32);',
    # errors: reaching these is a failed case, not a behaviour to model
    '__imp__KeBugCheck': 'throw std::runtime_error("KeBugCheck");',
    '__imp__KeBugCheckEx': 'throw std::runtime_error("KeBugCheckEx");',
    '__imp__RtlRaiseException': 'throw std::runtime_error("RtlRaiseException");',
    # time: a counter that advances 1 tick (100 ns) per query, so repeated queries differ and stay reproducible
    '__imp__KeQuerySystemTime': 'rt_time+=10000;store(ctx.r3.u32,uint32_t(rt_time>>32));store(ctx.r3.u32+4,uint32_t(rt_time));',
    '__imp__KeDelayExecutionThread': 'ctx.r3.u64=0;',        # a sleep: returns success at once
    '__imp__DbgPrint': 'ctx.r3.u64=0;',                      # debug text: dropped
}
# Not modelled (a case that reaches one throws): files (NtWriteFile, NtOpenFile, NtWaitForSingleObjectEx, ...), threads, semaphores,
# XAM / XNet / video / input calls. They need a recording of the running original.

# sprintf / _snprintf / _vsnprintf: the host C library with the Xenon varargs layout (each argument one 8-byte slot, a 32-bit value
# in the low word, a double as 8 bytes; for sprintf the arguments come from r5..r10 then the stack at r1+0x54, floats from f1..).
FORMATTING = r'''
struct VaIn { Context*c; uint32_t n; uint32_t list; uint32_t fn; };
static uint32_t va_int(VaIn&v){uint32_t r;if(v.list){r=load(v.list+4);v.list+=8;}else if(v.n<6){r=(&v.c->r5)[v.n++].u32;}else{r=load(v.c->r1.u32+0x54+8*(v.n-6));v.n++;}return r;}
static double va_dbl(VaIn&v){double r;if(v.list){uint64_t b=load64(v.list);std::memcpy(&r,&b,8);v.list+=8;}else{r=(&v.c->f1)[v.fn++].f64;v.n++;}return r;}
static std::string fmt_c(const std::string&f,VaIn&v){std::string o;
 for(size_t i=0;i<f.size();i++){if(f[i]!='%'){o+=f[i];continue;}size_t j=i+1;std::string spec="%";
  while(j<f.size()&&strchr("-+ #0123456789.",f[j]))spec+=f[j++];
  if(j<f.size()&&(f[j]=='l'||f[j]=='h'))j++;
  char t=j<f.size()?f[j]:'%';char buf[512];
  if(t=='%'){o+='%';}
  else if(t=='s'){uint32_t a=va_int(v);std::string s;for(;a&&*pointer(a);a++)s+=char(*pointer(a));snprintf(buf,sizeof buf,(spec+"s").c_str(),s.c_str());o+=buf;}
  else if(t=='d'||t=='i'){snprintf(buf,sizeof buf,(spec+"d").c_str(),int(va_int(v)));o+=buf;}
  else if(t=='u'||t=='x'||t=='X'||t=='o'){snprintf(buf,sizeof buf,(spec+t).c_str(),va_int(v));o+=buf;}
  else if(t=='c'){snprintf(buf,sizeof buf,(spec+"c").c_str(),int(va_int(v)));o+=buf;}
  else if(t=='f'||t=='g'||t=='e'||t=='G'||t=='E'){snprintf(buf,sizeof buf,(spec+t).c_str(),va_dbl(v));o+=buf;}
  else throw std::runtime_error("format conversion not modelled");
  i=j;}
 return o;}
static void fmt_out(Context&c,uint32_t dst,uint32_t cap,bool bounded,const std::string&s){
 uint32_t n=uint32_t(s.size());if(bounded){if(cap){uint32_t m=n<cap?n:cap;for(uint32_t i=0;i<m;i++)*pointer(dst+i)=uint8_t(s[i]);if(m<cap)*pointer(dst+m)=0;}}
 else{for(uint32_t i=0;i<=n;i++)*pointer(dst+i)=i<n?uint8_t(s[i]):0;}
 c.r3.u64=n;}
static std::string cfmt(uint32_t a){std::string s;for(;*pointer(a);a++)s+=char(*pointer(a));return s;}
'''
KERNEL['__imp__sprintf'] = 'VaIn v{&ctx,0,0,0};fmt_out(ctx,ctx.r3.u32,0,false,fmt_c(cfmt(ctx.r4.u32),v));'
KERNEL['__imp___snprintf'] = ('VaIn v{&ctx,0,0,0};v.n=0;'   # (dst, count, fmt, ...): arguments from r6
                              'VaIn w{&ctx,1,0,0};w.n=1;fmt_out(ctx,ctx.r3.u32,ctx.r4.u32,true,fmt_c(cfmt(ctx.r5.u32),w));')
KERNEL['__imp___vsnprintf'] = 'VaIn v{&ctx,0,ctx.r6.u32,0};fmt_out(ctx,ctx.r3.u32,ctx.r4.u32,true,fmt_c(cfmt(ctx.r5.u32),v));'

RUNTIME = r'''
static uint32_t arena_top=HEAP_START_, arena_hi=HEAP_START_;
static uint32_t rt_tls[64]; static uint64_t rt_time=0x01D0000000000000ull;
static uint32_t rt_alloc(uint32_t size,uint32_t align){if(align<16)align=16;arena_top=(arena_top+align-1)&~(align-1);uint32_t r=arena_top;
 arena_top+=(size+15)&~15u;if(arena_top>ARENA_+ARENA_SIZE_)throw std::runtime_error("arena full");if(arena_top>arena_hi)arena_hi=arena_top;return r;}
static void rt_reset(){std::memset(objects,0,sizeof objects);std::memset(arena,0,arena_hi-ARENA_);arena_top=arena_hi=HEAP_START_;std::memset(rt_tls,0,sizeof rt_tls);
 store(RT_THREAD_,RT_BLOCK_);store(RT_BLOCK_+28,RT_ALLOC_);store(RT_ALLOC_,RT_ALLOC_VT_);store(RT_ALLOC_VT_+8,FAKE_ALLOC_);store(RT_ALLOC_VT_+12,FAKE_FREE_);
 if(arena_hi<ARENA_+0x1000)arena_hi=ARENA_+0x1000;}
static Context fresh(){Context c{};c.r1.u32=0x2f00;c.r13.u32=RT_THREAD_;return c;}
'''

SHIM_FIXES = [
    ('uint16_t u16; uint8_t u8;', 'uint16_t u16; uint8_t u8; int16_t s16; int8_t s8;'),
    ('struct Context { ', 'struct Context { PPCRegister ctr; '),
    ('Condition cr0,cr6;', 'Condition cr0,cr1,cr2,cr3,cr4,cr5,cr6,cr7;'),
]


def vector_shim() -> str:
    s = rc.vector_shim()
    for a, b in SHIM_FIXES:
        s = s.replace(a, b, 1)
    # the arena is part of guest memory
    s = s.replace('static uint8_t* pointer(uint32_t a) {',
                  'static uint8_t* pointer(uint32_t a) {\n    if (a>=0x40000000u && a<0x40000000u+0x1000000u) return arena+a-0x40000000u;', 1)
    s = s.replace('static std::vector<uint8_t> image;', 'static std::vector<uint8_t> image;\nstatic uint8_t arena[0x1000000]{};', 1)
    return s + ('struct Context; static void ppc_trap(Context&,const uint8_t*,int){throw std::runtime_error("trap");}\n'
                '#define REX_QUERY_TIMEBASE() rt_time\n#define REX_ENTER_GLOBAL_LOCK()\n#define REX_LEAVE_GLOBAL_LOCK()\n#define REX_CHECK_GLOBAL_LOCK()\n')


def _consts(s):
    for k, v in (('ARENA_SIZE_', ARENA_SIZE), ('HEAP_START_', HEAP_START), ('ARENA_', ARENA), ('RT_THREAD_', RT_THREAD),
                 ('RT_BLOCK_', RT_BLOCK), ('RT_ALLOC_VT_', RT_ALLOC_VT), ('RT_ALLOC_', RT_ALLOC), ('FAKE_ALLOC_', FAKE_ALLOC),
                 ('FAKE_FREE_', FAKE_FREE)):
        s = s.replace(k, hex(v) + 'u')
    return s


def _targets(idx, img, vtables, targets):
    found = {}
    for a in targets:
        found[a] = f'sub_{a:08X}'
    for vt, n in vtables:
        slots = range(n) if isinstance(n, int) else n
        for i in slots:
            a = image_u32(img, vt + 4 * i)
            if f'sub_{a:08X}' in idx:
                found[a] = f'sub_{a:08X}'
    return found


class _Skip(set):
    """closure() leaves these alone: the named stubs and every import (an import has no generated body)."""

    def __contains__(self, n):
        return n.startswith('__imp__') or set.__contains__(self, n)


def build(idx, names, main, out: Path, tag, stubs=(), vector=True, scratch_end=0x3000, image=None, vtables=(), targets=(),
          fakes=None, kernel=None, limit=400):
    """Compile `names` + the closure of their direct callees + the closures of the indirect targets, with the kernel stubs and the
    runtime above. `main` is C++ using sub_XXXXXXXX, rt_reset(), fresh(), load/store ... `kernel` overrides/extends KERNEL;
    `stubs` as in recomp_closure.build (names with empty bodies, or name -> body). Returns the exe path."""
    stubs = dict.fromkeys(stubs, '') if not isinstance(stubs, dict) else dict(stubs)
    tg = _targets(idx, image, vtables, targets) if (vtables or targets) else {}
    kern = dict(KERNEL)
    kern.update(kernel or {})
    fakes = {FAKE_ALLOC: 'ctx.r3.u32=rt_alloc(ctx.r4.u32,ctx.r5.u32);', FAKE_FREE: '', **(fakes or {})}
    skip = _Skip(set(stubs) | set(kern))
    fs = rc.closure(idx, list(names) + list(tg.values()), skip, limit)
    # imports the closure reaches that nothing models: compiled as a throw
    called = set()
    for b in fs.values():
        called |= set(re.findall(r'\b(__imp__\w+)\(ctx, base\)', b))
    for n in called:
        if n not in kern and n not in stubs:
            stubs[n] = 'throw std::runtime_error("unmodelled import %s");' % n
    used = {n: kern[n] for n in called if n in kern and n not in stubs}
    stubs.update(used)
    for n in list(fs):                     # a function the closure pulled in that is itself a stub/kernel name
        if n in stubs:
            del fs[n]
    protos = ''.join(f'void {n}(Context &ctx,const uint8_t *base);\n' for n in list(fs) + list(stubs))
    stubdefs = ''.join('void %s(Context &ctx,const uint8_t *base){%s}\n' % (n, b) for n, b in stubs.items())
    fakedefs = ''.join('static void fake_%X(Context &ctx,const uint8_t *base){%s}\n' % (a, b) for a, b in fakes.items())
    cases = ''.join('  case 0x%08Xu: %s(ctx,base); return;\n' % (a, n) for a, n in sorted(tg.items()))
    cases += ''.join('  case 0x%08Xu: fake_%X(ctx,base); return;\n' % (a, a) for a in sorted(fakes))
    dispatch = ('static void indirect(Context &ctx,const uint8_t *base,uint32_t a){switch(a){\n' + cases +
                '  default: { char m[64]; snprintf(m,sizeof m,"unresolved indirect call 0x%08X",a); throw std::runtime_error(m); }}}\n')
    out.mkdir(parents=True, exist_ok=True)
    head = vector_shim() if vector else _shim(scratch_end)
    # uses of the macro come after its declaration
    pre = ('#define REX_CALL_INDIRECT_FUNC(a) indirect(ctx,base,(a))\n'
           'static void indirect(Context &ctx,const uint8_t *base,uint32_t a);\n')
    src = (head + rc.EXTRA + _consts(RUNTIME) + FORMATTING + pre + protos + fakedefs + stubdefs + ''.join(fs.values()) + dispatch + main)
    cpp = out / f'{tag}.cpp'
    cpp.write_text(src)
    import subprocess
    exe = out / f'{tag}.exe'
    r = subprocess.run(['g++', '-std=c++17', '-O2', '-ffp-contract=off', '-static', '-I' + str(rc.SIMDE), str(cpp), '-o', str(exe)],
                       capture_output=True, text=True)
    if r.returncode:
        raise RuntimeError(r.stderr[:1500])
    return exe


run = rc.run
closure = rc.closure
