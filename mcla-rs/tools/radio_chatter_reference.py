"""Run the original's police tactic radio chatter (F-26639, the tactic's chatter update) offline (static recompilation, native, flat
guest memory) and record every frame for the port's test (crates/game/src/testdata/radio_chatter_ref.txt); with --port, also
compare every frame with the port's `chatter_step` (extracted verbatim from police.rs, built with rustc).

The tactic object is built in guest memory as the game keeps it: the cops list (+72 count, +76 pointers), the target (+32), the
law record (+36, with the road +60, its timer +64 and the flag word +0), the far byte (+96), the nitrous tries (+100) and the
countdown (+112). The caller's countdown (the tactic update, F-63286: +112 -= dt, held at 0) is done by the harness before each
call, as that update does, and F-26639 is called only when the kind's vt+80 says so (the case's `on`). F-26639's own code runs
unchanged, with its callees as the game has them:
  unchanged: the scenario constructors (F-39945, F-75465, F-40445, F-53533, F-79977, F-17319, F-34270, F-89724, F-97436, F-99583,
             F-48598, F-47016, F-19465, F-31638), the scenario say F-14569 through its vtable, the wrap F-58674, the manager's
             count F-76898, the race test F-27863, the speech flag F-94735 (a byte in the speech manager), the random draws (the
             game's generator words in guest memory), the tune floats (fMinPerRdoTime, fMaxPerRdoTime, fRdoVicinityDist).
  stubbed (the answers are the case's inputs):
             F-86691  the road of the target's lane (the case's road, or none)
             F-28783  the road's name starts FW or XT (a bit mask per case, by road)
             F-13798  the contact lookup of a cop's entry in the target's list (per cop: no +48 / no entry / a value)
             F-29443  some cop within d of the target (the case's two bools: at the tune distance and at 10 m)
             F-23825  a cop's call sign (the cop object's own pointer, so the sender / receiver can be read back)
             F-70230  the speech manager's say (records the scenario name, sender and receiver it was given; returns refused)
             the target's vt+40 (is a player): fake vtable slots answering the case's bit
  not modelled: the scenarios' own say gates (Frequency, Probability, priority: inside F-70230), the target's real object.

File format (one case = a CASE line and its F lines; the tool writes the inputs, then `>` and the original's outputs):
  CASE n fw min max vic t0 road0 fwt0 nitro0 far0 rng0
       n cops; fw = bit mask of the roads (0 .. 7) that are freeways; min / max / vic = fMinPerRdoTime / fMaxPerRdoTime /
       fRdoVicinityDist (f32 bits); start: the countdown, the law record's road (- none) and its timer, the nitrous tries, the far
       byte, the generator (hi << 32 | lo, 16 digits)
  F dt on busy target player rec road hard chain nitro speed race mgr80 wv w10 h0 h1 h2 h3 > name s r t fwt road nitro far rng
       dt f32 bits; on / busy / target / player / rec / hard / race / wv / w10 0 or 1; road - or 0 .. 7 (the road of the target's
       lane); chain 0 .. 3 how much of the target's +48 +8 +664 chain exists (speed needs 2, the nitrous byte 3); nitro the byte;
       speed f32 bits; mgr80 the manager's count; h_k per cop: - no +48, n no entry, else the entry's value (f32 bits)
       out: the scenario said (- none), the sender's cop index, the receiver's (D = DISPATCH), the countdown and the law record's
       timer (f32 bits), the record's road, the nitrous tries, the far byte, the generator
"""
import argparse
import random
import struct
import subprocess
import sys
from pathlib import Path
import recomp_dispatch as rdisp

# the globals F-26639 reads (data words, not functions): the speech manager and the police manager pointers, the current race
# word, the tune block, the generator
SPEECH_G, POLICE_G, RACE_G, TUNE_G, RNG_G = 0x8290A0B0, 0x8286D8E0, 0x8286EBF8, 0x8282CAC8, 0x82860890
# the vtable slot 1 (+4) of each scenario class is its say; the ctors store the vtable
CTORS = ['F-39945', 'F-75465', 'F-40445', 'F-53533', 'F-79977', 'F-17319', 'F-34270', 'F-89724', 'F-97436', 'F-99583', 'F-48598',
         'F-47016', 'F-19465', 'F-31638']
