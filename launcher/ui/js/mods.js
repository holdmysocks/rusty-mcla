// The Mods tab (docs/MODS.md stage S5, docs/LAUNCHER.md "Mods tab"): the mods found in the mods folder with their state, an on / off
// switch, install by dropping a .zip on the tab or by a file dialog, removing a mod, and picking up mods copied into the folder by hand.
// All rules live in Rust (src/mods.rs over the mcla-mods crate: validation, duplicates, clashes, the update rule, settings.toml);
// this file draws the list model it returns and asks the questions. A mod is a package with contents (a car so far): nothing here assumes
// one car per mod. Every sentence of ours comes from the locales (`mods.*`, `dyn.mods.*`); the checker's own reasons (English
// diagnostics, one per problem) are shown as they are.
// Test hook: RUSTY_MCLA_MODS_TEST=<steps> (with RUSTY_MCLA_SELFTEST=<report>) drives the tab like a user and reports.
(function () {
  const $ = (sel, root) => (root || document).querySelector(sel);
  const $$ = (sel, root) => Array.from((root || document).querySelectorAll(sel));
  const t = (k, a) => I18N.t(k, a);
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text != null) e.textContent = text; return e; };

  const POLL_MS = 600;
  // listing: what Rust last returned; sel: path of the selected package; shown: stamp of the folder when the list was read;
  // seen: stamp at the previous poll (a change is read only when two polls agree, so a zip that is still being copied is not read half-written)
  const S = { app: null, listing: null, sel: null, shown: null, seen: null, timer: null, active: false, busy: false, hover: false, test: null, asked: [] };

  // ---------- helpers ----------
  const fileOf = (r) => r.file;
  const cap = (s) => s.charAt(0).toUpperCase() + s.slice(1);
  function plural(kind, n) {
    const one = 'dyn.mods.kind_' + kind + '_one', many = 'dyn.mods.kind_' + kind + '_n';
    if (kind === 'car' && n === 1) return t(one, [n]);
    return t(I18N.table()[many] ? many : 'dyn.mods.kind_other_n', [n]);
  }
  // "2008 R8 (SAMPLE)" for one car, "3 cars" for several; mixed kinds are counted per kind
  function contentsText(r) {
    const by = {};
    r.contents.forEach((c) => { (by[c.kind] = by[c.kind] || []).push(c); });
    return Object.keys(by).map((k) => (by[k].length === 1 && k === 'car' ? by[k][0].title : plural(k, by[k].length))).join(' · ');
  }
  function subText(r) {
    const left = [r.author || t('dyn.mods.noauthor'), contentsText(r)].filter(Boolean).join(' · ');
    return r.version ? t('dyn.mods.sub', [left, r.version]) : left;
  }
  const stateText = (r) => ({ loaded: r.on ? t('dyn.mods.st_on') : t('dyn.mods.st_off'), off: t('dyn.mods.st_off'), invalid: '', skipped: '' }[r.status]);
  const selected = () => (S.listing && S.listing.rows.find((r) => r.path === S.sel)) || null;

  const ask0 = (title, text, ok) => Support.ask(title, text, ok);
  async function ask(title, text, ok) {
    S.asked.push({ title, text, ok });
    if (S.test) return S.test.answer;
    return window.Support ? Support.ask(title, text, ok) : false;
  }

  // ---------- drawing ----------
  function render() {
    const L = S.listing;
    if (!L) return;
    const rows = L.rows;
    if (!rows.some((r) => r.path === S.sel)) S.sel = rows.length ? rows[0].path : null;
    $('#mods-count').textContent = rows.length ? t('dyn.mods.count', [L.summary.total, L.summary.loaded]) : t('dyn.mods.count_none');
    $('#mods-data-note').hidden = L.data_found;
    $('#mods-dir').textContent = L.mods_dir;
    $('#mods-dir').title = L.mods_dir;
    $('#mods-empty').hidden = rows.length > 0;
    $('#mods-list').hidden = rows.length === 0;
    const list = $('#mods-list');
    list.textContent = '';
    rows.forEach((r) => list.append(rowEl(r)));
    renderDetail();
  }

  function rowEl(r) {
    const row = el('div', 'mod-row' + (r.path === S.sel ? ' sel' : ''));
    row.dataset.path = r.path;
    row.tabIndex = 0;
    const thumb = el('span', 'thumb');
    thumb.innerHTML = '<svg class="ic"><use href="#i-car"/></svg>';
    const body = el('div', 'mod-body');
    const n = el('div', 'mod-n');
    n.append(el('span', 'mod-name', r.name));
    if (r.status === 'invalid') n.append(el('span', 'chip warn', t('dyn.mods.badge_invalid')));
    if (r.status === 'skipped') n.append(el('span', 'chip warn', t('dyn.mods.badge_skipped')));
    body.append(n, el('div', 'mod-s', subText(r)));
    const st = el('span', 'note xs mod-st', stateText(r));
    st.style.textAlign = 'right';
    row.append(thumb, body, st);
    if (r.can_switch) {
      const sw = el('button', 'switch');
      sw.setAttribute('role', 'switch');
      sw.setAttribute('aria-checked', r.on ? 'true' : 'false');
      sw.setAttribute('aria-label', r.name);
      sw.append(document.createElement('i'));
      sw.addEventListener('click', (e) => { e.stopPropagation(); setOn(r, !r.on); });
      row.append(sw);
    } else {
      row.append(el('span'));
    }
    row.addEventListener('click', () => { S.sel = r.path; render(); });
    row.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); S.sel = r.path; render(); } });
    return row;
  }

  function fact(kind, text, subs) {
    const d = el('div');
    const ic = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    ic.setAttribute('class', 'ic' + (kind === 'ok' ? '' : ' ' + kind));
    ic.innerHTML = '<use href="#' + (kind === 'ok' ? 'i-check' : kind === 'warn' ? 'i-warn' : kind === 'bad' ? 'i-close' : 'i-minus') + '"/>';
    const sp = el('span');
    sp.append(document.createTextNode(text));
    (subs || []).forEach((s) => sp.append(el('div', 'note s mod-sub', s)));
    d.append(ic, sp);
    return d;
  }

  function renderDetail() {
    const r = selected();
    const none = !r;
    $('#mods-detail').classList.toggle('empty', none);
    $('#mods-d-name').textContent = r ? r.name : '';
    $('#mods-d-sub').textContent = r ? subText(r) : '';
    $('#mods-d-desc').textContent = r ? r.description || t('dyn.mods.nodesc') : '';
    const facts = $('#mods-d-facts');
    facts.textContent = '';
    $('#mods-d-actions').hidden = none;
    if (none) return;
    // the state, in a sentence
    if (r.status === 'loaded') facts.append(fact('ok', t('dyn.mods.f_loaded')));
    else if (r.status === 'off') facts.append(fact('idle', t('dyn.mods.f_off')));
    else if (r.status === 'invalid') facts.append(fact('bad', t('dyn.mods.f_invalid'), r.reasons));
    else if (r.skipped_why === 'clash') {
      // plainly: which name clashed and which mod holds it
      r.clashes.forEach((c) => facts.append(fact('warn', t('dyn.mods.f_clash', [c.title, c.name, c.held_by_name, c.held_by_version, c.held_by_id]))));
      if (!r.on) facts.append(fact('idle', t('dyn.mods.f_off')));
    } else if (r.skipped_why === 'duplicate') {
      facts.append(fact('warn', r.used_instead ? t('dyn.mods.f_dup', [r.used_instead.version, r.used_instead.file]) : t('dyn.mods.f_dup_none')));
    }
    // what the package brings: each item with its own problems
    r.contents.forEach((c) => {
      const kind = t('dyn.mods.kind_label_' + c.kind) !== 'dyn.mods.kind_label_' + c.kind ? t('dyn.mods.kind_label_' + c.kind) : cap(c.kind);
      if (c.problems.length) facts.append(fact('bad', t('dyn.mods.f_item_bad', [kind, c.title, c.name]), c.problems));
      else facts.append(fact(r.status === 'loaded' ? 'ok' : 'idle', t('dyn.mods.f_item', [kind, c.title, c.name]), c.notes));
    });
    r.warnings.forEach((w) => facts.append(fact('warn', w)));
    facts.append(fact('idle', t(r.is_zip ? 'dyn.mods.f_file_zip' : 'dyn.mods.f_file_dir', [r.file])));
    const sw = $('#mods-d-switch');
    sw.hidden = !r.can_switch;
    sw.textContent = r.on ? t('dyn.mods.turn_off') : t('dyn.mods.turn_on');
    sw.classList.toggle('primary', !r.on);
  }

  // ---------- reading the folder ----------
  async function load() {
    try {
      S.listing = await Bridge.modsView();
    } catch (e) {
      Bridge.log('warn', 'mods_view failed: ' + JSON.stringify(e));
      return;
    }
    S.shown = S.listing.stamp;
    S.seen = S.listing.stamp;
    render();
  }

  // ---------- actions ----------
  async function setOn(r, on) {
    try {
      S.listing = await Bridge.modsSet(r.id, on);
      S.shown = S.listing.stamp;
      render();
      S.app.toast(t(on ? 'dyn.mods.switched_on' : 'dyn.mods.switched_off', [r.name]));
    } catch (e) {
      Bridge.log('warn', 'mods_set failed: ' + JSON.stringify(e));
      S.app.toast(t('dyn.mods.switch_failed', [String(e)]), true);
    }
  }

  async function removeSel() {
    const r = selected();
    if (!r) return;
    if (!(await ask(t('dyn.mods.rm_title'), t('dyn.mods.rm_text', [r.name, r.file]), t('dyn.mods.rm_ok')))) return;
    try {
      S.listing = await Bridge.modsRemove(r.path);
      S.shown = S.listing.stamp;
      render();
      S.app.toast(t('dyn.mods.removed', [r.name]));
    } catch (e) {
      Bridge.log('warn', 'mods_remove failed: ' + JSON.stringify(e));
      S.app.toast(t('dyn.mods.remove_failed', [String(e)]), true);
    }
  }

  const baseName = (p) => p.split(/[\\/]/).filter(Boolean).pop() || p;

  // Install every path (a dropped file, or the files of the dialog). An older version over a newer one asks first.
  async function installPaths(paths) {
    if (S.busy || !paths.length) return [];
    S.busy = true;
    const done = [], failed = [];
    try {
      for (const p of paths) {
        const file = baseName(p);
        if (!/\.zip$/i.test(p)) { failed.push([file, t('dyn.mods.not_zip', [file])]); continue; }
        try {
          const before = S.listing;
          let r = await Bridge.modsInstall(p, false);
          if (r.outcome === 'needs_confirmation') {
            if (!(await ask(t('dyn.mods.older_title'), t('dyn.mods.older_text', [r.id, r.installed, r.incoming]), t('dyn.mods.older_ok')))) { done.push({ declined: file }); continue; }
            r = await Bridge.modsInstall(p, true);
          }
          r.had = !!(before && before.rows.some((x) => x.id === r.id)); // a copy of this mod id was installed already: replaced in place
          done.push(r);
          S.listing = r.listing;
          S.shown = r.listing.stamp;
          const row = r.listing.rows.find((x) => x.id === r.id && x.version === r.version) || r.listing.rows.find((x) => x.id === r.id);
          if (row) S.sel = row.path;
        } catch (e) {
          Bridge.log('warn', 'mods_install failed: ' + JSON.stringify(e));
          failed.push([file, String(e)]);
        }
      }
    } finally {
      S.busy = false;
    }
    render();
    report(done, failed);
    return done;
  }

  function report(done, failed) {
    // (the same mod twice in one go counts once: the last one stands)
    const last = {};
    done.forEach((d) => { if (d.outcome === 'installed') last[d.id] = d; });
    const ok = Object.values(last);
    const declined = done.filter((d) => d.declined);
    if (failed.length) {
      S.app.toast(failed.length === 1 ? t('dyn.mods.failed', failed[0]) : t('dyn.mods.failed_n', [failed.length]), true);
    } else if (ok.length === 1) {
      const r = ok[0];
      const key = { loaded: r.replaced || r.had ? 'dyn.mods.installed_replaced' : 'dyn.mods.installed_loaded', off: 'dyn.mods.installed_off', invalid: 'dyn.mods.installed_invalid', skipped: 'dyn.mods.installed_skipped' }[r.status];
      S.app.toast(t(key, [r.name, r.version]), r.status === 'invalid' || r.status === 'skipped');
    } else if (ok.length > 1) {
      S.app.toast(t('dyn.mods.installed_n', [ok.length]), ok.some((r) => r.status === 'invalid' || r.status === 'skipped'));
    } else if (declined.length) {
      S.app.toast(t('dyn.mods.declined', [declined.map((d) => d.declined).join(', ')]));
    }
  }

  async function pickAndInstall() {
    let paths = [];
    try { paths = await Bridge.modsPick(); } catch (e) { Bridge.log('warn', 'file dialog failed: ' + e); }
    if (paths && paths.length) await installPaths(paths);
  }

  // ---------- picking up what happens outside the page ----------
  // While the tab is open: files dropped on the window (and whether a drag is over it), and the mods folder's change stamp.
  async function tick() {
    if (!S.active || document.hidden) return;
    let p = null;
    try { p = await Bridge.modsPoll(); } catch (e) { return; }
    if (!p) return;
    setHover(!!p.hover);
    if (p.dropped.length) await installPaths(p.dropped);
    else if (!S.busy && S.shown != null && p.stamp !== S.shown) {
      // the folder changed (a mod copied in or out by hand): read it again once two polls agree
      if (p.stamp === S.seen) { await load(); S.app.toast(t('dyn.mods.found')); }
    }
    S.seen = p.stamp;
  }

  function setHover(on) {
    S.hover = on;
    $('#mods-drop').hidden = !on;
  }

  function schedule() {
    clearTimeout(S.timer);
    S.timer = setTimeout(async () => { await tick(); if (S.active) schedule(); }, POLL_MS);
  }

  function show() {
    S.active = true;
    load().then(schedule);
  }
  function hide() {
    S.active = false;
    clearTimeout(S.timer);
    setHover(false);
  }

  // ---------- start ----------
  function init(app) {
    S.app = app;
    $('#mods-d-switch').addEventListener('click', () => { const r = selected(); if (r) setOn(r, !r.on); });
    $('#mods-d-remove').addEventListener('click', removeSel);
    $('#mods-install').addEventListener('click', pickAndInstall);
    $('#mods-open').addEventListener('click', () => Bridge.openFolder('mods').catch(() => app.toast(t('toast.folder_failed'), true)));
    // coming back to the launcher from the file manager: look at once
    window.addEventListener('focus', () => { if (S.active) tick(); });
    if (window.I18N && I18N.onChange) I18N.onChange(() => render());
  }

  // ---------- test hook (RUSTY_MCLA_MODS_TEST) ----------
  // Steps (comma separated): snapshot, answer:yes|no, drop:<path>[|<path>...] (as if dropped on the window), select:<n>, switch:<n>, switchbtn,
  // remove:<n>, open, shot:<name> (logs a marker; the run script screenshots the window), wait:<ms>, settle (wait until the poll has picked up changes made outside), reload, hover:on|off.
  async function test(steps) {
    S.test = { answer: true };
    show();
    await sleep(500);
    const rep = { steps: [] };
    const shot = (tag, extra) => Object.assign({
      tag,
      count: $('#mods-count').textContent,
      note: $('#mods-data-note').hidden ? null : $('#mods-data-note').textContent,
      dir: $('#mods-dir').textContent,
      empty: !$('#mods-empty').hidden,
      rows: $$('#mods-list .mod-row').map((r) => ({
        name: $('.mod-name', r).textContent,
        chips: $$('.chip', r).map((c) => c.textContent),
        sub: $('.mod-s', r).textContent,
        state: $('.mod-st', r).textContent,
        sw: $('.switch', r) ? $('.switch', r).getAttribute('aria-checked') : null,
        sel: r.classList.contains('sel'),
      })),
      detail: { name: $('#mods-d-name').textContent, sub: $('#mods-d-sub').textContent, desc: $('#mods-d-desc').textContent, facts: $$('#mods-d-facts > div').map((d) => d.innerText.replace(/\s*\n\s*/g, ' / ')), switchLabel: $('#mods-d-switch').hidden ? null : $('#mods-d-switch').textContent, actionsHidden: $('#mods-d-actions').hidden },
      toast: $('#toast').hidden ? null : $('#toast-text').textContent,
      dropVisible: !$('#mods-drop').hidden,
      asked: S.asked.splice(0),
    }, extra || {});
    for (const step of steps.split(',').map((s) => s.trim()).filter(Boolean)) {
      const [name, arg] = step.split(/:(.*)/s);
      let extra = {};
      try {
        if (name === 'answer') S.test.answer = arg !== 'no';
        else if (name === 'drop') { await installPaths(arg.split('|')); }
        else if (name === 'select') { const r = $$('#mods-list .mod-row')[Number(arg)]; extra = { clicked: !!r }; if (r) r.click(); }
        else if (name === 'switch') { const r = $$('#mods-list .mod-row')[Number(arg)]; const sw = r && $('.switch', r); extra = { clicked: !!sw }; if (sw) { sw.click(); await sleep(400); } }
        else if (name === 'switchbtn') { $('#mods-d-switch').click(); await sleep(400); }
        else if (name === 'remove') { const r = $$('#mods-list .mod-row')[Number(arg)]; if (r) r.click(); $('#mods-d-remove').click(); await sleep(500); }
        else if (name === 'open') { extra = { opened: await Bridge.openFolder('mods').then(() => true, (e) => String(e)) }; }
        else if (name === 'hover') setHover(arg === 'on');
        else if (name === 'dialog') {
          // the real confirmation dialog (screenshot), closed again by its Cancel button: arg = older | remove
          const q = arg === 'remove' ? ask0(t('dyn.mods.rm_title'), t('dyn.mods.rm_text', ['Test mod 01 (350Z copy)', 'test_mod_01.zip']), t('dyn.mods.rm_ok')) : ask0(t('dyn.mods.older_title'), t('dyn.mods.older_text', ['test.mod01', '1.1.0', '0.9.0']), t('dyn.mods.older_ok'));
          await sleep(300); Bridge.log('info', 'MODS-SHOT ' + arg + '_dialog'); await sleep(1500); $('#sup-dlg-no').click(); extra = { answered: await q };
        }
        else if (name === 'shot') { Bridge.log('info', 'MODS-SHOT ' + arg); await sleep(2500); } // the run script screenshots the window when it sees this line
        else if (name === 'reload') await load();
        else if (name === 'settle') await sleep(Number(arg) || 2600);
        else if (name === 'wait') await sleep(Number(arg) || 500);
      } catch (e) {
        extra = { thrown: String((e && e.stack) || JSON.stringify(e)) };
      }
      rep.steps.push(shot(step, extra));
    }
    hide();
    S.test = null;
    return rep;
  }

  window.Mods = { init, show, hide, render, test, state: S };
})();
