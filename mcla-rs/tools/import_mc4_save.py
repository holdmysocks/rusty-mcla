"""import_mc4_save.py <mc4.sav> <port save in.json> <out.json>

Carries what the port can use out of an original save (Xbox 360 mc4.sav, or a decrypted
PS3 RAGE.SAV) into a save of the port:

  * the statistics table ("Stats" section: records of 16 bytes - 4-character id, u32 sub
    id, type byte ')' int / '(' float, 3 flag bytes, value). Whole numbers with sub id 0
    go to career.stats under their id (the port counts under the same ids: class wins
    kTUW.., the free-parts flag tPAR ..); every record goes to career.ostats as
    "<id>/<sub>" for the statistics pages. Records by vehicle (ids starting with v) are
    keyed by the vehicle's name, "<id>/<car>", through the game's vehicle table.
  * the garage: the owned vehicles in the save's order, the first being the current one,
    and each one's thirteen performance part stages (0..2), wheel figures (ride height,
    tire width, rim size, tire profile, front and rear: bytes 0x83a..0x842 of its record)
    and plate (7 characters at 0x847; the codes 0x5B..0x5E stand for the plate's four signs).
    Each vehicle's rim (the game's rim table index, byte 0x844), tires (0x845) and brakes
    (0x846), and for the vehicle in use the rim's three materials (u32s at 0x178: 0 painted,
    1 chrome, 3 brushed).
    Each vehicle's fitted parts: one byte a part in the record, its value the part's index of
    the car's part file (found by highlighting parts in the running game and watching the
    record, step 53): hood 0x862, rear bumper 0x863, front bumper 0x865, sideskirts 0x868,
    spoiler 0x869, headlights 0x86b, taillights 0x86c, exhaust 0x877, front seats 0x88d,
    steering wheel 0x894, gauges 0x895, doors 0x89a, wide body 0x89b.
    The paint of the vehicle in use: six entries of 0x30 bytes from 0x660 (the body's parts
    when painted separately; the first is taken) - primary RGB, secondary RGB, then the
    paint's type (1 matte .. 5 sparkle, the port's numbers) and the fade type.
    (Not carried: fitted parts, vinyls, parts painted separately and the interior's
    colours - the table of RGB floats from 0x50 on.)
  * the career's missions ("Career" section: per mission a 128-byte name field, then its
    index, its place among the active ones, its place among the completed ones and its
    state - 0 not yet open, 3 active, 5 complete): complete and active missions with the
    active ones' order. A fresh game has two complete and two active; the 100 % save 130
    complete and the ten REPEAT_ missions active.
  * the money (tPUR) and the reputation.


Since the whole save is decoded (tools/mc4save: one module per region, layouts in
D:/MCLA/work/save/decode/*/*.md), the rest comes too:

  * every statistic by the table's own count (the old scan for the type byte missed some),
  * the vehicle in use by the garage's current slot, every vehicle's part brands, brake model,
  * the money from the Career entry's own field, missions in every state with their tallies,
  * the options (five volumes, subtitles, camera shake, steering sensitivity, music genre),
  * the 60 collectables' flags, the race editor's races, the favourite races, the rubber band
    (difficulty) level and points, hydraulics / window tint row / neon of the vehicle in use,
  * parts owned (the categories whose names are known).

Everything of the save that has no field of its own in the port's save goes, by its original
names, into the block "mc4" of the port's save (`mc4_block`): options, garage (per vehicle:
paint areas with both colours, tint, neon, air bags, hydraulics, ability, origin, plate
background, every part byte ...), the per-model purchase records, damage, ability levels,
the career's unlock state, the editor's races as saved, received races, vinyl packages,
achievements, tutorial flags. The port reads that block (crates/game/src/mc4.rs).
The complete decode of the save - every byte by name - is written beside the output as
<out>.mc4full.json for reference; the port does not read it.

Nothing else of the port's save is touched. Read only on the original save.
"""
import json, os, re, struct, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from mc4save import career as dec_career, options as dec_options, small as dec_small, vehicle as dec_vehicle

