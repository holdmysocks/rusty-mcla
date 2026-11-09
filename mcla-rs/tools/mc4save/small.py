"""small.py - decoder of the small entries of a Midnight Club: Los Angeles save (mc4.sav / decrypted RAGE.SAV):
Stats, EditorData, ColorData, UserVinylData, RubberbandMgr, RaceFavorites, enableGps.

    decode(sav: bytes) -> dict      (one key per entry; JSON-serialisable)
    python small.py <save>          prints the dict as indented JSON

Layouts and the functions they were read from: small.md beside this file.
Read only; needs nothing but the standard library. Race names of the per-race statistics are
resolved when the game's race list (tune/race/0x0f1f4d8d) is found, else left as indices.
"""
import json
import os
import re
import struct
import sys

# ------------------------------------------------------------------ container

def entries(sav):
    """name -> (data version, size, data offset)"""
    ver, cnt = struct.unpack_from('>II', sav, 0)
    out, off = {}, 8
    for _ in range(cnt):
        nl = struct.unpack_from('>I', sav, off)[0]
        off += 4
        name = sav[off:off + nl].decode('latin1')
        off += nl
        dv = sav[off]
        size = struct.unpack_from('>I', sav, off + 1)[0]
        off += 5
        out[name] = (dv, size, off)
        off += size
    return ver, out


def _cstr(b):
    return b.split(b'\0')[0].decode('latin1')


def _unk(base, off, raw):
    return {'off': base + off, 'len': len(raw), 'hex': raw.hex()}


# ------------------------------------------------------------------ Stats

# the game's vehicle table (sub id of the v.. statistics; same list as the importer's)
VEHICLES = ['vp_vw_golf_83', 'vp_nsn_240sx_98', 'vp_dsn_280z_75', 'vp_vw_scirocco_88', 'vp_chv_camaross_69', 'vp_frd_boss302_69', 'vp_pon_transam_79', 'vp_chv_impala_96',
            'vp_buk_grandnational_87', 'vp_dge_challenger_70', 'vp_frd_focus_04', 'vp_pon_solstice_06', 'vp_mit_eclipse_99', 'vp_chv_cobalt_05', 'vp_mzd_rx8_06', 'vp_duc_monster_s4r_07',
            'vp_frd_mustang_06', 'vp_chv_camaro_06', 'vp_chv_camaro_06_dub', 'vp_dge_charger_06', 'vp_dge_challenger_06', 'vp_dge_challenger_06_dub', 'vp_cry_300c_06', 'vp_vw_golf_07',
            'vp_nsn_350z_06', 'vp_mit_3000gt_99', 'vp_mzd_rx7_95', 'vp_nsn_skyline_99', 'vp_aud_rs4_07', 'vp_mit_evo_06', 'vp_duc_999r_06', 'vp_mbz_s600_07', 'vp_mbz_s600_07_dub',
            'vp_lam_miura_71', 'vp_aud_r8_08', 'vp_am_vantage_07', 'vp_mbz_sl65_07', 'vp_chv_corvettez06_07', 'vp_sln_s281_06', 'vp_lam_gallardo_06', 'vp_lam_murcielago_06',
            'vp_lam_murcielago_06_dub', 'vp_frd_cobra_04', 'vp_frd_gt_06', 'vp_sln_s7_06', 'vp_kaw_ninja_zx14_06', 'vp_chv_police_96', 'vp_citybus_07', 'vpd_chv_belair_55',
            'vpd_buk_riviera_63', 'vpd_chv_impala_64', 'vpd_chv_malibu_65', 'vpd_pon_gto_66', 'vpd_chv_chevelle_70', 'vpd_lr_hse_08', 'vpd_lr_sport_08', 'vpd_mit_eclipse_09',
            'vpd_aud_s5_08', 'vpd_mit_evox_08', 'vpd_cad_xlrv_08', 'vpd_mbz_clk63amg_08', 'vpd_mbz_cl65amg_08', 'vpd_am_db9_08', 'vpd_duc_1098_07']

STAT_TYPES = {0: 'float', 1: 'int', 2: 'uint'}
STAT_OPS = {0: 'add', 1: 'min', 2: 'max', 3: 'set', 4: 'set_once', 5: 'direct'}

