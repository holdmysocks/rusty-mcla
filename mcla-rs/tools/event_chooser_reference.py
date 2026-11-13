"""The event camera's chooser (mcPlayerCamera, F-24075) run offline on the original's code, one frame per case.

The chooser runs as compiled. Its callees that reach the director, the pad, the world or the game's globals are stubbed, and each case
sets what they answer:
  F-38782 (active(dir, cam): the event camera +84, the wipeout +72, the ability +68, the look back +64, +132), F-65810 (blending),
  F-54299 (isBike), F-58729 / F-94357 (the slipstream's / the nitrous' lens ramp complete), F-52044 / F-70872 / F-33017 (pressed / held /
  value of an action), the ball probe F-87651 and the segment probe F-20855 (their arguments are written to the log), F-59068 and F-52539
  (a replay / race-editor flag and the cine-script camera: 0 in every case, the port has neither).
The two calls that act are written to the log, in order: F-94591 (on / off: `on e b` / `off b`; on sets the event field cam +784 unless the
event camera is in the director, off sets it to -1, as set-event F-26269 does), F-70013 (`fin`, the director's blend finished) and the blend
camera's vt+40 (`vt40`). After the frame: cam +784, the enable bytes cam +2817 / +2828, the idle time (pad +112).
The structures it reads are built in the arena at their offsets (docs/reference/cameras/event_chooser_rule.md 'Objects and fields').
Usage: event_chooser_reference.py --out FILE [--n N] [--tool mcla-tool] [--recomp D:/MCLA-static] [--image original-image.bin] [--work DIR]
Writes one line per case: `<case words> -> <log>; e <cam+784> b <2817> <2828> i <idle>` (memcap: run under tools/memcap.py)."""
import argparse, random, struct, subprocess, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import recomp_dispatch as rd
import recomp_closure as rc
rc.EXTRA += '''#include <atomic>
alignas(16) static uint8_t VectorShiftTableL[]={0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0x00,
0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01,
0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02,
0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03,
0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05, 0x04,
0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06, 0x05,
0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07, 0x06,
0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08, 0x07,
0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09, 0x08,
0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A, 0x09,
0x19, 0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B, 0x0A,
0x1A, 0x19, 0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C, 0x0B,
0x1B, 0x1A, 0x19, 0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D, 0x0C,
0x1C, 0x1B, 0x1A, 0x19, 0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E, 0x0D,
0x1D, 0x1C, 0x1B, 0x1A, 0x19, 0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F, 0x0E,
0x1E, 0x1D, 0x1C, 0x1B, 0x1A, 0x19, 0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11, 0x10, 0x0F};
namespace rex { namespace ppc {
static inline simde__m128i simde_mm_perm_epi8_(simde__m128i a, simde__m128i b, simde__m128i c) {
 simde__m128i d = simde_mm_set1_epi8(0xF); simde__m128i e = simde_mm_sub_epi8(d, simde_mm_and_si128(c, d));
 return simde_mm_blendv_epi8(simde_mm_shuffle_epi8(a, e), simde_mm_shuffle_epi8(b, e), simde_mm_slli_epi32(c, 3)); } } }
'''

# the objects (arena addresses; the offsets are the original's)
MGR, P, P_VT, CAR, INP, CH, CH_VT = 0x40800000, 0x40801000, 0x40801800, 0x40802000, 0x40803000, 0x40804000, 0x40804F00
AERO, TRANS, BODY, W0, W1, DMG, GYRO = 0x40805000, 0x40805100, 0x40805200, 0x40805400, 0x40805800, 0x40806000, 0x40806100
NITRO, SST, AIR, HYD, PAD, INPMAP, DIR, BCAM, BC_VT, FX, CAM = (0x40806200, 0x40806300, 0x40806400, 0x40806500, 0x40807000, 0x40807200,
                                                              0x40808000, 0x40808100, 0x40808200, 0x40809000, 0x4080A000)
VIEWS, VC_VT, LB, AB, WO, C132, GS, ED = 0x4080C000, 0x4080C800, 0x4080D000, 0x4080D100, 0x4080D200, 0x4080D300, 0x4080E000, 0x4080E100
CASE = 0x40E00000
# the event records' BlendTime / ReturnTime (+800 + 112 e, +20 / +24) as the shipped files have them (event_chooser_rule.md 'Files');
# events 0 and 1 take the case's BlendTime
REC_BLEND = [0.7] * 13 + [1.5, 0.2, 0.0, 0.0, 1.5]
REC_RET = [1.5, 1.0] + [0.0] * 16
# (the globals the chooser reads: the game state (paused: +4 set and +10 clear) and the race editor (+156 its overhead view))
G_STATE, G_EDITOR = 0x828CCEA0, 0x8286E7FC
FAKE_READY, FAKE_AIRTIME, FAKE_LOOKBACK, FAKE_VT40 = 0x7F000010, 0x7F000011, 0x7F000012, 0x7F000013