# the game's vehicle table (index = the sub id of the v.. statistics and of the purchase
# records at profile + 0x3C370 + 84 * index), read from the running game
VEHICLES = ['vp_vw_golf_83', 'vp_nsn_240sx_98', 'vp_dsn_280z_75', 'vp_vw_scirocco_88', 'vp_chv_camaross_69', 'vp_frd_boss302_69', 'vp_pon_transam_79', 'vp_chv_impala_96',
            'vp_buk_grandnational_87', 'vp_dge_challenger_70', 'vp_frd_focus_04', 'vp_pon_solstice_06', 'vp_mit_eclipse_99', 'vp_chv_cobalt_05', 'vp_mzd_rx8_06', 'vp_duc_monster_s4r_07',
            'vp_frd_mustang_06', 'vp_chv_camaro_06', 'vp_chv_camaro_06_dub', 'vp_dge_charger_06', 'vp_dge_challenger_06', 'vp_dge_challenger_06_dub', 'vp_cry_300c_06', 'vp_vw_golf_07',
            'vp_nsn_350z_06', 'vp_mit_3000gt_99', 'vp_mzd_rx7_95', 'vp_nsn_skyline_99', 'vp_aud_rs4_07', 'vp_mit_evo_06', 'vp_duc_999r_06', 'vp_mbz_s600_07', 'vp_mbz_s600_07_dub',
            'vp_lam_miura_71', 'vp_aud_r8_08', 'vp_am_vantage_07', 'vp_mbz_sl65_07', 'vp_chv_corvettez06_07', 'vp_sln_s281_06', 'vp_lam_gallardo_06', 'vp_lam_murcielago_06',
            'vp_lam_murcielago_06_dub', 'vp_frd_cobra_04', 'vp_frd_gt_06', 'vp_sln_s7_06', 'vp_kaw_ninja_zx14_06', 'vp_chv_police_96', 'vp_citybus_07', 'vpd_chv_belair_55',
            'vpd_buk_riviera_63', 'vpd_chv_impala_64', 'vpd_chv_malibu_65', 'vpd_pon_gto_66', 'vpd_chv_chevelle_70', 'vpd_lr_hse_08', 'vpd_lr_sport_08', 'vpd_mit_eclipse_09',
            'vpd_aud_s5_08', 'vpd_mit_evox_08', 'vpd_cad_xlrv_08', 'vpd_mbz_clk63amg_08', 'vpd_mbz_cl65amg_08', 'vpd_am_db9_08', 'vpd_duc_1098_07',
            # (64..68: the police cars of the South Central pack)
            'vp_se_chv_plc_96', 'vp_se_chv_chp_96', 'vp_se_dge_plc_06', 'vp_se_dge_ubp_06', 'vp_se_dge_chp_06']
# a garage record: 8176 bytes, the vehicle's name 8109 bytes in (the display name 32 bytes
# after it), the thirteen performance stages at +28
RECORD, NAME_AT, PERF_AT = 8176, 8109, 28
# the fitted parts' bytes in a record (the port's names: its `cfg` slots, then `xparts`)
PART_BYTES = {'hood': 0x862, 'bumper_r': 0x863, 'bumper_f': 0x865, 'skirts': 0x868, 'spoiler': 0x869, 'exhaust': 0x877, 'seats': 0x88d, 'body': 0x89b,
              'headlight': 0x86b, 'taillight': 0x86c, 'steer_whl': 0x894, 'post_gauge': 0x895, 'scissor_door': 0x89a,
              # (by their places and the saved GTO, which wears a blower: not seen changing)
              'blower': 0x870, 'side_exhaust': 0x878,
              # (the intercooler is fitted: UseIntercooler - its own byte, 0x865, mirrors the front
              # bumper; the bikes' three parts; the hood in carbon fibre: CarbonFiberHood)
              'intercooler': 0x89d, 'bike_tail': 0x871, 'bike_tank': 0x872, 'bike_cowl': 0x873, 'carbon_hood': 0x898}
CFG_SLOTS = ('hood', 'bumper_r', 'bumper_f', 'skirts', 'spoiler', 'exhaust', 'seats', 'body')


def f32_shortest(v):
    """the shortest decimal that reads back as the same f32 (v is that f32 widened)"""
    if v != v or v in (float('inf'), float('-inf')):
        return v
    for p in range(1, 10):
        s = float('%.*g' % (p, v))
        if struct.unpack('>f', struct.pack('>f', s))[0] == v:
            return s
    return v