# achievement stats (id -> Xbox achievement id, title): the table built by sub_822CE1B8 + the
# executable's achievement resource. Value 1 = awarded.
ACHIEVEMENTS = {
    'aRUN': (18, 'Wanted Man'), 'aEVA': (19, 'Out On Bail'), 'aJDS': (20, 'Neither Bird nor Plane'), 'aJTM': (21, 'Jordan Jumpman'),
    'aWHL': (22, 'Unicycle'), 'a2WD': (23, 'Just Get A Motorcycle'), 'aROR': (24, 'Hear me Roar!'), 'aAGR': (27, 'Anger Management'),
    'aPUL': (28, 'Set To Stun'), 'aZON': (29, 'Zoning Out'), 'aULU': (30, 'In the Lap of... (beat the Luxury champ)'),
    'aUTU': (31, 'Name that Tune(r) (beat the Tuner champ)'), 'aUMU': (33, "World's Strongest (beat the Muscle champ)"),
    'aUMO': (34, 'Two Wheels of Fury (beat the Motorcycle champ)'), 'aUEX': (35, 'How Exotic (beat the Exotic champ)'),
    'aCPL': (36, 'The Basics (first mission complete)'), 'aFWW': (37, "What's a Speed Limit? (15 freeway wins)"), 'aw5C': (38, 'Streaker (5 wins in a row)'),
    'aBIK': (39, 'Winning With Less (10 motorcycle wins)'), 'aRRR': (40, 'Go on Green (15 red light wins)'), 'aD10': (41, 'Destroyer of Worlds (15 paybacks)'),
    'aC5D': (42, 'Here for a Pickup (15 deliveries)'), 'aB1T': (43, 'Father Time (one time trial fully done)'), 'aBAT': (44, 'Uncle Time (first mission of all 4 time trials)'),
    'aBTO': (45, 'Competitive Glory (all 4 tournaments)'), 'a200': (46, "It's How You Play The Game (300 races finished)"), 'aW18': (47, 'The Winner (180 wins)'),
    'aMIL': (48, 'Rags To Riches ($1 million)'), 'aDST': (49, 'Have Car, Will Travel (5000 miles)'), 'aNJR': (50, 'Getting Off the Bench'),
    'aNRE': (51, 'Who Made This?'), 'aSRE': (52, 'Saving Los Angeles (saved a race editor race)'), 'aFAB': (53, 'Paparazzi (16 photos)'),
    'aTMF': (55, 'T-Mobile myFaves'), 'aNRS': (56, 'Turn in the Work (Rate My Ride submit)'), 'aNRR': (57, "Everyone's a Critic (25 rides rated)"),
    'aNRC': (58, 'Crossing the Line (50 online races)'), 'aNST': (59, 'Master Thief (80 flags stolen)'), 'aNFL': (60, 'Postman (100 flags delivered)'),
    'aNRW': (61, 'Ranking Up (20 ranked wins)'), 'aCOH': (62, 'Where in the World? (30 collectables)'), 'aCOL': (64, "Waldo's Still Missing (60 collectables)"),
    'aBCC': (65, 'Retire the Master (beat the City champ)'), 'aAOR': (66, 'Mild Dedication (every goal of one goal attack race)'),
    'aAAR': (67, 'True Dedication (every goal of every race)'), 'aMSC': (69, "Goin' South (met the South Central mechanic)"),
    'aTDV': (70, 'South Central Test Drive'), 'aSUV': (71, "Big Ballin' (10 SUV wins)"), 'aLOW': (72, "Low Ballin' (10 lowrider wins)"),
    'aDSC': (73, 'Sign Here (10 South Central deliveries)'), 'aTSC': (74, 'Easier than the Hills (South Central tournament)'),
    'aWSC': (75, 'Money for nothing (South Central wager)'), 'aLSC': (76, 'Brother Time (South Central time trial)'),
    'aBSC': (77, 'Been there, Done that (South Central career)'), 'aARC': (78, 'GOOOOAAAAALLLLL!!!!! (all South Central goal attacks)'),
}