# the case's fields: (name, address, bytes)
FIELDS = [
    ('act84', CASE + 0, 4), ('act72', CASE + 4, 4), ('act68', CASE + 8, 4), ('act64', CASE + 12, 4), ('act132', CASE + 16, 4),
    ('blending', CASE + 20, 4), ('bike', CASE + 24, 4), ('sst_full', CASE + 28, 4), ('nitro_full', CASE + 32, 4),
    ('pressed', CASE + 36, 4), ('pressed_hi', CASE + 40, 4), ('held', CASE + 44, 4), ('held_hi', CASE + 48, 4),
    ('v27', CASE + 52, 4), ('v28', CASE + 56, 4), ('v29', CASE + 60, 4), ('ball', CASE + 64, 4), ('seg', CASE + 68, 4),
    ('lookback', CASE + 76, 4),
    ('view', MGR + 80, 4), ('cinematic', MGR + 88, 1), ('photo', MGR + 101, 1),
    ('corrected', CAM + 156, 1), ('track_x', CAM + 240, 4), ('event', CAM + 784, 4), ('rec0_blend', CAM + 820, 4), ('rec1_blend', CAM + 932, 4),
    ('idle_delay', FX + 36, 4), ('burn_thr', FX + 44, 4),
    ('pad_index', PAD + 8, 2), ('idle', PAD + 112, 4), ('g4', GS + 4, 1), ('g10', GS + 10, 1), ('editor', ED + 156, 1),
    ('brake', INP + 16, 4), ('handbrake', INP + 20, 4), ('stick_y', INP + 32, 4), ('stick_x', INP + 36, 4),
    ('hb_sim', CH + 144, 4), ('speed', CH + 224, 4), ('charge', CH + 580, 4), ('air_time', CH + 584, 4), ('flags', CH + 592, 4),
    ('nitro_p', CH + 664, 4), ('sst_p', CH + 668, 4), ('air_p', CH + 672, 4), ('hyd_p', CH + 676, 4),
    ('tuck', AERO + 40, 1), ('gear', TRANS + 20, 4), ('yaw', BODY + 292, 4), ('front_down', W0 + 529, 1), ('rear_down', W1 + 529, 1),
    ('wipe_timer', DMG + 44, 4), ('stunt', GYRO + 16, 2), ('g18', GYRO + 18, 2),
    ('nitro_on', NITRO + 32, 1), ('sst_on', SST + 40, 1), ('air_fitted', AIR + 12, 1), ('air_on', AIR + 13, 1), ('hyd_on', HYD + 16, 4),
] + [('m%d' % i, CAR + 144 + 4 * i, 4) for i in range(16)] + [('vel%d' % i, CAR + 400 + 4 * i, 4) for i in range(3)]