def stats(small):
    """(id, sub id, value, is float) of every record of the statistics table (its count is at
    +904 of the entry; byte 8 of a record: bits 0-2 the type - 0 float, 1 int, 2 unsigned)."""
    return [(r['id'], r['sub'], r.get('f32', r['value']), r['type'] == 'float') for r in small['Stats']['records']]


def garage(d):
    """The owned vehicles and their performance stages."""
    first = min((m.start() for m in re.finditer(rb'vpd?_[a-z0-9_]+' + bytes(1), d[:0x40000])), default=None)
    cars, perf, dims, plates, paint = [], {}, {}, {}, None
    wheels, mats, paints, parts = {}, None, {}, {}
    global finishes, liveries
    finishes = {}
    liveries = {}
    while first is not None and first + 40 < len(d):
        name = d[first:first + 40].split(bytes(1))[0]
        if not re.fullmatch(rb'vpd?_[A-Za-z0-9_]{4,36}', name):
            break
        cars.append(name.decode().lower())
        rec = first - NAME_AT
        perf[cars[-1]] = list(d[rec + PERF_AT:rec + PERF_AT + 13])
        dims[cars[-1]] = list(d[rec + 0x83a:rec + 0x842])
        plate = d[rec + 0x847:rec + 0x84e]
        if all(32 <= c < 127 for c in plate):
            plates[cars[-1]] = plate.decode('latin1')
        wheels[cars[-1]] = (d[rec + 0x844], d[rec + 0x845], d[rec + 0x846])
        parts[cars[-1]] = {k: d[rec + o] for k, o in PART_BYTES.items()}
        # (the port's rim finish: 0 painted, 1 chrome, 2 brushed metal, 3 carbon fibre - the record's 0, 1, 3, 2)
        own_mats = [{0: 0, 1: 1, 3: 2, 2: 3}.get(v, 1) for v in struct.unpack('>3I', d[rec + 0x178:rec + 0x184])]
        if mats is None:
            mats = own_mats
        # The vehicle's other colours (found by changing each in the running game, step 68):
        # RGB floats as the screen shows them - interior neon +0x60, gauge colour A +0x70,
        # B +0x80, rim spoke A +0x90, spoke B +0xa0, lip +0xb0, interior colour A +0x5c0,
        # B +0x5d0, C +0x5e0. In the port's order of slots (neon, gauges, interior, rims).
        import colorsys
        slots = []
        for k, off in enumerate((0x60, 0x70, 0x80, 0x5c0, 0x5d0, 0x5e0, 0x90, 0xa0, 0xb0)):
            c = struct.unpack('>3f', d[rec + off:rec + off + 12])
            if not all(0.0 <= v <= 1.0 for v in c):
                slots.append(None)
                continue
            h = colorsys.rgb_to_hsv(*c)
            # (a black cabin and grey gauge lights are the stock look, which the port draws
            # its own way - step 62)
            stock = (3 <= k <= 5 and h[2] < 0.08) or (1 <= k <= 2 and h[1] < 0.15)
            slots.append(None if stock else [round(v, 4) for v in h])
        finishes[cars[-1]] = [slots, own_mats]
        # The vehicle's vinyls: the set-up files' layer records (20 bytes from +0x904, the
        # zones top, left, right, front, back of 64, 64, 64, 16, 16 places) - as the port's
        # `vinyl::Livery`.
        zones, o, any = [], rec + 0x904, False
        for cap in (64, 64, 64, 16, 16):
            layers = []
            for _ in range(cap):
                r = d[o:o + 20]
                o += 20
                shape = struct.unpack('>H', r[16:18])[0]
                if shape == 0xffff or r[19] != 0:
                    continue
                sx, sy, px, py = struct.unpack('>4e', r[0:8])
                skew, rot = struct.unpack('>2e', r[12:16])
                layers.append({'shape': shape, 'scale': [sx, sy], 'pos': [px, py], 'rot': rot, 'skew': skew, 'color': [r[9], r[10], r[11], r[8]], 'blend': {1: 'AlphaCutout', 2: 'NegativeCutout'}.get(r[18], 'Normal')})
                any = True
            zones.append(layers)
        if any:
            liveries[cars[-1]] = {'zones': zones}
        e = rec + 0x660
        rgb = struct.unpack('>3f', d[e:e + 12])
        kind, fade = struct.unpack('>2I', d[e + 0x20:e + 0x28])
        if all(0.0 <= c <= 1.0 for c in rgb) and 1 <= kind <= 5:
            paints[cars[-1]] = (rgb, kind, fade)
            # (the six entries are the paint shop's PAINT AREAs in its order - main body,
            # doors, hood, front bumper, rear bumper, sideskirts: step 65)
            own = {}
            for n, key in enumerate(('doors', 'hood', 'bumper_f', 'bumper_r', 'skirts'), 1):
                h = struct.unpack('>3f', d[e + 0x30 * n:e + 0x30 * n + 12])
                if all(0.0 <= c <= 1.0 for c in h) and max(abs(a - b) for a, b in zip(h, rgb)) > 0.02:
                    own[key] = [int(round(c * 255)) for c in h]
            if own:
                parts[cars[-1]]['_part_paint'] = own
            if paint is None:
                paint = (rgb, kind, fade)
        first += RECORD
    return cars, perf, dims, plates, paint, wheels, mats, paints, parts