STUBS = ['F-86691', 'F-28783', 'F-13798', 'F-29443', 'F-23825', 'F-70230']
EXTRA = ['F-26639', 'F-14569', 'F-55001', 'F-58674', 'F-76898', 'F-27863']

MAIN = r'''
static void sf(uint32_t a,float f){uint32_t u;memcpy(&u,&f,4);store(a,u);}
static void sb(uint32_t a,uint8_t b){uint32_t w=load(a&~3u);int sh=(3-(a&3))*8;w=(w&~(0xffu<<sh))|((uint32_t)b<<sh);store(a&~3u,w);}
static uint8_t lb(uint32_t a){return (uint8_t)(load(a&~3u)>>((3-(a&3))*8));}
static float lf(uint32_t u){float f;memcpy(&f,&u,4);return f;}
static uint32_t hx(const std::string&s){return (uint32_t)strtoul(s.c_str(),0,16);}
int main(int argc,char**argv){setvbuf(stdout,nullptr,_IONBF,0);std::ifstream bin(argv[1],std::ios::binary);image.assign(std::istreambuf_iterator<char>(bin),{});
 std::ifstream in(argv[2]); std::string t; const uint32_t PL=strtoul(argv[3],0,16), AI=strtoul(argv[4],0,16); (void)PL;(void)AI;
 const uint32_t TAC=0x40800000u, LIST=0x40800400u, COP=0x40810000u, CO48=0x40820000u, TGT=0x40830000u, P1=0x40831000u, P2=0x40832000u,
  P3=0x40833000u, REC=0x40840000u, VTP=0x40850000u, VTA=0x40850100u, MGR=0x40860000u, PM=0x40870000u, ROAD=0x40D00000u;
 int n=0; int k=0;
 while(in>>t){
  if(t=="CASE"){ std::string s_n,s_fw,s_min,s_max,s_vic,s_t0,s_road,s_fwt,s_nit,s_far,s_rng; in>>s_n>>s_fw>>s_min>>s_max>>s_vic>>s_t0>>s_road>>s_fwt>>s_nit>>s_far>>s_rng;
   rt_reset(); k=0; n=atoi(s_n.c_str()); uint32_t fw=hx(s_fw);
   store(TAC+72,n); store(TAC+76,LIST);
   for(int c=0;c<n;c++) store(LIST+4*c,COP+0x100*c);
   for(int c=0;c<8;c++){ store(CO48+0x100*c+28,0x1000u+c); store(COP+0x100*c+0,0u); }
   for(int r=0;r<8;r++) store(0x40F00100u+4*r,(fw>>r)&1);
   store(VTP+40,0x7F000010u); store(VTA+40,0x7F000011u);
   store(SPEECH_G,MGR); store(POLICE_G,PM);
   sf(TUNE_G+376,lf(hx(s_min))); sf(TUNE_G+380,lf(hx(s_max))); sf(TUNE_G+384,lf(hx(s_vic)));
   uint64_t rng=strtoull(s_rng.c_str(),0,16); store(RNG_G,(uint32_t)rng); store(RNG_G+4,(uint32_t)(rng>>32));
   sf(TAC+112,lf(hx(s_t0))); sb(TAC+96,s_far=="1"); store(TAC+100,atoi(s_nit.c_str()));
   store(REC+60,s_road=="-"?0u:ROAD+0x100*atoi(s_road.c_str())); sf(REC+64,lf(hx(s_fwt)));
  } else if(t=="F"){
   std::string v[19]; for(int q=0;q<19;q++) in>>v[q];
   float dt=lf(hx(v[0])); int on=v[1]=="1", busy=v[2]=="1", target=v[3]=="1", player=v[4]=="1", rec=v[5]=="1"; std::string road=v[6];
   int hard=v[7]=="1", chain=atoi(v[8].c_str()); int nitro=atoi(v[9].c_str()); uint32_t speed=hx(v[10]); int race=v[11]=="1", mgr80=atoi(v[12].c_str());
   int wv=v[13]=="1", w10=v[14]=="1";
   store(TAC+32,target?TGT:0u); store(TAC+36,rec?REC:0u); store(TGT,player?VTP:VTA);
   store(TGT+640,(road!="-"||(k&1))?0x40834000u:0u); store(0x40F00000u,road=="-"?0u:ROAD+0x100*atoi(road.c_str()));
   store(TGT+48,chain>=1?P1:0u); store(P1+8,chain>=2?P2:0u); store(P2+224,speed); store(P2+664,chain>=3?P3:0u); sb(P3+32,(uint8_t)nitro);
   store(REC,(hard?0x20u:0u)|(((uint32_t)k*2654435761u)&~0x20u));
   sb(MGR+269,busy); store(PM+80,(uint32_t)mgr80); store(RACE_G,race?0x40880000u:0u);
   store(0x40F00200u,wv); store(0x40F00204u,w10);
   for(int c=0;c<n;c++){ const std::string&h=v[15+c];
    store(COP+0x100*c+48,h=="-"?0u:CO48+0x100*c);
    store(0x40F00300u+12*c,(h=="-"||h=="n")?0u:0x1000u+c); store(0x40F00300u+12*c+8,(h=="-"||h=="n")?0u:hx(h)); }
   for(int c=n;c<8;c++) store(0x40F00300u+12*c,0u);
   store(0x40E00000u,0u);
   float cd=lf(load(TAC+112)); if(cd>0){ cd=cd-dt; sf(TAC+112,cd); if(cd<0) sf(TAC+112,0.0f); }
   if(on){ Context c=fresh(); c.r3.u32=TAC; c.f1.f64=(double)dt; F26639(c,nullptr); }
   uint32_t cnt=load(0x40E00000u); std::string name="-",ss="-",rr="-";
   if(cnt>1){ printf("MULTI %u\n",cnt); }
   if(cnt>=1){ name=cfmt(load(0x40E00004u)); uint32_t s=load(0x40E00008u), r=load(0x40E0000Cu);
    ss=std::to_string((int)((s-COP)>>8)); rr=(r>=COP&&r<COP+0x800)?std::to_string((int)((r-COP)>>8)):std::string("D");
    if(s<COP||s>=COP+0x800) ss="X"; }
   uint32_t rd=load(REC+60);
   printf("%s %s %s %x %x %s %u %u %016llx\n",name.c_str(),ss.c_str(),rr.c_str(),load(TAC+112),load(REC+64),rd?std::to_string((int)((rd-ROAD)>>8)).c_str():"-",
          load(TAC+100),(unsigned)lb(TAC+96),(unsigned long long)load(RNG_G+4)<<32|load(RNG_G));
   k++;
  }
 }
}
'''