# id -> (meaning, source). source: 'label' = the game's own statistics label (string table key
# stat_<id>_<sub>), 'code' = the function of the executable that writes it, 'script' = the
# script that names it, 'guess'.
STAT_INFO = {
    # g: whole game (career and arcade, offline)
    'gAGH': ('total AGRO ambient hits', 'label'), 'gAGR': ('AGRO special ability uses', 'label'), 'gBRN': ('burnouts', 'label'),
    'gCOL': ('collectables found (the count; which ones: LocalOptions data + 0x3DF84, 60 flag bytes)', 'label+code'),
    'gCPA': ('cops activated', 'label'), 'gCPL': ('cops lost (pursuits evaded)', 'label'), 'gCPR': ('arrests', 'label'),
    'gCWN': ('non-career consecutive wins (running streak, set to 0 by a loss)', 'label+code'), 'gDAM': ('total damage points', 'label'),
    'gDIP': ('total drownings', 'label'), 'gHOP': ('opponents hit', 'label'), 'gHPR': ('props hit', 'label'), 'gHTR': ('traffic hit', 'label'),
    'gNIT': ('nitrous uses', 'label'), 'gOUT': ('times damaged out', 'label'), 'gRCP': ('races completed', 'label'),
    'gROR': ('ROAR special ability uses', 'label'), 'gSST': ('slipstream turbo uses', 'label'), 'gWIN': ('total wins', 'label'),
    'gWIP': ('wipeouts', 'label'), 'gZON': ('ZONE special ability uses', 'label'), 'ghTC': ('hours cruising', 'label'),
    'ghTP': ('hours played', 'label'), 'gk2W': ('distance driven on two wheels, miles (achievement at 50)', 'label+code'),
    'gkDS': ('distance travelled, miles (achievement at 5000)', 'label+code'), 'gkWL': ('wheelie distance, miles', 'label+code'),
    'gmJD': ('longest jump, metres (achievement at 310.896 m = 1020 ft)', 'label+code'), 'gsJT': ('total jump time, seconds (achievement at 600)', 'label+code'),
    'gsAT': ('most air time, seconds', 'label'), 'gvAS': ('average speed', 'label'), 'gvTS': ('top speed', 'label'), 'g:LO': ('longest race (time)', 'label'),
    # k: career
    'k$CE': ('total cash earned', 'label'), 'k$WA': ('wager win amount by hangout (sub 503 Hollywood, 511 Beaches, 521 Hills, 531 Downtown, 541 South Central)', 'label'),
    'k%AG': ('career percent completed', 'label'), 'k2ND': ('second places', 'label'), 'k3RD': ('third places', 'label'),
    'k:TB': ('time trial best time by time trial (sub 0..19), milliseconds', 'label+script'), 'kARS': ('average restarts a race', 'label'),
    'kCAR': ('cars unlocked', 'label'), 'kCCW': ('most consecutive wins', 'label'), 'kCDL': ('deliveries', 'label'),
    'kCWN': ('career consecutive wins (running streak; achievement at 5)', 'label+code'), 'kDLT': ('total deliveries (achievement at 15)', 'label+code'),
    'kEXW': ('exotic wins', 'label'), 'kFWN': ('freeway wins (achievement at 15)', 'label+code'), 'kHOK': ('hookmen beaten', 'label'),
    'kLED': ('most lead changes', 'label'), 'kLOW': ('lowrider wins', 'label'), 'kLUW': ('luxury wins', 'label'), 'kMOW': ('motorcycle wins', 'label'),
    'kMRS': ('most restarts in one race', 'label'), 'kMUW': ('muscle wins', 'label'), 'kPDS': ('payback vehicles destroyed (achievement at 15)', 'label+code'),
    'kRCP': ('career races completed (achievement at 300)', 'label+code'), 'kRLW': ('red light wins (achievement at 15)', 'label+code'),
    'kSKL': ('skill', 'label'), 'kSUW': ('SUV wins', 'label'), 'kTOU': ('tournaments beaten', 'label'), 'kTRS': ('total restarts', 'label'),
    'kTUW': ('tuner wins', 'label'), 'kWIN': ('career wins (achievement at 180)', 'label+code'), 'khTC': ('career hours cruising', 'label'),
    # n: online (the g/k counters' online twins)
    'n2ND': ('online second places', 'code sub_8229BF90'), 'nBRN': ('online burnouts', 'code sub_8232EA78'), 'nFIN': ('online races finished', 'code sub_8229BF90'),
    'nFLG': ('flags delivered (achievement at 100)', 'code sub_822CCCD0'), 'nFPK': ('enemy flag pickups', 'script'), 'nFST': ('flags stolen (achievement at 80)', 'code sub_822CCCD0'),
    'nHOP': ('online opponents hit', 'code sub_82332D30'), 'nHPR': ('online props hit', 'guess'), 'nNIT': ('online nitrous uses', 'code sub_82345820'),
    'nOUT': ('online damaged out', 'code sub_82342B70'), 'nRCP': ('online races completed (achievement at 50)', 'code sub_822CCCD0'),
    'nSST': ('online slipstream turbo uses', 'code sub_82345E40'), 'nSTA': ('online races started', 'guess'), 'nWIN': ('online ranked wins (achievement at 20)', 'code sub_822CCCD0'),
    'nWIP': ('online wipeouts', 'code sub_82342B70'), 'nkDS': ('online distance, miles', 'code sub_822A61B0'), 'nm2W': ('online two-wheel distance', 'code sub_823663D0'),
    'nmWL': ('online wheelie distance', 'code sub_823688B8'), 'nsJT': ('online jump time, seconds', 'code sub_82330378'),
    # r: by race (sub = index of the race in the race list tune/race/0x0f1f4d8d)
    'r:TI': ('best race time, milliseconds (kept when lower)', 'code sub_822D0438'),
    'rBTT': ('goal attack: beat the target time (1 = done)', 'code sub_822CD720 + label stat_Beat_TargetTime'),
    'rCGL': ('goal attack: all goals complete (1 = done)', 'code sub_822CD720 + label stat_Complete_All'),
    'rMDG': ('goal attack: minimum damage (1 = done)', 'code sub_822CD720 + label stat_Min_Damage'),
    'rWIN': ('goal attack: won the race (1 = done)', 'code sub_822CD720 + label stat_Won_Race'),
    # t: tracking / set-up
    'tCAM': ('camera (profile + 1208), written at every save', 'code sub_822D00F0'), 'tCIN': ('profile + 1192 (cinematics setting), written at every save', 'code sub_822D00F0'),
    'tDL1': ('content mask word 0 (static word at 0x827E9170)', 'code sub_822D00F0'), 'tDL2': ('content mask word 1 (0x827E9174)', 'code sub_822D00F0'),
    'tGAR': ('garage visited', 'code sub_826632C0'), 'tGRR': ('rides rated in Rate My Ride (achievement at 25)', 'code sub_822CCCD0'),
    'tIME': ('date of the last save: (year - 2000) << 24 | month << 16 | day << 8 | hour', 'code sub_822D00F0'),
    'tINP': ('controller configuration (profile word 0 or 1)', 'code sub_822D00F0'), 'tLNG': ('language (profile + 1320)', 'code sub_822D00F0'),
    'tMTC': ('music tracks played, total', 'code sub_82208930'), 'tMTP': ('times a music track was played (sub = track id)', 'code sub_82208930'),
    'tNOR': ('races started with no opponents ready / normal start', 'guess (scripts)'), 'tPAR': ('free-parts flag', 'code sub_8269FCC8'),
    'tPRF': ('performance parts bought', 'code sub_826AB520'), 'tPTS': ('photos saved (achievement at 16)', 'code sub_8263DCF8'),
    'tPUR': ('money (the purse)', 'code sub_826B93E0'), 'tRAC': ('?', 'guess'), 'tRES': ('screen height at load', 'code sub_822D1A78'),
    'tREW': ('reward flag words (sub 0..2)', 'code sub_822D1260'), 'tSPE': ('constant 1, written at every save', 'code sub_822D00F0'),
    'tUPG': ('upgrades bought in the garage', 'code sub_826AB520'), 'tVYL': ('vinyl layers applied', 'code sub_826AB520'), 'tWID': ('wide screen at load', 'code sub_822D1A78'),
    'tCER': ('?', 'code sub_826B0BB8'), 'tRPL': ('replays watched', 'code sub_82665CE0'), 'tUSR': ('user music', 'code sub_82208930'),
    # tT..: tutorial shown flags (scripts)
    'tTAG': ('tutorial shown: AGRO', 'script'), 'tTBM': ('tutorial shown: beat me there', 'script'), 'tTCH': ('tutorial shown: telephone challenge', 'script'),
    'tTDE': ('tutorial shown: delivery', 'script'), 'tTEM': ('tutorial shown: EMP', 'script'), 'tTFB': ('tutorial flag of the cruising hookmen script', 'script'),
    'tTFL': ('tutorial flag of the cruising hookmen script', 'script'), 'tTFR': ('tutorial shown: freeway race', 'script'), 'tTGS': ('tutorial shown: gas station', 'guess (script)'),
    'tTHY': ('tutorial shown: hydraulics', 'script'), 'tTMO': ('tutorial shown: motorcycle', 'script'), 'tTPA': ('tutorial shown: payback', 'script'),
    'tTPL': ('tutorial shown: police (code)', 'code sub_82700108'), 'tTPP': ('tutorial shown: police pull over (code)', 'code sub_82700108'),
    'tTPS': ('tutorial shown: pink slip', 'script'), 'tTRL': ('tutorial shown: red light race', 'script'), 'tTRO': ('tutorial shown: ROAR', 'script'),
    'tTSE': ('tutorial shown: series', 'script'), 'tTTA': ('time trial flags (sub 250..254)', 'script'), 'tTTO': ('tutorial shown: tournament', 'script'),
    'tTTT': ('tutorial shown: time trial', 'script'), 'tTWA': ('tutorial shown: wager', 'script'), 'tTZO': ('tutorial shown: ZONE', 'script'),
    # v: by vehicle (sub = index in the game's vehicle table)
    'vWIN': ('wins with the vehicle', 'code sub_822D0438'), 'vkDT': ('distance driven with the vehicle, miles', 'code sub_822A61B0'),
    # x: other counters
    'xAGR': ('AGRO uses (second counter)', 'code sub_82338E00'), 'xCDL': ('deliveries completed by garage (sub 504 Hollywood, 542 South Central)', 'label STAT_KCDL_504 + code'),
    'xFAO': ('arcade races started: ordered', 'guess, code sub_8265CC90'), 'xFAU': ('arcade races started: unordered', 'guess, code sub_8265CC90'),
    'xFAC': ('arcade races started: circuit', 'guess, code sub_8265CC90'), 'xFAL': ('arcade races started: landmark', 'guess, code sub_8265CC90'),
    'xGAD': ('game activity count: delivery', 'script'), 'xGAF': ('game activity count: freeway race', 'script'), 'xGAM': ('game activity count: time trial', 'script'),
    'xGAP': ('game activity count: pink slip', 'script'), 'xGAR': ('game activity count: cruising racer race', 'script'), 'xGAS': ('game activity count: series', 'script'),
    'xGAT': ('game activity count: tournament', 'script'), 'xGAW': ('game activity count: wager', 'script'), 'xGAK': ('game activity count: payback', 'guess (script)'),
    'xLRW': ('last race was won (1) - feeds the streaks', 'code sub_822D0438'), 'xMOW': ('wins (second counter, shown in the summary block)', 'code sub_822D0438'),
    'xRRT': ('race restarts by race (sub id)', 'code sub_82255F78'), 'xTCO': ('all collectables found flag', 'script Collectables.sco'),
}