def missions(d):
    """(name, active order, state) of every mission of the Career section."""
    i = d.find(b'Career', 0x40000)
    end = d.find(b'RaceFavorites', i)
    out = []
    for m in re.finditer(rb'[A-Z][A-Z0-9_]{5,60}' + bytes(1), d[i:end] if i > 0 else b''):
        a = i + m.start()
        j = a + len(m.group())
        if d[j:j + 4] != bytes([0xCD]) * 4:
            continue
        while d[j] == 0xCD:
            j += 1
        w = struct.unpack('>4I', d[j + 1:j + 17])
        name = m.group()[:-1].decode()
        # (the save calls the lowrider reputation mission UPLOWRIDER_REP; its file and id on
        # the disc are LOWRIDER_REP)
        name = {'UPLOWRIDER_REP': 'LOWRIDER_REP'}.get(name, name)
        out.append((name, w[1], w[3]))
    return out


def strip(x, drop):
    """A decoded dict without the keys that are layout notes, not data."""
    if isinstance(x, dict):
        return {k: strip(v, drop) for k, v in x.items() if k not in drop}
    if isinstance(x, list):
        return [strip(v, drop) for v in x]
    return x


# (layout notes, pointers and constant set-up data of the decoders' output)
NOTES = {'_layout', '_residue', '_unknown', '_pad_unexpected', '_not_in_this_entry', 'off', 'entry', 'vtable', 'class_key', 'base_unused_8',
         'dirty_flags', 'dirty_flags_all', 'carcfgint', 'vehicle_record_off', 'vehicle_record_len', 'slot29_raw_hex', '_version'}


def port_layers(zones):
    """Vinyl zones of the decoders (top, left, right, front, back) as the port's `vinyl::Livery`."""
    out = []
    for z in ('top', 'left', 'right', 'front', 'back'):
        out.append([{'shape': l['shape'], 'scale': l['scale'], 'pos': l['pos'], 'rot': l['rot'], 'skew': l['skew'], 'color': l['color'][:3] + [l['color'][3]] if len(l['color']) == 4 else l['color'], 'blend': l['blend']}
                    for l in zones.get(z, []) if not l.get('hidden')])
    return {'zones': out}