RS_REPLAY = r'''
    /// Replays the tool's file through `chatter_step`: per F line, the original's outputs and the port's.
    fn rdo_replay(text: &str) -> Vec<(usize, String, String)> {
        let h = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let f = |s: &str| f32::from_bits(h(s));
        let mut out = Vec::new();
        let (mut n, mut fw, mut min_t, mut max_t) = (0usize, 0u32, 0f32, 0f32);
        let mut st = RdoState { t: 0.0, road: None, fw_t: 0.0, nitro: 0, far: false, rng: (0, 0) };
        for (ln, l) in text.lines().enumerate() {
            let v: Vec<&str> = l.split_whitespace().collect();
            match v.first().copied() {
                Some("CASE") => {
                    n = v[1].parse().unwrap();
                    fw = h(v[2]);
                    min_t = f(v[3]);
                    max_t = f(v[4]);
                    let r = u64::from_str_radix(v[11], 16).unwrap();
                    st = RdoState { t: f(v[6]), road: v[7].parse().ok(), fw_t: f(v[8]), nitro: v[9].parse().unwrap(), far: v[10] == "1", rng: (r as u32, (r >> 32) as u32) };
                }
                Some("F") => {
                    let hit: Vec<Option<f32>> = v[16..20].iter().take(n).map(|s| if *s == "-" || *s == "n" { None } else { Some(f(s)) }).collect();
                    let chain: u32 = v[9].parse().unwrap();
                    let is_fw = |r: usize| fw >> r & 1 != 0;
                    let i = RdoIn {
                        dt: f(v[1]),
                        on: v[2] == "1",
                        busy: v[3] == "1",
                        n,
                        target: v[4] == "1",
                        is_player: v[5] == "1",
                        rec: v[6] == "1",
                        road: v[7].parse().ok(),
                        is_fw: &is_fw,
                        hard: v[8] == "1",
                        hit: &hit,
                        nitro_burning: chain == 3 && v[10] != "0",
                        speed: if chain >= 2 && v[4] == "1" { f(v[11]) } else { 0.0 },
                        in_race: v[12] == "1",
                        mgr80: v[13].parse().unwrap(),
                        within_vic: v[14] == "1",
                        within_10: v[15] == "1",
                        min_t,
                        max_t,
                    };
                    let call = chatter_step(&mut st, &i);
                    let got = format!(
                        "{} {} {} {:x} {:x} {} {} {} {:016x}",
                        call.map_or("-", |c| c.0),
                        call.map_or("-".to_string(), |c| c.1.to_string()),
                        call.map_or("-".to_string(), |c| c.2.map_or("D".to_string(), |r| r.to_string())),
                        st.t.to_bits(),
                        st.fw_t.to_bits(),
                        st.road.map_or("-".to_string(), |r| r.to_string()),
                        st.nitro,
                        st.far as u8,
                        (st.rng.1 as u64) << 32 | st.rng.0 as u64
                    );
                    out.push((ln + 1, v[21..].join(" "), got));
                }
                _ => {}
            }
        }
        out
    }
'''