_RACE_LIST = None


def race_list():
    """the game's race list (index = sub id of the r.. statistics), or [] when not at hand"""
    global _RACE_LIST
    if _RACE_LIST is None:
        _RACE_LIST = []
        for p in (os.environ.get('MCLA_RACE_LIST', ''), r'D:\MCLA\extracted\cache\tune\race\0x0f1f4d8d'):
            if p and os.path.isfile(p):
                d = open(p, 'rb').read()
                _RACE_LIST = [m.decode('latin1') for m in re.findall(rb'Id\s+([Gg]ame/Race/[A-Za-z0-9_/]+)', d)]
                break
    return _RACE_LIST


def stat_meaning(ident):
    if ident in ACHIEVEMENTS:
        n, title = ACHIEVEMENTS[ident]
        return 'achievement %d awarded: %s' % (n, title)
    return STAT_INFO.get(ident, ('?', 'unknown'))[0]


def decode_stats(sav, off, size):
    b = sav[off:off + size]
    f = lambda o: round(struct.unpack_from('>f', b, o)[0], 6)
    u = lambda o: struct.unpack_from('>I', b, o)[0]
    nveh = len(VEHICLES)
    summary = {
        'tag': _cstr(b[0:4]),                      # "005"
        'career_percent': u(4),                    # k%AG
        'win_percent': f(8),                       # gWIN * 100 / gRCP
        'cash_earned': u(12),                      # k$CE
        'current_vehicle_hash': '0x%08x' % u(16),  # word +4 of what sub_8238F998 returns for the vehicle in use
        'hours_played': f(20), 'distance': f(24), 'arrests': u(28), 'cops_lost': u(32),
        'average_restarts': f(36), 'max_jump_distance': f(40), 'damaged_out': u(44),
        'list_3dfc8': [u(48 + 4 * i) for i in range(32)],  # the valid ones of LocalOptions + 0x3DFC8 (32 words)
        'wins': u(176), 'zero_180': u(180), 'hours_cruising': f(184), 'top_speed': f(188),
        'nitrous': u(192), 'slipstream': u(196), 'zone': u(200), 'roar': u(204), 'agro': u(208), 'agro_hits': u(212),
        'two_wheel_distance': f(216), 'wheelie_distance': f(220), 'max_jump_distance_2': f(224), 'jump_time': f(228), 'most_air_time': f(232),
        'wipeouts': u(236), 'damaged_out_2': u(240), 'collectables': u(244), 'races_completed': u(248), 'wins_x': u(252),
        'career_wins': u(256), 'hookmen_beaten': u(260), 'most_consecutive_wins': u(264), 'agro_x': u(268), 'career_percent_2': u(272),
        'lead_changes': u(276), 'wager_winnings': u(280), 'career_win_streak': u(284), 'career_races_completed': u(288),
        'burnouts': u(292), 'opponents_hit': u(296), 'props_hit': u(300),
        'vehicle_distance': {VEHICLES[i]: f(304 + 4 * i) for i in range(nveh) if f(304 + 4 * i) != 0.0},
        'vehicle_wins': {VEHICLES[i]: struct.unpack_from('>H', b, 704 + 2 * i)[0] for i in range(nveh) if struct.unpack_from('>H', b, 704 + 2 * i)[0]},
    }
    unknown = []
    # the vehicle arrays have 100 places, the vehicle table 64: the rest is zero filler
    for o, n, w in ((304, 100, 4), (704, 100, 2)):
        tail = b[o + nveh * w:o + n * w]
        if any(tail):
            unknown.append(_unk(off, o + nveh * w, tail))
    count = u(904)
    races = race_list()
    records = []
    for i in range(count):
        r = b[908 + 16 * i:924 + 16 * i]
        ident = r[0:4].decode('latin1')
        sub = struct.unpack_from('>I', r, 4)[0]
        ty, op = r[8] & 7, (r[8] >> 3) & 7
        value = struct.unpack_from({0: '>f', 1: '>i'}.get(ty, '>I'), r, 12)[0]
        rec = {'id': ident, 'sub': sub, 'type': STAT_TYPES.get(ty, ty), 'op': STAT_OPS.get(op, op),
               'value': round(value, 6) if ty == 0 else value}
        if ty == 0:
            rec['f32'] = value   # the f32 exactly (the importer keeps it)
        if ident[0] == 'v' and sub < nveh:
            rec['vehicle'] = VEHICLES[sub]
        if ident[0] == 'r' and sub < len(races):
            rec['race'] = races[sub]
        if ident == 'tIME':
            rec['date'] = '%04d-%02d-%02d %02d h' % (2000 + (value >> 24 & 0x7f), value >> 16 & 0xff, value >> 8 & 0xff, value & 0xff)
        if r[8] & 0xc0:
            rec['flag_high_bits'] = r[8] >> 6
        if any(r[9:12]):
            rec['pad'] = r[9:12].hex()   # struct padding the game never initialises
        records.append(rec)
    tail = b[908 + 16 * count:]
    if any(tail):
        unknown.append(_unk(off, 908 + 16 * count, tail))
    out = {'summary': summary, 'count': count, 'records': records,
           'meaning': {i: stat_meaning(i) for i in sorted({r['id'] for r in records})}}
    if unknown:
        out['_unknown'] = unknown
    return out


