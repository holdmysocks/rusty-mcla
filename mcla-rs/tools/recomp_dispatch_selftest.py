"""Checks of recomp_dispatch itself (no port involved): RTTI lookup, the allocator behind operator new / delete (virtual calls through the
fake allocator object), a vtable slot resolved from the image, the import stubs (sprintf with the Xenon varargs layout).
Usage: recomp_dispatch_selftest.py --recomp D:/MCLA-static --image work/2026-10-10/original-image.bin --work DIR [--tool mcla-tool]"""
import argparse
import subprocess
from pathlib import Path
import recomp_dispatch as rd

NEW, DELETE, SHAPE_DRAW = 'F-15488', 'F-13813', 'F-19892'      # the game's operator new / delete; swfSHAPE's draw slot (docs/UI.md)

MAIN = r'''
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 rt_reset();
 Context a=fresh();a.r3.u32=100;sub_NEW(a,nullptr);Context b=fresh();b.r3.u32=33;sub_NEW(b,nullptr);
 printf("new %x %x\n",a.r3.u32,b.r3.u32);
 Context d=fresh();d.r3.u32=a.r3.u32;sub_DELETE(d,nullptr);
 // sprintf(buf, "%d|%s|%.2f|%x", -5, "abc", 2.5, 255): ints from r5.., the double in f1
 for(int i=0;i<4;i++)*pointer(0x1100+i)="abc"[i];
 const char*f="%d|%s|%.2f|%x";for(int i=0;i<14;i++)*pointer(0x1200+i)=f[i];
 Context s=fresh();s.r3.u32=0x1300;s.r4.u32=0x1200;s.r5.u32=uint32_t(-5);s.r6.u32=0x1100;s.f1.f64=2.5;s.r8.u32=255;__imp__sprintf(s,nullptr);
 printf("sprintf %u ",s.r3.u32);for(uint32_t p=0x1300;*pointer(p);p++)putchar(*pointer(p));printf("\n");
 // the same through _vsnprintf: the argument list in memory, one 8-byte slot each (value in the low word); a double also takes an integer slot
 store(0x1400,0);store(0x1404,uint32_t(-5));store(0x1408,0);store(0x140c,0x1100);store(0x1410,0);store(0x1414,255);
 const char*g="%d %s %x";for(int i=0;i<9;i++)*pointer(0x1500+i)=g[i];
 Context v=fresh();v.r3.u32=0x1600;v.r4.u32=64;v.r5.u32=0x1500;v.r6.u32=0x1404-4;__imp___vsnprintf(v,nullptr);
 printf("vsnprintf %u ",v.r3.u32);for(uint32_t p=0x1600;*pointer(p);p++)putchar(*pointer(p));printf("\n");
}
'''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--work', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    shape = rd.vtables_of(img, 'swfSHAPE@rage')
    addr = lambda f: subprocess.run([a.tool, 'registry', 'addr', f], capture_output=True, text=True, check=True).stdout.split()[1]
    # docs/UI.md "RTTI -> vtables": one vtable, and its slot 3 is the shape draw routine
    assert len(shape) == 1 and shape[0][1] == 0 and rd.image_u32(img, shape[0][0] + 12) == int(addr(SHAPE_DRAW), 16), shape
    new, dele = 'sub_' + addr(NEW), 'sub_' + addr(DELETE)
    src = MAIN.replace('sub_NEW', new).replace('sub_DELETE', dele)
    exe = rd.build(idx, [new, dele], src, a.work, 'dispatch_selftest', image=img,
                   stubs={'__imp__sprintf': rd.KERNEL['__imp__sprintf'], '__imp___vsnprintf': rd.KERNEL['__imp___vsnprintf']})
    out = rd.run(exe, a.image).splitlines()
    assert out[0] == 'new 40001000 40001070', out        # 16-aligned bump allocations in the arena, 100 and 33 bytes rounded to 16
    assert out[1] == 'sprintf 14 -5|abc|2.50|ff', out
    assert out[2] == 'vsnprintf 9 -5 abc ff', out
    print('selftest ok:', *out, sep='\n  ')


if __name__ == '__main__':
    main()