PORT_MAIN = r'''
fn main() {
    let t = std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap();
    let r = rdo_replay(&t);
    let bad: Vec<_> = r.iter().filter(|x| x.1 != x.2).collect();
    println!("frames {} differ {}", r.len(), bad.len());
    for b in bad.iter().take(12) { println!("line {}\n  original {}\n  port     {}", b.0, b.1, b.2); }
}
'''


def f32bits(x):
    return struct.unpack('>I', struct.pack('>f', x))[0]


def bits(x):
    return '%x' % f32bits(x)


def grab(s, pat):
    i = s.index(pat)
    j = s.index('{', i)
    d = 0
    for k in range(j, len(s)):
        if s[k] == '{':
            d += 1
        elif s[k] == '}':
            d -= 1
            if d == 0:
                return s[i:k + 1]


def gen_cases(count, frames, seed):
    """random multi-frame sequences: every input varies; timers at and around 0; busy frames; freeway changes inside and after 1 s"""
    rng = random.Random(seed)
    cases = []
    dts = [1 / 60, 1 / 60, 1 / 60, 1 / 30, 0.05, 0.0083, 0.25, 0.5, 1.0, 2.0, 1e-4, 16.0]
    hitvals = [-2.0, -1.0, -1.0000001, 0.0, 0.5, 1.0, 22.35, 30.0]
    for ci in range(count):
        n = rng.choice([1, 1, 2, 2, 3, 3, 4, 4])
        fw = rng.randrange(256)
        mn = rng.choice([15.0, 15.0, 0.5, 2.0, 15.5])
        mx = mn + rng.choice([15.0, 15.0, 0.0, 1.0, 7.25])
        vic = rng.choice([100.0, 100.0, 50.0, 12.5, 100.0001])
        dt_set = rng.sample(dts, rng.randrange(1, 5))
        t0 = rng.choice([0.0, 0.0, 1 / 60, 0.0083, 0.5, 1.0, rng.uniform(0, 3), rng.uniform(0, 30), 15.0])
        road0 = rng.choice([None, None] + list(range(8)))
        fwt0 = rng.choice([0.0, 0.0, 1.0, 1.0, 1 / 60, 0.001, rng.uniform(0, 1.5), -0.5])
        nitro0 = rng.choice([0, 0, 0, 1, 2, 3])
        far0 = rng.randrange(2)
        rng0 = rng.choice([0, rng.getrandbits(64), rng.getrandbits(64), rng.getrandbits(64), 0xFFFFFFFF])
        lines = ['CASE %d %x %s %s %s %s %s %s %d %d %016x' % (n, fw, bits(mn), bits(mx), bits(vic), bits(t0), '-' if road0 is None else road0,
                                                                  bits(fwt0), nitro0, far0, rng0)]
        road = road0
        nit_on = rng.randrange(2)
        race = rng.randrange(2)
        mgr80 = rng.choice([0, 1, 1, 2, 3, n])
        wv, w10 = rng.randrange(2), rng.randrange(2)
        speed = 0.0
        for k in range(frames):
            dt = rng.choice(dt_set)
            on = int(rng.random() < 0.92)
            busy = int(rng.random() < 0.12)
            target = int(rng.random() < 0.96)
            player = int(rng.random() < 0.88)
            rec = int(rng.random() < 0.88)
            if rng.random() < 0.25:
                road = rng.choice([None] + list(range(8)))
            hard = int(rng.random() < 0.12)
            chain = 3 if rng.random() < 0.85 else rng.randrange(3)
            if rng.random() < 0.15:
                nit_on ^= 1
            nitro = rng.randrange(1, 256) if nit_on else 0
            if rng.random() < 0.3:
                speed = rng.choice([0.0, 44.7, 20.0, 60.0, rng.uniform(0, 80)])
            sp_bits = f32bits(speed)
            if speed == 44.7 and rng.random() < 0.5:
                sp_bits += rng.choice([-1, 1])
            if rng.random() < 0.05:
                race ^= 1
            if rng.random() < 0.05:
                mgr80 = rng.choice([0, 1, 2, 3, n])
            if rng.random() < 0.3:
                wv = rng.randrange(2)
            if rng.random() < 0.3:
                w10 = rng.randrange(2)
            if rng.random() < 0.1:
                wv ^= 1
            hs = []
            for c in range(4):
                r = rng.random()
                if c >= n:
                    hs.append('-')
                elif r < 0.2:
                    hs.append('-')
                elif r < 0.4:
                    hs.append('n')
                elif r < 0.75:
                    hs.append(bits(rng.choice(hitvals)))
                else:
                    hs.append(bits(rng.uniform(0, 60)))
            lines.append('F %s %d %d %d %d %d %s %d %d %d %x %d %d %d %d %s' % (
                bits(dt), on, busy, target, player, rec, '-' if road is None else road, hard, chain, nitro, sp_bits, race, mgr80, wv, w10, ' '.join(hs)))
        cases.append(lines)
    return cases


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--recomp', type=Path, required=True)
    ap.add_argument('--image', type=Path, required=True)
    ap.add_argument('--out', type=Path, required=True, help='build dir')
    ap.add_argument('--ref', type=Path, required=True, help='the small file for the port test (a covering subset of the cases)')
    ap.add_argument('--tool', default='mcla-tool')
    ap.add_argument('--port', type=Path, help='police.rs: compare every frame of every case with the port')
    ap.add_argument('--cases', type=int, default=4000)
    ap.add_argument('--frames', type=int, default=60)
    ap.add_argument('--seed', type=int, default=26639)
    ap.add_argument('--max-bytes', type=int, default=300000, help='size of the committed file')
    a = ap.parse_args()
    ids = STUBS + EXTRA + CTORS
    r = subprocess.run([a.tool, 'registry', 'addr', *sorted(set(ids))], capture_output=True, text=True, check=True).stdout
    addr = {l.split()[0]: int(l.split()[1], 16) for l in r.splitlines() if l.startswith('F-')}
    name = lambda f: 'sub_%08X' % addr[f]
    img = rdisp.load_image(a.image)
    idx = rdisp.index(a.recomp)
    ai, pl = rdisp.vtables_of(img, 'mcAiRacer')[0][0], rdisp.vtables_of(img, 'mcPlayer')[0][0]
    vts = []
    for c in CTORS:
        vp = rdisp.stored_pointers(idx, name(c), 3)
        vts.append((vp[0], [1]))
    stubs = {
        name('F-86691'): 'ctx.r3.u64=load(0x40F00000u);',
        name('F-28783'): 'ctx.r3.u64=load(0x40F00100u+(((ctx.r4.u32-0x40D00000u)>>8)&7)*4)&1;',
        name('F-13798'): ('uint32_t key=ctx.r4.u32; ctx.r3.u64=0; for(int q=0;q<8;q++){ uint32_t a=0x40F00300u+12*q; if(load(a)==key&&key){'
                          ' if(ctx.r5.u32) store(ctx.r5.u32,load(a+8)); ctx.r3.u64=1; break; } }'),
        name('F-29443'): 'float d=(float)ctx.f1.f64; ctx.r3.u64=load(0x40F00200u+(d==10.0f?4u:0u));',
        name('F-23825'): '',
        name('F-70230'): ('uint32_t c=load(0x40E00000u); if(c<8){ store(0x40E00004u+16*c,ctx.r4.u32); store(0x40E00008u+16*c,ctx.r5.u32);'
                          ' store(0x40E0000Cu+16*c,ctx.r6.u32);} store(0x40E00000u,c+1); ctx.r3.u64=0;'),
    }
    # the say slot of every scenario class is its own slot-1 function; four indices (c * 16) would overlap, so the log keeps 8 at 16 bytes
    main_src = (MAIN.replace('F26639', name('F-26639')).replace('SPEECH_G', '0x%08Xu' % SPEECH_G).replace('POLICE_G', '0x%08Xu' % POLICE_G)
                .replace('RACE_G', '0x%08Xu' % RACE_G).replace('TUNE_G', '0x%08Xu' % TUNE_G).replace('RNG_G', '0x%08Xu' % RNG_G))
    exe = rdisp.build(idx, [name(f) for f in ('F-26639', 'F-14569', 'F-55001', 'F-58674', 'F-76898', 'F-27863') + tuple(CTORS)], main_src,
                      a.out, 'radio_chatter', image=img, vtables=vts, limit=600, stubs=stubs,
                      fakes={0x7F000010: 'ctx.r3.u64=1;', 0x7F000011: 'ctx.r3.u64=0;'})
    cases = gen_cases(a.cases, a.frames, a.seed)
    cf = a.out / 'radio_cases.txt'
    cf.write_text('\n'.join(l for c in cases for l in c) + '\n')
    r = subprocess.run([str(exe), str(a.image), str(cf), '%08X' % pl, '%08X' % ai], capture_output=True, text=True)
    if r.returncode:
        sys.exit(r.stdout[-500:] + r.stderr[-1500:])
    outs = r.stdout.splitlines()
    if any(o.startswith('MULTI') for o in outs):
        sys.exit('a call said more than one line: ' + next(o for o in outs if o.startswith('MULTI')))
    total = sum(len(c) - 1 for c in cases)
    assert len(outs) == total, (len(outs), total)
    full = []
    pos = 0
    for c in cases:
        cl = [c[0]]
        for l in c[1:]:
            cl.append(l + ' > ' + outs[pos])
            pos += 1
        full.append(cl)
    # coverage features of a case, from the outputs
    def features(cl):
        fs = set()
        n = int(cl[0].split()[1])
        prev = None
        for l in cl[1:]:
            v = l.split()
            o = v[v.index('>') + 1:]
            ins = v[1:v.index('>')]
            if o[0] != '-':
                fs.add('said ' + o[0] + (' n1' if n == 1 else ' n>1'))
                if n > 1:
                    fs.add('send %s/%d' % (o[1], n))
                    fs.add('recv ' + ('D' if o[2] == 'D' else 'wrap' if o[2] == '0' and o[1] == str(n - 1) else 'next'))
            if ins[1] == '0':
                fs.add('off')
            if ins[2] == '1':
                fs.add('busy')
            if ins[2] == '1' and prev and prev[3] == '0':
                fs.add('busy with t=0')
            if ins[3] == '0':
                fs.add('no target')
            if ins[4] == '0' and ins[5] == '1':
                fs.add('rec on an AI target')
            if prev is not None:
                if o[4] == bits(1.0) and prev[4] != bits(1.0):
                    fs.add('fw timer set to 1')
                if o[7] != prev[7]:
                    fs.add('far ' + prev[7] + '->' + o[7])
                if o[6] != prev[6]:
                    fs.add('nitro tries up')
                if prev[5] != o[5]:
                    fs.add('road changed')
            if ins[7] == '1' and ins[5] == '1' and o[0] != 'REPORT_PURSUIT_HIT_COP_CAR_HARD':
                fs.add('hard bit without a line')
            prev = o
        return fs

    feats = [features(cl) for cl in full]
    allf = set().union(*feats)
    chosen, covered = [], set()
    order = sorted(range(len(full)), key=lambda i: -len(feats[i]))
    size = 0
    while covered != allf:
        best = max(order, key=lambda i: len(feats[i] - covered) * 1000 - len(full[i]))
        if not feats[best] - covered:
            break
        chosen.append(best)
        covered |= feats[best]
    text = ''
    for i in chosen:
        text += '\n'.join(full[i]) + '\n'
    for i in range(len(full)):
        if len(text) > a.max_bytes:
            break
        if i not in chosen:
            text += '\n'.join(full[i]) + '\n'
            chosen.append(i)
    while len(text) > a.max_bytes:        # the greedy cover alone is too big: drop the last case
        chosen.pop()
        text = ''.join('\n'.join(full[i]) + '\n' for i in chosen)
    for tok in text.split():
        assert not (len(tok) == 8 and tok[:2] in ('82', '83')), tok     # no token reads as an address of the executable
    with open(a.ref, 'w', newline=chr(10)) as fh:
        fh.write(text)
    print('cases', len(cases), 'frames', total, 'features', len(allf), '->', a.ref, 'cases', len(chosen), 'bytes', len(text))
    for f in sorted(allf):
        print('  covered:', f)
    if a.port:
        src = a.port.read_text()
        items = [grab(src, "#[derive(Clone, Copy, Debug, PartialEq)]\npub(crate) struct RdoState {"), grab(src, "pub(crate) struct RdoIn<'a> {"),
                 grab(src, 'pub(crate) fn rdo_draw('), grab(src, 'fn rdo_cop('), grab(src, 'pub(crate) fn chatter_step(')]
        rs = '\n'.join(items) + '\n' + 'mod t {\n    use super::*;\n' + RS_REPLAY.replace('fn rdo_replay', 'pub fn rdo_replay') + '}\nuse t::rdo_replay;\n' + PORT_MAIN
        (a.out / 'port_radio.rs').write_text(rs)
        b = subprocess.run(['rustc', '-O', '--edition', '2021', '-A', 'warnings', str(a.out / 'port_radio.rs'), '-o', str(a.out / 'port_radio.exe')],
                           capture_output=True, text=True)
        if b.returncode:
            sys.exit(b.stderr[-3000:])
        full_file = a.out / 'radio_full.txt'
        full_file.write_text(''.join('\n'.join(cl) + '\n' for cl in full))
        p = subprocess.run([str(a.out / 'port_radio.exe'), str(full_file)], capture_output=True, text=True)
        print('port (all cases):', p.stdout.strip(), p.stderr[-500:])


if __name__ == '__main__':
    main()