# ------------------------------------------------------------------ EditorData

SLOT, CHECKPOINTS = 1056, 31


def _slot(b, base, index):
    count_word = struct.unpack_from('>I', b, 0)[0]
    city = b[1043]
    # the game's rule (sub_822502D8): city byte 0xCD (never written) -> the word, else the byte
    count = count_word if city == 0xCD else b[1042]
    legacy = False
    if count == 0 and city in (0, 0xCD) and 2 <= count_word <= 30:
        # a slot written by the other console's build: the count in the word only
        count, legacy = count_word, True
    count = min(count, 30)
    pts = []
    for i in range(CHECKPOINTS):
        c = b[48 + 32 * i:80 + 32 * i]
        x, y, z = struct.unpack_from('>3f', c, 0)
        pts.append({'pos': [x, y, z], 'activation_height': c[16], 'activation_radius': c[17], 'w_raw': c[12:16].hex(), 'raw': c})
    out = {'slot': index, 'name': _cstr(b[4:36]) if b[4] != 0xCD else '', 'in_use': count >= 2, 'count': count,
           'city': None if city == 0xCD else city,
           'grid_variant': None if b[1040] == 0xCD else struct.unpack_from('>b', b, 1040)[0],
           'flag_1041': None if b[1041] == 0xCD else b[1041],
           'count_word': count_word, 'count_byte': None if b[1042] == 0xCD else b[1042],
           'checkpoints': [{k: p[k] for k in ('pos', 'activation_height', 'activation_radius')} for p in pts[:count]]}
    if legacy:
        out['count_from_word'] = True
    stale = [i for i in range(count, CHECKPOINTS) if pts[i]['raw'][:18] != bytes(16) + b'\x14\x14']
    if stale:
        out['stale_checkpoints'] = [{'index': i, 'pos': pts[i]['pos'], 'activation_height': pts[i]['activation_height'],
                                     'activation_radius': pts[i]['activation_radius']} for i in stale]
    pad = b[36:48] + b''.join(p['raw'][18:] for p in pts) + b[1044:1056]
    if any(v != 0xCD for v in pad):
        out['padding_not_cd'] = True   # (a received slot is a copy of a network buffer: stale memory in the padding)
    return out