MAIN = r"""
static const uint32_t REC_BLEND[]={RECB}, REC_RET[]={RECR};
static const uint32_t FA[]={FIELD_ADDRS}; static const uint8_t FS[]={FIELD_SIZES};
int main(int argc,char**argv){std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); uint32_t w;
 for(;;){
  rt_reset();
  std::memset(pointer(0x40800000u),0,0x10000); std::memset(pointer(CASE_u),0,0x100);
  // (the structure)
  store(MGR_u+4,P_u); store(MGR_u+8,P_u); store(MGR_u+16,DIR_u); store(MGR_u+24,FX_u); store(MGR_u+84,CAM_u);
  store(MGR_u+64,LB_u); store(MGR_u+68,AB_u); store(MGR_u+72,WO_u); store(MGR_u+132,C132_u);
  for(int v=0;v<5;v++){ store(MGR_u+44+4*v,VIEWS_u+0x100*v); store(VIEWS_u+0x100*v,VC_VT_u); }
  store(VC_VT_u+56,FAKE_LOOKBACK_u);
  store(P_u,P_VT_u); store(P_VT_u+48,FAKE_READY_u); store(P_u+48,CAR_u); store(P_u+896,PAD_u); store(PAD_u+116,INPMAP_u);
  store(CAR_u+4,INP_u); store(CAR_u+8,CH_u); store(CAR_u+20,DMG_u); store(CAR_u+28,0x40809F00u); store(CAR_u+48,GYRO_u);
  store(CH_u,CH_VT_u); store(CH_VT_u+52,FAKE_AIRTIME_u); store(CH_u+8,AERO_u); store(CH_u+20,TRANS_u); store(CH_u+128,BODY_u);
  store(CH_u+412,W0_u); store(CH_u+416,W1_u);
  store(DIR_u+48,BCAM_u); store(BCAM_u,BC_VT_u); store(BC_VT_u+40,FAKE_VT40_u);
  store(G_STATE_u,GS_u); store(G_EDITOR_u,ED_u);
  for(int e=0;e<18;e++){ *pointer(CAM_u+2816+e)=1; store(CAM_u+800+112*e+20,REC_BLEND[e]); store(CAM_u+800+112*e+24,REC_RET[e]); }
  // (the case)
  for(size_t i=0;i<sizeof FA/4;i++){ if(!(in>>std::hex>>w)) return 0;
   if(FS[i]==4) store(FA[i],w); else if(FS[i]==2){*pointer(FA[i])=uint8_t(w>>8);*pointer(FA[i]+1)=uint8_t(w);} else *pointer(FA[i])=uint8_t(w); }
  store(DIR_u+56,load(CASE_u+20)?BCAM_u:0u);
  LOG.clear();
  {Context c=fresh(); c.r3.u64=MGR_u; CHOOSER(c,nullptr);}
  int32_t e=int32_t(load(CAM_u+784));
  printf("%s; e %d b %d %d i %08x\n",LOG.c_str(),e,*pointer(CAM_u+2817),*pointer(CAM_u+2828),load(PAD_u+112));
 }
}
"""


def addr(tool, fid):
    return int(subprocess.run([tool, 'registry', 'addr', fid], capture_output=True, text=True, check=True).stdout.split()[1], 16)


