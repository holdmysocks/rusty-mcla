// Controls > Keyboard (the game's own bindings, read from and written into the save by src/controls.rs over
// mcla_config::keybinds) and Settings > Display and game > Monitor (display.monitor in settings.toml, src/display.rs).
// The page keeps no binding rules of its own: every key goes to Rust, which says saved / refused / conflict.
// Test hook: RUSTY_MCLA_CONTROLS_TEST=1 (with RUSTY_MCLA_SELFTEST=<report>) drives both like a user and reports.
(function () {
  const $ = (sel, root) => (root || document).querySelector(sel);
  const $$ = (sel, root) => Array.from((root || document).querySelectorAll(sel));
  const t = (k, a) => I18N.t(k, a);

  const st = { view: null, editing: null, pending: null, display: null, ctx: null, statusTimer: null, busy: false };

  // key names as the game's page writes them ("LEFT SHIFT"), shown in the design's case ("Left Shift")
  const keyText = (k) => (k ? k.name.toLowerCase().replace(/\b[a-z]/g, (c) => c.toUpperCase()) : '-');
  const actText = (id) => t('act.' + id);

  function el(tag, cls, attrs) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (attrs) for (const k in attrs) e.setAttribute(k, attrs[k]);
    return e;
  }

  // ---------- small saved / failed indication beside RESET TO DEFAULTS ----------
  function status(kind, text) {
    const s = $('#kbd-status');
    s.textContent = text || '';
    s.dataset.kind = kind || '';
    clearTimeout(st.statusTimer);
    if (kind === 'ok') st.statusTimer = setTimeout(() => { s.textContent = ''; s.dataset.kind = ''; }, 2500);
  }

  // ---------- the table ----------
  function render() {
    const v = st.view;
    const host = $('#kbd-rows');
    host.textContent = '';
    if (!v) return;
    const readOnly = v.state !== 'ok';
    v.rows.forEach((r) => {
      const row = el('div', 'tbl-row');
      const name = el('span', 'act');
      name.textContent = actText(r.id);
      row.append(name);
      [0, 1].forEach((slot) => {
        const b = el('button', 'keycap kbtn', { 'data-act': r.id, 'data-slot': String(slot) });
        const k = r.keys[slot];
        const editing = st.editing && st.editing.act === r.id && st.editing.slot === slot;
        if (editing) { b.classList.add('editing'); b.textContent = t('ctl.press_key'); }
        else if (slot === 0 && r.must) { b.classList.add('must'); b.textContent = t('ctl.must'); }
        else { b.textContent = keyText(k); if (!k) b.classList.add('empty'); else b.classList.add('set'); }
        if (readOnly) b.disabled = true;
        b.addEventListener('click', (e) => { e.stopPropagation(); startEdit(r.id, slot); });
        row.append(b);
      });
      host.append(row);
    });
    const note = $('#kbd-note');
    if (v.state === 'no_save') { note.textContent = t('ctl.no_save'); note.dataset.kind = 'warn'; }
    else if (v.state === 'error') { note.textContent = t('ctl.save_error', [v.error ? v.error.detail : '']); note.dataset.kind = 'bad'; }
    else if (v.warning) { note.textContent = t('ctl.save_odd'); note.dataset.kind = 'warn'; }
    else { note.textContent = t('ctl.kbd_intro'); note.dataset.kind = ''; }
    $('#kbd-reset').disabled = readOnly || v.defaults;
    const fixed = $('#kbd-fixed');
    fixed.textContent = '';
    v.fixed.filter((f) => ['back', 'accept', 'window_mode'].includes(f[0])).forEach((f) => {
      const r = el('div', 'fixed-row');
      const a = el('span'); a.textContent = t('ctl.fixed.' + f[0]);
      const k = el('span', 'keycap fixed'); k.textContent = f[1];
      r.append(a, k);
      fixed.append(r);
    });
  }

  async function load() {
    st.view = await Bridge.kbdGet();
    render();
    return st.view;
  }

  // ---------- rebinding ----------
  function startEdit(act, slot) {
    if (!st.view || st.view.state !== 'ok' || st.busy) return;
    st.editing = { act, slot };
    status('', '');
    render();
  }

  function stopEdit() {
    st.editing = null;
    render();
  }

  function after(r) {
    st.view = r.view;
    if (r.outcome === 'saved') status('ok', r.backup ? t('ctl.saved_bak') : t('ctl.saved'));
    else if (r.outcome === 'unchanged') status('ok', t('ctl.saved'));
    else if (r.outcome === 'refused') status('warn', r.reason === 'alt_enter' ? t('ctl.ref_alt_enter') : r.reason === 'reserved' ? t('ctl.ref_reserved', [keyText(r.key)]) : t('ctl.ref_unknown'));
    else if (r.outcome === 'failed') status('bad', t('ctl.failed', [r.error ? r.error.detail : '']));
    render();
    return r;
  }

  async function call(p) {
    st.busy = true;
    try { return after(await p); }
    catch (e) { status('bad', t('ctl.failed', [String(e)])); return { outcome: 'failed', error: { detail: String(e) } }; }
    finally { st.busy = false; }
  }

  // a key while a box waits for one: Esc cancels, Backspace / Delete clears, anything else goes to Rust
  async function keyFor(code, alt) {
    const ed = st.editing;
    if (!ed) return null;
    if (code === 'Escape') { stopEdit(); status('', ''); return { outcome: 'cancelled' }; }
    st.editing = null;
    if (code === 'Backspace' || code === 'Delete') return call(Bridge.kbdClear(ed.act, ed.slot));
    const r = await call(Bridge.kbdSet(ed.act, ed.slot, code, alt, false));
    if (r.outcome === 'conflict') ask(ed, code, alt, r);
    return r;
  }

  function onKey(e) {
    if (st.pending) { if (e.code === 'Escape') { e.preventDefault(); closeAsk(false); } return; }
    if (!st.editing) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.repeat) return;
    // (a modifier pressed on its own is a key too: Left Shift, Left Ctrl, Left Alt)
    keyFor(e.code, e.altKey && e.code !== 'AltLeft' && e.code !== 'AltRight');
  }

  // ---------- "already used" question ----------
  function ask(ed, code, alt, r) {
    st.pending = { ed, code, alt };
    $('#kbd-ask-text').textContent = t('ctl.ask_text', [keyText(r.key), actText(r.conflict[0]), actText(ed.act)]);
    $('#kbd-dialog').hidden = false;
    status('', '');
  }

  async function closeAsk(replace) {
    const p = st.pending;
    st.pending = null;
    $('#kbd-dialog').hidden = true;
    if (!p) return null;
    if (!replace) { status('', t('ctl.kept')); render(); return { outcome: 'cancelled' }; }
    return call(Bridge.kbdSet(p.ed.act, p.ed.slot, p.code, p.alt, true));
  }

  async function reset() {
    if (!st.view || st.view.state !== 'ok') return null;
    st.editing = null;
    return call(Bridge.kbdReset());
  }

  // ---------- Settings > Display and game: monitor and the other live rows ----------
  function renderDisplay() {
    const d = st.display;
    const sel = $('#monitor-select');
    sel.textContent = '';
    if (!d) return;
    d.monitors.forEach((m) => {
      const o = el('option');
      o.value = String(m.index);
      o.textContent = t(m.primary ? 'set.mon_primary' : 'set.mon_opt', [m.index + 1, m.name || '?', m.width, m.height]);
      sel.append(o);
    });
    if (!d.monitors.some((m) => m.index === d.monitor)) {
      const o = el('option');
      o.value = String(d.monitor);
      o.textContent = t('set.mon_gone', [d.monitor + 1]);
      sel.append(o);
    }
    sel.value = String(d.monitor);
    sel.disabled = d.file === 'broken' || !d.implemented;
    const s = $('#monitor-status');
    s.textContent = d.file === 'broken' ? t('set.mon_broken') : '';
    s.dataset.kind = d.file === 'broken' ? 'bad' : '';
    // (intro, window mode, resolution, frame-rate cap, vsync, fps counter, anti-aliasing, bloom, depth of field, motion blur: live where the key table says the game reads the key)
    const fill = (sel, values, label, cur) => {
      sel.textContent = '';
      values.forEach((v) => { const o = el('option'); o.value = String(v); o.textContent = label(v); sel.append(o); });
      sel.value = String(cur);
    };
    fill($('#res-select'), d.resolutions || [], (v) => t('set.mon_res', v.split('x')), d.resolution);
    fill($('#fps-select'), d.fps_caps || [], (v) => (v === 0 ? t('set.o_off') : t('set.mon_fps', [v])), d.fps_cap);
    $('#vsync-switch').setAttribute('aria-checked', d.vsync ? 'true' : 'false');
    // (a cap below the monitor's refresh rate that does not divide it, with vsync: frames stay up one refresh or two in turn -
    // a steady judder; the game logs the same warning - GitHub #11)
    const mon = (d.monitors || []).find((m) => m.index === d.monitor) || (d.monitors || [])[0];
    const hz = mon ? mon.hz || 0 : 0;
    const warn = $('#fps-warn');
    const judder = d.vsync && d.fps_cap > 0 && hz > 0 && d.fps_cap < hz && hz % d.fps_cap !== 0;
    warn.hidden = !judder;
    warn.textContent = judder ? t('set.fps_judder', [d.fps_cap, hz]) : '';
    const cur = { 'game.intro': d.intro, 'display.window_mode': d.window_mode, 'display.fps_monitor': d.fps_monitor, 'display.aa': d.aa, 'display.bloom': String(d.bloom), 'display.dof': String(d.dof), 'display.motion_blur': String(d.motion_blur), 'display.texture_filtering': d.texture_filtering, 'display.render_scale': String(d.render_scale), 'game.shadow_resolution': d.shadow_resolution, 'game.draw_distance': d.draw_distance, 'game.detail_level': d.detail_level };
    $$('.opt[data-dkey]').forEach((row) => {
      const key = row.dataset.dkey;
      const live = d.file !== 'broken' && (d.live_keys || []).includes(key);
      if (live) row.removeAttribute('aria-disabled'); else row.setAttribute('aria-disabled', 'true');
      row.querySelectorAll('select, button').forEach((c) => { c.disabled = !live; });
      row.querySelectorAll('.seg [data-v]').forEach((b) => {
        const on = b.dataset.v === cur[key];
        b.classList.toggle('on', on);
        b.setAttribute('aria-checked', on ? 'true' : 'false');
        b.tabIndex = live ? 0 : -1;
      });
    });
  }

  async function setKey(key, value) {
    const row = $(`.opt[data-dkey="${key}"]`);
    if (!row || row.getAttribute('aria-disabled') === 'true') return 'greyed';
    const s = row.querySelector('.mon-status');
    try {
      st.display = await Bridge.displaySet(key, value);
      renderDisplay();
      s.textContent = t('set.mon_saved');
      s.dataset.kind = 'ok';
      return 'saved';
    } catch (e) {
      renderDisplay();
      s.textContent = e === 'broken' ? t('set.mon_broken') : t('set.mon_failed');
      s.dataset.kind = 'bad';
      return 'failed:' + e;
    }
  }

  async function loadDisplay() {
    st.display = await Bridge.displayGet();
    renderDisplay();
    return st.display;
  }

  async function setMonitor(index) {
    const s = $('#monitor-status');
    try {
      st.display = await Bridge.displaySetMonitor(index);
      renderDisplay();
      s.textContent = t('set.mon_saved');
      s.dataset.kind = 'ok';
      return 'saved';
    } catch (e) {
      s.textContent = e === 'broken' ? t('set.mon_broken') : t('set.mon_failed');
      s.dataset.kind = 'bad';
      return 'failed:' + e;
    }
  }

  function init(ctx) {
    st.ctx = ctx;
    document.addEventListener('keydown', onKey, true);
    document.addEventListener('click', (e) => { if (st.editing && !e.target.closest('.kbtn')) { stopEdit(); status('', ''); } });
    $('#kbd-reset').addEventListener('click', reset);
    $('#kbd-ask-yes').addEventListener('click', () => closeAsk(true));
    $('#kbd-ask-no').addEventListener('click', () => closeAsk(false));
    $('#monitor-select').addEventListener('change', (e) => setMonitor(Number(e.target.value)));
    $('#res-select').addEventListener('change', (e) => setKey('display.resolution', e.target.value));
    $('#fps-select').addEventListener('change', (e) => setKey('display.fps_cap', Number(e.target.value)));
    $('#vsync-switch').addEventListener('click', () => setKey('display.vsync', !(st.display && st.display.vsync)));
    $$('.opt[data-dkey] .seg [data-v]').forEach((b) => {
      const pick = () => setKey(b.closest('.opt').dataset.dkey, b.dataset.v === 'true' ? true : b.dataset.v === 'false' ? false : b.closest('.opt').dataset.dkey === 'display.render_scale' ? Number(b.dataset.v) : b.dataset.v);
      b.addEventListener('click', pick);
      b.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); pick(); } });
    });
    load().catch((e) => Bridge.log('error', 'controls: ' + e));
    loadDisplay().catch((e) => Bridge.log('error', 'display: ' + e));
  }

  // ---------- test hook: the page driven like a user ----------
  async function test() {
    const wait = (ms) => new Promise((r) => setTimeout(r, ms));
    const report = { ok: true, errors: [], steps: [] };
    const bad = (m) => { report.ok = false; report.errors.push(m); };
    const box = (act, slot) => $('.kbtn[data-act="' + act + '"][data-slot="' + slot + '"]');
    const shown = (act, slot) => (box(act, slot) || {}).textContent;
    const press = (code, alt) => document.dispatchEvent(new KeyboardEvent('keydown', { code, altKey: !!alt, bubbles: true, cancelable: true }));
    const settle = async () => { for (let i = 0; i < 100 && (st.busy || (!st.editing && false)); i++) await wait(10); await wait(30); };
    const step = async (name, fn) => {
      const t0 = performance.now();
      let r;
      try { r = await fn(); } catch (e) { bad(name + ' threw ' + e); }
      await settle();
      report.steps.push({ name, ms: Math.round(performance.now() - t0), status: $('#kbd-status').textContent, kind: $('#kbd-status').dataset.kind, dialog: !$('#kbd-dialog').hidden, result: r && r.outcome });
      return r;
    };
    try {
      st.ctx.showTab('controls');
      await wait(50);
      await load();
      report.initial = { state: st.view.state, rows: $$('#kbd-rows .tbl-row').length, present: st.view.present, defaults: st.view.defaults, fixed: $$('#kbd-fixed .fixed-row').map((r) => r.innerText.replace(/\s+/g, ' ')), first: [actText('STEER_LEFT'), shown('STEER_LEFT', 0), shown('STEER_LEFT', 1)], note: $('#kbd-note').textContent };
      if (report.initial.rows !== 32) bad('rows ' + report.initial.rows);
      if (st.view.state !== 'ok') {
        // no save: read-only, nothing may be written
        box('NITRO', 0).click();
        press('ControlLeft');
        await settle();
        report.no_save = { editing: !!st.editing, disabled: box('NITRO', 0).disabled, reset_disabled: $('#kbd-reset').disabled };
        if (st.editing) bad('a box took a key without a save');
      } else {
        await step('click NITRO key 1, press Left Ctrl', async () => { box('NITRO', 0).click(); report.editing_text = shown('NITRO', 0); press('ControlLeft'); await wait(150); });
        report.after_nitro = [shown('NITRO', 0), shown('NITRO', 1)];
        await step('click HORN key 1, press E (used by Special ability)', async () => { box('HORN', 0).click(); press('KeyE'); await wait(150); });
        report.ask_text = $('#kbd-ask-text').textContent;
        await step('REPLACE', async () => { $('#kbd-ask-yes').click(); await wait(150); });
        report.after_horn = [shown('HORN', 0), shown('IN_ZONE', 0)];
        await step('click LOOKBACK key 1, press W, CANCEL', async () => { box('LOOKBACK', 0).click(); press('KeyW'); await wait(150); $('#kbd-ask-no').click(); await wait(100); });
        report.after_cancel = [shown('LOOKBACK', 0), shown('GAS', 0)];
        await step('click GPS_MAP key 2, press Backspace', async () => { box('GPS_MAP', 1).click(); press('Backspace'); await wait(150); });
        report.after_clear = [shown('GPS_MAP', 0), shown('GPS_MAP', 1)];
        await step('click HORN key 2, press Esc', async () => { box('HORN', 1).click(); press('Escape'); await wait(50); });
        report.after_esc = [shown('HORN', 1), !!st.editing];
        await step('click HORN key 2, press F5 (reserved)', async () => { box('HORN', 1).click(); press('F5'); await wait(150); });
        report.ref_f5 = $('#kbd-status').textContent;
        await step('click HORN key 2, press Alt+Enter', async () => { box('HORN', 1).click(); press('Enter', true); await wait(150); });
        report.ref_alt_enter = $('#kbd-status').textContent;
        await step('click HORN key 2, press Enter', async () => { box('HORN', 1).click(); press('Enter'); await wait(150); });
        report.ref_enter = $('#kbd-status').textContent;
        await step('click HORN key 2, press the Windows key', async () => { box('HORN', 1).click(); press('MetaLeft'); await wait(150); });
        report.ref_meta = $('#kbd-status').textContent;
        await step('click GAS key 1, press Backspace twice (must be mapped)', async () => { box('GAS', 0).click(); press('Backspace'); await wait(150); box('GAS', 0).click(); press('Backspace'); await wait(150); });
        report.must_reset_enabled = !$('#kbd-reset').disabled;
        report.must = [shown('GAS', 0), shown('GAS', 1), box('GAS', 0).className];
        report.before_reset = (await Bridge.kbdGet()).rows.filter((r) => ['NITRO', 'HORN', 'IN_ZONE', 'GPS_MAP', 'GAS'].includes(r.id)).map((r) => r.id + '=' + r.keys.map((k) => (k ? k.name : '-')).join('/'));
        await step('RESET TO DEFAULTS', async () => { $('#kbd-reset').click(); await wait(150); });
        report.after_reset = { defaults: st.view.defaults, present: st.view.present, horn: shown('HORN', 0), reset_disabled: $('#kbd-reset').disabled };
        await step('click NITRO key 1, press Right Shift (left in the save for the check)', async () => { box('NITRO', 0).click(); press('ShiftRight'); await wait(150); });
        report.final = (await Bridge.kbdGet()).rows.filter((r) => r.id === 'NITRO').map((r) => r.keys.map((k) => (k ? k.code : null)));
      }
      // the monitor row
      st.ctx.showTab('settings');
      st.ctx.showSub('display');
      await wait(50);
      await loadDisplay();
      const opt = $('#monitor-select').closest('.opt');
      report.display = { monitors: st.display.monitors, value: $('#monitor-select').value, options: $$('#monitor-select option').map((o) => o.textContent), enabled: !$('#monitor-select').disabled, greyed_rows: $$('.opt[aria-disabled="true"]').length, monitor_row_greyed: opt.getAttribute('aria-disabled') === 'true', file: st.display.file, implemented: st.display.implemented };
      const pick = st.display.monitors.length > 1 ? 1 : 0;
      const sel = $('#monitor-select');
      sel.value = String(pick);
      sel.dispatchEvent(new Event('change'));
      await wait(200);
      report.display.after = { value: st.display.monitor, status: $('#monitor-status').textContent };
      report.display.out_of_range = await Bridge.displaySetMonitor(16).then(() => 'accepted', (e) => 'refused:' + e);
      // the other live rows, each set and put back
      const before = { intro: st.display.intro, window_mode: st.display.window_mode, resolution: st.display.resolution, vsync: st.display.vsync, fps_cap: st.display.fps_cap, fps_monitor: st.display.fps_monitor };
      report.display.rows = { live_keys: st.display.live_keys, greyed_rows: $$('.opt[aria-disabled="true"]').length, before, steps: [] };
      const stepRow = async (what, fn, read) => { await fn(); await wait(150); report.display.rows.steps.push([what, read(), $(`.opt[data-dkey] .mon-status[data-kind]`) ? 'status' : '']); };
      await stepRow('intro fast', () => $('.opt[data-dkey="game.intro"] [data-v="fast"]').click(), () => st.display.intro);
      await stepRow('window borderless', () => $('.opt[data-dkey="display.window_mode"] [data-v="borderless"]').click(), () => st.display.window_mode);
      const rs = $('#res-select'); rs.value = rs.options[0].value;
      await stepRow('resolution first', () => rs.dispatchEvent(new Event('change')), () => st.display.resolution);
      const fs = $('#fps-select'); fs.value = '60';
      await stepRow('fps 60', () => fs.dispatchEvent(new Event('change')), () => st.display.fps_cap);
      await stepRow('vsync toggled', () => $('#vsync-switch').click(), () => st.display.vsync);
      await stepRow('fps counter frame_time', () => $('.opt[data-dkey="display.fps_monitor"] [data-v="frame_time"]').click(), () => st.display.fps_monitor + (($('.opt[data-dkey="display.fps_monitor"] [data-v="frame_time"]').classList.contains('on')) ? '/on' : '/not-shown'));
      report.display.rows.fps_monitor_bad = await Bridge.displaySet('display.fps_monitor', 'graph').then(() => 'accepted', (e) => 'refused:' + e);
      report.display.rows.refused = await Bridge.displaySet('display.fps_cap', 5).then(() => 'accepted', (e) => 'refused:' + e);
      report.display.rows.unknown = await Bridge.displaySet('display.upscaler', 'fsr').then(() => 'accepted', (e) => 'refused:' + e);
      await Bridge.displaySet('game.intro', before.intro);
      await Bridge.displaySet('display.window_mode', before.window_mode);
      await Bridge.displaySet('display.resolution', before.resolution);
      await Bridge.displaySet('display.fps_cap', before.fps_cap);
      await Bridge.displaySet('display.vsync', before.vsync);
      await Bridge.displaySet('display.fps_monitor', before.fps_monitor);
      await loadDisplay();
      report.display.rows.restored = { intro: st.display.intro, window_mode: st.display.window_mode, resolution: st.display.resolution, vsync: st.display.vsync, fps_cap: st.display.fps_cap, fps_monitor: st.display.fps_monitor };
    } catch (e) {
      bad('test threw: ' + (e && e.stack || e));
    }
    return report;
  }

  window.Controls = { init, load, test, keyFor, reset };
})();