def decode_editor(sav, off, size):
    b = sav[off:off + size]
    mine = [_slot(b[i * SLOT:(i + 1) * SLOT], off + i * SLOT, i) for i in range(20)]
    recv = [_slot(b[(20 + i) * SLOT:(21 + i) * SLOT], off + (20 + i) * SLOT, i) for i in range(20)]
    out = {'slots': mine, 'received': [s for s in recv if s['name'] or s['count']],
           'received_cursor': struct.unpack_from('>I', b, 42240)[0]}
    tail = b[42244:]
    if any(v != 0xCD for v in tail):
        out['_unknown'] = [_unk(off, 42244, tail)]
    return out


# ------------------------------------------------------------------ ColorData

# the table the executable copies into the structure (0x82828DF8); nothing ever changes it
DEFAULT_COLORS = bytes.fromhex(
    '0000000000ffffff00ddd8bc00d8cfa0000100320010186e00204465006a777d0085c2e500524636006b594500934c3c'
    '002d1111006d616400967886002f4432004c4900004d756900009b7f002a84000047484c00595f650072727200917f63'
    '0093908b00ada39a00a3a3a300d8d8d800e2441200c17a3800e57d3200edba3600bfa62400b2ac100093346f008c705c'
    '003a0808008a0b2200af261300df2b12')