def f2w(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def cases(n, seed):
    """Random frames around the tests' edges: an event running or none, each gate on and off."""
    rnd = random.Random(seed)
    edges = [0.0, 0.01, 0.1, 0.2, 0.5, 0.75, 1.0, 1.5, 10.0, 60.0, -0.5, -0.75, -1.0]

    def fl(lo=-20.0, hi=20.0):
        if rnd.random() < 0.6:
            x = rnd.choice(edges)
            return x if rnd.random() < 0.6 else x + rnd.choice((-1, 1)) * rnd.choice((1e-6, 1e-3, 0.05))
        return rnd.uniform(lo, hi)

    def b(p=0.5):
        return 1 if rnd.random() < p else 0
    out = []
    while len(out) < n:
        c = {}
        running = rnd.random() < 0.6
        ev = rnd.choice(list(range(14)) + [17]) if running else -1
        c['event'] = ev & 0xFFFFFFFF
        # (the event camera: in the director while its event runs - current or blending in; out of it otherwise, or a blend's source)
        # (running: the director's current camera or its blend's target; not running: out of the director, or the source of the blend
        # back to the drive view)
        if running:
            c['act84'], c['blending'] = 1, b(0.3)
        else:
            c['act84'], c['blending'] = rnd.choice(((0, 0), (0, 0), (0, 0), (0, 1), (0xFFFFFFFF, 1)))
        crash = rnd.random() < 0.12
        c['act72'] = rnd.choice((1, 0xFFFFFFFF)) if crash else 0
        c['act68'] = 1 if rnd.random() < 0.05 else 0
        c['act64'] = 1 if rnd.random() < 0.05 else 0
        c['act132'] = 0   # (the director-level camera +132: the port has none)
        c['bike'] = b(0.4)
        c['sst_full'], c['nitro_full'] = b(0.6), b(0.6)
        c['pressed'] = sum(1 << a for a in (12, 13, 22, 23) if rnd.random() < 0.15)
        c['pressed_hi'] = 0
        c['held'] = sum(1 << a for a in (12, 13, 14, 19, 20, 21, 26) if rnd.random() < 0.06)
        c['held_hi'] = (1 << (44 - 32)) if rnd.random() < 0.06 else 0
        for k in ('v27', 'v28', 'v29'):
            c[k] = f2w(rnd.choice((0.0, 0.0, 0.0, 0.2, 0.2 + 1e-6, 0.5, 1.0)))
        c['ball'], c['seg'] = b(0.3), b(0.4)
        c['lookback'] = b(0.2)
        c['view'] = rnd.choice((0, 1, 2, 3, 4, 4, 4))
        c['cinematic'] = rnd.choice((1, 1, 1, 1, 0, 2))
        c['photo'] = 1 if rnd.random() < 0.05 else 0
        c['corrected'] = b(0.2)
        c['track_x'] = f2w(rnd.choice((0.0, 1.0, -1.0, 1.25, -0.0, 0.25)))
        c['rec0_blend'] = f2w(rnd.choice((0.7, 0.7, 0.0)))
        c['rec1_blend'] = f2w(rnd.choice((0.7, 0.7, 0.0)))
        c['idle_delay'] = f2w(rnd.choice((60.0, 60.0, 0.0)))
        c['burn_thr'] = f2w(1.5)
        c['pad_index'] = 0xFFFF if rnd.random() < 0.08 else 0
        c['idle'] = f2w(rnd.choice((0.0, 0.0, 10.0, 59.99, 60.0, 61.0)))
        c['g4'] = b(0.1)
        c['g10'] = b(0.5)
        c['editor'] = 1 if rnd.random() < 0.03 else 0
        c['brake'] = f2w(rnd.choice((0.0, 0.0, 0.5, 1.0)))
        c['handbrake'] = f2w(rnd.choice((0.0, 0.0, 1.0)))
        c['stick_y'], c['stick_x'] = f2w(fl(-1, 1)), f2w(fl(-1, 1))
        c['hb_sim'] = f2w(rnd.choice((0.0, 0.0, 1.0, 0.005, 0.01)))
        c['speed'] = f2w(rnd.choice((0.0, 5.0, 10.0, 10.0001, 30.0, 30.0)))
        c['charge'] = f2w(rnd.choice((0.0, 0.0, 0.4, 0.5, 0.6, 1.0, 1.0)))
        c['air_time'] = f2w(rnd.choice((0.0, 0.0, 0.05, 0.1, 0.11, 0.5, 0.6, -0.0167)))
        c['flags'] = rnd.choice((0, 0, 0, 4, 0x10, 0x20, 0x14, 0x24))
        c['nitro_p'] = NITRO   # (every car has both: the chooser reads them unchecked)
        c['sst_p'] = SST
        c['air_p'] = AIR if rnd.random() < 0.6 else 0
        c['hyd_p'] = HYD if rnd.random() < 0.6 else 0
        c['tuck'] = b(0.3)
        c['gear'] = rnd.choice((0, 1, 2, 3))
        c['yaw'] = f2w(rnd.choice((0.0, 0.5, 0.75, 1.0, 1.5, 2.0, -2.0, -0.7)))
        c['front_down'], c['rear_down'] = b(0.6), b(0.6)
        c['wipe_timer'] = f2w(rnd.choice((0.0, 0.0, 1.0, -0.1)))
        c['stunt'] = rnd.choice((0, 0, 0, 1, 99, 0xFFFF, 0xFF9D))
        c['g18'] = rnd.choice((0, 0, 0, 1, 0xFFFF))
        c['nitro_on'], c['sst_on'] = b(0.25), b(0.2)
        c['air_fitted'], c['air_on'] = b(0.7), b(0.3)
        c['hyd_on'] = b(0.4)
        # (a car's pose: a turned upright matrix; its position and velocity)
        yaw, pitch = rnd.uniform(-3.14, 3.14), rnd.uniform(-0.3, 0.3)
        import math
        cy, sy, cp, sp = math.cos(yaw), math.sin(yaw), math.cos(pitch), math.sin(pitch)
        rows = [(cy, 0.0, -sy), (sy * sp, cp, cy * sp), (sy * cp, -sp, cy * cp)]
        pos = (rnd.uniform(-3000, 3000), rnd.uniform(0, 100), rnd.uniform(-3000, 3000))
        m = [*rows[0], 0.0, *rows[1], 0.0, *rows[2], 0.0, *pos, 1.0]
        for i in range(16):
            c['m%d' % i] = f2w(m[i])
        for i in range(3):
            c['vel%d' % i] = f2w(rnd.uniform(-40, 40))
        words = [c[f[0]] for f in FIELDS]
        # (a word that reads like an address of the original is drawn again: tracked files may not hold one)
        if any(f'{w:08x}'.startswith('82') for w in words):
            continue
        out.append(words)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--out', type=Path, required=True)
    ap.add_argument('--n', type=int, default=6000)
    ap.add_argument('--seed', type=int, default=20261113)
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--recomp', type=Path, default=Path('D:/MCLA-static'))
    ap.add_argument('--image', type=Path, default=Path('D:/MCLA/work/2026-10-10/original-image.bin'))
    ap.add_argument('--work', type=Path, default=Path('event_chooser_work'))
    a = ap.parse_args()
    ids = ('F-24075', 'F-38782', 'F-65810', 'F-54299', 'F-58729', 'F-94357', 'F-52044', 'F-70872', 'F-33017', 'F-87651', 'F-20855',
           'F-59068', 'F-52539', 'F-94591', 'F-70013')
    fn = {k: 'sub_%08X' % addr(a.tool, k) for k in ids}
    # active(dir, cam): the case's answer for each of the five cameras
    act = ('uint32_t c=ctx.r4.u32; uint32_t r=0;'
           f'if(c==load({MGR + 84:#x}u))r=load({CASE:#x}u); else if(c==load({MGR + 72:#x}u))r=load({CASE + 4:#x}u);'
           f'else if(c==load({MGR + 68:#x}u))r=load({CASE + 8:#x}u); else if(c==load({MGR + 64:#x}u))r=load({CASE + 12:#x}u);'
           f'else if(c==load({MGR + 132:#x}u))r=load({CASE + 16:#x}u); ctx.r3.u64=(uint64_t)(int64_t)(int32_t)r;')
    bit = ('{uint32_t a=ctx.r4.u32; uint32_t m=load(%#xu+(a>=32?4:0)); ctx.r3.u64=(m>>(a&31))&1;}')
    stubs = {
        fn['F-38782']: act,
        fn['F-65810']: f'ctx.r3.u64=load({CASE + 20:#x}u);',
        fn['F-54299']: f'ctx.r3.u64=load({CASE + 24:#x}u);',
        fn['F-58729']: f'ctx.r3.u64=load({CASE + 28:#x}u);',
        fn['F-94357']: f'ctx.r3.u64=load({CASE + 32:#x}u);',
        fn['F-52044']: bit % (CASE + 36),
        fn['F-70872']: bit % (CASE + 44),
        fn['F-33017']: (f'{{uint32_t a=ctx.r4.u32; float f=a==27?flt({CASE + 52:#x}u):a==28?flt({CASE + 56:#x}u):a==29?flt({CASE + 60:#x}u):0.0f;'
                        'ctx.f1.f64=f;}'),
        fn['F-87651']: ('{uint32_t q=ctx.r4.u32; char b[128]; snprintf(b,sizeof b,"ball %08x %08x %08x %08x|",'
                        f'load(q),load(q+4),load(q+8),(uint32_t)[&]{{float r=(float)ctx.f1.f64;uint32_t w;memcpy(&w,&r,4);return w;}}()); LOG+=b;'
                        f'ctx.r3.u64=load({CASE + 64:#x}u);}}'),
        fn['F-20855']: ('{uint32_t q=ctx.r4.u32; char b[128]; snprintf(b,sizeof b,"seg %08x %08x %08x %08x %08x %08x|",'
                        'load(q),load(q+4),load(q+8),load(q+16),load(q+20),load(q+24)); LOG+=b;'
                        f'ctx.r3.u64=load({CASE + 68:#x}u);}}'),
        fn['F-59068']: 'ctx.r3.u64=0;',
        fn['F-52539']: 'ctx.r3.u64=0;',
        # on: refused while the event camera is in the director; set-event, then (unless chassis +592 bit 0x20) the push over the
        # record's BlendTime: the event camera the blend's target. off: the drive view pushed over b (b < 0: the record's ReturnTime,
        # else its BlendTime) when the event camera is in the director - a blend above 0.001 s turns the event camera into its source, a
        # cut takes it out; then set-event(-1). (the director's own code is not run: event_chooser_rule.md part 3)
        fn['F-94591']: ('{uint32_t on=ctx.r4.u32; int32_t e=ctx.r5.s32; float bl=(float)ctx.f1.f64; uint32_t bw; memcpy(&bw,&bl,4); char b[64];'
                        f'int32_t act=(int32_t)load({CASE:#x}u);'
                        'if(on){ if(act==0){ snprintf(b,sizeof b,"on %d %08x|",e,bw); LOG+=b;'
                        f'  store({CAM + 784:#x}u,(uint32_t)e); if(!(load({CH + 592:#x}u)&0x20)){{ store({CASE:#x}u,1); store({CASE + 20:#x}u,1); store({DIR + 56:#x}u,{BCAM:#x}u); }} }} }}'
                        'else { snprintf(b,sizeof b,"off %08x|",bw); LOG+=b;'
                        f'  int32_t c=(int32_t)load({CAM + 784:#x}u); float t=bl;'
                        f'  if(t<0&&c>=0&&c<18){{ t=flt({CAM + 800 + 24:#x}u+112*c); if(!(t>0)) t=flt({CAM + 800 + 20:#x}u+112*c); }}'
                        f'  if(act!=0){{ if(t>0.001f){{ store({CASE:#x}u,0xFFFFFFFFu); store({CASE + 20:#x}u,1); store({DIR + 56:#x}u,{BCAM:#x}u); }}'
                        f'   else {{ store({CASE:#x}u,0); store({CASE + 20:#x}u,0); store({DIR + 56:#x}u,0); }} }}'
                        f'  store({CAM + 784:#x}u,0xFFFFFFFFu); }} }}'),
        # the finish: the blend's target current (the event camera, when it was the target), no blend
        fn['F-70013']: (f'LOG+="fin|"; if((int32_t)load({CASE:#x}u)<0) store({CASE:#x}u,0); store({CASE + 20:#x}u,0); store({DIR + 56:#x}u,0);'),
    }
    fakes = {FAKE_READY: 'ctx.r3.u64=1;', FAKE_AIRTIME: f'ctx.f1.f64=flt({CH + 584:#x}u);',
             FAKE_LOOKBACK: f'ctx.r3.u64=load({CASE + 76:#x}u);', FAKE_VT40: 'LOG+="vt40|";'}
    names = {'MGR': MGR, 'P': P, 'P_VT': P_VT, 'CAR': CAR, 'INP': INP, 'CH': CH, 'CH_VT': CH_VT, 'AERO': AERO, 'TRANS': TRANS, 'BODY': BODY,
             'W0': W0, 'W1': W1, 'DMG': DMG, 'GYRO': GYRO, 'PAD': PAD, 'INPMAP': INPMAP, 'DIR': DIR, 'BCAM': BCAM, 'BC_VT': BC_VT, 'FX': FX,
             'CAM': CAM, 'VIEWS': VIEWS, 'VC_VT': VC_VT, 'LB': LB, 'AB': AB, 'WO': WO, 'C132': C132, 'GS': GS, 'ED': ED, 'CASE': CASE,
             'G_STATE': G_STATE, 'G_EDITOR': G_EDITOR, 'FAKE_READY': FAKE_READY, 'FAKE_AIRTIME': FAKE_AIRTIME,
             'FAKE_LOOKBACK': FAKE_LOOKBACK, 'FAKE_VT40': FAKE_VT40}
    src = MAIN.replace('CHOOSER', fn['F-24075']).replace('RECB', ','.join('0x%08Xu' % f2w(x) for x in REC_BLEND)).replace('RECR', ','.join('0x%08Xu' % f2w(x) for x in REC_RET))
    src = src.replace('FIELD_ADDRS', ','.join('0x%08Xu' % f[1] for f in FIELDS)).replace('FIELD_SIZES', ','.join(str(f[2]) for f in FIELDS))
    for k in sorted(names, key=len, reverse=True):
        src = src.replace(k + '_u', '0x%08Xu' % names[k])
    rc.EXTRA += ('static std::string LOG;\n'
                 'static float flt(uint32_t a){uint32_t w=load(a);float f;memcpy(&f,&w,4);return f;}\n')
    idx = rd.index(a.recomp)
    img = rd.load_image(a.image)
    work = a.work.resolve()
    exe = rd.build(idx, [fn['F-24075']], src, work, 'event_chooser', stubs=stubs, image=img, fakes=fakes)
    cs = cases(a.n, a.seed)
    cf = work / 'cases.txt'
    cf.write_text(''.join(' '.join(f'{w:x}' for w in c) + '\n' for c in cs))
    r = subprocess.run([str(exe), str(a.image), str(cf)], capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-1500:] or f'exit {r.returncode}')
    lines = r.stdout.splitlines()
    assert len(lines) == len(cs), (len(lines), len(cs))
    a.out.write_text('# ' + ' '.join(f[0] for f in FIELDS) + '\n' +
                     ''.join(' '.join(f'{w:x}' for w in c) + ' -> ' + l + '\n' for c, l in zip(cs, lines)))
    print(len(lines), 'cases')


if __name__ == '__main__':
    main()
