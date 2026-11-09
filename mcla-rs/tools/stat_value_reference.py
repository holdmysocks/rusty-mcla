"""The statistics record's value arithmetic run offline as generated: the add F-45364 (record += value: f32 add for
type 0, a 32-bit word add otherwise) and the compare F-80717 (value < record, by the record's type: 0 f32, 1 signed,
else unsigned). A record is 16 bytes: the code at +0, the sub id at +4, the type in byte +8's top three bits, the
value's 32 bits at +12. Writes <out-dir>/stat_value_ref.txt: "type a b -> sum less" with a (the record), b (the
value) and the sum as 32-bit words in decimal and less = F-80717(record a, value b)."""
import argparse
import struct
import subprocess
from pathlib import Path
import recomp_closure
from recomp_closure import index, build, run

_shim = recomp_closure.shim
recomp_closure.shim = lambda *a: (_shim(*a).replace('uint16_t u16; uint8_t u8;', 'uint16_t u16; int16_t s16; uint8_t u8; int8_t s8;', 1)
                                  .replace('Condition cr0,cr6;', 'Condition cr0,cr1,cr5,cr6,cr7;', 1))

F = lambda x: struct.unpack('>I', struct.pack('>f', x))[0]
FLOATS = [F(x) for x in (0.0, -0.0, 1.0, 0.1, 0.2, 1e-7, 3.3333333, 54.0912, 5785.3, 16777216.0, 16777217.0, 1e30, -1e30, 3.4e38,
                         -2.5, 1e-40, 0.016666668, 1234567.9)] + [0x7FC00000, 0x7F800000]
INTS = [0, 1, 2, 0xFFFFFFFF, 0x7FFFFFFF, 0x80000000, 0x80000001, 100, 0xFFFFFF9C, 1000000, 0x40000000, 0xC0000000]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out-dir', type=Path, required=True)
    ap.add_argument('--tool', default='mcla-tool')
    a = ap.parse_args()
    addr = lambda fid: subprocess.run([a.tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[2][4:]
    add, less = addr('F-45364'), addr('F-80717')
    nl = chr(10)
    cs = [(t, x, y) for t in range(4) for vals in [FLOATS if t == 0 else INTS] for x in vals for y in vals]
    rows = (',' + nl).join('{%d,0x%Xu,0x%Xu}' % c for c in cs)
    body = ('struct K{int t;uint32_t a,b;};static const K C[]={' + nl + rows + nl + '};' + nl +
            'int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});' + nl +
            'for(auto&c:C){memset(objects,0,sizeof(objects));' + nl +
            # record at 0x3000, value at 0x3100; the type in the top three bits of +8, op bits 0
            '*pointer(0x3008)=(uint8_t)(c.t<<5);store(0x300C,c.a);*pointer(0x3108)=(uint8_t)(c.t<<5);store(0x310C,c.b);' + nl +
            'Context k{};k.r1.u32=0x7F00;k.r3.u32=0x3000;k.r4.u32=0x3100;sub_' + less + '(k,nullptr);uint32_t lt=k.r3.u32&0xFF;' + nl +
            'Context s{};s.r1.u32=0x7F00;s.r3.u32=0x3000;s.r4.u32=0x3100;sub_' + add + '(s,nullptr);' + nl +
            'printf("%u %u' + chr(92) + 'n",load(0x300C),lt);}' + nl + '}' + nl)
    exe = build(index(a.recomp), ['sub_' + add, 'sub_' + less], body, a.out_dir, 'stat_value', scratch_end=0x8000)
    out = run(exe, a.image).split(nl)[:len(cs)]
    assert len(out) == len(cs), (len(out), len(cs))
    a.out_dir.mkdir(parents=True, exist_ok=True)
    (a.out_dir / 'stat_value_ref.txt').write_text(''.join('%d %d %d -> %s' % (*c, o) + nl for c, o in zip(cs, out)))
    print(len(cs), 'cases')


if __name__ == '__main__':
    main()