def decode_colors(sav, off, size):
    b = sav[off:off + size]
    return {'swatches': ['#%02x%02x%02x' % (b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]) for i in range(40)],
            'alpha_bytes_zero': all(b[4 * i] == 0 for i in range(40)), 'is_default': b == DEFAULT_COLORS}


# ------------------------------------------------------------------ UserVinylData

ZONES = (('top', 64), ('left', 64), ('right', 64), ('front', 16), ('back', 16))
BLEND = {0: 'Normal', 1: 'AlphaCutout', 2: 'NegativeCutout'}
VINYL_SLOT = 5764


def _layer(r):
    sx, sy, px, py = struct.unpack('>4e', r[0:8])
    skew, rot = struct.unpack('>2e', r[12:16])
    return {'shape': struct.unpack('>H', r[16:18])[0], 'scale': [sx, sy], 'pos': [px, py], 'rot': rot, 'skew': skew,
            'color': [r[9], r[10], r[11], r[8]], 'blend': BLEND.get(r[18], r[18]), 'hidden': bool(r[19])}


def decode_user_vinyls(sav, off, size):
    b = sav[off:off + size]
    slots, unknown = [], []
    for s in range(3):
        v = b[s * VINYL_SLOT:(s + 1) * VINYL_SLOT]
        zones, o, layers = {}, 4, 0
        for name, cap in ZONES:
            zl = []
            for i in range(cap):
                r = v[o:o + 20]
                o += 20
                if r[16:18] != b'\xff\xff':
                    lay = _layer(r)
                    lay['place'] = i
                    zl.append(lay)
            zones[name] = zl
            layers += len(zl)
        slots.append({'slot': s, 'used': bool(v[0]), 'layers': layers, 'zones': zones})
        for o2, raw in ((1, v[1:4]), (4484, v[4484:])):
            if any(c != 0xCD for c in raw):
                unknown.append(_unk(off + s * VINYL_SLOT, o2, raw))
    out = {'slots': slots}
    if unknown:
        out['_unknown'] = unknown
    return out