def mc4_block(d, small, opt, veh, car):
    """What the original save holds beyond the port's own fields, by the original's names."""
    dmg = {g['slot']: g for g in opt['garage_damage']}
    names = {s['slot']: s for s in opt['garage']['slots']}
    garage = []
    for c in veh['slots']:
        if c['empty']:
            continue
        v = strip(c, NOTES | {'vinyl', 'vinyl_spare_layers_224_287', 'vinyl_zone_group', 'PreDmg', 'stages_raw', 'brand_raw'})
        v['vehicle'] = c['TypeName'].lower()
        v['damage'] = {k: dmg.get(c['slot'], {}).get(k) for k in ('damage', 'zones')}
        v['name_hash'] = names.get(c['slot'], {}).get('name_hash')
        # (layers the port's livery leaves out: byte 19 set)
        skipped = [l for z in c['visual']['vinyl'].values() for l in z if l.get('m_bIsVectorShape')]
        if skipped:
            v['vinyl_layers_flagged'] = skipped
        garage.append(v)
    st = small['Stats']
    recs = [r for r in opt['vehicle_records'] if r['in_use']]
    # (ColorData: the 40 colour slots every colour strip of the garage shows - the executable's table at first, a slot
    # edited in the colour picker 0xFFRRGGBB - big-endian words; docs/UI.md "ColorData: the 40 shared slots")
    cd = dec_small.entries(d)[1].get('ColorData')
    color_data = list(struct.unpack_from('>%dI' % min(40, cd[1] // 4), d, cd[2])) if cd else []
    return {
        'container_version': struct.unpack('>I', d[:4])[0],
        'options': strip(opt['options'], NOTES),
        'options_unknown': opt.get('_unknown', []),
        'enable_gps': small['enableGps']['enabled'],
        'color_data': color_data,
        'garage_current_slot': opt['garage']['current_slot'],
        'garage': garage,
        'vehicle_records': strip(recs, NOTES),
        'ability_levels': opt['ability_levels'],
        'career': strip({k: v for k, v in car.items() if k not in ('money', 'reputation')}, NOTES),
        'editor': {'races': strip([s for s in small['EditorData']['slots'] if s['in_use']], NOTES),
                   'received': strip([s for s in small['EditorData']['received'] if s['in_use']], NOTES),
                   'received_cursor': small['EditorData']['received_cursor']},
        'vinyl_packages': [dict(slot=s['slot'], used=s['used'], livery=port_layers(s['zones'])) for s in small['UserVinylData']['slots']],
        'race_favorites_removed': small['RaceFavorites'].get('removed', []),
        'rubberband_step_in_use': small['RubberbandMgr']['step_in_use'],
        'achievements': sorted(r['id'] for r in st['records'] if r['id'][0] == 'a' and r['value']),
        'tutorials_shown': sorted(r['id'] for r in st['records'] if r['id'].startswith('tT') and r['id'] != 'tTTA' and r['value']),
        'stats_summary': strip(st['summary'], NOTES),
    }


def main():
    src, inp, outp = sys.argv[1:4]
    d = open(src, 'rb').read()
    save = json.load(open(inp, encoding='utf-8'))
    small, opt, veh, car = dec_small.decode(d), dec_options.decode(d), dec_vehicle.decode(d), dec_career.decode(d)
    career = save.setdefault('career', {})
    st = career.setdefault('stats', {})
    ost = {}
    for ident, sub, v, is_float in stats(small):
        key = '%s/%s' % (ident, VEHICLES[sub] if ident[0] == 'v' and sub < len(VEHICLES) else sub)
        # (a float record's f32 exactly, as the shortest decimal that gives it back - savedecode util::f32_shortest:
        # the port adds to it in f32, mc4::stat_type)
        ost[key] = f32_shortest(v) if is_float else v
        if not is_float and sub == 0:
            st[ident] = v
    career['ostats'] = ost
    cars, perf, dims, plates, paint, wheels, mats, paints, parts = garage(d)
    # (the vehicle in use: the garage's current slot, LocalOptions + 1344 - not always slot 0)
    cur_slot = opt['garage']['current_slot']
    cur_car = cars[cur_slot] if 0 <= cur_slot < len(cars) else (cars[0] if cars else '')
    slots = {c['TypeName'].lower(): c for c in veh['slots'] if not c['empty']}
    if cars:
        save['owned'] = cars
        save['car'] = cur_car
        paint = paints.get(cur_car, paint)
        save['perf_brand'] = {k: c['performance']['brand_raw'] for k, c in slots.items()}
        save.setdefault('perf_parts', {}).update({k: v for k, v in perf.items() if all(x <= 2 for x in v)})
        save.setdefault('dims', {}).update({k: v for k, v in dims.items() if 12 <= v[4] <= 28 and 12 <= v[5] <= 28})
        save.setdefault('plates', {}).update(plates)
        save['rim_ids'] = {k: v[0] for k, v in wheels.items()}
        xp = save.setdefault('xparts', {})
        for k, v in wheels.items():
            # (0x845 is the tires, seen changing in the running game; 0x846 was not seen to be
            # the brakes - its values exceed that list - and is left out)
            xp.setdefault(k, {}).update({'tires': v[1]})
            # (0x846 IS the brake model: an index into the `brakes` list of
            # tune/vehicle/wheel_models.list; the BRAKES menu offers rows 1..4, the other
            # values are the cars' own stock models, which the port draws without a value)
            if 1 <= v[2] <= 4:
                xp[k]['brakes'] = v[2]
            else:
                xp[k].pop('brakes', None)
        if mats:
            save['rim_mats'] = mats
        # (the port's `cfg` of the vehicle in use, `cfgs` of the others; rims come by `rim_ids`)
        cfgs = {}
        for k, v in parts.items():
            c = {'paint': 0, 'rim': 0}
            c.update({slot: v[slot] for slot in CFG_SLOTS})
            if '_part_paint' in v:
                c['part_paint'] = v.pop('_part_paint')
            cfgs[k] = c
            xp.setdefault(k, {}).update({slot: v[slot] for slot in v if slot not in CFG_SLOTS})
        save['cfgs'] = cfgs
        save['finishes'] = finishes
        if liveries:
            save.setdefault('liveries', {}).update(liveries)
            save.setdefault('vinyls', {}).update({k: 'custom' for k in liveries})
        # (the vehicle in use: its own slots)
        cur = save.get('car', '').lower()
        if cur in finishes:
            save['slot_hsv'] = finishes[cur][0]
            save['rim_mats'] = finishes[cur][1]
        cur = dict(cfgs[cur_car])
        cur['rim'] = save.get('cfg', {}).get('rim', 0)
        save['cfg'] = cur
        # (hydraulics and air bags fitted, the window tint's row and colour, the neons and
        # their colour, the second paint colour, the special ability, the plate background,
        # the brake model outside the menu's rows, the stored damage: each vehicle's own, in
        # its record of the mc4 block - the port takes them from there when a vehicle comes
        # into use (crates/game/src/mc4.rs, `mc4_take_car`) and writes them back. The port's
        # older fields for the vehicle in use (`hydraulics`, `tint_level`, `tint_color`,
        # `neon_on`, `neon_color`, `int_neon`, `paint_b`) are not written here any more.)
        # (parts owned, from the per-model purchase records: every category, under the name of
        # its section of the vehicle's part file - the port's own tags, "<section>:<index>";
        # rims are "rim:<wheel_models.list index>", performance "performance:<part * 3 + stage>",
        # the paint shop and the vinyl editor "paint:0" / "vinyl:0". Part 0 is the stock part,
        # which needs no tag - but tire 0 and rim 0 are rows of their lists. The port makes the
        # same tags from the block when it loads it: crates/game/src/mc4.rs, `owned_tags`)
        po = save.setdefault('parts_owned', {})
        for r in opt['vehicle_records']:
            if r['in_use'] and r['vehicle']:
                tags = ['%s:%d' % ('rim' if c == 'rims' else c, i) for c, v in sorted(r['owned_parts'].items())
                        for i in v if i or c in ('rims', 'tires', 'performance', 'paint', 'vinyl')]
                if tags:
                    have = po.setdefault(r['vehicle'].lower(), [])
                    have.extend(t for t in tags if t not in have)
        import colorsys
        # (every vehicle's paint: mixed colour, swatch, type, fade - the port's `paints`)
        save['paints'] = {k: [[round(x, 4) for x in colorsys.rgb_to_hsv(*(c ** 2.2 for c in rgb))], 0, kind, fade if fade <= 4 else 0] for k, (rgb, kind, fade) in paints.items()}
        if paint:
            import colorsys
            (r, g, b), kind, fade = paint
            # (the port's paint is a linear colour; the record's is the screen's: the save's red
            # GTO, 0.686 0.149 0.075, comes out salmon unless it is taken through the 2.2 curve)
            r, g, b = (c ** 2.2 for c in (r, g, b))
            save['paint_hsv'] = [round(x, 4) for x in colorsys.rgb_to_hsv(r, g, b)]
            save['paint_type'] = kind
            save['paint_fade'] = fade if fade <= 4 else 0
    # The missions (the Career entry, save routine sub_826BF7B0): state 0 INACTIVE (not in the
    # port's save), 1 CHECKING_PREREQUISITES (Armed), 2 READY_TO_START, 3 CHECKING_OBJECTIVES,
    # 4 READY_TO_COMPLETE (Active), 5 COMPLETE; the tallies of the objectives in their order
    # (a statistic's condition counts from its value when the mission opened).
    old = career.get('missions', {})
    new = {}
    for m in car['missions']:
        stage = {1: 'Armed', 2: 'Active', 3: 'Active', 4: 'Active', 5: 'Complete'}.get(m['state_id'])
        if stage is None:
            continue
        e = old.get(m['name'], {'progress': [], 'won': [], 'pts': {}})
        e['stage'] = stage
        # (a mission under way: its tallies. A complete one keeps what the port had - its
        # conditions are met, and conditions of other kinds have no count to carry)
        if stage != 'Complete':
            prog = list(e.get('progress', []))
            for k, o in enumerate(m['objectives']):
                c = o['conditions'][0] if o['conditions'] else {}
                prog += [0] * (k + 1 - len(prog))
                if 'count' in c or 'value' in c:
                    prog[k] = max(0, int(c['count'] if 'count' in c else c['value'] - c.get('start_value', 0)))
            e['progress'] = prog
        new[m['name']] = e
    career['missions'] = new
    career['order'] = list(car['active_order'])
    # (the reputation is the Progression system's amount, the money the Career entry's own
    # word at 0x2D9 - the statistic tPUR mirrors it)
    save['rep'] = car['reputation']
    save['money'] = car['money']

    # The options (LocalOptions 0..1339): volumes 0..1 -> the port's 0..10; the camera shake
    # is one half of the "cinematics" value (1 and 3 have it); steering sensitivity -1..1 in
    # 21 steps -> the port's 0..10; the genre: the original's 7 (all) is the port's 0, its
    # 0..6 the port's 1..7 (8, the custom playlist, has no number in the port: all).
    o = opt['options']
    po = save.setdefault('opts', {})
    po.update({'master': round(o['volume_effects'] * 10), 'music': round(o['volume_music'] * 10), 'voice': round(o['volume_voice'] * 10),
               'vehicle': round(o['volume_player_vehicle'] * 10), 'cutscene': round(o['volume_cutscene'] * 10),
               'subtitles': bool(o['subtitles']), 'shake': bool(o['camera_shake']), 'sens': max(0, min(10, round((o['steering_sensitivity'] + 1) * 5)))})
    g = o['music_genre']
    save['music_genre'] = g + 1 if 0 <= g <= 6 else 0
    # The collectables: one flag byte each at LocalOptions + 0x3DF84, by the index the
    # Collectables script hands to LocalOptions_SaveCollectables (the order of its position
    # array, which is the port's barrel order).
    save['collected'] = list(opt['collectables_found'])
    # The race editor's own races (EditorData: slots in use, their checkpoints in order).
    save['user_races'] = [[c['pos'] for c in s['checkpoints'][:s['count']]] for s in small['EditorData']['slots'] if s['in_use']]
    # The favourite races ("Game/Race/<District>/<Type>/<name>"; the port's ids are the same
    # paths without "game/race/", compared without case).
    save['race_favs'] = [re.sub(r'(?i)^game/race/', '', f).lower() for f in small['RaceFavorites']['favorites']]
    # The rubber band manager: level and points (rule sub_82742F28 = the port's Adaptive::result).
    rb = small['RubberbandMgr']
    save['adaptive'] = {'level': rb['level'], 'points': rb['points']}
    save['mc4'] = mc4_block(d, small, opt, veh, car)
    json.dump(save, open(outp, 'w', encoding='utf-8'))
    # (the whole save by name, for reference)
    json.dump({'small': small, 'options': opt, 'vehicle': veh, 'career': car}, open(outp + '.mc4full.json', 'w', encoding='utf-8'))
    print('%d statistics, %d vehicles (%s ...), %d missions, money %s' % (len(ost), len(cars), ', '.join(cars[:3]), len(career.get('missions', {})), save.get('money')))


if __name__ == '__main__':
    main()