# ------------------------------------------------------------------ RubberbandMgr, RaceFavorites, enableGps

def decode_rubberband(sav, off, size):
    b = sav[off:off + size]
    points, step, level = struct.unpack_from('>3i', b, 0)
    out = {'points': points, 'step_in_use': step, 'level': level}
    if any(c != 0xCD for c in b[12:]):
        out['_unknown'] = [_unk(off, 12, b[12:])]
    return out


def decode_favorites(sav, off, size):
    b = sav[off:off + size]
    favs, removed, junk = [], [], []
    for i in range(100):
        r = b[i * 64:(i + 1) * 64]
        if r[0]:
            favs.append({'slot': i, 'race': _cstr(r)})
        elif any(c not in (0, 0xCD) for c in r[1:]):
            removed.append({'slot': i, 'race_without_first_letter': _cstr(r[1:])})   # removal zeroes the first byte only
    out = {'favorites': favs, 'races': [f['race'] for f in favs]}
    if removed:
        out['removed'] = removed
    return out


def decode(sav):
    ver, ent = entries(sav)
    out = {'_container_version': ver}
    for name, fn in (('Stats', decode_stats), ('EditorData', decode_editor), ('ColorData', decode_colors),
                     ('UserVinylData', decode_user_vinyls), ('RubberbandMgr', decode_rubberband), ('RaceFavorites', decode_favorites)):
        dv, size, off = ent[name]
        d = fn(sav, off, size)
        d['_version'] = dv
        out[name] = d
    dv, size, off = ent['enableGps']
    out['enableGps'] = {'enabled': bool(sav[off]), '_version': dv}
    if sav[off] > 1:
        out['enableGps']['_unknown'] = [_unk(off, 0, sav[off:off + 1])]
    # where the collectables are: outside this region, given here because Stats holds the count only
    dv, size, off = ent['LocalOptions']
    flags = sav[off + 0x3DF84:off + 0x3DF84 + 60]
    out['collected_note'] = {'where': 'LocalOptions data + 0x3DF84, 60 bytes (file offset 0x%x)' % (off + 0x3DF84),
                             'collected': [i for i, v in enumerate(flags) if v]}
    return out


if __name__ == '__main__':
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    print(json.dumps(decode(open(sys.argv[1], 'rb').read()), indent=1))
